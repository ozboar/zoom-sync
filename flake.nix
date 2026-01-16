{
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  outputs =
    { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
    in
    {
      packages.${system}.default = pkgs.callPackage (
        { lib, rustPlatform }:
        rustPlatform.buildRustPackage {
          name = "zoom-sync";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = with pkgs; [
            pkg-config
            addDriverRunpath
          ];
          buildInputs = with pkgs; [
            systemd # for libudev
            openssl # for http request to ipinfo and open-meteo
            gtk3 # for tray icon and file dialogs (includes glib, cairo, pango, etc.)
            libayatana-appindicator # for system tray on Linux
          ];
          # for exposing nvml dynamic library
          fixupPhase = ''addDriverRunpath $out/bin/zoom-sync'';
        }
      ) { };

      devShells.${system}.default = pkgs.mkShell {
        packages = with pkgs; [
          rustfmt
          clippy
          rust-analyzer
        ];
        inputsFrom = [ self.packages.${system}.default ];
        LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [
          pkgs.xdotool
          pkgs.libayatana-appindicator
        ];
      };

      nixosModules.default =
        { config, lib, pkgs, ... }:
        let
          cfg = config.services.zoom-sync;
        in
        {
          options.services.zoom-sync = {
            enable = lib.mkEnableOption "zoom-sync keyboard screen sync service";

            package = lib.mkPackageOption self.packages.${system} "default" { };

            user = lib.mkOption {
              type = lib.types.str;
              description = "User to run zoom-sync and add to input group for reactive mode.";
            };

            extraArgs = lib.mkOption {
              type = lib.types.listOf lib.types.str;
              default = [ ];
              example = [ "--screen" "weather" "--no-system" ];
              description = "Extra arguments to pass to zoom-sync.";
            };
          };

          config = lib.mkIf cfg.enable {
            # Udev rule for accessing the Zoom65 v3 keyboard without root
            services.udev.extraRules = ''
              SUBSYSTEM=="usb", ATTR{idVendor}=="35ef", MODE="0666"
            '';

            # Add user to input group for reactive mode (evdev access)
            users.users.${cfg.user}.extraGroups = lib.mkIf pkgs.stdenv.isLinux [ "input" ];

            systemd.user.services.zoom-sync = {
              description = "Screen module sync for Zoom65 v3 keyboards";
              wantedBy = [ "graphical-session.target" ];
              partOf = [ "graphical-session.target" ];
              after = [ "graphical-session.target" ];
              serviceConfig = {
                ExecStart = "${cfg.package}/bin/zoom-sync ${lib.escapeShellArgs cfg.extraArgs}";
                Restart = "on-failure";
                RestartSec = 5;
              };
            };
          };
        };
    };
}
