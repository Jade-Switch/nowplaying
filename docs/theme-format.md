# Themes and the browser playback API

Each theme is a single `.toml` file containing a manifest and its HTML, CSS,
JavaScript, and optional embedded assets. The theme's HTML and CSS create the
display; the runtime supplies playback data through `window.NowPlaying`.

## Small working theme

This minimal theme displays the current title, artists, and playback status.
Save it as `sample-player.toml` and import it from the Themes page.

```toml
[theme]
id = "sample-player"
name = "Sample Player"
version = "1.0.0"
format_version = 1

[theme.compatibility]
minimum_runtime = "0.1.0"

[html]
template = '''
<main class="player">
  <p id="status">Waiting for music</p>
  <h1 id="title">Nothing playing</h1>
  <p id="artist"></p>
</main>
'''

[css]
stylesheet = '''
body { margin: 0; color: #eee; background: #171625; font: 16px sans-serif; }
.player { padding: 24px; }
h1 { margin: 8px 0; }
'''

[javascript]
source = '''
window.NowPlaying.subscribe((state) => {
  const metadata = state.metadata;
  document.querySelector("#status").textContent = state.status;
  document.querySelector("#title").textContent = metadata.title || "Nothing playing";
  document.querySelector("#artist").textContent = metadata.artists.join(" · ");
});
'''
```

TOML strings delimited by three single quotes make it convenient to include
HTML, CSS, and JavaScript without escaping their quotes. The runtime inserts
these strings into the page; it does not substitute variables into the HTML.

## Playback data

`NowPlaying.subscribe(callback)` calls the callback with `(snapshot, kind)`
after the initial state arrives and whenever it changes. `kind` is an event
name such as `trackChanged` or `positionUpdated`. It returns an unsubscribe
function. The snapshot is the complete current state, so render from it instead
of reconstructing state from event names.

The snapshot has this shape:

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

`player`, `title`, `album`, `trackId`, `artworkUrl`, `durationMs`, and
`shuffle` can be `null`. `artists` is always an array; `positionMs` is an
integer in milliseconds. Status is `playing`, `paused`,
`stopped`, or `unknown`. Repeat is `none`, `track`, `playlist`, or `unknown`.
The player is `null` when disconnected. Artwork is served locally as
`artworkUrl`; themes do not receive the original media URI.

Use `NowPlaying.getState()` to read the latest snapshot (or `null` before one
has arrived). `NowPlaying.on(kind, callback)` subscribes to one event kind;
its callback receives `(snapshot, message)`, where `message` contains `kind`
and `snapshot`. Both subscription methods return an unsubscribe function.
Events include `snapshot`, `playerConnected`, `playerDisconnected`,
`playerChanged`, `trackChanged`, `playbackStarted`, `playbackPaused`,
`playbackStopped`, `positionUpdated`, and `metadataUpdated`.

The browser runtime fetches the initial state and then receives live updates
over a WebSocket, reconnecting if it disconnects. Themes normally only need
`NowPlaying.subscribe`; they do not need to connect to either endpoint
themselves. The HTTP state endpoint is `/api/state` and the WebSocket endpoint
is `/api/ws`.

## File fields

Required fields are `[theme]`, `[theme.compatibility]`, and `[html]`.
`theme.id` uses 1–64 lowercase letters, digits, or hyphens and starts with a
letter. `version` and `minimum_runtime` are semantic versions.
`format_version` must be `1` for this format. Optional manifest fields are
`author` and `description`.

`[css].stylesheet` and `[javascript].source` default to empty strings. Optional
tables `[canvas]`, `[layout]`, `[artwork]`, `[metadata]`, `[progress]`,
`[animations]`, and `[colors]` provide defaults and hints for common
properties. They do not generate markup or control a theme's layout: the
shipped themes implement their presentation in HTML and CSS and use JavaScript
to update it. Start with the minimal example above; add these tables only when
they are useful to document your theme's choices.

Artwork is only resolved and exposed as `metadata.artworkUrl` when
`[artwork].enabled` is `true` and both `width` and `height` are positive.
Missing dimensions default to `0`, and a dimension set to `0` disables artwork
loading. This lets themes that do not display artwork avoid fetching or caching
it.

## Embedded assets

Optional `[assets.<id>]` entries embed Base64-encoded data in the theme file:

```toml
[assets.logo]
mime_type = "image/png"
data = "<standard padded Base64 data>"
```

Asset IDs use lowercase letters, digits, hyphens, or underscores. Each asset
needs an explicit MIME type. Its URL is `/theme/assets/<id>`; use that URL in
HTML, CSS, or JavaScript. Themes can also draw visuals directly in HTML, CSS,
or inline SVG, as the shipped Starfall theme does.

## Importing and safety

Theme files must be plain TOML. Markdown code fences such as ` ```toml ` are
documentation formatting and must not be included in a theme file.

Import validates the complete TOML and stores it in
`$XDG_DATA_HOME/nowplaying/themes` (by default,
`~/.local/share/nowplaying/themes`). File imports load the contents into the
editor; importing saves a copy and leaves the selected file untouched. The
editor also accepts pasted TOML. Files and input are limited to 8 MiB. The app
does not overwrite an existing theme with the same ID. Invalid themes are
reported without preventing other themes from loading. Edit opens the installed
file in the system's default text editor.

Theme HTML and JavaScript run as browser content. They have no native command
or filesystem bridge. The runtime's Content Security Policy limits images,
scripts, styles, and network connections to the app's local origin (with data
URLs allowed for images and fonts). Import themes only from authors you trust.
