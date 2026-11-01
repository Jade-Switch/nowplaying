# Local OBS server

The server module exposes the active theme and read-only playback API through
an Axum HTTP/WebSocket service. It binds only to loopback addresses; the
application's intended default is `127.0.0.1:8765`.

## Endpoints

| URL | Behaviour |
| --- | --- |
| `/` | Renders the currently selected theme with a fresh CSP nonce. |
| `/preview/{theme-id}` | Renders a catalog theme without selecting it, using live playback state. |
| `/api/state` | Returns the latest playback snapshot as JSON. |
| `/api/ws` | Sends an initial snapshot and subsequent playback events. |
| `/theme/assets/{id}` | Serves explicitly typed embedded theme assets. |
| `/artwork/{id}` | Serves cached artwork and the bundled fallback image. |

Preview pages use `/preview/{theme-id}/assets/{asset-id}` for that theme's
embedded assets. Opening a preview from the GUI does not change the OBS source
or the active theme.

The server sends `Content-Security-Policy` on the theme document, `no-store`
for the document and state response, and `X-Content-Type-Options: nosniff` for
binary assets. Artwork content IDs are immutable and may be cached for one year.
Each WebSocket client has its own bounded broadcast receiver; a lagging client
is resynchronized with a complete current snapshot. Closing a browser only
drops that client connection. Successful theme selection sends a
`themeChanged` control message to connected clients; the browser runtime reloads
the document while playback monitoring continues in the application process.

WebSocket requests with an `Origin` must use an HTTP origin whose host is a
literal loopback IP and whose port matches the listener. Requests without an
Origin header are allowed for non-browser clients. OBS should use the same
loopback URL for the Browser Source and its WebSocket connection.

The `LocalServer::start` API accepts a loopback `SocketAddr`; bind errors,
including port conflicts, are returned to the caller. Application startup
loads `server_port` from the user's TOML settings and shuts the server down
cleanly on Ctrl+C. HTTP endpoint tests exercise the Axum router, and a loopback
WebSocket test covers initial snapshots, playback updates, theme changes and
reconnection.
