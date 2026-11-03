# Now Playing Studio

A lightweight, theme-driven Now Playing application for Linux, built around MPRIS and OBS Browser Sources.

Now Playing Studio monitors local media players, collects playback information and artwork, and serves it through a local web interface. OBS can use that interface as a Browser Source, while the desktop application provides a simple way to manage themes and playback sources.

The application is designed to keep the runtime relatively small and predictable while leaving the visual presentation to themes.

## Features

- **MPRIS integration** — discover and monitor compatible media players on Linux.
- **OBS Browser Source** — display playback information through a local HTTP endpoint.
- **Live updates** — keep track information, playback state, position, and artwork synchronised.
- **Single-file themes** — each theme is distributed as one self-contained TOML file containing its HTML, CSS, JavaScript, and embedded assets.
- **Custom presentation** — themes control their own layout, styling, and animations.
- **Artwork handling** — retrieve and cache album artwork for display.
- **Desktop interface** — manage installed themes, select playback sources, and access runtime settings.
- **System tray integration** — keep the runtime running without leaving the main window open.
- **Linux packages** — Nix, Flatpak, and Snap builds are produced by GitHub Actions.

## Themes

Themes are intentionally independent of the Rust application's layout.

Each theme is a single `.toml` file. It contains its own HTML, CSS, JavaScript, configuration, and optional embedded assets. No theme-specific folders, external stylesheets, or archives are required.

This makes themes easy to share, back up, and install without modifying the application itself.

The runtime exposes playback information through a documented JavaScript API, allowing themes to handle their own presentation and behaviour.

See [Themes and the browser playback API](docs/theme-format.md) for the file
format, playback data, and a working example.

## OBS

Now Playing Studio provides a local endpoint that can be added to OBS as a Browser Source.

Default address:

`http://127.0.0.1:9005/`

The displayed content is controlled by the currently selected theme. The local runtime handles playback information and updates independently of the OBS browser window.

## Technology

- Rust
- Iced
- MPRIS / D-Bus
- Tokio
- Axum
- WebSockets
- TOML
- HTML, CSS, and JavaScript

## Building

A Rust toolchain with Cargo is required.

Clone the repository:

```bash
git clone https://github.com/Jade-Switch/nowplaying.git
cd nowplaying
```

Build the application:

```bash
cargo build --release
```

Run the test suite:

```bash
cargo test --workspace --locked
```

For development, you can use:

```bash
cargo build
```

Linux systems require a working D-Bus session and compatible MPRIS media players for playback integration.

## How to install with Nix

To install it into your user profile, so KDE can index its desktop launcher, run:

```sh
nix profile install github:Jade-Switch/nowplaying
```

You can also run the latest version directly from GitHub without adding it to
your profile:

```sh
nix run github:Jade-Switch/nowplaying
```

On NixOS with KDE Plasma, enable the desktop portal backend for the Import
Theme file picker:

```nix
xdg.portal = {
  enable = true;
  extraPortals = [ pkgs.kdePackages.xdg-desktop-portal-kde ];
};
```

Rebuild and log in to the graphical session again after enabling it. The app
expects a logged-in desktop session with session D-Bus. KDE also supplies the
StatusNotifier watcher used for the tray icon. MPRIS players must be running in
the same session. Without a tray watcher, closing the app window exits it.

To include the app in a NixOS flake configuration, add it as an input:

```nix
inputs.now-playing-studio = {
  url = "github:Jade-Switch/nowplaying";
  inputs.nixpkgs.follows = "nixpkgs";
};
```

Then add it to your package list:

```nix
environment.systemPackages = [
  inputs.now-playing-studio.packages.${pkgs.system}.default
];
```

Rebuild with `sudo nixos-rebuild switch --flake /etc/nixos#<hostname>`. The
package includes a desktop launcher and icon. Its Settings startup checkbox
manages an XDG autostart entry in `~/.config/autostart`.

The OBS service listens on loopback (default `127.0.0.1:9005`) and is intended
for OBS on the same machine. Copy the URL from the Playback page if you changed
the port in Settings.

## Project structure

The project is organised into separate components for playback integration, application state, theme handling, the local server, and the desktop interface.

The intention is to keep these responsibilities separated without introducing unnecessary complexity into the runtime.

## Documentation

- [Themes and the browser playback API](docs/theme-format.md)
- [Packaging and automated releases](docs/packaging.md)

## License

This repository does not currently include a license file.
