use std::{collections::HashMap, time::Duration};

use futures_util::{StreamExt, stream::SelectAll};
use thiserror::Error;
use tokio::sync::{broadcast, mpsc, watch};
use tracing::{debug, warn};
use zbus::{Connection, Proxy, fdo::DBusProxy, zvariant::OwnedValue};

use crate::playback::{
    PlaybackEvent, PlaybackSnapshot, PlaybackStatus, PlayerInfo, RepeatMode, TrackMetadata, diff,
};

const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";
const PLAYER_PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER_INTERFACE: &str = "org.mpris.MediaPlayer2.Player";
const ROOT_INTERFACE: &str = "org.mpris.MediaPlayer2";
const EVENT_BUFFER: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PlayerSelection {
    #[default]
    Automatic,
    /// Keep this bus name selected across disconnects and wait for it to return.
    Manual(String),
}

#[derive(Debug, Error)]
pub enum MprisError {
    #[error("failed to connect to the session D-Bus: {0}")]
    Connect(#[from] zbus::Error),
}

/// Cloneable selection control for consumers that do not own the monitor's
/// shutdown handle.
#[derive(Debug, Clone)]
pub struct PlayerSelector {
    selection: watch::Sender<PlayerSelection>,
}

impl PlayerSelector {
    pub fn set_selection(&self, selection: PlayerSelection) {
        self.selection.send_replace(selection);
    }
}

/// A live playback provider. Latest snapshots use a watch channel so a slow
/// consumer always recovers the newest state; semantic events use a bounded
/// broadcast channel and may be skipped by lagging consumers.
pub struct PlaybackHandle {
    state: watch::Receiver<PlaybackSnapshot>,
    players: watch::Receiver<Vec<PlayerInfo>>,
    events: broadcast::Sender<PlaybackEvent>,
    selection: watch::Sender<PlayerSelection>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl PlaybackHandle {
    pub async fn connect(selection: PlayerSelection) -> Result<Self, MprisError> {
        let connection = Connection::session().await?;
        Ok(Self::start(connection, selection).await)
    }

    /// Start against an existing bus connection. This is also useful for
    /// isolated D-Bus tests and for applications that already own a connection.
    pub async fn start(connection: Connection, initial_selection: PlayerSelection) -> Self {
        let (state_tx, state) = watch::channel(PlaybackSnapshot::default());
        let (players_tx, players) = watch::channel(Vec::new());
        let (events, _) = broadcast::channel(EVENT_BUFFER);
        let (selection, selection_rx) = watch::channel(initial_selection);
        let (shutdown, shutdown_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(run_monitor(
            connection,
            state_tx,
            players_tx,
            events.clone(),
            selection_rx,
            shutdown_rx,
        ));
        Self {
            state,
            players,
            events,
            selection,
            shutdown: Some(shutdown),
            task,
        }
    }

    pub fn state(&self) -> watch::Receiver<PlaybackSnapshot> {
        self.state.clone()
    }

    pub fn available_players(&self) -> watch::Receiver<Vec<PlayerInfo>> {
        self.players.clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<PlaybackEvent> {
        self.events.subscribe()
    }

    pub fn set_selection(&self, selection: PlayerSelection) {
        self.selection.send_replace(selection);
    }

    pub fn selector(&self) -> PlayerSelector {
        PlayerSelector {
            selection: self.selection.clone(),
        }
    }

    pub async fn shutdown(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        let _ = self.task.await;
    }
}

async fn run_monitor(
    connection: Connection,
    state_tx: watch::Sender<PlaybackSnapshot>,
    players_tx: watch::Sender<Vec<PlayerInfo>>,
    events: broadcast::Sender<PlaybackEvent>,
    mut selection_rx: watch::Receiver<PlayerSelection>,
    mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
) {
    let dbus = match DBusProxy::new(&connection).await {
        Ok(proxy) => proxy,
        Err(error) => {
            warn!(%error, "Could not create D-Bus proxy");
            return;
        }
    };
    let mut owner_changes = match dbus.receive_name_owner_changed().await {
        Ok(stream) => stream,
        Err(error) => {
            warn!(%error, "Could not monitor D-Bus name ownership");
            return;
        }
    };

    let mut selected = None::<String>;
    let mut signal_task = None::<tokio::task::JoinHandle<()>>;
    let (signal_tx, mut signal_rx) = mpsc::channel(1);
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    if refresh_players(&connection, &dbus, &players_tx).await {
        let players = players_tx.borrow().clone();
        let selection = selection_rx.borrow().clone();
        selected = resolve_selection(&connection, &players, &selection).await;
        signal_task = make_property_watchers(&connection, &players, signal_tx.clone()).await;
        refresh_selected(&connection, selected.as_deref(), &state_tx, &events).await;
    }

    loop {
        tokio::select! {
            _ = &mut shutdown_rx => break,
            changed = selection_rx.changed() => {
                if changed.is_err() { break; }
                let players = players_tx.borrow().clone();
                let selection = selection_rx.borrow().clone();
                selected = resolve_selection(&connection, &players, &selection).await;
                if let Some(task) = signal_task.take() { task.abort(); }
                let players = players_tx.borrow().clone();
                signal_task = make_property_watchers(&connection, &players, signal_tx.clone()).await;
                refresh_selected(&connection, selected.as_deref(), &state_tx, &events).await;
            }
            change = owner_changes.next() => {
                let Some(change) = change else { break; };
                let Ok(args) = change.args() else { continue; };
                let name = args.name().as_str();
                if name.starts_with(MPRIS_PREFIX) {
                    debug!(bus_name = name, "MPRIS player ownership changed");
                    refresh_players(&connection, &dbus, &players_tx).await;
                    let players = players_tx.borrow().clone();
                    let selection = selection_rx.borrow().clone();
                    let next = resolve_selection(&connection, &players, &selection).await;
                    selected = next;
                    if let Some(task) = signal_task.take() { task.abort(); }
                    signal_task = make_property_watchers(&connection, &players, signal_tx.clone()).await;
                    refresh_selected(&connection, selected.as_deref(), &state_tx, &events).await;
                }
            }
            signal = signal_rx.recv() => {
                if signal.is_some() {
                    if matches!(&*selection_rx.borrow(), PlayerSelection::Automatic) {
                        let players = players_tx.borrow().clone();
                        let next = resolve_selection(&connection, &players, &PlayerSelection::Automatic).await;
                        if next != selected {
                            selected = next;
                            if let Some(task) = signal_task.take() { task.abort(); }
                            signal_task = make_property_watchers(&connection, &players, signal_tx.clone()).await;
                        }
                    }
                    refresh_selected(&connection, selected.as_deref(), &state_tx, &events).await;
                }
            }
            _ = ticker.tick(), if state_tx.borrow().status == PlaybackStatus::Playing => {
                refresh_position(&connection, selected.as_deref(), &state_tx, &events).await;
            }
        }
    }
    if let Some(task) = signal_task {
        task.abort();
    }
    publish(&state_tx, &events, PlaybackSnapshot::default());
}

async fn refresh_players(
    connection: &Connection,
    dbus: &DBusProxy<'_>,
    players_tx: &watch::Sender<Vec<PlayerInfo>>,
) -> bool {
    let names = match dbus.list_names().await {
        Ok(names) => names,
        Err(error) => {
            warn!(%error, "Could not list D-Bus names");
            return false;
        }
    };
    let mut players = Vec::new();
    for name in names {
        let bus_name = name.as_str();
        if !bus_name.starts_with(MPRIS_PREFIX) {
            continue;
        }
        let identity = read_identity(connection, bus_name)
            .await
            .unwrap_or_else(|| {
                bus_name
                    .strip_prefix(MPRIS_PREFIX)
                    .unwrap_or(bus_name)
                    .to_owned()
            });
        players.push(PlayerInfo {
            bus_name: bus_name.to_owned(),
            identity,
        });
    }
    players.sort_by(|left, right| left.bus_name.cmp(&right.bus_name));
    players_tx.send_replace(players);
    true
}

async fn read_identity(connection: &Connection, bus_name: &str) -> Option<String> {
    let proxy = Proxy::new(connection, bus_name, PLAYER_PATH, ROOT_INTERFACE)
        .await
        .ok()?;
    proxy.get_property("Identity").await.ok()
}

async fn resolve_selection(
    connection: &Connection,
    players: &[PlayerInfo],
    selection: &PlayerSelection,
) -> Option<String> {
    match selection {
        PlayerSelection::Manual(bus_name) => resolve_manual(players, bus_name),
        PlayerSelection::Automatic => {
            let mut candidates = Vec::with_capacity(players.len());
            for player in players {
                let status = read_status(connection, &player.bus_name)
                    .await
                    .unwrap_or(PlaybackStatus::Unknown);
                candidates.push((status.selection_rank(), player.bus_name.clone()));
            }
            choose_automatic(candidates)
        }
    }
}

fn choose_automatic(mut candidates: Vec<(u8, String)>) -> Option<String> {
    candidates.sort();
    candidates.into_iter().next().map(|(_, name)| name)
}

fn resolve_manual(players: &[PlayerInfo], bus_name: &str) -> Option<String> {
    players
        .iter()
        .any(|player| player.bus_name == bus_name)
        .then(|| bus_name.to_owned())
}

async fn make_property_watchers(
    connection: &Connection,
    players: &[PlayerInfo],
    signal_tx: mpsc::Sender<()>,
) -> Option<tokio::task::JoinHandle<()>> {
    let mut streams = SelectAll::new();
    for player in players {
        let proxy = match Proxy::new(
            connection,
            player.bus_name.as_str(),
            PLAYER_PATH,
            "org.freedesktop.DBus.Properties",
        )
        .await
        {
            Ok(proxy) => proxy,
            Err(error) => {
                debug!(%error, bus_name = player.bus_name, "Could not create player properties proxy");
                continue;
            }
        };
        match proxy.receive_signal("PropertiesChanged").await {
            Ok(stream) => streams.push(stream),
            Err(error) => {
                debug!(%error, bus_name = player.bus_name, "Could not subscribe to player properties")
            }
        }
    }
    if streams.is_empty() {
        return None;
    }
    Some(tokio::spawn(async move {
        while streams.next().await.is_some() {
            if signal_tx.try_send(()).is_err() {
                // Coalesce bursts; a full one-slot channel means refresh is pending.
            }
        }
    }))
}

async fn refresh_selected(
    connection: &Connection,
    selected: Option<&str>,
    state_tx: &watch::Sender<PlaybackSnapshot>,
    events: &broadcast::Sender<PlaybackEvent>,
) {
    let Some(bus_name) = selected else {
        publish(state_tx, events, PlaybackSnapshot::default());
        return;
    };
    match read_snapshot(connection, bus_name).await {
        Ok(snapshot) => publish(state_tx, events, snapshot),
        Err(error) => {
            debug!(%error, bus_name, "Selected MPRIS player is not currently available");
            publish(state_tx, events, PlaybackSnapshot::default());
        }
    }
}

async fn refresh_position(
    connection: &Connection,
    selected: Option<&str>,
    state_tx: &watch::Sender<PlaybackSnapshot>,
    events: &broadcast::Sender<PlaybackEvent>,
) {
    let Some(bus_name) = selected else {
        return;
    };
    match read_position(connection, bus_name).await {
        Ok(position) => {
            let mut snapshot = state_tx.borrow().clone();
            snapshot.position = position;
            publish(state_tx, events, snapshot);
        }
        Err(error) => debug!(%error, bus_name, "Could not update MPRIS position"),
    }
}

fn publish(
    state_tx: &watch::Sender<PlaybackSnapshot>,
    events: &broadcast::Sender<PlaybackEvent>,
    snapshot: PlaybackSnapshot,
) {
    let previous = state_tx.borrow().clone();
    if previous == snapshot {
        return;
    }
    for kind in diff(&previous, &snapshot) {
        let _ = events.send(PlaybackEvent {
            kind,
            snapshot: snapshot.clone(),
        });
    }
    state_tx.send_replace(snapshot);
}

async fn read_snapshot(connection: &Connection, bus_name: &str) -> zbus::Result<PlaybackSnapshot> {
    let proxy = Proxy::new(connection, bus_name, PLAYER_PATH, PLAYER_INTERFACE).await?;
    let status = read_status(connection, bus_name)
        .await
        .unwrap_or(PlaybackStatus::Unknown);
    let metadata: HashMap<String, OwnedValue> = proxy.get_property("Metadata").await?;
    let position = read_position(connection, bus_name)
        .await
        .unwrap_or_default();
    let shuffle = optional_property::<bool>(&proxy, "Shuffle").await;
    let loop_status = optional_property::<String>(&proxy, "LoopStatus").await;
    Ok(PlaybackSnapshot {
        player: Some(PlayerInfo {
            bus_name: bus_name.to_owned(),
            identity: read_identity(connection, bus_name)
                .await
                .unwrap_or_else(|| {
                    bus_name
                        .strip_prefix(MPRIS_PREFIX)
                        .unwrap_or(bus_name)
                        .to_owned()
                }),
        }),
        status,
        metadata: normalize_metadata(metadata),
        position,
        shuffle,
        repeat: match loop_status.as_deref() {
            Some("Track") => RepeatMode::Track,
            Some("Playlist") => RepeatMode::Playlist,
            Some("None") => RepeatMode::None,
            Some(_) => RepeatMode::Unknown,
            None => RepeatMode::Unknown,
        },
    })
}

async fn read_status(connection: &Connection, bus_name: &str) -> zbus::Result<PlaybackStatus> {
    let proxy = Proxy::new(connection, bus_name, PLAYER_PATH, PLAYER_INTERFACE).await?;
    let status: String = proxy.get_property("PlaybackStatus").await?;
    Ok(match status.as_str() {
        "Playing" => PlaybackStatus::Playing,
        "Paused" => PlaybackStatus::Paused,
        "Stopped" => PlaybackStatus::Stopped,
        _ => PlaybackStatus::Unknown,
    })
}

async fn read_position(connection: &Connection, bus_name: &str) -> zbus::Result<Duration> {
    let proxy = Proxy::new(connection, bus_name, PLAYER_PATH, PLAYER_INTERFACE).await?;
    let microseconds: i64 = proxy.get_property("Position").await?;
    Ok(Duration::from_micros(microseconds.max(0) as u64))
}

async fn optional_property<T>(proxy: &Proxy<'_>, property: &str) -> Option<T>
where
    T: TryFrom<OwnedValue>,
{
    let value: OwnedValue = proxy.get_property(property).await.ok()?;
    T::try_from(value).ok()
}

fn normalize_metadata(properties: HashMap<String, OwnedValue>) -> TrackMetadata {
    let title = take_string(&properties, "xesam:title");
    let artists = take_strings(&properties, "xesam:artist");
    let album = take_string(&properties, "xesam:album");
    let artwork_uri = take_string(&properties, "mpris:artUrl");
    let track_id = take_string(&properties, "mpris:trackid");
    let duration = properties
        .get("mpris:length")
        .and_then(|value| i64::try_from(value.clone()).ok())
        .filter(|value| *value > 0)
        .map(|value| Duration::from_micros(value as u64));
    TrackMetadata {
        title,
        artists,
        album,
        duration,
        artwork_uri,
        track_id,
    }
}

fn take_string(properties: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    properties
        .get(key)
        .and_then(|value| String::try_from(value.clone()).ok())
}

fn take_strings(properties: &HashMap<String, OwnedValue>, key: &str) -> Vec<String> {
    properties
        .get(key)
        .and_then(|value| Vec::<String>::try_from(value.clone()).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(name: &str) -> PlayerInfo {
        PlayerInfo {
            bus_name: format!("{MPRIS_PREFIX}{name}"),
            identity: name.to_owned(),
        }
    }

    #[test]
    fn automatic_selection_prioritizes_playing_then_paused_then_name() {
        let candidates = vec![
            (
                PlaybackStatus::Paused.selection_rank(),
                format!("{MPRIS_PREFIX}z"),
            ),
            (
                PlaybackStatus::Playing.selection_rank(),
                format!("{MPRIS_PREFIX}b"),
            ),
            (
                PlaybackStatus::Playing.selection_rank(),
                format!("{MPRIS_PREFIX}a"),
            ),
        ];
        assert_eq!(
            choose_automatic(candidates),
            Some(format!("{MPRIS_PREFIX}a"))
        );
    }

    #[test]
    fn automatic_selection_uses_paused_before_stopped_and_unknown() {
        let candidates = vec![
            (
                PlaybackStatus::Unknown.selection_rank(),
                format!("{MPRIS_PREFIX}a"),
            ),
            (
                PlaybackStatus::Stopped.selection_rank(),
                format!("{MPRIS_PREFIX}b"),
            ),
            (
                PlaybackStatus::Paused.selection_rank(),
                format!("{MPRIS_PREFIX}z"),
            ),
        ];
        assert_eq!(
            choose_automatic(candidates),
            Some(format!("{MPRIS_PREFIX}z"))
        );
    }

    #[test]
    fn manual_selection_does_not_fall_back_when_player_is_absent() {
        let players = vec![player("available")];
        assert_eq!(
            resolve_manual(&players, &format!("{MPRIS_PREFIX}missing")),
            None
        );
        assert_eq!(
            resolve_manual(&players, &players[0].bus_name),
            Some(players[0].bus_name.clone())
        );
    }

    #[test]
    fn cloneable_player_selector_updates_the_monitor_preference() {
        let (selection_tx, selection_rx) = watch::channel(PlayerSelection::Automatic);
        let selector = PlayerSelector {
            selection: selection_tx,
        };
        selector.set_selection(PlayerSelection::Manual(format!("{MPRIS_PREFIX}fixture")));
        assert_eq!(
            *selection_rx.borrow(),
            PlayerSelection::Manual(format!("{MPRIS_PREFIX}fixture"))
        );
        selector.set_selection(PlayerSelection::Automatic);
        assert_eq!(*selection_rx.borrow(), PlayerSelection::Automatic);
    }

    #[test]
    fn metadata_values_are_normalized_and_invalid_duration_is_ignored() {
        let mut properties = HashMap::new();
        properties.insert(
            "xesam:title".into(),
            OwnedValue::from(zbus::zvariant::Str::from("Track")),
        );
        properties.insert("mpris:length".into(), OwnedValue::from(-3_i64));
        let metadata = normalize_metadata(properties);
        assert_eq!(metadata.title.as_deref(), Some("Track"));
        assert_eq!(metadata.duration, None);
    }
}
