# Now Playing Studio

Now Playing Studio is a local Rust application for monitoring MPRIS media
players and serving customizable Now Playing themes to OBS Browser Source.
Runtime features are tracked in [`TODO.md`](TODO.md).

## Current status

The current implementation monitors MPRIS players, publishes typed playback
snapshots/events, loads and manages self-contained TOML themes, and serves a
Catppuccin Mocha browser source through the loopback HTTP/WebSocket service.
The first Iced GUI shell now includes theme selection, refresh, TOML import,
system-editor launch, confirmed theme deletion and non-activating browser
previews, plus a live playback summary with MPRIS source selection and contained
artwork thumbnails. The sidebar displays and copies the OBS source URL. GUI
settings include an in-app server-port field that saves and restarts the
application, plus a shortcut to the themes folder. The app includes the
verified tray lifecycle, a single-instance lock, and a startup checkbox. The
Nix package includes the scalable app icon and desktop launcher. See
[`TODO.md`](TODO.md).

Run `cargo run` to start the desktop window and local OBS source at
`http://127.0.0.1:8765/`. On first run, the app installs and selects the bundled
Catppuccin Mocha theme and starts automatic MPRIS monitoring when a session
D-Bus is available. If D-Bus is unavailable, the server still starts with a
disconnected playback snapshot. With a registered StatusNotifier tray, closing
the window leaves the app running in the tray; **Open Now Playing Studio**
creates a fresh window, and **Quit** shuts down the runtime. Without a tray
watcher, closing the window exits normally. Pressing `Ctrl+C` also shuts down
the runtime. Set `RUST_LOG` to adjust structured log output. The server port
can be changed with `server_port` in `settings.toml`.

## Development

Requirements: Rust 1.88 or newer and Cargo. Linux is the initial target; the
runtime core is intended to remain independent of NixOS and KDE-specific APIs.
The private D-Bus integration test also requires `dbus-daemon`.

```sh
cargo fmt --all -- --check
cargo check --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

The workspace flags are kept in the project quality-gate commands so they
continue to work if the project becomes a Cargo workspace. For now Cargo treats
the root package as a one-member workspace.

## Documentation

- [Architecture and platform assumptions](docs/architecture.md)
- [Playback state and player selection](docs/playback.md)
- [Browser playback API](docs/playback-api.md)
- [Local OBS HTTP/WebSocket server](docs/server.md)
- [Artwork cache and source policy](docs/artwork.md)
- [Theme file format](docs/theme-format.md)
- [Desktop GUI design system](docs/gui-design.md)
- [NixOS installation and packaging](docs/nixos.md)
- [Development roadmap](TODO.md)
