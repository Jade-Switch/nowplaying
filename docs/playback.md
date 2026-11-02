# Playback and browser runtime

Now Playing Studio monitors MPRIS players over the session D-Bus, publishes a
complete playback snapshot, and makes that state available to themes through a
loopback HTTP and WebSocket service. Artwork is resolved into the local cache
and exposed through the same service. This guide documents player selection,
the browser contract, server routes, and artwork handling.

## Player selection and playback state

The playback provider publishes a latest-value snapshot and semantic change
events:

- `state()` returns a Tokio watch receiver containing the latest complete
  snapshot, even if a consumer falls behind.
- `available_players()` returns discovered MPRIS players, sorted by their
  well-known D-Bus name.
- `subscribe()` returns a bounded broadcast receiver. Events are notifications;
  after a lag warning, consumers should read the latest snapshot.
- `set_selection()` chooses automatic selection or pins a well-known bus name.
- `selector()` returns a cloneable `PlayerSelector` for UI components that do
  not own the monitor's shutdown handle.
- `shutdown()` stops the monitor and clears its published snapshot.

Automatic selection ranks players by status: playing, paused, stopped, then
unknown or unreadable. The well-known D-Bus name breaks ties lexically. A
manually selected player stays pinned; if it disconnects, the app reports a
disconnected snapshot and waits for that player to return instead of silently
switching to another one. Returning to automatic selection resumes ranking.

The monitor discovers players through `NameOwnerChanged` and watches their
`PropertiesChanged` signals. While the selected player is playing, position is
refreshed once per second; it is not polled while paused or stopped. Negative
D-Bus positions and durations are ignored or clamped. Playback controls are
not exposed by this read-only API.

Snapshots include player identity, status, title, artists, album, track ID,
position, duration, shuffle, repeat, and the source artwork URI internally.
Events are emitted for player connection or selection changes, track and
metadata changes, playback transitions, and position updates. Each event
carries the complete resulting snapshot. The bounded event channel does not
block D-Bus monitoring.

## Browser API

Themes receive a read-only `window.NowPlaying` object. It provides no native
command or filesystem access. State is JSON and time values are integer
milliseconds.

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

Nullable fields are JSON `null`; `artists` is always an array. `player` is
`null` while disconnected. Status is `playing`, `paused`, `stopped`, or
`unknown`; repeat is `none`, `track`, `playlist`, or `unknown`. The browser API
does not expose the MPRIS bus name or original artwork URI. `artworkUrl` points
to locally served cached content, or to the bundled fallback image.

The browser object has these methods:

- `NowPlaying.getState()` returns the latest snapshot, or `null` before the
  initial request succeeds.
- `NowPlaying.subscribe(callback)` calls `callback(snapshot, kind)` when the
  initial state arrives and after every update. It returns an unsubscribe
  function.
- `NowPlaying.on(kind, callback)` listens for one event kind. The callback
  receives `(snapshot, event)` and the method returns an unsubscribe function.

On page load, the runtime fetches `/api/state`, then connects to `/api/ws` and
reconnects after disconnection with bounded exponential backoff. A successful
WebSocket connection starts with a `snapshot` event so updates between the HTTP
request and socket connection are not missed. Each WebSocket message is one
JSON object:

```json
{"kind":"trackChanged","snapshot":{"connected":true}}
```

Event kinds are `snapshot`, `playerConnected`, `playerDisconnected`,
`playerChanged`, `trackChanged`, `playbackStarted`, `playbackPaused`,
`playbackStopped`, `positionUpdated`, and `metadataUpdated`. Every event
contains the full resulting snapshot. Treat that snapshot as the source of
truth instead of rebuilding state from event names. If a bounded receiver
lags, the server sends a fresh snapshot before later updates.

## Local HTTP and WebSocket service

The server exposes the selected theme and playback API through Axum. It binds
only to a loopback address and defaults to `127.0.0.1:8765`; the port can be
changed in Settings.

| Path | Behaviour |
| --- | --- |
| `/` | Renders the active theme with a fresh Content Security Policy nonce. |
| `/preview/{theme-id}` | Renders a catalog theme without selecting it, using live playback state. |
| `/api/state` | Returns the latest playback snapshot as JSON. |
| `/api/ws` | Sends an initial snapshot and subsequent playback events. |
| `/theme/assets/{id}` | Serves explicitly typed embedded theme assets. |
| `/preview/{theme-id}/assets/{asset-id}` | Serves an asset embedded in the previewed theme. |
| `/artwork/{id}` | Serves cached artwork or the bundled fallback image. |

Previewing a theme does not change the active OBS source. Selecting a theme
sends a `themeChanged` control message to connected clients, causing the
browser runtime to reload while playback monitoring continues in the app.
Closing a browser drops only its connection.

Theme documents receive a Content Security Policy and `Cache-Control:
no-store`; state responses are also not stored. Binary assets use
`X-Content-Type-Options: nosniff`. Artwork content IDs are immutable and can be
cached for one year.

For browser WebSocket requests that include an `Origin`, the host must be a
literal loopback IP and the port must match the server listener. Requests
without an Origin header are allowed for non-browser clients. Use the same
loopback address for the OBS Browser Source and its WebSocket connection.

The server accepts a loopback `SocketAddr`; bind failures, including port
conflicts, are returned to the caller. It loads `server_port` from the user's
settings TOML and shuts down cleanly on Ctrl+C.

## Artwork cache

The artwork cache resolves the `mpris:artUrl` value from MPRIS and returns an
opaque content ID with a stable `/artwork/{id}` path. The browser sees only
this local URL, never the original URI. Missing or unreadable artwork uses the
bundled, script-free SVG fallback.

Local absolute paths and `file:` URIs are accepted only when their canonical
target is a regular file inside an allowed root. The default allowed root is
the user's home directory; symlinks are resolved before checking it.

`http:` and `https:` sources are supported on standard ports. User information,
fragments, redirects, proxy use, and private or reserved IP destinations are
rejected. DNS results are checked and pinned for the request to prevent
rebinding. Downloads have a five-second connection timeout, a 15-second total
timeout, and an 8 MiB body limit; streaming stops as soon as the limit is
exceeded.

Supported raster formats are PNG, JPEG, GIF, BMP, ICO, WebP, and AVIF. SVG and
other active or unrecognized formats are rejected. The cache holds up to 256
entries and 64 MiB by default, evicting least-recently-used content to stay
within both limits. Files are named by SHA-256 content ID, so identical bytes
share a file. Cache contents survive process restarts; corrupt or unsupported
entries are ignored. `read()` accepts only validated content IDs or the
built-in `fallback` ID, never a filesystem path.

## Using the API in a theme

See the [theme file format](theme-format.md) for how JavaScript is embedded.
For example, a theme can update its title and artwork from each snapshot:

```js
const unsubscribe = window.NowPlaying.subscribe((state) => {
  document.querySelector("#title").textContent = state.metadata.title ?? "";
  document.querySelector("#cover").src = state.metadata.artworkUrl ?? "";
});
```

Themes run as browser content and should be treated as untrusted. The runtime
does not give them a native filesystem or command bridge. Loopback binding
does not prevent browser JavaScript from making other network requests, so
only import themes from authors you trust.
