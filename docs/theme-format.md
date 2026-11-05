# Themes and the browser playback API

Each theme is a single `.toml` file containing a manifest and its HTML, CSS,
JavaScript, and optional embedded assets. The theme's HTML and CSS create the
display; the runtime supplies playback data through `window.NowPlaying`.

For a copy-paste prompt aimed at AI coding agents, see the
[theme agent guide](theme-agent-guide.md).

## Small working theme

This minimal theme displays the current title, artists, and playback status.
Save it as `sample-player.toml` and import it from the Themes page.


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

The browser runtime receives its initial state and live updates over a
WebSocket, reconnecting if it disconnects. Hidden pages suspend their socket
and reconnect when shown; closing a page releases its connection. Preview
pages scope playback and artwork requests to the previewed theme. Themes
normally only need `NowPlaying.subscribe`; they do not need to connect to
either endpoint themselves. The HTTP state endpoint is `/api/state` and the
WebSocket endpoint is `/api/ws`.

## File fields

Required fields are `[theme]`, `[theme.compatibility]`, and `[html]`.
`theme.id` uses 1–64 lowercase letters, digits, or hyphens and starts with a
letter. `version` and `minimum_runtime` are semantic versions.
`format_version` must be `1` for this format. Optional manifest fields are
`author`, `description`, and `family_id`. `family_id` follows the same ID
rules as `theme.id`. Give all variants in a family the same `family_id` and
display name; keep any provided author value consistent across variants.
Themes without a family ID remain standalone. The selector groups by explicit
family ID only, never by display name or filename.

```toml
[theme]
id = "dreamwave-compact"
name = "Dreamwave"
family_id = "dreamwave"
version = "1.0.0"
format_version = 1
```

A family with multiple variants appears as one theme card with a size menu.
The menu is built from variants that actually exist. A single-variant family
uses the ordinary card. Size names come from the canvas dimensions described
below; custom sizes show their dimensions. Duplicate dimensions remain
selectable and are disambiguated by theme ID. Inconsistent names or authors
prevent grouping and are shown as a visible family warning.

Variant selection persists using the selected variant's `theme.id`. The app
also remembers the last selected variant for each family. Existing
`selected_theme_id` settings continue to identify their individual theme.

`[css].stylesheet` and `[javascript].source` default to empty strings. Optional
tables `[canvas]`, `[layout]`, `[artwork]`, `[metadata]`, `[progress]`,
`[animations]`, and `[colors]` provide defaults and hints for common
properties. They do not generate markup or control a theme's layout: the
shipped themes implement their presentation in HTML and CSS and use JavaScript
to update it. Start with the minimal example above; add these tables only when
they are useful to document your theme's choices.

`[canvas].width` and `[canvas].height` describe the theme's recommended native
browser-source size in CSS pixels. They are informational: the app does not
resize OBS sources, force these dimensions, or reject themes that use other
sizes. Existing themes without a `[canvas]` table keep the format defaults of
640 × 180 px. The theme selector labels exact matches to the official profiles
below; other dimensions are shown as `Custom` with their actual size. No
profile field is required in a theme file.

## Official built-in size profiles

These are Nowplaying's design standards for themes shipped with the app. They
are recommendations for browser-source dimensions, not universal standards or
limits on imported themes.

| Profile | Recommended size | Intended use |
| --- | ---: | --- |
| Slim | 420 × 64 px | Minimal typography, title and artist, a small status indicator, and an optional 1–2 px progress line. |
| Compact | 420 × 120 px | Essential metadata with optional 88–96 px artwork and a restrained progress indicator. |
| Standard | 800 × 150 px | General-purpose layout with artwork, title, artist, and progress. |
| Wide | 960 × 150 px | Horizontal compositions that use the extra width for larger artwork or additional metadata. |
| Ultra-wide | 1000 × 180 px | Cinematic, decorative, waveform, or visualizer-led layouts, including portrait OBS scenes. |

Slim layouts should be deliberately minimal, not ordinary layouts with smaller
type. Keep title and artist primary, truncate long text, and avoid large cards,
borders, and unused metadata. Compact themes should keep artwork around 88–96
px and limit secondary details. Standard, Wide, and Ultra-wide themes should
use their width intentionally instead of stretching a smaller design. The
built-in Starfall and Spectral Tide themes are distinct Ultra-wide examples:
Starfall uses cinematic artwork and scenery; Spectral Tide uses a typography
and decorative-spectrum composition. Its spectrum is decorative and does not
claim to visualize audio samples.

### OBS setup and sharpness

For a built-in theme, set the OBS Browser Source width and height to the
recommendation shown on its theme card. First compare it at 100% source scale
with the browser page opened by the theme's Preview action. Then check it at
the intended stream output resolution. A source rendered at its native
dimensions can still look soft if OBS scales it to a different size; matching
dimensions helps avoid extra resampling but cannot guarantee perfect sharpness.

If a theme still looks soft at native size, check OBS transforms and output
resolution, browser rendering/device-pixel ratio, fractional CSS positioning or
transforms, and artwork resolution before changing the theme. Prefer whole CSS
pixel sizes and positions for static details, inline SVG for vector decoration,
and artwork that is not needlessly enlarged. Keep animations contained and
avoid moving fine text by fractional pixels.

Users can choose any OBS source dimensions. Theme recommendations never change
those settings automatically. Third-party themes may use any valid canvas size;
they are not required to match an official profile and theme loading does not
enforce the profile dimensions.

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
