# nowplaying

**A lightweight Linux desktop application for bringing your music playback into OBS.**

nowplaying monitors MPRIS media players and serves a customizable browser page for OBS Browser Sources. Manage themes, playback sources, and runtime settings through its desktop interface.

## Features

- **MPRIS integration** — Automatically discovers and monitors media players over session D-Bus.
- **Local HTTP service** — Serves playback state and theme pages through a loopback-only HTTP server.
- **Live updates** — Pushes playback changes to connected browser clients over WebSockets.
- **Custom themes** — Uses single-file TOML themes containing embedded HTML, CSS, JavaScript, and optional assets.
- **Artwork caching** — Caches album artwork when requested by the active theme.
- **Desktop integration** — Includes a graphical interface, system tray integration, and optional autostart.

## OBS Browser Source

Add a **Browser Source** in OBS using the following URL:

```text
http://127.0.0.1:9005/
```

The service listens only on the local machine.

If you change the HTTP port in the application settings, copy the updated URL from the Playback page.

## Installation

### Nix

nowplaying provides a Nix flake supporting `x86_64-linux` and `aarch64-linux`.

**Install into your user profile:**

```bash
nix profile install github:Jade-Switch/nowplaying
```

**Run without installing:**

```bash
nix run github:Jade-Switch/nowplaying
```

#### NixOS

Add nowplaying as an input in your system's `flake.nix`:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

    nowplaying = {
      url = "github:Jade-Switch/nowplaying";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };
}
```

Then expose the flake inputs to your NixOS modules using `specialArgs`:

```nix
outputs = inputs@{ nixpkgs, ... }: {
  nixosConfigurations.snowflake = nixpkgs.lib.nixosSystem {
    system = "x86_64-linux";

    specialArgs = {
      inherit inputs;
    };

    modules = [
      ./configuration.nix
    ];
  };
};
```

In your `configuration.nix`, accept `inputs` and add nowplaying to `environment.systemPackages`:

```nix
{ config, pkgs, inputs, ... }:

{
  environment.systemPackages = [
    inputs.nowplaying.packages.${pkgs.system}.default
  ];
}
```

Rebuild your system:

```bash
sudo nixos-rebuild switch --flake .#snowflake
```

### Flatpak

Tagged GitHub releases include a downloadable `.flatpak` bundle.

Install a downloaded bundle with:

```bash
flatpak install --user ./nowplaying-VERSION.flatpak
```

The bundle uses the Freedesktop 25.08 runtime from Flathub.

Flatpak updates are distributed through new release bundles. Automatic update delivery is not currently configured by this repository.

## Themes

Themes are self-contained TOML files containing their own HTML, CSS, JavaScript, and optional embedded assets.

You can:

- Import an existing `.toml` theme file.
- Paste theme configuration directly into the importer.
- Validate themes before importing them.

Imported themes are stored by the application without modifying the original files. Existing themes with the same ID are protected against accidental overwrites.

For the complete theme specification, browser API documentation, and a working example, see:

**[Theme Format & Browser API](https://github.com/Jade-Switch/nowplaying/blob/main/docs/theme-format.md)**

## Build from Source

### Requirements

- Rust 1.88 or newer.
- Linux GUI development libraries required by Iced.
- A running session D-Bus for desktop integration.

### Clone the repository

```bash
git clone https://github.com/Jade-Switch/nowplaying.git
cd nowplaying
```

### Build

```bash
cargo build --release --locked
```

### Run tests

```bash
cargo test --workspace --locked
```

### Run locally

For development:

```bash
cargo run
```

The optimized release binary is generated at:

```text
target/release/nowplaying
```

## License

The repository does not currently include a license file. All rights are reserved by default unless a license is added.