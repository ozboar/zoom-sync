//! HID packet protocol implementation for Zoom TKL Dyna.
//!
//! Packet structure (32 bytes):
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
    /// DateTime sync command
    pub const DATETIME: u8 = 0x38;
    /// Screen control command
    pub const SCREEN: u8 = 0x39;
    /// Image upload command
    pub const IMAGE: u8 = 0xFC;
    /// Theme settings command
    pub const THEME: u8 = 0xFD;
    /// Weather display command
    pub const WEATHER: u8 = 0xFE;
    /// Sysinfo Command
    pub const SYSINFO: u8 = 0xFF;
}

/// Build a 32-byte HID packet with proper framing, checksums, and CRC.
///
/// # Arguments
/// * `command` - Command byte (e.g., 0x38 for datetime)
/// * `payload` - Payload data (excluding command byte)
/// * `sub_type` - Sub-type byte (0x02 for most commands, 0x03 for datetime)
pub fn build_packet(command: u8, payload: &[u8], sub_type: u8) -> [u8; 32] {
    let mut packet = [0u8; 32];
    let data_length = payload.len();

    // Magic and command structure
    packet[8] = 0xA5; // Magic identifier
    packet[9] = command; // Command byte
    packet[10] = 0; // Reserved
    packet[11] = data_length as u8; // Payload length

    // Copy payload data
    for (i, &byte) in payload.iter().enumerate() {
        if 12 + i < 31 {
            packet[12 + i] = byte;
        }
    }

    // Calculate checksum (sum of bytes 9+ XOR 0xFF)
    let mut checksum: u16 = 0;
    for &byte in packet.iter().skip(9) {
        checksum += byte as u16;
    }
    let checksum_pos = 12 + data_length;
    if checksum_pos < 32 {
        packet[checksum_pos] = (checksum ^ 0xFF) as u8;
    }

    // Set packet header
    packet[0] = 0x1C; // Report type
    packet[1] = sub_type; // Sub-type
    packet[5] = (4 + data_length + 1) as u8; // Payload size field

    // Calculate and set CRC16 (little-endian)
    let crc = crc16(&packet);
    packet[6] = (crc & 0xFF) as u8; // CRC low byte
    packet[7] = (crc >> 8) as u8; // CRC high byte

    // Additional 0 padding byte - NOTE, this loses one byte off of the original packet, 
    // so if we're setting packet[31] we're losing that byte. As far as I know, the largest
    // thing we send through here is 16 bytes, which 12(starting point of data)+16=28, so
    // we *should* be alright.
    let mut full_packet: [u8; 32] = [0u8; 32];
    full_packet[0] = 0x0;
    full_packet[1..32].copy_from_slice(&packet[..31]);
    full_packet
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
) -> [u8; 32] {
    let payload = [
        0x00,                // Unknown flag 1
        0x01,                // Unknown flag 2
        (year >> 8) as u8,   // Year high byte
        (year & 0xFF) as u8, // Year low byte
        month,               // Month (1-12)
        day,                 // Day of month
        hour,                // Hours (0-23)
        minute,              // Minutes (0-59)
        second,              // Seconds (0-59)
        day_of_week,         // Day of week (0=Sun)
    ];
    build_packet(cmd::DATETIME, &payload, 0x03)
}

/// Build a screen control packet.
pub fn screen_control(mode: ScreenMode) -> [u8; 32] {
    build_packet(cmd::SCREEN, &mode.to_bytes(), 0x02)
}

/// Build a theme settings packet.
///
/// Payload structure (6 bytes):
/// - Byte 0: Sub-command (0x00)
/// - Bytes 1-2: Background color (RGB565, big-endian)
/// - Bytes 3-4: Font color (RGB565, big-endian)
/// - Byte 5: Theme ID
pub fn theme(bg_color: Rgb565, font_color: Rgb565, theme_id: u8) -> [u8; 32] {
    let bg = bg_color.to_be_bytes();
    let font = font_color.to_be_bytes();
    let payload = [0x00, bg[0], bg[1], font[0], font[1], theme_id];
    build_packet(cmd::THEME, &payload, 0x02)
}

/// Build a weather display packet.
///
/// Payload structure (8 bytes):
/// - Byte 0: Sub-command (0x00)
/// - Byte 1: Weather icon (1-8)
/// - Bytes 2-3: Current temp (encoded, big-endian)
/// - Bytes 4-5: Max temp (encoded, big-endian)
/// - Bytes 6-7: Min temp (encoded, big-endian)
pub fn weather(icon: WeatherIcon, current: u16, max: u16, min: u16) -> [u8; 32] {
    let payload = [
        0x00,
        icon as u8,
        (current >> 8) as u8,
        (current & 0xFF) as u8,
        (max >> 8) as u8,
        (max & 0xFF) as u8,
        (min >> 8) as u8,
        (min & 0xFF) as u8,
    ];
    build_packet(cmd::WEATHER, &payload, 0x02)
}

/// Build an image data chunk packet.
///
/// Payload structure:
/// - Byte 0: Sub-command (0x00)
/// - Bytes 1-2: Page index (big-endian)
/// - Bytes 3+: Chunk data (up to 16 bytes)
pub fn image_chunk(page_index: u16, chunk: &[u8]) -> [u8; 32] {
    let mut payload = vec![0x00, (page_index >> 8) as u8, (page_index & 0xFF) as u8];
    payload.extend_from_slice(chunk);
    build_packet(cmd::IMAGE, &payload, 0x02)
}

/// Build an image upload termination packet.
pub fn image_end() -> [u8; 32] {
    let payload = [0x00, 0xFF, 0xFF];
    build_packet(cmd::IMAGE, &payload, 0x02)
}

/// Build a sysinfo data packet
///
/// Payload structure (12 bytes):
/// - Byte 0: Unknown flag (0x00)
/// - Byte 1-2: CPU Temp (big-endian)
/// - Bytes 3-4: GPU Temp (big-endian)
/// - Byte 5-6: Unknown Identifier, I speculated that it might SSD Temp looking at 
///             the payload at one point but it doesn't display on screen anywhere.
/// - Byte 7-8: Fan RPM (big-endian)
/// - Byte 9-10: Download Speed (big-endian)
/// - Byte 11: 0xFF, (magic identifier)
pub fn set_system_info(cpu_temp: u8, gpu_temp: u32, download: f32, fan_rpm: u32 ) -> [u8; 32] {
    let gpu_temp_bytes: [u8; 4] = gpu_temp.to_be_bytes();
    let download_bytes: [u8; 4] = ((download * 10.0) as u32).to_be_bytes();
    let fan_rpm_bytes: [u8; 4] = fan_rpm.to_be_bytes();
    // data length should be 11
    let payload = [
        0x00,
        0x00, // if this is ever >= 1, your cpu might be on fire
        cpu_temp,
        gpu_temp_bytes[2], // if this is ever >= 1, your gpu might be on fire
        gpu_temp_bytes[3],
        0x00,
        0x00, // unsure what this value is, believe it might be a random thermal identifier not shown on screen.
        fan_rpm_bytes[2],
        fan_rpm_bytes[3],
        download_bytes[2],
        download_bytes[3],
        0xFF
    ];
    build_packet(cmd::SYSINFO, &payload, 0x02)
}
