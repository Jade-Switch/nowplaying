//! Loopback-only HTTP and WebSocket endpoints for OBS Browser Source.

use std::{net::SocketAddr, sync::Arc};

use axum::{
    Json, Router,
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::{SinkExt, StreamExt, stream::SplitSink};
use thiserror::Error;
use tokio::{io::AsyncReadExt, net::TcpListener, sync::oneshot, task::JoinHandle};
use tracing::{debug, warn};

use crate::{
    artwork::ArtworkCache,
    runtime::{ApiEvent, ApiSnapshot, PlaybackApi, render_theme},
    theme::ThemeManager,
};

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("the local server address must be a loopback address")]
    NonLoopback,
    #[error("could not bind the local HTTP server")]
    Bind(#[source] std::io::Error),
    #[error("the local HTTP server stopped with an error")]
    Serve(#[source] std::io::Error),
    #[error("could not read secure random bytes for a page nonce")]
    Random(#[source] std::io::Error),
    #[error("the local HTTP server task failed")]
    Task(#[from] tokio::task::JoinError),
}

#[derive(Clone)]
struct ServerState {
    playback: Arc<PlaybackApi>,
    themes: ThemeManager,
    artwork: Arc<ArtworkCache>,
    address: SocketAddr,
}

pub struct LocalServer {
    address: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<std::io::Result<()>>>,
}

impl LocalServer {
    pub async fn start(
        address: SocketAddr,
        playback: Arc<PlaybackApi>,
        themes: ThemeManager,
        artwork: Arc<ArtworkCache>,
    ) -> Result<Self, ServerError> {
        if !address.ip().is_loopback() {
            return Err(ServerError::NonLoopback);
        }
        let listener = TcpListener::bind(address)
            .await
            .map_err(ServerError::Bind)?;
        let address = listener.local_addr().map_err(ServerError::Bind)?;
        let app = router(ServerState {
            playback,
            themes,
            artwork,
            address,
        });
        let (shutdown, shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                })
                .await
        });
        Ok(Self {
            address,
            shutdown: Some(shutdown),
            task: Some(task),
        })
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    pub async fn shutdown(mut self) -> Result<(), ServerError> {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            task.await?.map_err(ServerError::Serve)?;
        }
        Ok(())
    }
}

impl Drop for LocalServer {
    fn drop(&mut self) {
        // Dropping the sender resolves the graceful-shutdown future.
        self.shutdown.take();
    }
}

fn router(state: ServerState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/preview/{theme_id}", get(preview_theme))
        .route("/preview-family/{family_id}", get(preview_family))
        .route(
            "/preview/{theme_id}/assets/{asset_id}",
            get(preview_theme_asset),
        )
        .route("/api/state", get(playback_state))
        .route("/api/ws", get(playback_socket))
        .route("/theme/assets/{id}", get(theme_asset))
        .route("/artwork/{id}", get(artwork_asset))
        .with_state(state)
}

async fn index(State(state): State<ServerState>) -> Response {
    let Some(theme) = state.themes.active().await else {
        return (StatusCode::SERVICE_UNAVAILABLE, "No active theme selected").into_response();
    };
    if !state.themes.is_active(theme.id()).await {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let nonce = match create_nonce().await {
        Ok(nonce) => nonce,
        Err(error) => {
            warn!(%error, "Could not create a CSP nonce");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let page = match render_theme(&theme, &nonce) {
        Ok(page) => page,
        Err(error) => {
            warn!(%error, theme_id = theme.id(), "Could not render active theme");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    html_response(page)
}

async fn preview_theme(Path(theme_id): Path<String>, State(state): State<ServerState>) -> Response {
    if !state.themes.is_active(&theme_id).await {
        return StatusCode::FORBIDDEN.into_response();
    }
    let catalog = state.themes.catalog().await;
    let Some(theme) = catalog.themes.get(&theme_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let nonce = match create_nonce().await {
        Ok(nonce) => nonce,
        Err(error) => {
            warn!(%error, theme_id, "Could not create a preview CSP nonce");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let mut page = match render_theme(theme, &nonce) {
        Ok(page) => page,
        Err(error) => {
            warn!(%error, theme_id, "Could not render theme preview");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    page.html = page
        .html
        .replace("/theme/assets/", &format!("/preview/{theme_id}/assets/"));
    html_response(page)
}

async fn preview_family(
    Path(family_id): Path<String>,
    State(state): State<ServerState>,
) -> Response {
    let active_ids = state.themes.active_ids().await;
    let catalog = state.themes.catalog().await;
    let Some(mut group) = catalog
        .selector_groups()
        .into_iter()
        .find(|group| group.family_id.as_deref() == Some(family_id.as_str()))
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    group.theme_ids.retain(|id| active_ids.contains(id));
    if group.theme_ids.is_empty() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let active = state
        .themes
        .active()
        .await
        .map(|theme| theme.id().to_owned());
    let remembered = state.themes.selected_family_variants().await;
    let Some(theme_id) = group.preferred_variant_id(active.as_deref(), &remembered) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(theme) = catalog.themes.get(&theme_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let nonce = match create_nonce().await {
        Ok(nonce) => nonce,
        Err(error) => {
            warn!(%error, theme_id, "Could not create a family preview CSP nonce");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let mut page = match render_theme(theme, &nonce) {
        Ok(page) => page,
        Err(error) => {
            warn!(%error, theme_id, "Could not render family theme preview");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    page.html = page
        .html
        .replace("/theme/assets/", &format!("/preview/{theme_id}/assets/"));
    html_response(page)
}

fn html_response(page: crate::runtime::RenderedPage) -> Response {
    let Ok(csp) = HeaderValue::from_str(&page.content_security_policy) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let mut response = Html(page.html).into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_SECURITY_POLICY, csp);
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

async fn playback_state(State(state): State<ServerState>) -> Response {
    let snapshot = resolve_current_snapshot(&state.playback, &state.artwork, &state.themes).await;
    let mut response = Json(snapshot).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

async fn playback_socket(
    State(state): State<ServerState>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    if !origin_allowed(&headers, state.address) {
        return StatusCode::FORBIDDEN.into_response();
    }
    upgrade.on_upgrade(move |socket| {
        handle_socket(socket, state.playback, state.artwork, state.themes)
    })
}

fn origin_allowed(headers: &HeaderMap, address: SocketAddr) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    };
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    let Ok(origin) = url::Url::parse(origin) else {
        return false;
    };
    let Some(host) = origin.host_str() else {
        return false;
    };
    let Ok(ip) = host.parse::<std::net::IpAddr>() else {
        return false;
    };
    origin.scheme() == "http"
        && origin.username().is_empty()
        && origin.password().is_none()
        && origin.path() == "/"
        && origin.query().is_none()
        && origin.fragment().is_none()
        && ip.is_loopback()
        && origin.port_or_known_default() == Some(address.port())
}

async fn handle_socket(
    socket: WebSocket,
    playback: Arc<PlaybackApi>,
    artwork: Arc<ArtworkCache>,
    themes: ThemeManager,
) {
    let mut updates = playback.subscribe();
    let mut theme_changes = themes.subscribe_changes();
    let (mut sender, mut receiver) = socket.split();
    let mut initial = playback.current_event();
    let mut artwork_source = initial.source_artwork_uri.clone();
    let mut should_load_artwork = themes
        .active()
        .await
        .is_some_and(|theme| theme.document.artwork.should_load());
    let mut artwork_url = if should_load_artwork {
        Some(resolve_artwork_url(&artwork, &initial).await)
    } else {
        None
    };
    initial.snapshot.metadata.artwork_url = artwork_url.clone();
    if !send_event(&mut sender, &initial).await {
        return;
    }

    loop {
        tokio::select! {
            changed = theme_changes.changed() => {
                if changed.is_err() { break; }
                let mut event = playback.current_event();
                should_load_artwork = themes
                    .active()
                    .await
                    .is_some_and(|theme| theme.document.artwork.should_load());
                artwork_source = event.source_artwork_uri.clone();
                artwork_url = if should_load_artwork {
                    Some(resolve_artwork_url(&artwork, &event).await)
                } else {
                    None
                };
                event.snapshot.metadata.artwork_url = artwork_url.clone();
                if sender.send(Message::Text(r#"{"kind":"themeChanged"}"#.into())).await.is_err() { break; }
            }
            update = updates.recv() => match update {
                Ok(mut event) => {
                    if should_load_artwork && event.source_artwork_uri != artwork_source {
                        artwork_url = Some(resolve_artwork_url(&artwork, &event).await);
                        artwork_source = event.source_artwork_uri.clone();
                    }
                    if !should_load_artwork {
                        artwork_url = None;
                    }
                    event.snapshot.metadata.artwork_url = artwork_url.clone();
                    if !send_event(&mut sender, &event).await { break; }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    debug!(skipped, "Browser client lagged; sending a fresh playback snapshot");
                    let mut event = playback.current_event();
                    if should_load_artwork && event.source_artwork_uri != artwork_source {
                        artwork_url = Some(resolve_artwork_url(&artwork, &event).await);
                        artwork_source = event.source_artwork_uri.clone();
                    }
                    if !should_load_artwork {
                        artwork_url = None;
                    }
                    event.snapshot.metadata.artwork_url = artwork_url.clone();
                    if !send_event(&mut sender, &event).await { break; }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            },
            message = receiver.next() => match message {
                Some(Ok(Message::Ping(payload))) => {
                    if sender.send(Message::Pong(payload)).await.is_err() { break; }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(Message::Pong(_) | Message::Text(_) | Message::Binary(_))) => {}
            }
        }
    }
}

async fn resolve_current_snapshot(
    playback: &PlaybackApi,
    artwork: &ArtworkCache,
    themes: &ThemeManager,
) -> ApiSnapshot {
    let source = playback.playback_snapshot();
    let should_load_artwork = themes
        .active()
        .await
        .is_some_and(|theme| theme.document.artwork.should_load());
    let artwork_url = if should_load_artwork {
        let cached_artwork = artwork
            .resolve_or_fallback(source.metadata.artwork_uri.as_deref())
            .await;
        Some(cached_artwork.local_url)
    } else {
        None
    };
    ApiSnapshot::from_playback(&source, artwork_url)
}

async fn resolve_artwork_url(artwork: &ArtworkCache, event: &ApiEvent) -> String {
    artwork
        .resolve_or_fallback(event.source_artwork_uri.as_deref())
        .await
        .local_url
}

async fn send_event(sender: &mut SplitSink<WebSocket, Message>, event: &ApiEvent) -> bool {
    match serde_json::to_string(event) {
        Ok(payload) => sender.send(Message::Text(payload.into())).await.is_ok(),
        Err(error) => {
            warn!(%error, "Could not encode a playback API event");
            false
        }
    }
}

async fn theme_asset(Path(id): Path<String>, State(state): State<ServerState>) -> Response {
    let Some(theme) = state.themes.active().await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !state.themes.is_active(theme.id()).await {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Ok(asset) = theme.asset(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(content_type) = HeaderValue::from_str(&asset.mime_type) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let mut response = asset.bytes.clone().into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, content_type);
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

async fn preview_theme_asset(
    Path((theme_id, asset_id)): Path<(String, String)>,
    State(state): State<ServerState>,
) -> Response {
    if !state.themes.is_active(&theme_id).await {
        return StatusCode::FORBIDDEN.into_response();
    }
    let catalog = state.themes.catalog().await;
    let Some(theme) = catalog.themes.get(&theme_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(asset) = theme.asset(&asset_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(content_type) = HeaderValue::from_str(&asset.mime_type) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let mut response = asset.bytes.clone().into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, content_type);
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

async fn artwork_asset(Path(id): Path<String>, State(state): State<ServerState>) -> Response {
    let Ok(Some(artwork)) = state.artwork.read(&id).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(content_type) = HeaderValue::from_str(&artwork.mime_type) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let mut response = artwork.bytes.into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, content_type);
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=31536000, immutable"),
    );
    response
}

async fn create_nonce() -> Result<String, ServerError> {
    let mut random = [0_u8; 24];
    let mut source = tokio::fs::File::open("/dev/urandom")
        .await
        .map_err(ServerError::Random)?;
    source
        .read_exact(&mut random)
        .await
        .map_err(ServerError::Random)?;
    Ok(URL_SAFE_NO_PAD.encode(random))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use axum::{body::to_bytes, http::Request};
    use tokio::sync::{broadcast, watch};
    use tokio_tungstenite::{connect_async, tungstenite::Message as ClientMessage};
    use tower::ServiceExt;

    use crate::{
        artwork::ArtworkCacheConfig,
        playback::{
            PlaybackEvent, PlaybackEventKind, PlaybackSnapshot, PlaybackStatus, PlayerInfo,
        },
        theme::{ThemeLoader, ThemeManager},
    };

    static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "nps-server-test-{}-{}",
                std::process::id(),
                NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const THEME: &str = r#"
[theme]
id = "server-theme"
name = "Server Theme"
version = "1.0.0"
format_version = 1
[theme.compatibility]
minimum_runtime = "0.1.0"
[artwork]
enabled = true
width = 144
height = 144
[html]
template = "<main>Server test</main>"
[assets.logo]
mime_type = "image/gif"
data = "R0lGODlhAQABAAD/ACwAAAAAAQABAAACADs="
"#;

    async fn test_router(root: &std::path::Path, theme_source: &[u8]) -> Router {
        let themes_path = root.join("themes");
        let settings_path = root.join("settings.toml");
        let source_path = root.join("import.toml");
        tokio::fs::write(&source_path, theme_source).await.unwrap();
        let themes = ThemeManager::new(
            &themes_path,
            &settings_path,
            ThemeLoader::new("0.1.0").unwrap(),
        )
        .await
        .unwrap();
        let imported = themes.import_file(source_path).await.unwrap();
        let theme_id = imported.id().to_owned();
        themes.select(&theme_id).await.unwrap();
        themes.restore_active_ids().await.unwrap();

        let artwork = Arc::new(
            ArtworkCache::new(root.join("artwork"), ArtworkCacheConfig::default())
                .await
                .unwrap(),
        );
        let (_, playback_state) = watch::channel(crate::playback::PlaybackSnapshot::default());
        let (_, playback_events) = broadcast::channel(8);
        let playback = Arc::new(PlaybackApi::from_receivers(playback_state, playback_events));
        router(ServerState {
            playback,
            themes,
            artwork,
            address: "127.0.0.1:0".parse().unwrap(),
        })
    }

    #[test]
    fn server_refuses_non_loopback_bind_addresses() {
        assert!(
            "0.0.0.0:8765"
                .parse::<SocketAddr>()
                .unwrap()
                .ip()
                .is_unspecified()
        );
        assert!(ServerError::NonLoopback.to_string().contains("loopback"));
    }

    #[tokio::test]
    async fn generated_nonces_are_valid_and_fresh() {
        let first = create_nonce().await.unwrap();
        let second = create_nonce().await.unwrap();
        assert_eq!(first.len(), 32);
        assert_ne!(first, second);
    }

    #[test]
    fn websocket_origins_must_be_literal_loopback_addresses_on_the_listener_port() {
        let address = "127.0.0.1:8765".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(header::ORIGIN, "http://127.0.0.1:8765".parse().unwrap());
        assert!(origin_allowed(&headers, address));
        headers.insert(header::ORIGIN, "http://localhost:8765".parse().unwrap());
        assert!(!origin_allowed(&headers, address));
        headers.insert(header::ORIGIN, "http://127.0.0.1:9999".parse().unwrap());
        assert!(!origin_allowed(&headers, address));
        headers.remove(header::ORIGIN);
        assert!(origin_allowed(&headers, address));
    }

    #[tokio::test]
    async fn serves_the_active_theme_state_embedded_assets_and_fallback_art() {
        let root = TestDirectory::new();
        let app = test_router(&root.0, THEME.as_bytes()).await;

        let page = app
            .clone()
            .oneshot(Request::get("/").body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(page.status(), StatusCode::OK);
        assert!(page.headers().contains_key(header::CONTENT_SECURITY_POLICY));
        let page_body = to_bytes(page.into_body(), 1024 * 1024).await.unwrap();
        assert!(
            std::str::from_utf8(&page_body)
                .unwrap()
                .contains("<main>Server test</main>")
        );

        let state = app
            .clone()
            .oneshot(
                Request::get("/api/state")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(state.status(), StatusCode::OK);
        let state_body = to_bytes(state.into_body(), 64 * 1024).await.unwrap();
        let state_json: serde_json::Value = serde_json::from_slice(&state_body).unwrap();
        assert_eq!(state_json["metadata"]["artworkUrl"], "/artwork/fallback");

        let asset = app
            .clone()
            .oneshot(
                Request::get("/theme/assets/logo")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(asset.status(), StatusCode::OK);
        assert_eq!(asset.headers()[header::CONTENT_TYPE], "image/gif");
        let asset_body = to_bytes(asset.into_body(), 4096).await.unwrap();
        assert!(asset_body.starts_with(b"GIF89a"));

        let fallback = app
            .oneshot(
                Request::get("/artwork/fallback")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(fallback.status(), StatusCode::OK);
        assert_eq!(fallback.headers()[header::CONTENT_TYPE], "image/svg+xml");
    }

    #[tokio::test]
    async fn serves_the_bundled_reference_theme_through_the_browser_endpoint() {
        let root = TestDirectory::new();
        let app = test_router(&root.0, include_bytes!("../themes/catppuccin-mocha.toml")).await;
        let page = app
            .oneshot(Request::get("/").body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(page.status(), StatusCode::OK);
        let page_body = to_bytes(page.into_body(), 1024 * 1024).await.unwrap();
        let page = std::str::from_utf8(&page_body).unwrap();
        assert!(page.contains("--mauve: #cba6f7"));
        assert!(page.contains("id=\"artwork\""));
        assert!(page.contains("window.NowPlaying.subscribe(update)"));
    }

    #[tokio::test]
    async fn websocket_delivers_initial_state_playback_updates_theme_switch_and_reconnect() {
        let root = TestDirectory::new();
        let themes = ThemeManager::new(
            root.0.join("themes"),
            root.0.join("settings.toml"),
            ThemeLoader::new("0.1.0").unwrap(),
        )
        .await
        .unwrap();
        let first_path = root.0.join("first.toml");
        let second_path = root.0.join("second.toml");
        tokio::fs::write(&first_path, THEME).await.unwrap();
        tokio::fs::write(&second_path, THEME.replace("server-theme", "second-theme"))
            .await
            .unwrap();
        themes.import_file(&first_path).await.unwrap();
        themes.import_file(&second_path).await.unwrap();
        themes.select("server-theme").await.unwrap();

        let artwork = Arc::new(
            ArtworkCache::new(root.0.join("artwork"), ArtworkCacheConfig::default())
                .await
                .unwrap(),
        );
        let (state_sender, state_receiver) = watch::channel(PlaybackSnapshot::default());
        let (event_sender, event_receiver) = broadcast::channel(8);
        let playback = Arc::new(PlaybackApi::from_receivers(state_receiver, event_receiver));
        let server = LocalServer::start(
            "127.0.0.1:0".parse().unwrap(),
            playback.clone(),
            themes.clone(),
            artwork,
        )
        .await
        .unwrap();
        let url = format!("ws://{}/api/ws", server.address());

        let (mut first_client, _) = connect_async(&url).await.unwrap();
        let initial = tokio::time::timeout(std::time::Duration::from_secs(2), first_client.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let ClientMessage::Text(initial) = initial else {
            panic!("expected initial playback snapshot text frame");
        };
        let initial: serde_json::Value = serde_json::from_str(&initial).unwrap();
        assert_eq!(initial["kind"], "snapshot");
        assert_eq!(initial["snapshot"]["connected"], false);
        assert_eq!(
            initial["snapshot"]["metadata"]["artworkUrl"],
            "/artwork/fallback"
        );

        let updated = PlaybackSnapshot {
            player: Some(PlayerInfo {
                bus_name: "org.mpris.MediaPlayer2.fixture".into(),
                identity: "Fixture Player".into(),
            }),
            status: PlaybackStatus::Playing,
            metadata: crate::playback::TrackMetadata {
                title: Some("A New Track".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        state_sender.send(updated.clone()).unwrap();
        event_sender
            .send(PlaybackEvent {
                kind: PlaybackEventKind::PlaybackStarted,
                snapshot: updated,
            })
            .unwrap();
        let update = tokio::time::timeout(std::time::Duration::from_secs(2), first_client.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let ClientMessage::Text(update) = update else {
            panic!("expected playback event text frame");
        };
        let update: serde_json::Value = serde_json::from_str(&update).unwrap();
        assert_eq!(update["kind"], "playbackStarted");
        assert_eq!(update["snapshot"]["metadata"]["title"], "A New Track");

        themes.select("second-theme").await.unwrap();
        let change = tokio::time::timeout(std::time::Duration::from_secs(2), first_client.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let ClientMessage::Text(change) = change else {
            panic!("expected theme change text frame");
        };
        let change: serde_json::Value = serde_json::from_str(&change).unwrap();
        assert_eq!(change["kind"], "themeChanged");

        drop(first_client);
        let (mut reconnected, _) = connect_async(&url).await.unwrap();
        let current = tokio::time::timeout(std::time::Duration::from_secs(2), reconnected.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let ClientMessage::Text(current) = current else {
            panic!("expected fresh snapshot after reconnect");
        };
        let current: serde_json::Value = serde_json::from_str(&current).unwrap();
        assert_eq!(current["kind"], "snapshot");
        assert_eq!(current["snapshot"]["metadata"]["title"], "A New Track");
        drop(reconnected);

        server.shutdown().await.unwrap();
        drop(playback);
    }
}
