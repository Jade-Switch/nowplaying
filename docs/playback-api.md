# Browser playback API, version 1

Themes receive a read-only `window.NowPlaying` object. The API has no native
command or filesystem access. State is represented in JSON and all time values
are integer milliseconds.

## State

`GET /api/state` returns the latest complete snapshot:

```json
{
  "connected": true,
  "player": { "identity": "Example Player" },
  "status": "playing",
  "metadata": {
    "title": "Example Track",
    "artists": ["Example Artist"],
    "album": "Example Album",
    "trackId": "/org/mpris/MediaPlayer2/Track/1",
    "artworkUrl": "/artwork/opaque-cache-id"
  },
  "positionMs": 12000,
  "durationMs": 185000,
  "shuffle": false,
  "repeat": "none"
}
```

Nullable values are JSON `null`; `artists` is always an array. `player` is
`null` while disconnected. Status values are `playing`, `paused`, `stopped` or
`unknown`; repeat values are `none`, `track`, `playlist` or `unknown`. The API
does not expose the MPRIS bus name or original artwork URI. `artworkUrl` is
provided only after the artwork manager resolves content to a local cached URL.

## Browser object

- `NowPlaying.getState()` returns the latest snapshot, or `null` before the
  initial request succeeds.
- `NowPlaying.subscribe(callback)` calls `callback(snapshot, kind)` once the
  initial state arrives and after every update. It returns an unsubscribe
  function.
- `NowPlaying.on(kind, callback)` subscribes to one event kind. The callback
  receives `(snapshot, event)` and an unsubscribe function is returned.

The renderer fetches `/api/state` on page load and reconnects to
`/api/ws` after disconnection with bounded exponential backoff. A successful
WebSocket connection must immediately send a `snapshot` event so updates that
occurred between the HTTP request and socket connection are not lost. Each
WebSocket message is one JSON object:

```json
{"kind":"trackChanged","snapshot":{"connected":true}}
```

Event kinds are `snapshot`, `playerConnected`, `playerDisconnected`,
`playerChanged`, `trackChanged`, `playbackStarted`, `playbackPaused`,
`playbackStopped`, `positionUpdated` and `metadataUpdated`. Every event carries
the complete resulting snapshot. Consumers should use the snapshot as source
of truth rather than reconstructing state from event deltas.

The runtime bridge now forwards MPRIS changes as bounded API events and retains
the latest complete snapshot for recovery. The HTTP/WebSocket service and
artwork URL resolution are implemented in Phase 5; this document defines the
stable payload and browser contract.

If a WebSocket consumer's bounded receiver reports that it lagged, the server
must send a fresh `snapshot` event using the current API state before forwarding
new updates. The bridge performs the same recovery when it lags behind MPRIS.
