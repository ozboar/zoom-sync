//! HID packet protocol implementation for Zoom TKL Dyna.
//!
//! Packet structure (32 bytes, sent behind a 0x00 report id):
//! - Byte 0: 0x1C (report type)
//! - Byte 1: 0x02 or 0x03 (sub-type)
//! - Bytes 2-4: Reserved (0)
//! - Byte 5: Payload size (4 + dataLength + 1)
//! - Bytes 6-7: CRC16 (little-endian)
//! - Byte 8: 0xA5 (magic identifier)
//! - Byte 9: Command byte
//! - Byte 10: Reserved (0)
//! - Byte 11: Payload length
//! - Bytes 12+: Payload data
//! - Final byte: Checksum (sum of bytes 9+ XOR 0xFF)

use crate::crc::crc16;
use crate::types::{Rgb565, ScreenMode, WeatherIcon};

/// Command identifiers
pub mod cmd {
    /// Screen restart command
    pub const RESTART: u8 = 0x34;
    /// DateTime sync command
    pub const DATETIME: u8 = 0x38;
    /// Screen control command
    pub const SCREEN: u8 = 0x39;
    /// Device info query command
    pub const INFO: u8 = 0xFB;
    /// Image upload command
    pub const IMAGE: u8 = 0xFC;
    /// Theme settings command
    pub const THEME: u8 = 0xFD;
    /// Weather display command
    pub const WEATHER: u8 = 0xFE;
    /// Sysinfo Command
    pub const SYSINFO: u8 = 0xFF;
}

/// Maximum payload length that fits in a packet alongside the trailing checksum
pub const MAX_PAYLOAD: usize = 19;

/// Build a HID packet with proper framing, checksums, and CRC.
///
/// Returns 33 bytes: a 0x00 report id followed by the 32 byte packet.
///
/// # Arguments
/// * `command` - Command byte (e.g., 0x38 for datetime)
/// * `payload` - Payload data (excluding command byte), at most [`MAX_PAYLOAD`] bytes
/// * `sub_type` - Sub-type byte (0x02 for most commands, 0x03 for datetime)
pub fn build_packet(command: u8, payload: &[u8], sub_type: u8) -> [u8; 33] {
    assert!(payload.len() <= MAX_PAYLOAD, "payload too large");
    let mut packet = [0u8; 32];
    let data_length = payload.len();

    packet[8] = 0xA5;
    packet[9] = command;
    packet[11] = data_length as u8;
    packet[12..12 + data_length].copy_from_slice(payload);

    // Checksum (sum of bytes 9+ XOR 0xFF)
    let checksum = packet[9..].iter().fold(0u8, |acc, &b| acc.wrapping_add(b));
    packet[12 + data_length] = checksum ^ 0xFF;

    packet[0] = 0x1C;
    packet[1] = sub_type;
    packet[5] = (4 + data_length + 1) as u8;

    // CRC16 over the whole packet (little-endian)
    let crc = crc16(&packet);
    packet[6] = crc as u8;
    packet[7] = (crc >> 8) as u8;

    let mut buf = [0u8; 33];
    buf[1..].copy_from_slice(&packet);
    buf
}

/// Build a datetime sync packet.
///
/// Payload structure (10 bytes):
/// - Bytes 0-1: Unknown flags (0x00, 0x01)
/// - Bytes 2-3: Year (big-endian)
/// - Byte 4: Month (1-12)
/// - Byte 5: Day of month (1-31)
/// - Byte 6: Hours (0-23)
/// - Byte 7: Minutes (0-59)
/// - Byte 8: Seconds (0-59)
/// - Byte 9: Day of week (0=Sunday)
pub fn datetime(
    year: u16,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
    day_of_week: u8,
) -> [u8; 33] {
    let [year_hi, year_lo] = year.to_be_bytes();
    let payload = [
        0x00,
        0x01,
        year_hi,
        year_lo,
        month,
        day,
        hour,
        minute,
        second,
        day_of_week,
    ];
    build_packet(cmd::DATETIME, &payload, 0x03)
}

/// Build a screen control packet.
pub fn screen_control(mode: ScreenMode) -> [u8; 33] {
    build_packet(cmd::SCREEN, &mode.to_bytes(), 0x02)
}

/// Build a screen restart packet.
pub fn restart() -> [u8; 33] {
    build_packet(cmd::RESTART, &[0x00, 0x01], 0x02)
}

/// Build a device info query packet. The response carries the device mac address.
pub fn info() -> [u8; 33] {
    build_packet(cmd::INFO, &[0x00], 0x02)
}

/// Build a theme settings packet.
///
/// Payload structure (6 bytes):
/// - Byte 0: Sub-command (0x00)
/// - Bytes 1-2: Background color (RGB565, big-endian)
/// - Bytes 3-4: Font color (RGB565, big-endian)
/// - Byte 5: Theme ID
pub fn theme(bg_color: Rgb565, font_color: Rgb565, theme_id: u8) -> [u8; 33] {
    let bg = bg_color.to_be_bytes();
    let font = font_color.to_be_bytes();
    let payload = [0x00, bg[0], bg[1], font[0], font[1], theme_id];
    build_packet(cmd::THEME, &payload, 0x02)
}

/// Build a weather display packet.
///
/// Payload structure (8 bytes):
/// - Byte 0: Sub-command (0x00)
/// - Byte 1: Weather icon (0-8)
/// - Bytes 2-3: Current temp (encoded, big-endian)
/// - Bytes 4-5: Max temp (encoded, big-endian)
/// - Bytes 6-7: Min temp (encoded, big-endian)
pub fn weather(icon: WeatherIcon, current: u16, max: u16, min: u16) -> [u8; 33] {
    let [cur_hi, cur_lo] = current.to_be_bytes();
    let [max_hi, max_lo] = max.to_be_bytes();
    let [min_hi, min_lo] = min.to_be_bytes();
    let payload = [
        0x00, icon as u8, cur_hi, cur_lo, max_hi, max_lo, min_hi, min_lo,
    ];
    build_packet(cmd::WEATHER, &payload, 0x02)
}

/// Build an image data chunk packet.
///
/// Payload structure:
/// - Byte 0: Sub-command (0x00)
/// - Bytes 1-2: Page index (big-endian)
/// - Bytes 3+: Chunk data (up to 16 bytes)
pub fn image_chunk(page_index: u16, chunk: &[u8]) -> [u8; 33] {
    let mut payload = [0u8; MAX_PAYLOAD];
    let [idx_hi, idx_lo] = page_index.to_be_bytes();
    payload[1] = idx_hi;
    payload[2] = idx_lo;
    payload[3..3 + chunk.len()].copy_from_slice(chunk);
    build_packet(cmd::IMAGE, &payload[..3 + chunk.len()], 0x02)
}

/// Build an image upload termination packet.
pub fn image_end() -> [u8; 33] {
    build_packet(cmd::IMAGE, &[0x00, 0xFF, 0xFF], 0x02)
}

/// Build a sysinfo data packet
///
/// Payload structure (12 bytes):
/// - Byte 0: Unknown flag (0x00)
/// - Byte 1-2: CPU Temp (big-endian)
/// - Bytes 3-4: GPU Temp (big-endian)
/// - Byte 5-6: Unknown, not shown on screen
/// - Byte 7-8: Fan RPM (big-endian)
/// - Byte 9-10: Download Speed (big-endian)
/// - Byte 11: 0xFF
pub fn set_system_info(cpu_temp: u8, gpu_temp: u16, download: f32, fan_rpm: u16) -> [u8; 33] {
    let [gpu_hi, gpu_lo] = gpu_temp.to_be_bytes();
    let [fan_hi, fan_lo] = fan_rpm.to_be_bytes();
    let [dl_hi, dl_lo] = ((download * 10.0) as u16).to_be_bytes();
    let payload = [
        0x00, 0x00, cpu_temp, gpu_hi, gpu_lo, 0x00, 0x00, fan_hi, fan_lo, dl_hi, dl_lo, 0xFF,
    ];
    build_packet(cmd::SYSINFO, &payload, 0x02)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_packet_matches_reference() {
        let buf = datetime(2026, 10, 19, 12, 30, 45, 1);
        assert_eq!(buf[0], 0x00);
        let p = &buf[1..];
        assert_eq!(&p[..2], &[0x1C, 0x03]);
        assert_eq!(p[5], 15);
        assert_eq!(&p[8..12], &[0xA5, 0x38, 0x00, 10]);
        let sum = p[9..22].iter().fold(0u8, |a, &b| a.wrapping_add(b));
        assert_eq!(p[22], sum ^ 0xFF);
        let mut zeroed = [0u8; 32];
        zeroed.copy_from_slice(p);
        zeroed[6] = 0;
        zeroed[7] = 0;
        assert_eq!(u16::from_le_bytes([p[6], p[7]]), crc16(&zeroed));
    }

    #[test]
    fn image_chunk_keeps_checksum() {
        let p = &image_chunk(0x0102, &[0xAA; 16])[1..];
        assert_eq!(p[11], 19);
        assert_eq!(&p[12..15], &[0x00, 0x01, 0x02]);
        let sum = p[9..31].iter().fold(0u8, |a, &b| a.wrapping_add(b));
        assert_eq!(p[31], sum ^ 0xFF);
    }
}
