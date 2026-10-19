//! Type definitions for Tiga-based keyboards.

/// Weather condition icons for the keyboard display.
///
/// The web driver only ever sends 0, 1, 2, 5, 6, 7 and 8, reusing 6 and 7 for both night and
/// rain/snow. Indices 3 and 4 are assumed to be rain and snow so each condition stays distinct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WeatherIcon {
    Unknown = 0,
    SunnyDay = 1,
    PartlyCloudyDay = 2,
    Rain = 3,
    Snow = 4,
    Cloudy = 5,
    ClearNight = 6,
    PartlyCloudyNight = 7,
    Thunderstorm = 8,
}

impl WeatherIcon {
    /// Convert a WMO weather code to a display icon, adapting for day/night.
    pub fn from_wmo(wmo: u8, is_day: bool) -> Self {
        match wmo {
            // Clear and mainly clear
            0 | 1 if is_day => WeatherIcon::SunnyDay,
            0 | 1 => WeatherIcon::ClearNight,
            // Partly cloudy
            2 if is_day => WeatherIcon::PartlyCloudyDay,
            2 => WeatherIcon::PartlyCloudyNight,
            // Overcast and foggy
            3 | 45 | 48 => WeatherIcon::Cloudy,
            // Drizzle, freezing drizzle, rain, freezing rain, rain showers
            51 | 53 | 55 | 56 | 57 | 61 | 63 | 65 | 66 | 67 | 80..=82 => WeatherIcon::Rain,
            // Snowfall and snow showers
            71 | 73 | 75 | 77 | 85 | 86 => WeatherIcon::Snow,
            // Thunderstorm
            95 | 96 | 99 => WeatherIcon::Thunderstorm,
            _ => WeatherIcon::Unknown,
        }
    }
}

/// Screen navigation commands, sent as the second byte of the 0x39 payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ScreenMode {
    Up = 1,
    Down = 2,
    Enter = 3,
    Return = 4,
}

/// Encode a temperature value for the keyboard protocol.
/// Format: value * 10, with bit 15 set for negative temperatures.
#[inline]
pub fn encode_temperature(temp_celsius: i16) -> u16 {
    let tenths = temp_celsius.unsigned_abs().saturating_mul(10).min(0x7FFF);
    if temp_celsius >= 0 {
        tenths
    } else {
        tenths | 0x8000
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
        assert_eq!(encode_temperature(i16::MIN), 0xFFFF);
    }

    #[test]
    fn test_weather_icon_fallback() {
        assert_eq!(WeatherIcon::from_wmo(0, false), WeatherIcon::ClearNight);
        assert_eq!(WeatherIcon::from_wmo(200, true), WeatherIcon::Unknown);
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
