//! Stable browser-facing playback types and theme document rendering.

use serde::Serialize;
use thiserror::Error;
use tokio::sync::{broadcast, watch};

use crate::{
    mpris::PlaybackHandle,
    playback::{PlaybackEvent, PlaybackEventKind, PlaybackSnapshot},
    theme::LoadedTheme,
};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ApiSnapshot {
    pub connected: bool,
    pub player: Option<ApiPlayer>,
    pub status: ApiPlaybackStatus,
    pub metadata: ApiMetadata,
    pub position_ms: u64,
    pub duration_ms: Option<u64>,
    pub shuffle: Option<bool>,
    pub repeat: ApiRepeatMode,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ApiPlayer {
    pub identity: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ApiMetadata {
    pub title: Option<String>,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub track_id: Option<String>,
    /// A same-origin cached URL supplied by the artwork layer. Raw MPRIS URIs
    /// are intentionally never exposed to theme JavaScript.
    pub artwork_url: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ApiPlaybackStatus {
    Playing,
    Paused,
    Stopped,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ApiRepeatMode {
    None,
    Track,
    Playlist,
    Unknown,
}

impl ApiSnapshot {
    /// Convert provider state without leaking the source artwork URI. The
    /// caller supplies a URL only after resolving it through the local cache.
    pub fn from_playback(snapshot: &PlaybackSnapshot, artwork_url: Option<String>) -> Self {
        use crate::playback::{PlaybackStatus, RepeatMode};

        Self {
            connected: snapshot.is_connected(),
            player: snapshot.player.as_ref().map(|player| ApiPlayer {
                identity: player.identity.clone(),
            }),
            status: match snapshot.status {
                PlaybackStatus::Playing => ApiPlaybackStatus::Playing,
                PlaybackStatus::Paused => ApiPlaybackStatus::Paused,
                PlaybackStatus::Stopped => ApiPlaybackStatus::Stopped,
                PlaybackStatus::Unknown => ApiPlaybackStatus::Unknown,
            },
            metadata: ApiMetadata {
                title: snapshot.metadata.title.clone(),
                artists: snapshot.metadata.artists.clone(),
                album: snapshot.metadata.album.clone(),
                track_id: snapshot.metadata.track_id.clone(),
                artwork_url,
            },
            position_ms: snapshot.position.as_millis().min(u64::MAX as u128) as u64,
            duration_ms: snapshot
                .metadata
                .duration
                .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64),
            shuffle: snapshot.shuffle,
            repeat: match snapshot.repeat {
                RepeatMode::None => ApiRepeatMode::None,
                RepeatMode::Track => ApiRepeatMode::Track,
                RepeatMode::Playlist => ApiRepeatMode::Playlist,
                RepeatMode::Unknown => ApiRepeatMode::Unknown,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ApiEvent {
    pub kind: ApiEventKind,
    pub snapshot: ApiSnapshot,
    #[serde(skip)]
    pub(crate) source_artwork_uri: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ApiEventKind {
    Snapshot,
    PlayerConnected,
    PlayerDisconnected,
    PlayerChanged,
    TrackChanged,
    PlaybackStarted,
    PlaybackPaused,
    PlaybackStopped,
    PositionUpdated,
    MetadataUpdated,
}

impl From<PlaybackEventKind> for ApiEventKind {
    fn from(kind: PlaybackEventKind) -> Self {
        match kind {
            PlaybackEventKind::PlayerConnected => Self::PlayerConnected,
            PlaybackEventKind::PlayerDisconnected => Self::PlayerDisconnected,
            PlaybackEventKind::PlayerChanged => Self::PlayerChanged,
            PlaybackEventKind::TrackChanged => Self::TrackChanged,
            PlaybackEventKind::PlaybackStarted => Self::PlaybackStarted,
            PlaybackEventKind::PlaybackPaused => Self::PlaybackPaused,
            PlaybackEventKind::PlaybackStopped => Self::PlaybackStopped,
            PlaybackEventKind::PositionUpdated => Self::PositionUpdated,
            PlaybackEventKind::MetadataUpdated => Self::MetadataUpdated,
        }
    }
}

impl ApiEvent {
    pub fn new(kind: ApiEventKind, snapshot: ApiSnapshot) -> Self {
        Self {
            kind,
            snapshot,
            source_artwork_uri: None,
        }
    }
}

/// Bridges the MPRIS provider into the browser API. Latest state remains
/// available independently of the bounded event stream, so clients can
/// resynchronize after a lag notification.
pub struct PlaybackApi {
    state: watch::Receiver<PlaybackSnapshot>,
    events: broadcast::Sender<ApiEvent>,
    task: tokio::task::JoinHandle<()>,
}

impl PlaybackApi {
    /// Create an API that reports a disconnected snapshot when MPRIS cannot be
    /// started. The HTTP service can still run and playback monitoring can be
    /// brought back independently on a later application restart.
    pub fn disconnected() -> Self {
        let (state_tx, state_rx) = watch::channel(PlaybackSnapshot::default());
        let (events_tx, events_rx) = broadcast::channel(1);
        drop(state_tx);
        drop(events_tx);
        Self::from_receivers(state_rx, events_rx)
    }

    pub fn start(provider: &PlaybackHandle) -> Self {
        Self::from_receivers(provider.state(), provider.subscribe())
    }

    pub(crate) fn from_receivers(
        state: watch::Receiver<PlaybackSnapshot>,
        provider_events: broadcast::Receiver<PlaybackEvent>,
    ) -> Self {
        let (events, _) = broadcast::channel(64);
        let task = tokio::spawn(forward_playback_events(
            state.clone(),
            provider_events,
            events.clone(),
        ));
        Self {
            state,
            events,
            task,
        }
    }

    pub fn snapshot(&self) -> ApiSnapshot {
        ApiSnapshot::from_playback(&self.state.borrow(), None)
    }

    pub(crate) fn playback_snapshot(&self) -> PlaybackSnapshot {
        self.state.borrow().clone()
    }

    pub(crate) fn current_event(&self) -> ApiEvent {
        let playback = self.state.borrow();
        ApiEvent {
            kind: ApiEventKind::Snapshot,
            snapshot: ApiSnapshot::from_playback(&playback, None),
            source_artwork_uri: playback.metadata.artwork_uri.clone(),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ApiEvent> {
        self.events.subscribe()
    }
}

impl Drop for PlaybackApi {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn forward_playback_events(
    state: watch::Receiver<PlaybackSnapshot>,
    mut provider_events: broadcast::Receiver<PlaybackEvent>,
    events: broadcast::Sender<ApiEvent>,
) {
    let mut state_updates = state.clone();
    loop {
        tokio::select! {
            changed = state_updates.changed() => {
                if changed.is_err() { break; }
            }
            event = provider_events.recv() => match event {
                Ok(event) => forward_event(&events, event),
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "Playback API bridge lagged; publishing a fresh snapshot");
                    let snapshot = state.borrow();
                    let _ = events.send(ApiEvent {
                        kind: ApiEventKind::Snapshot,
                        snapshot: ApiSnapshot::from_playback(&snapshot, None),
                        source_artwork_uri: snapshot.metadata.artwork_uri.clone(),
                    });
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }
}

fn forward_event(events: &broadcast::Sender<ApiEvent>, event: PlaybackEvent) {
    let _ = events.send(ApiEvent {
        kind: event.kind.into(),
        snapshot: ApiSnapshot::from_playback(&event.snapshot, None),
        source_artwork_uri: event.snapshot.metadata.artwork_uri,
    });
}

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("script nonce must be at least 22 URL-safe Base64 characters")]
    InvalidNonce,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedPage {
    pub html: String,
    /// The HTTP layer must send this as the Content-Security-Policy header.
    pub content_security_policy: String,
}

/// Render a validated theme as a complete HTML document. The caller must use a
/// fresh cryptographically random URL-safe Base64 nonce for each response and
/// return the included CSP as an HTTP response header.
pub fn render_theme(theme: &LoadedTheme, nonce: &str) -> Result<RenderedPage, RenderError> {
    if nonce.len() < 22
        || !nonce
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(RenderError::InvalidNonce);
    }

    let mut html = String::with_capacity(
        theme.document.html.template.len()
            + theme.document.css.stylesheet.len()
            + theme.document.javascript.source.len()
            + 4096,
    );
    html.push_str("<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">");
    html.push_str("<style nonce=\"");
    html.push_str(nonce);
    html.push_str("\">");
    html.push_str(&escape_raw_text(&theme.document.css.stylesheet, "style"));
    html.push_str("</style><script nonce=\"");
    html.push_str(nonce);
    html.push_str("\">");
    html.push_str(API_BOOTSTRAP);
    html.push_str("</script></head><body>");
    html.push_str(&theme.document.html.template);
    html.push_str("<script nonce=\"");
    html.push_str(nonce);
    html.push_str("\">");
    html.push_str(&escape_raw_text(
        &theme.document.javascript.source,
        "script",
    ));
    html.push_str("</script></body></html>");

    Ok(RenderedPage {
        html,
        content_security_policy: format!(
            "default-src 'none'; img-src 'self' data:; media-src 'self'; \
             style-src 'self' 'nonce-{nonce}'; script-src 'self' 'nonce-{nonce}'; \
             connect-src 'self'; font-src 'self' data:; base-uri 'none'; \
             form-action 'none'; frame-ancestors 'none'; object-src 'none'"
        ),
    })
}

fn escape_raw_text(source: &str, tag: &str) -> String {
    let needle = format!("</{tag}");
    let lower = source.to_ascii_lowercase();
    let mut result = String::with_capacity(source.len());
    let mut copied_through = 0;
    let mut search_from = 0;
    while let Some(relative) = lower[search_from..].find(&needle) {
        let start = search_from + relative;
        result.push_str(&source[copied_through..start]);
        result.push_str(&format!("<\\/{tag}"));
        copied_through = start + needle.len();
        search_from = copied_through;
    }
    result.push_str(&source[copied_through..]);
    result
}

const API_BOOTSTRAP: &str = r#"(function(){
  let current = null;
  const subscribers = new Set();
  const listeners = new Map();
  function publish(message) {
    if (!message) return;
    if (message.kind === "themeChanged") { location.reload(); return; }
    if (!message.snapshot) return;
    current = message.snapshot;
    for (const callback of subscribers) callback(current, message.kind || "snapshot");
    const callbacks = listeners.get(message.kind);
    if (callbacks) for (const callback of callbacks) callback(current, message);
  }
  window.NowPlaying = Object.freeze({
    getState: () => current,
    subscribe(callback) {
      if (typeof callback !== "function") throw new TypeError("callback must be a function");
      subscribers.add(callback);
      if (current !== null) callback(current, "snapshot");
      return () => subscribers.delete(callback);
    },
    on(kind, callback) {
      if (typeof callback !== "function") throw new TypeError("callback must be a function");
      if (!listeners.has(kind)) listeners.set(kind, new Set());
      listeners.get(kind).add(callback);
      return () => listeners.get(kind).delete(callback);
    }
  });
  let retry = 500;
  async function loadState() {
    try {
      const response = await fetch("/api/state", { cache: "no-store" });
      if (!response.ok) throw new Error("state endpoint returned " + response.status);
      publish({ kind: "snapshot", snapshot: await response.json() });
      retry = 500;
    } catch (_) {
      setTimeout(loadState, retry);
      retry = Math.min(retry * 2, 10000);
    }
  }
  loadState();
  let socketRetry = 500;
  function connectEvents() {
    const scheme = location.protocol === "https:" ? "wss:" : "ws:";
    const socket = new WebSocket(scheme + "//" + location.host + "/api/ws");
    socket.onopen = () => { socketRetry = 500; };
    socket.onmessage = event => {
      try { publish(JSON.parse(event.data)); } catch (_) {}
    };
    socket.onclose = () => {
      setTimeout(connectEvents, socketRetry);
      socketRetry = Math.min(socketRetry * 2, 10000);
    };
    socket.onerror = () => socket.close();
  }
  connectEvents();
})();"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        playback::{PlaybackStatus, PlayerInfo, RepeatMode, TrackMetadata},
        theme::ThemeLoader,
    };

    const THEME: &str = r#"
[theme]
id = "browser-theme"
name = "Browser Theme"
version = "1.0.0"
format_version = 1
[theme.compatibility]
minimum_runtime = "0.1.0"
[html]
template = "<main id='now-playing'></main>"
[css]
stylesheet = "body { color: white }</style><script>bad()"
[javascript]
source = "window.ready = true;</ScRiPt><script>bad()"
"#;

    fn loaded_theme() -> LoadedTheme {
        ThemeLoader::new("0.1.0")
            .unwrap()
            .parse(THEME.as_bytes())
            .unwrap()
    }

    #[test]
    fn playback_api_has_stable_units_and_never_infers_artwork_urls() {
        let snapshot = PlaybackSnapshot {
            player: Some(PlayerInfo {
                bus_name: "org.mpris.MediaPlayer2.player".into(),
                identity: "Player".into(),
            }),
            status: PlaybackStatus::Playing,
            metadata: TrackMetadata {
                title: Some("Track".into()),
                artists: vec!["Artist".into()],
                duration: Some(std::time::Duration::from_millis(1234)),
                artwork_uri: Some("file:///home/user/private.jpg".into()),
                ..TrackMetadata::default()
            },
            position: std::time::Duration::from_millis(456),
            repeat: RepeatMode::Track,
            ..PlaybackSnapshot::default()
        };
        let api = ApiSnapshot::from_playback(&snapshot, None);
        let json = serde_json::to_string(&api).unwrap();
        assert!(json.contains("\"positionMs\":456"));
        assert!(json.contains("\"durationMs\":1234"));
        assert!(json.contains("\"artworkUrl\":null"));
        assert!(!json.contains("file://"));
        assert!(json.contains("\"status\":\"playing\""));
    }

    #[test]
    fn renderer_applies_nonce_csp_and_escapes_raw_text_end_tags() {
        let page = render_theme(&loaded_theme(), "a-very-long-random-nonce_123456").unwrap();
        assert!(page.html.contains("window.NowPlaying"));
        assert!(
            page.html
                .contains("body { color: white }<\\/style><script>bad()")
        );
        assert!(
            page.html
                .contains("window.ready = true;<\\/script><script>bad()")
        );
        assert!(
            page.content_security_policy
                .contains("script-src 'self' 'nonce-a-very-long-random-nonce_123456'")
        );
        assert!(render_theme(&loaded_theme(), "unsafe\" nonce").is_err());
    }

    #[test]
    fn bundled_reference_theme_renders_with_its_real_embedded_content() {
        let theme = ThemeLoader::current()
            .parse(include_bytes!("../themes/catppuccin-mocha-standard.toml"))
            .unwrap();
        let page = render_theme(&theme, "a-very-long-random-nonce_123456").unwrap();
        assert!(page.html.contains("id=\"artwork\""));
        assert!(page.html.contains("window.NowPlaying.subscribe(update)"));
        assert!(page.html.contains("prefers-reduced-motion"));
    }

    #[tokio::test]
    async fn playback_api_forwards_events_and_keeps_latest_state_available() {
        use crate::playback::PlaybackEvent;

        let initial = PlaybackSnapshot::default();
        let (state_tx, state_rx) = watch::channel(initial.clone());
        let (provider_events, provider_rx) = broadcast::channel(8);
        let api = PlaybackApi::from_receivers(state_rx, provider_rx);
        let mut updates = api.subscribe();
        assert_eq!(api.snapshot().status, ApiPlaybackStatus::Unknown);

        let changed = PlaybackSnapshot {
            status: PlaybackStatus::Playing,
            metadata: TrackMetadata {
                title: Some("Live Track".into()),
                ..TrackMetadata::default()
            },
            ..initial
        };
        provider_events
            .send(PlaybackEvent {
                kind: PlaybackEventKind::TrackChanged,
                snapshot: changed.clone(),
            })
            .unwrap();
        state_tx.send_replace(changed);

        let event = tokio::time::timeout(std::time::Duration::from_secs(1), updates.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event.kind, ApiEventKind::TrackChanged);
        assert_eq!(event.snapshot.metadata.title.as_deref(), Some("Live Track"));
        assert_eq!(api.snapshot().status, ApiPlaybackStatus::Playing);
    }

    #[tokio::test]
    async fn playback_api_recovers_from_upstream_event_lag_with_latest_snapshot() {
        let (state_tx, state_rx) = watch::channel(PlaybackSnapshot::default());
        let (provider_events, provider_rx) = broadcast::channel(1);
        let api = PlaybackApi::from_receivers(state_rx, provider_rx);
        let mut updates = api.subscribe();

        for title in ["Older", "Latest"] {
            let snapshot = PlaybackSnapshot {
                metadata: TrackMetadata {
                    title: Some(title.into()),
                    ..TrackMetadata::default()
                },
                ..PlaybackSnapshot::default()
            };
            state_tx.send_replace(snapshot.clone());
            provider_events
                .send(crate::playback::PlaybackEvent {
                    kind: PlaybackEventKind::TrackChanged,
                    snapshot,
                })
                .unwrap();
        }

        let recovery = tokio::time::timeout(std::time::Duration::from_secs(1), updates.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(recovery.kind, ApiEventKind::Snapshot);
        assert_eq!(recovery.snapshot.metadata.title.as_deref(), Some("Latest"));
    }
}
