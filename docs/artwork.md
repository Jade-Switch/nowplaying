# Artwork cache

`ArtworkCache` accepts the artwork URI normalized from MPRIS `mpris:artUrl` and
returns an opaque content ID plus a stable `/artwork/{id}` path for the future
HTTP service. The cache is independent of the MPRIS monitor; callers can
resolve a track's artwork when handling a track change.

## Inputs and limits

- Local absolute paths and `file:` URIs are accepted only when their canonical
  target is a regular file below an explicitly allowed root. By default the
  user's home directory is the allowed root. Symlinks are resolved before the
  root check.
- `http:` and `https:` are supported on their standard ports. User information,
  fragments, redirects, proxy use, and private/reserved IP destinations are
  rejected. DNS results are checked and pinned for the request to prevent
  rebinding between validation and connection.
- Downloads have a 5 second connection timeout, a 15 second total timeout, and
  an 8 MiB default body limit. The response stream is stopped as soon as it
  exceeds the limit.
- Supported artwork formats are PNG, JPEG, GIF, BMP, ICO, WebP and AVIF. SVG
  and other active or unrecognized formats are rejected. Validation checks
  format signatures; the browser remains responsible for decoding the image.
- The default cache holds at most 256 entries and 64 MiB. Oldest-used content
  is evicted to stay within both bounds. Files are named by SHA-256 content ID,
  so identical image bytes share one cached file. Cache contents survive
  process restart; corrupt or unsupported cache entries are ignored.

The cache path should be a dedicated application directory. `read()` accepts
only validated content IDs (or the built-in `fallback` ID), not filesystem
paths. `resolve_or_fallback()` returns a bundled, script-free SVG when artwork
is missing or cannot be loaded.

This phase does not start the HTTP server. Its path format is ready for the
server's artwork route in Phase 5; until then `ArtworkCache::read()` provides
the cached bytes to that route.
