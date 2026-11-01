use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerInfo {
    /// The well-known MPRIS bus name, suitable for manual selection.
    pub bus_name: String,
    pub identity: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PlaybackStatus {
    Playing,
    Paused,
    Stopped,
    #[default]
    Unknown,
}

impl PlaybackStatus {
    pub(crate) fn selection_rank(self) -> u8 {
        match self {
            Self::Playing => 0,
            Self::Paused => 1,
            Self::Stopped => 2,
            Self::Unknown => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RepeatMode {
    #[default]
    None,
    Track,
    Playlist,
    Unknown,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrackMetadata {
    pub title: Option<String>,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub duration: Option<Duration>,
    pub artwork_uri: Option<String>,
    pub track_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlaybackSnapshot {
    pub player: Option<PlayerInfo>,
    pub status: PlaybackStatus,
    pub metadata: TrackMetadata,
    pub position: Duration,
    pub shuffle: Option<bool>,
    pub repeat: RepeatMode,
}

impl PlaybackSnapshot {
    pub fn is_connected(&self) -> bool {
        self.player.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackEventKind {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackEvent {
    pub kind: PlaybackEventKind,
    pub snapshot: PlaybackSnapshot,
}

/// Calculate semantic events from two snapshots. Consumers can recover from a
/// missed broadcast by reading the latest snapshot from the watch channel.
pub(crate) fn diff(previous: &PlaybackSnapshot, next: &PlaybackSnapshot) -> Vec<PlaybackEventKind> {
    let mut events = Vec::new();
    match (&previous.player, &next.player) {
        (None, Some(_)) => events.push(PlaybackEventKind::PlayerConnected),
        (Some(_), None) => events.push(PlaybackEventKind::PlayerDisconnected),
        (Some(before), Some(after)) if before != after => {
            events.push(PlaybackEventKind::PlayerChanged)
        }
        _ => {}
    }

    if previous.metadata != next.metadata
        || previous.shuffle != next.shuffle
        || previous.repeat != next.repeat
    {
        let before = &previous.metadata;
        let after = &next.metadata;
        let track_changed = before.track_id != after.track_id
            || before.title != after.title
            || before.artists != after.artists
            || before.album != after.album;
        events.push(if track_changed {
            PlaybackEventKind::TrackChanged
        } else {
            PlaybackEventKind::MetadataUpdated
        });
    }

    if previous.status != next.status {
        match next.status {
            PlaybackStatus::Playing => events.push(PlaybackEventKind::PlaybackStarted),
            PlaybackStatus::Paused => events.push(PlaybackEventKind::PlaybackPaused),
            PlaybackStatus::Stopped => events.push(PlaybackEventKind::PlaybackStopped),
            PlaybackStatus::Unknown => {}
        }
    }

    if previous.position != next.position {
        events.push(PlaybackEventKind::PositionUpdated);
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_changes_produce_playback_events() {
        let before = PlaybackSnapshot::default();
        let after = PlaybackSnapshot {
            status: PlaybackStatus::Playing,
            ..before.clone()
        };
        assert_eq!(diff(&before, &after), [PlaybackEventKind::PlaybackStarted]);

        let paused = PlaybackSnapshot {
            status: PlaybackStatus::Paused,
            ..after.clone()
        };
        assert_eq!(diff(&after, &paused), [PlaybackEventKind::PlaybackPaused]);
    }

    #[test]
    fn metadata_track_changes_are_distinguished_from_metadata_updates() {
        let before = PlaybackSnapshot::default();
        let track = PlaybackSnapshot {
            metadata: TrackMetadata {
                title: Some("A song".into()),
                ..TrackMetadata::default()
            },
            ..before.clone()
        };
        assert_eq!(diff(&before, &track), [PlaybackEventKind::TrackChanged]);

        let artwork = PlaybackSnapshot {
            metadata: TrackMetadata {
                artwork_uri: Some("file:///cover.jpg".into()),
                ..track.metadata.clone()
            },
            ..track.clone()
        };
        assert_eq!(diff(&track, &artwork), [PlaybackEventKind::MetadataUpdated]);

        let shuffle = PlaybackSnapshot {
            shuffle: Some(true),
            ..artwork.clone()
        };
        assert_eq!(
            diff(&artwork, &shuffle),
            [PlaybackEventKind::MetadataUpdated]
        );
    }

    #[test]
    fn connection_and_player_changes_are_reported() {
        let first = PlaybackSnapshot {
            player: Some(PlayerInfo {
                bus_name: "org.mpris.MediaPlayer2.a".into(),
                identity: "A".into(),
            }),
            ..PlaybackSnapshot::default()
        };
        assert_eq!(
            diff(&PlaybackSnapshot::default(), &first),
            [PlaybackEventKind::PlayerConnected]
        );

        let second = PlaybackSnapshot {
            player: Some(PlayerInfo {
                bus_name: "org.mpris.MediaPlayer2.b".into(),
                identity: "B".into(),
            }),
            ..first.clone()
        };
        assert_eq!(diff(&first, &second), [PlaybackEventKind::PlayerChanged]);
        assert_eq!(
            diff(&first, &PlaybackSnapshot::default()),
            [PlaybackEventKind::PlayerDisconnected]
        );
    }
}
