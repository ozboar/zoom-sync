//! Type definitions for Zoom TKL Dyna keyboard.

/// Weather condition icons for the keyboard display.
/// These map to the display icon indices.
/// Note: Some icons may share visual representations on the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WeatherIcon {
    Unknown = 0,
    SunnyDay = 1,
    PartlyCloudyDay = 2,
    Cloudy = 5,
    ClearNight = 6,
    PartlyCloudyNight = 7,
    Thunderstorm = 8,
    // Rain and Snow use separate indices based on the deobfuscated protocol
    Rain = 3,
    Snow = 4,
}

impl WeatherIcon {
    /// Convert a WMO weather code to a display icon, adapting for day/night.
    /// Based on the Weather API condition codes from the deobfuscated protocol.
    pub fn from_wmo(wmo: u8, is_day: bool) -> Option<Self> {
        match wmo {
            // Clear and mainly clear
            0 | 1 => Some(if is_day {
                WeatherIcon::SunnyDay
            } else {
                WeatherIcon::ClearNight
            }),

            // Partly cloudy
            2 => Some(if is_day {
                WeatherIcon::PartlyCloudyDay
            } else {
                WeatherIcon::PartlyCloudyNight
            }),

            // Overcast and foggy
            3 | 45 | 48 => Some(WeatherIcon::Cloudy),

            // Drizzle, freezing drizzle, rain, freezing rain
            51 | 53 | 55 | 56 | 57 | 61 | 63 | 65 | 66 | 67 => Some(WeatherIcon::Rain),

            // Rain showers
            80..=82 => Some(WeatherIcon::Rain),

            // Snowfall and snow showers
            71 | 73 | 75 | 77 | 85 | 86 => Some(WeatherIcon::Snow),

            // Thunderstorm
            95 | 96 | 99 => Some(WeatherIcon::Thunderstorm),

            _ => None,
        }
    }
}

/// Screen control modes.
/// These correspond to the 0x39 command with two-byte payload.
/// The second byte is a checksum: 0xC4 - first_byte for navigation, 0xC8 for reset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenMode {
    /// Navigate up in menu (cmd=2)
    Up,
    /// Navigate down in menu (cmd=1)
    Down,
    /// Enter/select menu item (cmd=3)
    Enter,
    /// Return/back from menu (cmd=4)
    Return,
    /// Reset themes, gifs and images (cmd=1, special checksum)
    Reset,
}

impl ScreenMode {
    /// Get the two-byte payload for this screen mode.
    /// Format: [command, checksum] where checksum = 0xC4 - command (or 0xC8 for reset)
    pub fn to_bytes(self) -> [u8; 2] {
        match self {
            ScreenMode::Up => [0x02, 0xC4 - 0x02],
            ScreenMode::Down => [0x01, 0xC4 - 0x01],
            ScreenMode::Enter => [0x03, 0xC4 - 0x03],
            ScreenMode::Return => [0x04, 0xC4 - 0x04],
            ScreenMode::Reset => [0x01, 0xC8],
        }
    }
}

/// Encode a temperature value for the keyboard protocol.
/// Format: value * 10, with bit 15 set for negative temperatures.
#[inline]
pub fn encode_temperature(temp_celsius: i16) -> u16 {
    if temp_celsius >= 0 {
        (temp_celsius * 10) as u16
    } else {
        ((-temp_celsius) * 10) as u16 | 0x8000
    }
}

/// RGB565 color for theme settings.
/// 5 bits red, 6 bits green, 5 bits blue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb565(pub u16);

impl Rgb565 {
    /// Create RGB565 from 8-bit RGB values.
    pub fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        let r5 = ((r >> 3) & 0x1F) as u16;
        let g6 = ((g >> 2) & 0x3F) as u16;
        let b5 = ((b >> 3) & 0x1F) as u16;
        Rgb565((r5 << 11) | (g6 << 5) | b5)
    }

    /// Get the big-endian bytes for transmission.
    #[inline]
    pub fn to_be_bytes(self) -> [u8; 2] {
        self.0.to_be_bytes()
    }

    /// Get the little-endian bytes for transmission.
    #[inline]
    pub fn to_le_bytes(self) -> [u8; 2] {
        self.0.to_le_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_temperature_positive() {
        assert_eq!(encode_temperature(25), 250);
        assert_eq!(encode_temperature(0), 0);
        assert_eq!(encode_temperature(100), 1000);
    }

    #[test]
    fn test_encode_temperature_negative() {
        assert_eq!(encode_temperature(-10), 100 | 0x8000);
        assert_eq!(encode_temperature(-1), 10 | 0x8000);
    }

    #[test]
    fn test_rgb565() {
        // Pure red
        let red = Rgb565::from_rgb(255, 0, 0);
        assert_eq!(red.0, 0xF800);

        // Pure green
        let green = Rgb565::from_rgb(0, 255, 0);
        assert_eq!(green.0, 0x07E0);

        // Pure blue
        let blue = Rgb565::from_rgb(0, 0, 255);
        assert_eq!(blue.0, 0x001F);
    }
}
