# zoom-sync

Cross-platform utility to sync Meletrix Zoom Keyboard screen modules.

## Features

> Note: All features marked "simulated" are not supported by the screen firmware natively, but rather achieved by the zoom-sync process.

|                     | zoom-sync              | MeletrixID / WuqueID            |
| ------------------- | ---------------------- | ------------------------------- |
| Supported platforms | Cross-platform         | Windows, OSX                    |
| FOSS ?              | FOSS. Always.          | Free, but not open sourced      |
| Languages           | English                | Chinese or English              |
| Weather API         | [open-meteo](https://open-meteo.com) | Unknown centralized service |
| Geolocation API     | [ipinfo](https://ipinfo.io) or manual | Bundled into weather api |
| VPN workaround      | Manual geo coordinates | Not supported                   |
| Temperature units   | °C or simulated °F     | °C only                         |
| Time sync           | Supported              | Supported                       |
| 12hr time           | Simulated              | Not supported                   |
| CPU temperature     | Supported              | Supported                       |
| GPU temperature     | Nvidia only            | Supported                       |
| Download rate       | Manual only            | Supported                       |
| Manually set data   | Supported              | Not supported                   |
| Image/gif upload    | Supported w/ custom bg | Not supported (use web driver)  |
| Reactive image/gif  | Simulated              | Not supported                   |
| Future-proof        | Will always work       | Overflow errors after year 2255 |

## Supported Boards

| Feature             | Zoom65 V3          | Zoom TKL Dyna          |
| ------------------- | ------------------ | ---------------------- |
| Screen size         | 110x110            | 320x172                |
| Time sync           | Yes                | Yes                    |
| Weather             | Yes                | Yes                    |
| System info         | Yes (CPU/GPU/DL)   | Needs research         |
| Screen control      | Yes                | Not fully implemented  |
| Image upload        | Yes                | Yes                    |
| GIF upload          | Yes                | Yes                    |
| Theme customization | Yes (Presets)      | Yes (Colors + Presets) |
| 12hr time format    | Yes (simulated)    | No                     |

## Third Party Services

The following free third-party services are used to fetch some information:

- Weather forcasting: [open-meteo](https://open-meteo.com)
- Geolocation (optional for automatic weather coordinates): [ipinfo.io](https://ipinfo.io)

## Installation

> See the [latest release notes](https://github.com/ozboar/zoom-sync/releases/latest) for pre-built windows and linux binaries

Build requirements:

- rust/rustup
- openssl
- libudev (linux only, included with systemd)

### Source

```bash
git clone https://github.com/ozwaldorf/zoom-sync && cd zoom-sync
cargo install --path .
```

### Crates.io

```bash
cargo install zoom-sync
```

### Nix

> Note: On NixOS, you must use the flake for nvidia gpu temp to work

```bash
nix run github:ozwaldorf/zoom-sync
```

#### NixOS Module

The flake provides a NixOS module for running zoom-sync as a user service:

```nix
{
  inputs.zoom-sync.url = "github:ozwaldorf/zoom-sync";

  outputs = { nixpkgs, zoom-sync, ... }: {
    nixosConfigurations.myhost = nixpkgs.lib.nixosSystem {
      modules = [
        zoom-sync.nixosModules.default
        {
          services.zoom-sync = {
            enable = true;
            user = "myuser";
            # extraArgs = [ "--screen" "weather" ];
          };
        }
      ];
    };
  };
}
```

## Usage

### CLI

Detailed command line documentation can be found in [docs/README.md](./docs/README.md).

### Running on startup

#### Linux / systemd

A systemd user service can be set up to run zoom-sync with your graphical session.
An example can be found at [docs/zoom-sync.service](./docs/zoom-sync.service).

```bash
# copy to user services
mkdir -p ~/.config/systemd/user
cp docs/zoom-sync.service ~/.config/systemd/user/

# enable and start the service
systemctl --user enable --now zoom-sync.service
```

#### Windows

1. Press Windows + R and enter `%userprofile%\.cargo\bin` to open the install location
2. Right-click `zoom-sync.exe` and select "Create shortcut"
3. Press Windows + R and enter `shell:startup` to open the startup folder
4. Move the shortcut to the startup folder

#### OSX

> TODO

### Simple examples

```bash
# Run the tray application (default)
zoom-sync

# Set weather using coordinates (skips ipinfo geolocation)
zoom-sync set weather --coords 27.1127 109.3497

# Set weather manually (wmo code, current, min, max)
zoom-sync set weather -w 0 10 20 5

# Set system temps in fahrenheit
zoom-sync set system -f

# Change the current screen
zoom-sync set screen -s weather
zoom-sync set screen -s cpu

# Upload a custom image or gif
zoom-sync set image my-image.png
zoom-sync set gif my-anim.gif

# Clear image and gif back to the defaults
zoom-sync set image clear
zoom-sync set gif clear

# Sync time to system clock
zoom-sync set time
```

## Feature Checklist

- [x] Reverse engineer updating each value
  - [x] Time
  - [x] Weather (current, min, max)
  - [x] CPU/GPU temp
  - [x] Download rate
  - [x] Screen up/down/switch
  - [x] GIF image
  - [x] Static image
- [x] Fetch current weather report
- [x] Fetch CPU temp
- [x] Fetch GPU temp
  - [x] Nvidia
  - [ ] AMD
- [ ] Monitor download rate
- [x] Poll and reconnect to keyboard
- [x] CLI arguments
- [x] Update intervals for each value
- [x] Simulate reactive gif mode (linux)
- [x] System tray menu
- [ ] Package releases
  - [x] Crates.io
  - [ ] Nixpkgs
  - [ ] Windows
  - [ ] OSX
