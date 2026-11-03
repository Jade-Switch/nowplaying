{
  description = "nowplaying desktop application and OBS source runtime";

  nixConfig = {
    extra-substituters = [ "https://nowplaying.cachix.org" ];
    extra-trusted-public-keys = [
      "nowplaying.cachix.org-1:InttALbGwOhUqNj6k1qBYVwuS/xsAxoGI0gRMCy54eI="
    ];
  };

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in {
      packages = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
          version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
          runtimeLibraries = with pkgs; [ libxkbcommon wayland libGL ];
          app = pkgs.rustPlatform.buildRustPackage {
            pname = "nowplaying";
            inherit version;
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;

            # Package builds compile the binary; project tests are run separately.
            doCheck = false;
            buildInputs = runtimeLibraries;
            nativeBuildInputs = with pkgs; [ makeWrapper pkg-config copyDesktopItems ];
            desktopItems = [ (pkgs.makeDesktopItem {
              name = "nowplaying";
              desktopName = "nowplaying";
              comment = "MPRIS player monitor and OBS browser source";
              exec = "nowplaying";
              icon = "nowplaying";
              terminal = false;
              categories = [ "AudioVideo" "Audio" "Player" ];
            }) ];

            postInstall = ''
              install -Dm644 ${./assets/nowplaying.svg} \
                $out/share/icons/hicolor/scalable/apps/nowplaying.svg
              wrapProgram $out/bin/nowplaying \
                --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.xdg-utils pkgs.kdePackages.kdialog ]} \
                --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath runtimeLibraries}
            '';

            meta = {
              description = "MPRIS player monitor and customizable OBS browser source";
              mainProgram = "nowplaying";
              platforms = systems;
            };
          };
        in {
          default = app;
          nowplaying = app;
        });

      apps = forAllSystems (system: {
        default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/nowplaying";
        };
      });
    };
}
