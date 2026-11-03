# Agent guide: generate valid nowplaying themes

```text
Create one complete, importable nowplaying theme as a single TOML file.

Theme brief: [describe the visual style and content]
Theme name: [name]
Theme ID: [lowercase-id]
Recommended size: [official profile and dimensions, or custom width × height]
Optional requirements: [artwork, colors, details, accessibility, motion]

Follow every rule below:

OUTPUT
- Return the complete plain TOML file only. Do not wrap it in Markdown fences
  or add commentary before or after it.
- Use TOML literal multiline strings (`'''`) for HTML, CSS, and JavaScript.
- Make the file self-contained. Do not rely on external files, web fonts,
  CDNs, remote scripts, remote images, or network services.

MANIFEST AND FORMAT
- Include required `[theme]`, `[theme.compatibility]`, and `[html]` tables.
- Set `format_version = 1`. Use semantic versions for `version` and
  `minimum_runtime`; use `0.1.0` unless the task specifies another supported
  minimum runtime.
- The ID must be 1–64 ASCII characters, start with a lowercase letter, and
  contain only lowercase letters, digits, and hyphens. Keep it unique.
- `name` is required and non-empty (maximum 128 bytes). `author` and
  `description` are optional, but if present must be non-empty; their maximum
  lengths are 128 and 1024 bytes respectively. Use valid semantic versions;
  `minimum_runtime` must not exceed the app version.
- Optional `[theme].family_id` uses the same ID rules as `theme.id`. Set it
  only when the theme intentionally belongs to a named family of size or
  layout variants (the other variants may not be installed yet). All variants
  in a family must share the same `family_id`, display name, and compatible
  author metadata. Do not assign a family to unrelated designs.
- Use only documented top-level tables and fields. Unknown fields are rejected.
- Include `[canvas]` width and height as the recommended native CSS-pixel
  dimensions. Match the root design's actual CSS size to these dimensions.
  Canvas dimensions are recommendations, not restrictions on other themes or
  users' OBS settings.
- If using an official profile, choose exactly one:
  Slim 420×64, Compact 420×120, Standard 800×150, Wide 960×150, or
  Ultra-wide 1000×180. There is no profile field in TOML; dimensions identify
  the profile. Arbitrary custom dimensions are also valid.
- Optional known tables are `[layout]`, `[artwork]`, `[metadata]`,
  `[progress]`, `[animations]`, `[colors]`, `[css]`, `[javascript]`, and
  `[assets.<id>]`. They document choices; HTML/CSS implement the actual layout.
- If adding optional tables, use only these fields:
  - `[canvas]`: `width`, `height`, `background`.
  - `[layout]`: `direction`, `alignment`, `spacing`, `padding`.
  - `[artwork]`: `enabled`, `position`, `width`, `height`, `fit`, `rendering`.
  - `[metadata]`: `show_title`, `show_artist`, `show_album`, `alignment`.
  - `[progress]`: `enabled`, `position`, `height`.
  - `[animations]`: `enabled`, `reduced_motion_support`, `track_change`,
    `track_change_duration_ms`.
  - `[colors]`: `background`, `text`, `muted`, `accent`.
  - `[css]`: `stylesheet`. `[javascript]`: `source`.
  - `[assets.<id>]`: `mime_type`, `data`.
- `[theme].family_id` is an optional manifest field; do not add a `profile`
  field. Official size profiles are inferred from canvas dimensions.

THEME CONTENT
- Make the theme usable before playback starts, while disconnected, and for
  `playing`, `paused`, `stopped`, and `unknown` statuses.
- Support absent title, album, artwork, duration, and player values. Artists is
  an array and may be empty. Insert metadata with `textContent`, not
  `innerHTML`.
- Use `window.NowPlaying.subscribe((snapshot, kind) => { ... })` for normal
  updates. The callback receives the complete snapshot initially and on
  subsequent changes. `window.NowPlaying.getState()` returns the latest
  snapshot or `null`. `window.NowPlaying.on(kind, callback)` is available for
  event-specific updates; its callback receives `(snapshot, message)`.
- Snapshot fields: `connected` (boolean), `player` (`null` or
  `{ identity }`), `status` (`playing`, `paused`, `stopped`, `unknown`),
  `metadata` (`title`, `artists`, `album`, `trackId`, `artworkUrl`),
  `positionMs` (integer), `durationMs` (`null` or integer), `shuffle`
  (`null` or boolean), and `repeat` (`none`, `track`, `playlist`, `unknown`).
  Metadata title/album/trackId/artworkUrl can be `null`; artists is always an
  array. The API never exposes the original artwork URI.
- Artwork is loaded only when `[artwork].enabled = true` and both artwork
  dimensions are positive. Otherwise keep it disabled with dimensions `0`.
  Use `metadata.artworkUrl` when present, preserve its aspect ratio, avoid
  unnecessary upscaling, and show a fallback when absent.
- Use CSS truncation for long titles and artist lists. Keep content inside the
  declared canvas. Respect `prefers-reduced-motion`; avoid fractional CSS
  positioning, text transforms, and layout-shifting animation.
- Prefer whole-pixel static geometry, sensible readable type, inline SVG for
  vector decoration, and deliberate 1–2 px rules. A decorative visualizer must
  not be described as audio-reactive unless it actually receives audio data.
- Do not make assumptions about `layout`, `artwork`, or other hint tables
  controlling the HTML; write the corresponding markup and CSS yourself.

SECURITY AND SIZE LIMITS
- Theme HTML and JavaScript run as browser content with no native command or
  filesystem bridge. The page's CSP permits same-origin connections and assets;
  images and fonts may also use `data:` URLs. External network resources are
  blocked. Do not fetch remote URLs.
- Use the same-origin asset URL `/theme/assets/<id>` for an embedded asset.
  Asset IDs are 1–64 characters, start lowercase, and contain lowercase
  letters, digits, hyphens, or underscores. Use a standard Base64 data string
  and an explicit simple MIME type such as `image/png` (no MIME parameters).
- Limits: whole theme file 8 MiB; HTML 4 MiB; CSS 1 MiB; JavaScript 1 MiB;
  at most 64 assets; each decoded asset 4 MiB; decoded assets total 8 MiB.
- Validation bounds: canvas width/height 1–4096; layout spacing/padding at
  most 256; artwork width/height at most 2048; progress height 1–64;
  track-change duration at most 10000 ms. Use alignment `start`, `center`, or
  `end`; layout direction `horizontal`, `vertical`, or `freeform`; artwork
  position `hidden`, `left`, `right`, `top`, or `bottom`.
- Use `[html].template` as non-empty HTML. Do not place Markdown fences inside
  the theme file. Avoid inline HTML `style="..."` attributes; put static styles
  in `[css].stylesheet` and use native controls such as `<progress>` when
  practical.

VALIDATE BEFORE RETURNING
1. Check the whole output with a TOML parser; TOML-valid alone is not enough.
2. Check all required fields, known field names, bounds, IDs, versions,
   Base64, asset MIME types, and size limits above.
3. Check that every DOM ID used by JavaScript exists in the HTML and that each
   nullable/empty playback value has a safe display.
4. Check canvas dimensions against the root CSS dimensions and inspect for
   overflow at the declared size.
5. If working in the nowplaying repository, use the app's theme importer to
   validate the finished file, then open its Preview page. Test long metadata,
   disconnected state, artwork absent/present, and reduced motion where
   applicable. Do not claim OBS sharpness without checking the source at the
   recommended size and at the intended output resolution.
```

## Official profile quick reference

These profiles are design guidance for built-in themes. Third-party themes may
use any valid dimensions, and users may choose any OBS Browser Source size.
nowplaying never changes OBS dimensions automatically.

| Profile | Native dimensions | Design target |
| --- | ---: | --- |
| Slim | 420 × 64 px | Typography-first title and artist, small status, optional 1–2 px progress line; no default artwork. |
| Compact | 420 × 120 px | Essential metadata, optional 88–96 px artwork, restrained progress. |
| Standard | 800 × 150 px | General player with artwork, title, artist, and progress. |
| Wide | 960 × 150 px | Use horizontal space for larger artwork or additional metadata. |
| Ultra-wide | 1000 × 180 px | Cinematic, waveform, visualizer, or extended decorative layouts. |
