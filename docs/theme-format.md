# Theme file format, version 1

Each theme is one `.toml` file. There are no sidecar directories or archives.
The runtime reads the embedded HTML body, CSS, JavaScript and explicitly typed
Base64 assets from that file. There is no template-variable substitution;
dynamic playback data comes from the documented [JavaScript API](playback-api.md).

## Example

```toml
[theme]
id = "sample-player"
name = "Sample Player"
version = "1.0.0"
format_version = 1
author = "Theme Author"
description = "A compact player display"

[theme.compatibility]
minimum_runtime = "0.1.0"

[canvas]
width = 640
height = 180
background = "transparent"

[layout]
direction = "horizontal" # horizontal, vertical, or freeform
alignment = "center"    # start, center, or end
spacing = 12
padding = 8

[artwork]
enabled = true
position = "left"       # hidden, left, right, top, or bottom
width = 144
height = 144
fit = "cover"
rendering = "auto"

[metadata]
show_title = true
show_artist = true
show_album = false
alignment = "start"

[progress]
enabled = true
position = "bottom"
height = 4

[animations]
enabled = true
reduced_motion_support = true
track_change = "crossfade"
track_change_duration_ms = 450

[colors]
background = "transparent"
text = "#cdd6f4"
muted = "#a6adc8"
accent = "#cba6f7"

[html]
template = """
<main class="player">
  <img id="cover" alt="">
  <section><h1 id="title"></h1><p id="artist"></p></section>
</main>
"""

[css]
stylesheet = """
/* This CSS and the theme's HTML decide the complete visual presentation. */
"""

[javascript]
source = """
// Subscribe to window.NowPlaying when the Phase 4 runtime API is available.
"""

[assets.brand_mark]
mime_type = "image/gif"
data = "R0lGODlhAQABAAD/ACwAAAAAAQABAAACADs="
```

## Schema and defaults

`[theme]` and `[html]` are required. Theme IDs are 1–64 lowercase ASCII letters,
digits or hyphens, starting with a letter. `version` and
`compatibility.minimum_runtime` use semantic versions. The loader accepts only
the exact `format_version` it implements; a future incompatible format must
increment this value.

Canvas defaults to 640 × 180 with a transparent background. Layout defaults to
freeform, centered, 12 px spacing and 8 px padding. Artwork defaults to hidden;
metadata defaults to title and artist visible, album hidden; progress defaults
to disabled; reduced-motion support defaults to enabled. CSS and JavaScript
default to empty strings, colors to transparent/white/gray/mauve, and embedded
assets to an empty table. Unknown keys are rejected so spelling errors do not
silently change the result.

Canvas, layout, artwork, metadata, progress, animation and color tables provide
small declarative hints for common properties. They do not impose a component
tree or constrain the custom markup. A theme can ignore these hints and use
arbitrary HTML, CSS and browser JavaScript to create a different composition.

## Embedded assets

`[assets.<id>]` entries use URL-safe IDs (lowercase letters, digits, hyphens or
underscores), require an explicit MIME type, and store bytes in standard padded
Base64. IDs become resource names at `/theme/assets/<id>` in the local browser
runtime. Assets can be used from CSS or JavaScript without external folders.

The loader limits a TOML file to 8 MiB, each decoded asset to 4 MiB, the decoded
asset total to 8 MiB, and the asset count to 64. HTML, CSS and JavaScript also
have individual size limits. These bounds are checked before large asset
allocations wherever possible.

## Loading, import and switching

The loader discovers `.toml` files, validates required fields, enum values,
canvas/layout limits, Base64, MIME types, IDs and runtime compatibility. Invalid
files are reported individually and do not prevent other themes from loading.
Duplicate IDs are reported deterministically; the lexically first file wins.

The GUI's Import action validates the complete theme before storing it as
`<id>.toml` in the application's per-user themes directory, then removes the
selected source file so importing acts as a move. If the source cannot be
removed, the theme remains imported and the GUI reports that the original file
is still present. The default themes directory is
`$XDG_DATA_HOME/now-playing-studio/themes` (usually
`~/.local/share/now-playing-studio/themes`). Existing themes are never
overwritten. The GUI's Edit action opens that stored file with the system's
default application for text files. Deleting a theme asks for confirmation and
is blocked while that theme is active. Export copies one validated TOML file
and also refuses to overwrite a destination. Selecting a theme persists its ID
in the application settings TOML while preserving unrelated settings. A failed
load, selection or settings write leaves the currently active theme intact.

## Trust and browser boundary

Theme HTML and JavaScript are untrusted browser content. They receive no native
command or filesystem bridge. The runtime will expose only the documented
playback API and embedded resources, with a restrictive Content Security Policy
where practical. Loopback binding does not prevent browser JavaScript from
making other network requests; import themes only from authors you trust.
