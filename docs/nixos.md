# NixOS installation

The flake builds Now Playing Studio for `x86_64-linux` and `aarch64-linux`.
It includes a desktop launcher and scalable app icon.

## Run directly from GitHub

```sh
nix run github:Jade-Switch/nowplaying
```

This starts the app but does not install its desktop entry into KDE's app
search. To make the app and its icon available there, install it into your
profile:

```sh
nix profile install github:Jade-Switch/nowplaying
```

Then launch **Now Playing Studio** from KDE's application launcher. The desktop
entry refers to the bundled `now-playing-studio` icon.

## Install in a NixOS configuration

Add the GitHub repository as a flake input. Following the system's nixpkgs
input keeps both flakes on the same package set:

```nix
inputs.now-playing-studio = {
  url = "github:Jade-Switch/nowplaying";
  inputs.nixpkgs.follows = "nixpkgs";
};
```

Then add the package to the relevant system or user package list:

```nix
environment.systemPackages = [
  inputs.now-playing-studio.packages.${pkgs.system}.default
];
```

Run `sudo nixos-rebuild switch --flake /etc/nixos#<hostname>` and launch **Now
Playing Studio** from the desktop application menu, or run
`now-playing-studio` from a terminal. Nix fetches the source from GitHub and
records the selected repository revision in the system flake lock file.

## Desktop session requirements

The app expects a logged-in graphical session with a session D-Bus. The Import
Theme button uses the XDG desktop portal file chooser. For KDE Plasma on NixOS,
enable the KDE portal backend in the system configuration:

```nix
xdg.portal = {
  enable = true;
  extraPortals = [ pkgs.kdePackages.xdg-desktop-portal-kde ];
};
```

Rebuild and log in to the graphical session again after enabling it. KDE Plasma
also provides the StatusNotifier tray watcher used by the app.
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

Build from GitHub with:

```sh
nix build github:Jade-Switch/nowplaying#now-playing-studio
```

The resulting executable is available at `result/bin/now-playing-studio`.
For local development from a checkout, use `nix build .#now-playing-studio`.
