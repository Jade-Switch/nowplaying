# Development roadmap

Tasks are checked only after implementation and verification. Partially
implemented work remains unchecked.

## Phase 0 — Project foundation and architecture

- [x] Create the new Cargo project/workspace.
- [x] Establish the project directory structure.
- [x] Define supported Linux targets and platform assumptions.
- [x] Establish Rust edition and MSRV policy.
- [x] Select and document minimum necessary dependencies.
- [x] Define error handling conventions.
- [x] Define logging conventions.
- [x] Define application configuration format.
- [x] Define runtime lifecycle.
- [x] Document overall architecture.
- [x] Document module responsibilities.
- [x] Establish CI or reproducible local verification commands.
- [x] Add a basic README.

**Deliverable:** A clean, compiling foundation with documented architecture and no unnecessary features.

## Phase 1 — Playback integration

- [x] Implement MPRIS player discovery.
- [x] Implement player connection and disconnection monitoring.
- [x] Implement automatic player selection.
- [x] Implement manual player selection.
- [x] Define deterministic player priority.
- [x] Normalize MPRIS metadata.
- [x] Implement playback-state monitoring.
- [x] Implement position and duration handling.
- [x] Monitor relevant MPRIS property changes.
- [x] Implement typed playback events.
- [x] Handle player restarts and reconnection.
- [x] Add unit tests for state transitions and player selection.
- [x] Add integration tests using a mock or private D-Bus service.

**Deliverable:** A reliable playback state provider independent of the GUI and renderer.

## Phase 2 — Artwork management

- [x] Implement artwork URL extraction.
- [x] Support local artwork sources.
- [x] Support HTTP(S) artwork sources.
- [x] Implement a local artwork cache.
- [x] Implement content-based deduplication.
- [x] Add sensible cache limits and cleanup.
- [x] Serve cached artwork through the local server.
- [x] Handle invalid artwork and download failures.
- [x] Implement fallback artwork.
- [x] Add tests for cache behaviour and URL validation.

**Deliverable:** Reliable local artwork URLs available to themes.

## Phase 3 — Theme format and loader

- [x] Design the single-file TOML theme specification.
- [x] Define theme metadata and compatibility versioning.
- [x] Define embedded HTML, CSS and JavaScript fields.
- [x] Define embedded asset representation.
- [x] Define canvas and optional layout metadata.
- [x] Define theme validation rules.
- [x] Implement theme discovery.
- [x] Implement theme parsing.
- [x] Implement validation and compatibility checking.
- [x] Implement theme selection and persistence.
- [x] Implement theme import.
- [x] Implement theme export.
- [x] Implement theme refresh.
- [x] Implement safe theme switching.
- [x] Preserve the previous active theme when loading, selection or persistence fails.
- [x] Document the theme format.

**Deliverable:** A complete, documented theme loader capable of loading one self-contained TOML file.

## Phase 4 — Playback API and theme runtime

- [x] Design the stable playback JavaScript API.
- [x] Document API state and event structures.
- [x] Implement initial playback-state delivery.
- [x] Implement event-driven updates in the bounded playback API bridge.
- [x] Implement HTML template rendering.
- [x] Implement embedded CSS handling.
- [x] Implement embedded JavaScript handling.
- [x] Implement embedded asset serving.
- [x] Implement artwork URL integration.
- [x] Expose playback position in snapshots and forward position events.
- [x] Handle browser refresh and reconnection in the browser client.
- [x] Implement theme reload without restarting playback monitoring.
- [x] Add a nonce-based CSP policy to rendered themes.
- [x] Handle rendering errors gracefully.
- [x] Add API and renderer integration tests.

**Deliverable:** A functioning browser theme runtime with a documented playback API.

**Important:** Complete and test the theme format and playback API before starting GUI implementation.

## Phase 5 — Local HTTP and WebSocket service

- [x] Implement loopback-only HTTP server component.
- [x] Implement the main OBS Browser Source endpoint.
- [x] Implement playback state endpoint.
- [x] Implement WebSocket playback updates.
- [x] Implement artwork serving.
- [x] Support multiple simultaneous browser clients.
- [x] Accept a configurable loopback socket address.
- [x] Return bind errors, including port conflicts, to the caller.
- [x] Handle browser disconnects independently.
- [x] Wire server startup and graceful shutdown into the application runtime.
- [x] Ensure theme changes are reflected in connected browsers.
- [x] Add HTTP endpoint and WebSocket transport integration tests.
- [x] Document OBS Browser Source endpoints and configuration.

**Deliverable:** A working local Now Playing source at `http://127.0.0.1:8765/`.

## Phase 6 — Catppuccin Mocha reference theme

- [x] Create `catppuccin-mocha.toml`.
- [x] Place it in the actual application theme directory (bundled and installed on first run).
- [x] Implement its embedded HTML.
- [x] Implement its embedded CSS.
- [x] Implement its embedded JavaScript.
- [x] Use the Catppuccin Mocha palette.
- [x] Implement album artwork.
- [x] Implement metadata display.
- [x] Implement progress display.
- [x] Implement track-change transitions.
- [x] Implement playback-state reactions.
- [x] Implement reduced-motion handling.
- [x] Load it through the real theme loader.
- [x] Serve it through the real HTTP endpoint.
- [x] Verify it in a browser.
- [x] Test it with mock playback data (mock playback event covered by WebSocket integration test).
- [x] Test it with a real MPRIS player.

**Deliverable:** A complete reference theme proving that the format, playback API and runtime work together.

## Phase 7 — Native GUI and design system

Progress note: the initial window shell, theme catalog/select/refresh flow, and
live playback summary are implemented in `src/gui/app.rs`. The first launch
revealed that the Iced event loop must run on the main thread, while Tokio
services run on its worker threads. That startup path is now in place, and the
user confirmed the window appears. Theme/player interactions and layouts at
multiple sizes still need review. TOML theme import now uses an asynchronous
native file picker and updates the catalog after validation. The Playback source
dropdown is wired to the MPRIS provider's automatic/manual selection and
player-discovery watches; artwork thumbnails load through the existing cache.
Rectangular browser-source artwork now uses a contained fit in the reference
theme. The first import-picker launch lacked the runtime `libdbus` path and
zenity fallback, so the dialog returned no selection; the picker now reports
that state in the UI, and the latest GUI launch includes those runtime paths.
Theme cards now offer editor launch and confirmed deletion; deleting the active
theme is blocked. Playback artwork thumbnails use a contained fit for
non-square covers. Theme cards now open a non-activating browser preview with
preview-specific embedded asset routes. The sidebar now has a clipboard action
for the OBS Browser Source URL. Settings edits `server_port` in-app and restarts
the application to apply it, and opens the themes folder in the system file
manager; the redundant config-file opener has been removed. The window has an
800×560 minimum size. The user has confirmed the themes, playback, preview,
URL-copy and port-restart flows; folder opening and layouts at multiple sizes
still need review.

- [x] Establish the smoked-glass lavender design system.
- [x] Define reusable colours, typography, spacing and component styles.
- [x] Implement consistent main window layout.
- [x] Implement navigation.
- [x] Implement Themes view.
- [x] Implement theme preview.
- [x] Implement theme selection.
- [x] Implement theme import.
- [x] Implement theme refresh.
- [x] Implement Playback view.
- [x] Implement MPRIS source selection.
- [x] Display current playback metadata.
- [x] Display artwork thumbnail.
- [x] Display player connection status.
- [x] Display runtime status.
- [x] Display OBS URL and copy action.
- [x] Implement minimal settings menu.
- [x] Ensure background operations do not block the GUI.
- [ ] Verify layouts at different window sizes.
- [ ] Test GUI updates when themes and players change.

**Deliverable:** A polished, compact and functional smoked-glass lavender theme switcher.

## Phase 8 — System tray and application lifecycle

Progress note: the `ksni` StatusNotifierItem starts on Tokio and provides Open
and Quit menu actions. With a registered watcher, window close destroys the
window but keeps the Iced daemon and runtime alive; Open creates a fresh window.
This avoids Wayland's unsupported unminimize operation. Without a watcher, close
exits as usual. Tray close, Open, and Quit behavior has been verified by the user
on KDE Plasma Wayland. Settings includes a startup checkbox that adds or removes
`~/.config/autostart/now-playing-studio.desktop`.

- [x] Implement system tray integration.
- [x] Implement open/show application.
- [x] Implement hide-on-close.
- [x] Keep runtime active while GUI is hidden.
- [x] Implement tray Quit.
- [x] Implement graceful shutdown.
- [x] Implement optional autostart.
- [x] Ensure only one runtime instance is active.
- [ ] Test startup and shutdown behaviour.
- [x] Verify behaviour on KDE Plasma Wayland.

**Deliverable:** A background application that behaves reliably as a desktop utility.

## Phase 9 — NixOS packaging and documentation

Progress note: a Nix flake defines Linux packages and `nix run`, with a
desktop launcher, bundled scalable SVG application icon, and runtime library
paths for the GUI. NixOS installation, desktop session, OBS, and troubleshooting
instructions are in `docs/nixos.md`; system installs use the GitHub flake input
`github:Jade-Switch/nowplaying`. `nix flake metadata` and `nix flake show`
confirm the repository resolves and exports the documented package.
`nix build path:.#now-playing-studio` succeeds. The derivation compiles the
release binary without running the project's tests; clean-install and desktop
runtime verification remain.

- [x] Create a reproducible Nix package.
- [x] Add required runtime dependencies.
- [x] Package application assets.
- [x] Package the reference theme.
- [x] Implement user configuration and theme directory handling.
- [x] Document NixOS installation.
- [x] Document OBS setup.
- [x] Document theme installation and creation.
- [x] Document troubleshooting.
- [ ] Verify a clean installation on NixOS.
- [ ] Verify application startup and tray operation.
- [ ] Verify runtime operation after the GUI is hidden.

**Deliverable:** A reproducible NixOS installation with complete user documentation.

## Phase 10 — Stability and release preparation

- [ ] Run the complete test suite.
- [ ] Run formatting checks.
- [ ] Run Clippy with warnings denied.
- [ ] Verify theme import and export.
- [ ] Verify invalid-theme recovery.
- [ ] Verify player disconnect and reconnect.
- [ ] Verify multiple MPRIS players.
- [ ] Verify artwork cache behaviour.
- [ ] Verify multiple OBS browser clients.
- [ ] Verify runtime shutdown.
- [ ] Verify memory and CPU usage during idle playback.
- [ ] Review dependencies for unnecessary bloat.
- [ ] Review error handling and logging.
- [ ] Review documentation for accuracy.
- [ ] Update version and release notes.

**Deliverable:** A stable, documented initial release.
