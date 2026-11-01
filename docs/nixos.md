# NixOS installation

The flake builds Now Playing Studio for `x86_64-linux` and `aarch64-linux`.
It includes the reference theme, a desktop launcher and scalable app icon,
and the Wayland/XKB runtime libraries needed by the GUI. Theme files and settings remain in the
user's standard XDG directories; the package does not write into the Nix store.

## Install from this checkout

Build and launch with:

```sh
nix run .
```

To install it into a NixOS system configuration, add the checkout as a flake
input in `flake.nix`:

```nix
inputs.now-playing-studio.url = "path:/path/to/nowplaying";
```

Then add the package to the relevant system or user package list:

```nix
environment.systemPackages = [
  inputs.now-playing-studio.packages.${pkgs.system}.default
];
```

Run `sudo nixos-rebuild switch --flake /etc/nixos#<hostname>` and launch **Now
Playing Studio** from the desktop application menu, or run
`now-playing-studio` from a terminal. For a remote source checkout, replace the
`path:` input with its Git URL.

## Desktop session requirements

The app expects a logged-in graphical session with a session D-Bus. KDE Plasma
provides the StatusNotifier tray watcher and XDG desktop portal used by the app.
The tray's Open and Quit actions work when the watcher is available; without a
tray watcher, closing the window exits the app. MPRIS player discovery also
uses the session bus. Ensure the media player publishes an MPRIS interface for
Now Playing Studio to detect it.

The package adds `xdg-utils` to the app's executable search path for opening
theme files, folders, and browser previews. The desktop file uses the bundled
Now Playing Studio icon. The Settings page's startup checkbox
adds or removes an XDG autostart entry under `~/.config/autostart`.

## OBS Browser Source

Start the app and copy its OBS URL from the Playback page. The default is
`http://127.0.0.1:8765/`; the port can be changed in Settings. Add an OBS
Browser Source using that URL. The service listens on loopback and is intended
for OBS running in the same user session or machine.

## Troubleshooting

- If no players appear, check that the player is running in the same user
  session and exports MPRIS over the session D-Bus.
- If Import Theme cannot open a picker, check the desktop portal service and
  its file chooser backend for the current desktop environment.
- If the tray icon is missing, check whether the desktop provides a
  StatusNotifier watcher. Closing the window without one exits the app.
- If OBS cannot connect, confirm that the service port matches the Browser
  Source URL and is not already in use.

## Build verification

From the repository root, use:

```sh
nix build .#now-playing-studio
```

The resulting executable is available at `result/bin/now-playing-studio`.
