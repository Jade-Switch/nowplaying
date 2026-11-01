{
  description = "Now Playing Studio desktop application and OBS source runtime";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in {
      packages = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
          runtimeLibraries = with pkgs; [ libxkbcommon wayland libGL ];
          app = pkgs.rustPlatform.buildRustPackage {
            pname = "now-playing-studio";
            version = "0.1.0";
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;

            # Package builds compile the binary; project tests are run separately.
            doCheck = false;
            buildInputs = runtimeLibraries;
            nativeBuildInputs = with pkgs; [ makeWrapper pkg-config copyDesktopItems ];
            desktopItems = [ (pkgs.makeDesktopItem {
              name = "now-playing-studio";
              desktopName = "Now Playing Studio";
              comment = "MPRIS player monitor and OBS browser source";
              exec = "now-playing-studio";
              icon = "now-playing-studio";
              terminal = false;
              categories = [ "AudioVideo" "Audio" "Player" ];
            }) ];

            postInstall = ''
              install -Dm644 ${./assets/now-playing-studio.svg} \
                $out/share/icons/hicolor/scalable/apps/now-playing-studio.svg
              wrapProgram $out/bin/now-playing-studio \
                --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.xdg-utils ]} \
                --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath runtimeLibraries}
            '';

            meta = {
              description = "MPRIS player monitor and customizable OBS browser source";
              mainProgram = "now-playing-studio";
              platforms = systems;
            };
          };
        in {
          default = app;
          now-playing-studio = app;
        });

      apps = forAllSystems (system: {
        default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/now-playing-studio";
        };
      });
    };
}
