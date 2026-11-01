# Playback state and player selection

The `playback` module defines UI- and server-independent state types. The
`mpris` module publishes them through `PlaybackHandle`:

- `state()` returns a Tokio watch receiver containing the latest complete
  snapshot. A consumer that falls behind can still read the current state.
- `available_players()` returns the currently discovered MPRIS players, sorted
  by their well-known D-Bus name.
- `subscribe()` returns a bounded broadcast receiver for semantic changes.
  Consumers should treat events as notifications and use the latest snapshot
  as the source of truth after a lag warning.
- `set_selection()` changes between automatic selection and a specific bus
  name. This is an in-memory preference; configuration persistence is a later
  phase.
- `selector()` returns a cloneable `PlayerSelector` for UI components that
  need to change the source without owning the monitor's shutdown handle.
- `shutdown()` stops the monitor and clears its published playback snapshot.

## Selection rules

Automatic selection ranks players in this order:

1. Playing.
2. Paused.
3. Stopped.
4. Unknown or unreadable playback status.

The well-known MPRIS bus name breaks ties lexically, so selection is stable
across runs. A manual selection stays pinned to its bus name. If it disconnects,
the snapshot becomes disconnected and the monitor waits for that same name to
return; it does not silently switch to another player. Changing the preference
back to automatic resumes ranked selection.

The monitor discovers players from `NameOwnerChanged` and reads live metadata
from MPRIS properties. It subscribes to `PropertiesChanged` for discovered
players, allowing a newly playing player to take automatic priority. Current
position is refreshed once per second while the selected player is Playing;
negative D-Bus positions and durations are clamped/ignored. No high-frequency
position polling occurs while paused or stopped.

## Snapshot and event semantics

Snapshots contain the selected player identity/bus name, playback status,
normalized title/artists/album, duration, position, artwork URI, track ID,
shuffle and repeat values. A missing selected player is represented by a
snapshot without a player and with the default Unknown status.

Events are emitted when the snapshot changes: player connected/disconnected or
changed, track changed, metadata updated, playback started/paused/stopped, and
position updated. The event carries the resulting full snapshot. The event
channel has capacity 32; slow consumers do not block D-Bus monitoring.

The monitor is read-only. Playback controls and artwork fetching are outside
this phase.
