# Linux packaging and releases

The executable is `now-playing-studio`. The project uses Iced with its
Wayland backend, reads MPRIS over the session D-Bus, registers a StatusNotifier
tray item, and serves the OBS page and playback API on loopback. The default
server address is `http://127.0.0.1:9005/`.

The repository currently has no license file or SPDX license metadata. The
Flatpak ID below is derived from the GitHub repository as
`io.github.Jade-Switch.nowplaying`; it should be kept stable once published.
These bundles can be built and attached to GitHub Releases, but the project
still needs an explicit license before submission to package stores that
require one. “Open source” describes a category; it does not identify a
license.

## Nix and Cachix

The existing flake remains the source of the Nix package definition:

```sh
nix build .#now-playing-studio
nix run github:Jade-Switch/nowplaying
```

The package version is read from `Cargo.toml`. GitHub Actions builds the
x86_64 package on pull requests and pushes to `main`; trusted main builds and
version tags also publish newly built store paths to Cachix when the repository
cache is configured. Cachix's GitHub Action installs a Nix post-build hook, so
paths built by the job are uploaded as they are produced.

Set the GitHub repository variable `CACHIX_CACHE_NAME` to the actual Cachix
cache name and add a per-cache write token as the `CACHIX_AUTH_TOKEN` Actions
secret. The workflow never passes the write token to pull request jobs. If the
cache name is unset, CI still builds the Nix package and reports that upload
was skipped. If a cache name is set without a write token, the trusted build
fails with a configuration error.

For Nix to substitute the prebuilt package when using `nix run`, configure the
cache as a trusted substituter. This flake declares the public cache URL and
signing key; accept the flake's Nix configuration when prompted. With the
Cachix CLI:

```sh
cachix use nowplaying
```

On NixOS, add the cache URL to `nix.settings.substituters` and its public key
to `nix.settings.trusted-public-keys`, then rebuild:

```nix
nix.settings = {
  substituters = [ "https://nowplaying.cachix.org" ];
  trusted-public-keys = [
    "nowplaying.cachix.org-1:InttALbGwOhUqNj6k1qBYVwuS/xsAxoGI0gRMCy54eI="
  ];
};
```

The public signing key is distinct from the private `CACHIX_AUTH_TOKEN`.

## Flatpak

`packaging/flatpak/io.github.Jade-Switch.nowplaying.yml` builds from this
checkout using the Freedesktop 25.08 runtime and its Rust SDK extension. The
sandbox shares networking so OBS on the host can reach the loopback service
and so remote artwork URLs can load. It has Wayland access and the specific
session-bus names needed for MPRIS, the file-picker portal, and KDE's tray
watcher. The app uses the Flatpak ID as its Wayland application ID so KDE can
match the exported desktop entry and icon. It does not request general host
filesystem or device access.

Install a built bundle:

```sh
flatpak install --user ./Now-Playing-Studio-v0.1.0.flatpak
flatpak run io.github.Jade-Switch.nowplaying
```

To build locally, install Flatpak, Flatpak Builder, and the Freedesktop
25.08 Platform, SDK, and Rust extension, then run:

```sh
python3 packaging/flatpak/generate-cargo-sources.py
flatpak-builder --force-clean --repo=repo build-dir \
  packaging/flatpak/io.github.Jade-Switch.nowplaying.yml
flatpak build-bundle repo now-playing-studio.flatpak \
  io.github.Jade-Switch.nowplaying stable
```

The generator uses checksums already recorded in `Cargo.lock` to create
Flatpak Builder's offline Cargo source list. Regenerate it after changing Rust
dependencies; the generated file is ignored by Git.

## Snap

The Snap uses the `core24` base and strict confinement. It requests desktop
and Wayland integration, outbound network access, and permission to bind the
loopback playback/OBS service. The MPRIS interface does not auto-connect on
Snap, so connect it after installation if the system has not already done so:

```sh
sudo snap install --dangerous ./now-playing-studio_0.1.0_amd64.snap
sudo snap connect now-playing-studio:mpris
snap run now-playing-studio
```

Build on Ubuntu 24.04 with Snapcraft installed:

```sh
cd packaging/snap
snapcraft pack --destructive-mode
```

The Snap Store name `now-playing-studio` must be available and claimed before
store publication. The workflow only attaches the built `.snap` to GitHub
Releases; it does not upload to the Snap Store.

## GitHub Actions and releases

`.github/workflows/ci.yml` runs formatting, Clippy, Rust tests, and builds or
validates the Nix, Flatpak, and Snap packages for pull requests and pushes to
`main`. The Cachix write action runs only for trusted pushes to `main`.

`.github/workflows/release.yml` runs on tags matching `v*`. It builds Nix,
Flatpak, and Snap in separate jobs. Only after all three succeed does it create
a draft GitHub Release, attach the `.flatpak`, `.snap`, and `SHA256SUMS`, then
publish the release. Nix store paths are published to Cachix by the Nix build
job. GitHub's `GITHUB_TOKEN` is granted `contents: write` only to the final
publishing job.

Before enabling these workflows, configure:

- Actions secret `CACHIX_AUTH_TOKEN` with a Cachix write token.
- Actions variable `CACHIX_CACHE_NAME` with the cache's real name.
- A public Cachix signing key for users' NixOS substituter configuration; it
  is safe to publish, but is not present in this repository yet.
- A project license before applying to stores that require license metadata.

The release tag must match the Rust package version. Keep the versions in
`Cargo.toml`, `flake.nix`, and `packaging/snap/snapcraft.yaml` aligned before
tagging. For example, version `0.2.0` should be released from tag `v0.2.0`.
The Flatpak bundle uses the tag in its filename and the stable Flatpak branch.

Build artifacts locally with `nix build .#now-playing-studio`, the Flatpak
commands above, and `snapcraft pack --destructive-mode`. GitHub Releases are
created only after each package job succeeds.
