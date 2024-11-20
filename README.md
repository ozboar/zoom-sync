# zoom-sync

Cross-platform utility to sync Zoom65 v3 screen modules.

## Comparison

|                     | zoom-sync        | MeletrixID                      |
| ------------------- | ---------------- | ------------------------------- |
| Supported platforms | cross-platform   | Windows, OSX                    |
| FOSS ?              | FOSS. Always.    | Free, but not open sourced      |
| Languages           | English          | Chinese or english              |
| Weather api         | [open-meteo](https://open-meteo.com) | Unknown centralized service |
| Geolocation api     | [ipinfo](https://ipinfo.io) or manual | Bundled into weather api |
| VPN workaround      | With manual geo  | Only uses vpn's ip for location |
| Temperature units   | °C or °F         | °C only                         |
| Time sync           | Supported        | Supported                       |
| CPU temperature     | Supported        | Supported                       |
| GPU temperature     | Nvidia           | Supported ?                     |
| Download rate       | WIP              | Supported                       |
| Manual data         | Supported        | Not supported                   |
| Single update mode  | Supported        | Not supported                   |
| Future-proof        | Will always work | Overflow errors after year 2255 |

## Third Party Services

The following free third-party services are used to fetch some information:

- Weather forcasting: [open-meteo](https://open-meteo.com)
- Geolocation (optional for automatic weather coordinates): [ipinfo.io](https://ipinfo.io)

## Installation

### Source

Requirements:

- libudev (linux, included with systemd)
- openssl

```
git clone https://github.com/ozwaldorf/zoom-sync && cd zoom-sync
cargo install --path .
```

### Nix

> Note: On nixos, you must use the flake for nvidia gpu temp to work

```
nix run github:ozwaldorf/zoom-sync
```

## Usage

```
Cross-platform utility for syncing zoom65v3 screen modules

Usage: zoom-sync
        ([-r=ARG] [-R=ARG] [-f] [-s=POSITION | --up | --down | --switch]
        (--no-weather | [--coords LAT LON] | -w WMO CUR MIN MAX)
        (--no-system | ([--cpu=LABEL] | -c=TEMP) ([--gpu=ID] | -g=TEMP) [-d=ARG]))
        | COMMAND ...

Screen options:
    -s, --screen=POSITION  Reset and move the screen to a specific position.
                           [cpu|gpu|download|time|weather|meletrix|zoom65|image|gif|battery]
        --up               Move the screen up
        --down             Move the screen down
        --switch           Switch the screen offset

Weather forecast options:
        --no-weather       Disable updating weather info completely
  --coords LAT LON
        --coords           Optional coordinates to use for fetching weather data, skipping ipinfo
                           geolocation api.
    LAT                    Latitude
    LON                    Longitude

  -w WMO CUR MIN MAX
    -w, --weather          Manually provide weather data, skipping open-meteo weather api. All
                           values are unitless.
    WMO                    WMO Index
    CUR                    Current temperature
    MIN                    Minumum temperature
    MAX                    Maximum temperature

System info options:
        --no-system        Disable updating system info completely
        --cpu=LABEL        Sensor label to search for
                           [default: coretemp Package]
    -c, --cpu-temp=TEMP    Manually set CPU temperature
        --gpu=ID           GPU device id to fetch temperature data for (nvidia only)
                           [default: 0]
    -g, --gpu-temp=TEMP    Manually set GPU temperature
    -d, --download=ARG     Manually set download speed

Available options:
    -r, --refresh=ARG      Continuously refresh the data at a given interval
                           [default: 30]
    -R, --retry=ARG        Retry interval for reconnecting to keyboard
                           [default: 5]
    -f, --farenheit        Use farenheit for all fetched temperatures. May cause clamping for
                           anything greater than 99F.No effect on any manually provided data.
    -h, --help             Prints help information
    -V, --version          Prints version information

Available commands:
    set                    Set specific options on the keyboard
```

### Set subcommand

```
Set specific options on the keyboard

Usage: zoom-sync set COMMAND ...

Available options:
    -h, --help  Prints help information

Available commands:
    time        Sync time to system clock
    weather     Set weather data
    system      Set system info
    screen      Change current screen
```

## Feature Checklist

- [x] Reverse engineer updating each value
  - [x] Time
  - [x] Weather (current, min, max)
  - [x] CPU/GPU temp
  - [x] Download rate
- [x] Fetch current weather report
- [x] Fetch CPU temp
- [x] Fetch Nvidia GPU temp
- [ ] Fetch AMD GPU temp
- [ ] Monitor download rate
- [x] Poll and reconnect to keyboard
- [x] CLI arguments
- [ ] System tray menu
- [ ] Update intervals for each value
- [ ] Package releases
  - [ ] Crates.io
  - [ ] Nixpkgs
  - [ ] Windows
  - [ ] OSX
