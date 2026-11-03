# nowplaying

nowplaying is a Linux desktop app that reads playback data from MPRIS media
players and serves a customizable page for OBS Browser Sources. Its desktop
interface manages themes, playback sources, and runtime settings.

## Features

- Discovers and monitors MPRIS players over the session D-Bus.
- Serves playback state and theme pages over a loopback-only HTTP service.
- Updates connected browser clients over WebSockets.
- Loads single-file TOML themes with embedded HTML, CSS, JavaScript, and assets.
- Caches artwork when the selected theme requests it.
- Provides a desktop interface, system tray integration, and optional autostart.

## OBS

Add a Browser Source with this default URL:

```text
http://127.0.0.1:9005/
```

The service listens only on the local machine. If you change its port in the
app's Settings, copy the updated URL from the Playback page.

## Install

### Nix

Install into your user profile:

```sh
nix profile install github:Jade-Switch/nowplaying
```

Or run without installing:

```sh
nix run github:Jade-Switch/nowplaying
```

The flake supports `x86_64-linux` and `aarch64-linux`.

For NixOS, add the flake as an input:

```nix
inputs.nowplaying = {
  url = "github:Jade-Switch/nowplaying";
  inputs.nixpkgs.follows = "nixpkgs";
};
```

Then add `inputs.nowplaying.packages.${pkgs.system}.default` to
`environment.systemPackages` and rebuild your system.

### Flatpak

Tagged GitHub releases include a `.flatpak` bundle. Install a downloaded bundle
with:

```sh
flatpak install --user ./nowplaying-VERSION.flatpak
```

The bundle uses the Freedesktop 25.08 runtime from Flathub. Flatpak updates are
installed through a new release bundle; automatic update delivery is not
configured by this repository.

## Themes

Themes are TOML files containing their own HTML, CSS, JavaScript, and optional
embedded assets. Import a file or paste its contents on the Themes page. The
app validates the theme before storing it and does not overwrite another theme
with the same ID.

See [the theme format and browser API](docs/theme-format.md) for the schema,
playback data, and a working example.

## Build from source

Requires Rust 1.88 or newer, the Linux GUI development libraries for Iced, and a
session D-Bus for desktop integration.

```sh
git clone https://github.com/Jade-Switch/nowplaying.git
cd nowplaying
cargo build --release --locked
cargo test --workspace --locked
```

Run the development build with `cargo run`. The release binary is
`target/release/nowplaying`.

## License

The repository does not currently include a license file.
