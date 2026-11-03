use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};

use nowplaying::{
    mpris::{PlaybackHandle, PlayerSelection},
    playback::{PlaybackEventKind, PlaybackStatus},
};
use zbus::{
    Proxy, interface,
    zvariant::{OwnedValue, Str},
};

const PLAYER_NAME: &str = "org.mpris.MediaPlayer2.nps_test_fixture";
const PLAYER_PATH: &str = "/org/mpris/MediaPlayer2";

struct PrivateBus {
    child: Child,
    address: String,
}

impl PrivateBus {
    fn start() -> Self {
        let mut child = Command::new("dbus-daemon")
            .args([
                "--session",
                "--nofork",
                "--print-address=1",
                "--print-pid=1",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("dbus-daemon is needed for this isolated integration test");
        let stdout = child.stdout.take().unwrap();
        let mut lines = BufReader::new(stdout).lines();
        let address = lines.next().unwrap().unwrap();
        let _pid = lines.next();
        Self { child, address }
    }
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct MockRoot;

#[interface(name = "org.mpris.MediaPlayer2")]
impl MockRoot {
    #[zbus(property)]
    fn identity(&self) -> &str {
        "Fixture Player"
    }
}

struct MockPlayer(Arc<Mutex<String>>);

#[interface(name = "org.mpris.MediaPlayer2.Player")]
impl MockPlayer {
    #[zbus(property)]
    fn playback_status(&self) -> String {
        self.0.lock().unwrap().clone()
    }

    #[zbus(property)]
    fn set_playback_status(&mut self, status: String) {
        *self.0.lock().unwrap() = status;
    }

    #[zbus(property)]
    fn metadata(&self) -> std::collections::HashMap<String, OwnedValue> {
        let mut metadata = std::collections::HashMap::new();
        metadata.insert(
            "xesam:title".into(),
            OwnedValue::from(Str::from("Fixture Track")),
        );
        metadata.insert("mpris:length".into(), OwnedValue::from(180_000_000_i64));
        metadata
    }

    #[zbus(property)]
    fn position(&self) -> i64 {
        12_000_000
    }

    #[zbus(property)]
    fn shuffle(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn loop_status(&self) -> &str {
        "None"
    }
}

async fn serve_fixture(address: &str, status: Arc<Mutex<String>>) -> zbus::Connection {
    zbus::connection::Builder::address(address)
        .unwrap()
        .name(PLAYER_NAME)
        .unwrap()
        .serve_at(PLAYER_PATH, MockRoot)
        .unwrap()
        .serve_at(PLAYER_PATH, MockPlayer(status))
        .unwrap()
        .build()
        .await
        .unwrap()
}

#[tokio::test]
async fn discovers_and_reads_a_mock_mpris_player_on_a_private_bus() {
    let _bus = PrivateBus::start();
    let address = _bus.address.clone();
    let server = serve_fixture(&address, Arc::new(Mutex::new("Playing".to_owned()))).await;
    let client = zbus::connection::Builder::address(address.as_str())
        .unwrap()
        .build()
        .await
        .unwrap();
    let control_connection = client.clone();

    let playback = PlaybackHandle::start(client, PlayerSelection::Automatic).await;
    let mut snapshot = playback.state();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if snapshot.borrow().player.is_some() {
                break;
            }
            snapshot.changed().await.unwrap();
        }
    })
    .await
    .expect("MPRIS player should be detected");

    let state = snapshot.borrow().clone();
    assert_eq!(state.player.as_ref().unwrap().identity, "Fixture Player");
    assert_eq!(state.status, PlaybackStatus::Playing);
    assert_eq!(state.metadata.title.as_deref(), Some("Fixture Track"));
    assert_eq!(state.metadata.duration, Some(Duration::from_secs(180)));
    assert_eq!(state.position, Duration::from_secs(12));

    let mut playback_events = playback.subscribe();
    let player_proxy = Proxy::new(
        &control_connection,
        PLAYER_NAME,
        PLAYER_PATH,
        "org.mpris.MediaPlayer2.Player",
    )
    .await
    .unwrap();
    player_proxy
        .set_property("PlaybackStatus", "Paused")
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if snapshot.borrow().status == PlaybackStatus::Paused {
                break;
            }
            snapshot.changed().await.unwrap();
        }
    })
    .await
    .expect("PropertiesChanged should update playback status");
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), playback_events.recv())
            .await
            .unwrap()
            .unwrap()
            .kind,
        PlaybackEventKind::PlaybackPaused
    );

    let mut players = playback.available_players();
    assert!(
        players
            .borrow_and_update()
            .iter()
            .any(|player| player.bus_name == PLAYER_NAME)
    );

    drop(server);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if !snapshot.borrow().is_connected() {
                break;
            }
            snapshot.changed().await.unwrap();
        }
    })
    .await
    .expect("lost MPRIS name should clear the snapshot");

    let restarted_server =
        serve_fixture(&address, Arc::new(Mutex::new("Playing".to_owned()))).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if snapshot.borrow().is_connected() {
                break;
            }
            snapshot.changed().await.unwrap();
        }
    })
    .await
    .expect("restarted MPRIS name should reconnect");
    assert_eq!(
        snapshot.borrow().metadata.title.as_deref(),
        Some("Fixture Track")
    );
    playback.shutdown().await;
    drop(restarted_server);
}
