//! High level hidapi abstraction for interacting with Zoom 98 LCD modules.
//!
//! Reverse engineered from the `WuqueStudioInstall` companion app (a PyInstaller
//! bundle of a Vial-derived PySide6 client). Unlike the Tiga screen-module boards,
//! the Zoom 98 has no image/gif/theme upload: the LCD is driven by firmware modes
//! which the host only feeds data values to (time, weather, CPU/GPU/fan temps,
//! network speed). LCD mode is cycled by physical QMK keycodes (`LCD_TG`/`LCD_MI`/
//! `LCD_MD`), not by HID packets.
//!
//! ## Packet format
//!
//! 32-byte HID packets framed like the Tiga protocol but with a disjoint command
//! namespace and a slightly different checksum scope. On the wire each packet is
//! prepended with a `0x00` HID report ID, so `HidDevice::write` receives 33 bytes.
//!
//! ```text
//! Byte  0    : 0x1C             (frame marker)
//! Byte  1    : 0x00             (sub-type; always 0 on zoom98)
//! Bytes 2-4  : 0x00              reserved
//! Byte  5    : payload size      (varies per command)
//! Bytes 6-7  : CRC-16/CCITT-FALSE over the 32-byte packet (LE)
//! Byte  8    : 0xA5              (magic)
//! Byte  9    : command byte
//! Byte 10    : 0x00              reserved
//! Byte 11    : inner payload length
//! Bytes 12+  : inner payload
//! Byte 12+len: inner checksum   = sum(bytes 8..12+len) ^ 0xFF   (note: includes 0xA5)
//! Bytes ..31 : zero padding
//! ```

use std::sync::{LazyLock, RwLock};

use chrono::{DateTime, Datelike, Local, TimeZone, Timelike};
use hidapi::{HidApi, HidDevice};
use zoom_sync_core::{
    Board, BoardError, BoardInfo, Capabilities, HasSystemInfo, HasTime, HasWeather, Result,
};

pub mod consts {
    /// USB Vendor ID
    pub const VENDOR_ID: u16 = 0x1EA7;
    /// USB Product ID
    pub const PRODUCT_ID: u16 = 0xCD68;
    /// HID usage page
    pub const USAGE_PAGE: u16 = 0xFF60;
    /// HID usage
    pub const USAGE: u16 = 0x61;
}

/// Static board info for detection
pub static INFO: BoardInfo = BoardInfo {
    name: "Zoom 98",
    cli_name: "zoom98",
    vendor_id: Some(consts::VENDOR_ID),
    product_id: Some(consts::PRODUCT_ID),
    usage_page: Some(consts::USAGE_PAGE),
    usage: Some(consts::USAGE),
    capabilities: Capabilities {
        time: true,
        weather: true,
        system_info: true,
        screen_pos: false,
        screen_nav: false,
        image: false,
        gif: false,
        theme: false,
    },
};

/// Lazy handle to hidapi
static API: LazyLock<RwLock<HidApi>> =
    LazyLock::new(|| RwLock::new(HidApi::new().expect("failed to init hidapi")));

/// High level abstraction for managing a Zoom 98 keyboard LCD
pub struct Zoom98 {
    pub device: HidDevice,
    buf: [u8; 64],
}

// === Protocol primitives (inline — no shared crate yet) ===

mod protocol {
    /// Command identifiers for the zoom98 LCD protocol.
    pub mod cmd {
        /// Time sync (hours/minutes/seconds + calendar fields)
        pub const TIME: u8 = 0x3E;
        /// Date sync (same 8-byte calendar body as TIME, different subcmd)
        pub const DATE: u8 = 0x3F;
        /// CPU temperature (u16 BE Celsius)
        pub const CPU_TEMP: u8 = 0x37;
        /// GPU temperature (u16 BE Celsius)
        pub const GPU_TEMP: u8 = 0x38;
        /// Fan RPM (u16 BE)
        pub const FAN_RPM: u8 = 0x39;
        /// Weather (icon + u16 BE Celsius current temp)
        pub const WEATHER: u8 = 0x3B;
        /// Network download speed (u32 BE, value = MiB/s * 100)
        pub const NETWORK: u8 = 0x3D;
    }

    /// CRC-16/CCITT-FALSE (poly 0x1021, init 0xFFFF, no reflection, no xor-out).
    /// Verified identical to `zoom_tiga_protocol::crc16` for all test inputs.
    pub fn crc16(data: &[u8]) -> u16 {
        let mut crc: u16 = 0xFFFF;
        for &byte in data {
            crc ^= (byte as u16) << 8;
            for _ in 0..8 {
                if crc & 0x8000 != 0 {
                    crc = (crc << 1) ^ 0x1021;
                } else {
                    crc <<= 1;
                }
            }
        }
        crc
    }

    /// Build a 32-byte packet.
    ///
    /// `cmd_byte` is the inner command (goes at packet[9]).
    /// `body` is the inner payload (goes at packet[12..12+body.len()]).
    ///
    /// The inner checksum is computed over `[0xA5, cmd_byte, 0x00, body.len(), body...]`
    /// (i.e. packet bytes 8..12+body.len(), inclusive of the 0xA5 magic — this differs
    /// from the Tiga protocol, which skips the magic).
    pub fn build_packet(cmd_byte: u8, body: &[u8]) -> [u8; 32] {
        let body_len = body.len();
        // Wire size field = 4 + body_len + 1 (matches observed values: date/time=15,
        // cpu/gpu/fan=8, weather=9, network=10)
        let size_field = (4 + body_len + 1) as u8;

        let mut packet = [0u8; 32];
        packet[0] = 0x1C;
        packet[1] = 0x00;
        packet[5] = size_field;
        // bytes 6-7: CRC placeholder
        packet[8] = 0xA5;
        packet[9] = cmd_byte;
        packet[10] = 0x00;
        packet[11] = body_len as u8;
        packet[12..12 + body_len].copy_from_slice(body);

        // Inner checksum: sum of bytes 8..12+body_len, XOR 0xFF, truncated to u8.
        let sum: u32 = packet[8..12 + body_len].iter().map(|&b| b as u32).sum();
        packet[12 + body_len] = ((sum ^ 0xFF) & 0xFF) as u8;

        // Outer CRC16 over the whole packet (bytes 6-7 still zero).
        let crc = crc16(&packet);
        packet[6] = (crc & 0xFF) as u8;
        packet[7] = (crc >> 8) as u8;

        packet
    }

    /// Build a datetime body. The calendar fields (year..weekday) are identical for
    /// TIME and DATE commands; only the sub-command byte (`sub`) and the outer cmd
    /// byte differ.
    ///
    /// Weekday encoding: Monday=1..Sunday=7 (i.e. `weekday_mon0 + 1`), different from
    /// the Tiga protocol's Sunday=0..Saturday=6.
    #[allow(clippy::too_many_arguments)]
    fn datetime_body(
        sub: u8,
        year: u16,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
        weekday_mon1: u8,
    ) -> [u8; 10] {
        [
            0x00,
            sub,
            (year >> 8) as u8,
            (year & 0xFF) as u8,
            month,
            day,
            hour,
            minute,
            second,
            weekday_mon1,
        ]
    }

    /// Build a time-sync packet (cmd 0x3E, sub 0x00).
    pub fn time(
        year: u16,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
        weekday_mon1: u8,
    ) -> [u8; 32] {
        let body = datetime_body(0x00, year, month, day, hour, minute, second, weekday_mon1);
        build_packet(cmd::TIME, &body)
    }

    /// Build a date-sync packet (cmd 0x3F, sub 0x01).
    pub fn date(
        year: u16,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
        weekday_mon1: u8,
    ) -> [u8; 32] {
        let body = datetime_body(0x01, year, month, day, hour, minute, second, weekday_mon1);
        build_packet(cmd::DATE, &body)
    }

    /// Build a CPU temperature packet (°C, big-endian u16).
    pub fn cpu_temp(temp_c: u16) -> [u8; 32] {
        let body = [0x00, (temp_c >> 8) as u8, (temp_c & 0xFF) as u8];
        build_packet(cmd::CPU_TEMP, &body)
    }

    /// Build a GPU temperature packet (°C, big-endian u16).
    pub fn gpu_temp(temp_c: u16) -> [u8; 32] {
        let body = [0x00, (temp_c >> 8) as u8, (temp_c & 0xFF) as u8];
        build_packet(cmd::GPU_TEMP, &body)
    }

    /// Build a fan RPM packet (big-endian u16).
    pub fn fan_rpm(rpm: u16) -> [u8; 32] {
        let body = [0x00, (rpm >> 8) as u8, (rpm & 0xFF) as u8];
        build_packet(cmd::FAN_RPM, &body)
    }

    /// Build a weather packet.
    ///
    /// `icon` is the zoom98 icon index (see [`WeatherIcon`](super::WeatherIcon)),
    /// `temp_c` is the current temperature as a big-endian u16 Celsius value.
    pub fn weather(icon: u8, temp_c: u16) -> [u8; 32] {
        let body = [0x00, icon, (temp_c >> 8) as u8, (temp_c & 0xFF) as u8];
        build_packet(cmd::WEATHER, &body)
    }

    /// Build a network-speed packet.
    ///
    /// The wire value is `(bytes_per_sec / 1_048_576) * 100` as a big-endian u32 —
    /// i.e. 2-decimal fixed-point MiB/s.
    pub fn network(wire_value: u32) -> [u8; 32] {
        let bytes = wire_value.to_be_bytes();
        let body = [0x00, bytes[0], bytes[1], bytes[2], bytes[3]];
        build_packet(cmd::NETWORK, &body)
    }
}

pub use protocol::cmd;

/// Weather condition icon for the Zoom 98 LCD.
///
/// The companion app maps Chinese weather-description text to these 7 indices; there
/// are no day/night variants. WMO codes are mapped in [`WeatherIcon::from_wmo`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WeatherIcon {
    /// 晴 — sunny / clear
    Sunny = 1,
    /// 云 — cloudy
    Cloudy = 2,
    /// 阴 — overcast
    Overcast = 3,
    /// 雨 — rain
    Rain = 4,
    /// 雪 — snow
    Snow = 5,
    /// 阵 — showers
    Shower = 6,
    /// 雾 — fog
    Fog = 7,
}

impl WeatherIcon {
    /// Convert a WMO weather code to a zoom98 icon. The `is_day` argument is ignored
    /// because the zoom98 icon set has no day/night variants.
    pub fn from_wmo(wmo: u8, _is_day: bool) -> Option<Self> {
        match wmo {
            // Clear and mainly clear
            0 | 1 => Some(Self::Sunny),
            // Partly cloudy
            2 => Some(Self::Cloudy),
            // Overcast
            3 => Some(Self::Overcast),
            // Fog
            45 | 48 => Some(Self::Fog),
            // Drizzle / rain (continuous)
            51 | 53 | 55 | 56 | 57 | 61 | 63 | 65 | 66 | 67 => Some(Self::Rain),
            // Rain showers
            80..=82 => Some(Self::Shower),
            // Snow
            71 | 73 | 75 | 77 | 85 | 86 => Some(Self::Snow),
            // Thunderstorms — closest match is shower
            95 | 96 | 99 => Some(Self::Shower),
            _ => None,
        }
    }
}

impl Zoom98 {
    /// Find and open the device.
    pub fn open() -> Result<Self> {
        API.write().unwrap().refresh_devices()?;
        let api = API.read().unwrap();
        let this = Self {
            device: api
                .device_list()
                .find(|d| {
                    d.vendor_id() == consts::VENDOR_ID
                        && d.product_id() == consts::PRODUCT_ID
                        && d.usage_page() == consts::USAGE_PAGE
                        && d.usage() == consts::USAGE
                })
                .ok_or(BoardError::DeviceNotFound)?
                .open_device(&api)?,
            buf: [0u8; 64],
        };
        Ok(this)
    }

    /// Write a 32-byte packet with the leading HID report-id byte, then drain any
    /// response the firmware sends back.
    fn execute(&mut self, packet: [u8; 32]) -> Result<()> {
        let mut framed = [0u8; 33];
        framed[1..].copy_from_slice(&packet);
        self.device.write(&framed)?;
        // Best-effort read: the companion waits up to ~200ms; we mirror that.
        let _ = self.device.read_timeout(&mut self.buf, 200);
        Ok(())
    }

    /// Sync the current date and time to the keyboard display.
    ///
    /// This sends **two** packets: one with the DATE command and one with the TIME
    /// command, matching how the Wuque companion app drives the board.
    pub fn set_time<Tz: TimeZone>(&mut self, time: DateTime<Tz>, _12hr: bool) -> Result<()> {
        // weekday: Wuque Studio sends +1 so Mon=1..Sun=7.
        let weekday_mon1 = time.weekday().num_days_from_monday() as u8 + 1;
        let year = time.year() as u16;
        let month = time.month() as u8;
        let day = time.day() as u8;
        let hour = if _12hr { time.hour12().1 } else { time.hour() } as u8;
        let minute = time.minute() as u8;
        let second = time.second() as u8;

        let date_packet = protocol::date(year, month, day, hour, minute, second, weekday_mon1);
        self.execute(date_packet)?;
        let time_packet = protocol::time(year, month, day, hour, minute, second, weekday_mon1);
        self.execute(time_packet)
    }

    /// Update the weather display. Only the current temperature is shown on the
    /// Zoom 98 LCD — min/max are not supported.
    pub fn set_weather(&mut self, icon: WeatherIcon, current_c: i16) -> Result<()> {
        // Wire format is unsigned u16; negative temps clamp to 0 like the companion
        // app (the LCD has no sign indicator anyway).
        let temp = current_c.max(0) as u16;
        self.execute(protocol::weather(icon as u8, temp))
    }

    /// Push system info values. The zoom98 has a separate command per metric, so
    /// this issues up to four packets back-to-back.
    pub fn set_system_info(
        &mut self,
        cpu_temp: u8,
        gpu_temp: u32,
        download_bytes_per_sec: f32,
        fan_rpm: u32,
    ) -> Result<()> {
        self.execute(protocol::cpu_temp(cpu_temp as u16))?;

        // Clamp 32-bit source values into the 16-bit wire fields. Realistic temps
        // and fan RPMs never approach u16::MAX, but the CLI's u32-typed inputs can.
        let gpu = u16::try_from(gpu_temp).unwrap_or(u16::MAX);
        self.execute(protocol::gpu_temp(gpu))?;

        let fan = u16::try_from(fan_rpm).unwrap_or(u16::MAX);
        self.execute(protocol::fan_rpm(fan))?;

        // Network: bytes/s → MiB/s * 100 → BE u32
        let mib_x100 = (download_bytes_per_sec / 1_048_576.0 * 100.0) as u32;
        self.execute(protocol::network(mib_x100))
    }
}

// === Trait Implementations ===

impl Board for Zoom98 {
    fn info(&self) -> &'static BoardInfo {
        &INFO
    }

    fn as_time(&mut self) -> Option<&mut dyn HasTime> {
        Some(self)
    }

    fn as_weather(&mut self) -> Option<&mut dyn HasWeather> {
        Some(self)
    }

    fn as_system_info(&mut self) -> Option<&mut dyn HasSystemInfo> {
        Some(self)
    }
}

impl HasTime for Zoom98 {
    fn set_time(&mut self, time: DateTime<Local>, use_12hr: bool) -> Result<()> {
        // Zoom 98 firmware uses 24-hour format internally; _use_12hr is ignored.
        Zoom98::set_time(self, time, use_12hr)
    }
}

impl HasWeather for Zoom98 {
    fn set_weather(
        &mut self,
        wmo: u8,
        is_day: bool,
        current: i16,
        _low: i16,
        _high: i16,
    ) -> Result<()> {
        let icon = WeatherIcon::from_wmo(wmo, is_day)
            .ok_or(BoardError::CommandFailed("unknown WMO code"))?;
        Zoom98::set_weather(self, icon, current)
    }
}

impl HasSystemInfo for Zoom98 {
    fn set_system_info(&mut self, cpu: u8, gpu: u32, download: f32, fan_rpm: u32) -> Result<()> {
        Zoom98::set_system_info(self, cpu, gpu, download, fan_rpm)
    }
}

#[cfg(test)]
mod tests {
    use super::protocol::{build_packet, crc16, date, time};

    /// Canonical CCITT-FALSE test vector.
    #[test]
    fn crc16_canonical() {
        assert_eq!(crc16(b"123456789"), 0x29B1);
    }

    /// The size field, magic, command byte, payload length, and checksum should
    /// match the companion app's observed layout for a date-sync packet.
    #[test]
    fn date_packet_layout() {
        // 2026-04-11 (Saturday) 16:42:09
        let pkt = date(2026, 4, 11, 16, 42, 9, /* Sat = 6 */ 6);
        assert_eq!(pkt[0], 0x1C);
        assert_eq!(pkt[1], 0x00);
        assert_eq!(pkt[5], 15); // 4 + 10 + 1
        assert_eq!(pkt[8], 0xA5);
        assert_eq!(pkt[9], 0x3F);
        assert_eq!(pkt[11], 10);
        // body
        assert_eq!(&pkt[12..22], &[0x00, 0x01, 0x07, 0xEA, 4, 11, 16, 42, 9, 6]);
        // inner checksum = (sum(pkt[8..22]) ^ 0xFF) as u8
        let expected_cs: u32 = pkt[8..22].iter().map(|&b| b as u32).sum();
        assert_eq!(pkt[22] as u32, (expected_cs ^ 0xFF) & 0xFF);
    }

    /// Time and date packets share the same calendar body but differ on cmd byte
    /// and the `sub` byte inside the body.
    #[test]
    fn time_vs_date_differ_only_by_cmd_and_sub() {
        let t = time(2026, 4, 11, 16, 42, 9, 6);
        let d = date(2026, 4, 11, 16, 42, 9, 6);
        assert_eq!(t[9], 0x3E);
        assert_eq!(d[9], 0x3F);
        assert_eq!(t[13], 0x00); // time sub-command
        assert_eq!(d[13], 0x01); // date sub-command
                                 // All other body bytes match
        assert_eq!(&t[14..22], &d[14..22]);
    }

    /// build_packet with a short body should place the inner checksum right after
    /// the body and leave the rest zero-padded.
    #[test]
    fn build_packet_padding() {
        // CPU temp body = [0x00, temp_h, temp_l] (3 bytes)
        let pkt = build_packet(0x37, &[0x00, 0x00, 0x3C]);
        assert_eq!(pkt[5], 8); // 4 + 3 + 1
        assert_eq!(pkt[11], 3);
        assert_eq!(&pkt[12..15], &[0x00, 0x00, 0x3C]);
        // pkt[15] is the inner checksum
        let expected_cs: u32 = pkt[8..15].iter().map(|&b| b as u32).sum();
        assert_eq!(pkt[15] as u32, (expected_cs ^ 0xFF) & 0xFF);
        // Everything after the inner checksum should be zero.
        assert!(pkt[16..].iter().all(|&b| b == 0));
    }

    /// The outer CRC should round-trip: after stamping, CRC over the packet with
    /// the CRC bytes zeroed out should still equal the stamped value.
    #[test]
    fn outer_crc_roundtrip() {
        let pkt = time(2026, 4, 11, 16, 42, 9, 6);
        let stamped = u16::from_le_bytes([pkt[6], pkt[7]]);
        let mut zeroed = pkt;
        zeroed[6] = 0;
        zeroed[7] = 0;
        assert_eq!(crc16(&zeroed), stamped);
    }
}
