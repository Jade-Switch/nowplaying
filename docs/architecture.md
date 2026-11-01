# Architecture and platform assumptions

## Initial scope

The project starts as one Rust package and one application process. The
runtime, local HTTP/WebSocket service, MPRIS integration and Iced GUI will be
components in that process. Modules will be extracted only when a real
responsibility or dependency boundary warrants it. The Phase 0 foundation added
no runtime dependencies; Phase 1 adds only the MPRIS, async-runtime and logging
dependencies required by the playback provider.

The target is Linux, initially NixOS with KDE Plasma 6 on Wayland. NixOS is a
packaging target, not a runtime dependency. The core should use portable Rust
interfaces where practical; desktop integration belongs at the application
edge.

Application settings will use TOML, separate from theme files. Missing settings
will use validated defaults; malformed files must be preserved while the
application reports the problem and recovers with defaults. Updates should use
a temporary file followed by an atomic replacement where the platform permits.

Errors will use typed results at component boundaries and retain context when
propagated. Expected external failures (such as a player disconnect or bad
theme) should be reported and isolated rather than silently discarded.
`tracing` is the planned structured logging API: runtime diagnostics should
include component/event context, avoid logging artwork contents or secrets,
and use an adjustable verbosity setting. These conventions will be applied
when runtime components are introduced.

## Runtime boundaries

- **Core state** will own shared, typed playback state, player identity,
  configuration and runtime errors without depending on GUI or HTTP types.
- **MPRIS** will discover and monitor players over the session D-Bus and publish
  normalized state/events. Automatic selection will have a documented stable
  priority; manual selection will be persisted separately.
- **Artwork** will validate local and HTTP(S) sources, enforce size/cache bounds,
  and expose cached content through opaque local URLs rather than filesystem
  paths.
- **Themes** will be single TOML files containing metadata, HTML, CSS,
  JavaScript and optional explicitly typed embedded resources. The runtime will
  validate size and compatibility before activation and preserve the last
  working theme on failure.
- **HTTP/WebSocket** will bind to loopback by default, serve the active theme,
  a small read-only playback API and cached artwork, and isolate slow browser
  clients from playback monitoring.
- **Desktop UI** will be a small Iced application for theme/source selection,
  status, the OBS URL and basic settings. The OBS page owns its visual layout;
  the native UI does not become a theme editor.

Theme JavaScript is untrusted browser content. It will receive only the public
playback interface, with no native command or filesystem bridge. A restrictive
Content Security Policy will be used where compatible with embedded content.
Loopback binding is not treated as a complete browser security boundary.

## Lifecycle

The intended single-process startup sequence is: load and validate
configuration; discover/load the selected theme; start MPRIS monitoring and
the local server; then show the GUI. These components should fail independently
where possible. Closing the GUI should hide it while the runtime remains active;
explicit Quit should stop background tasks and close the server cleanly.

Async work must not block the GUI thread. Channels and per-client update queues
will be bounded, and position updates will use a modest cadence. Theme changes
must not restart MPRIS monitoring.

Iced owns the process main thread because winit requires its event loop there.
The Tokio multi-thread runtime is started explicitly; its worker tasks keep
MPRIS and the local server running while Iced handles window events. Returning
from the window loop lets the main thread await orderly runtime shutdown.

## Initial technology choices

The specification's initial choices remain the working direction:

| Concern | Direction | Phase 0 decision |
| --- | --- | --- |
| Language | Rust | Edition 2024; proposed MSRV 1.88 |
| GUI | Iced 0.14 | Tokio integration, Wayland and the tiny-skia software renderer; keep compositor blur optional |
| D-Bus/MPRIS | zbus | zbus 5.19 with its Tokio feature; default async-io and blocking API features are disabled |
| Remote artwork | Reqwest | Reqwest 0.13.5 with rustls and streamed bodies; default features are disabled |
| HTTP/WebSocket | Axum 0.8.9 | Loopback HTTP endpoints and WebSocket playback events |
| Async runtime | Tokio | Add when concurrent runtime components begin |
| Serialization/config | serde, TOML, semver and Base64 | Theme manifests, compatibility checks and embedded assets |
| Logging | tracing | Add with runtime startup and structured diagnostics |
| Theme content | HTML, CSS, JavaScript | Browser-native; no frontend framework planned |
| Desktop tray | ksni 0.3.7 | Freedesktop/KDE StatusNotifierItem over D-Bus with Tokio support |

Dependencies are intentionally added only when a phase uses them. This keeps
the build lean and makes each dependency's compatibility and feature selection
reviewable alongside its first use. Before adding a crate, check its current
MSRV, platform support, maintenance and feature graph, then lock the resolved
versions.

## Linux desktop constraints

Iced does not provide a built-in system tray API. Plasma's tray supports
StatusNotifierItem through D-Bus, while legacy X11 tray mechanisms are not an
appropriate Wayland baseline. `ksni` implements StatusNotifierItem on the
application's existing Tokio runtime. When the tray watcher is available,
closing destroys the window while the Iced daemon keeps the runtime alive; the
tray's Open action creates a fresh window. This avoids relying on Wayland
unminimize, which winit does not support. Without a tray watcher, close exits
normally. Compositor blur remains platform-dependent, and the GUI keeps a
readable opaque/layered fallback when blur is absent.

## Verification policy

The intended quality gates are formatting, locked workspace check, locked
workspace tests and Clippy with warnings denied. Platform-specific tray,
Wayland and real-player checks must be documented when they cannot run in CI.
Phase tasks are checked in `TODO.md` only after implementation and verification.
