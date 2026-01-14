use std::str::FromStr;

use hidapi::HidError;

use crate::abi::Arg;

pub type Zoom65Result<T> = Result<T, Zoom65Error>;

#[derive(thiserror::Error)]
pub enum Zoom65Error {
    #[error("failed to find device")]
    DeviceNotFound,
    #[error("firmware version is unknown. open an issue for support")]
    UnknownFirmwareVersion,
    #[error("keyboard responded with error while updating")]
    UpdateCommandFailed,
    #[error("the provided image was the invalid (must be rgb565 with 0xff alpha channel)")]
    InvalidImage,
    #[error("the provided gif was too large")]
    GifTooLarge,
    #[error("{_0}")]
    Hid(#[from] HidError),
}

impl std::fmt::Debug for Zoom65Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self}")
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScreenTheme {
    #[default]
    Blue = 1,
    Pink = 2,
}

impl Arg for ScreenTheme {
    const SIZE: usize = 1;
    fn to_bytes(&self) -> Vec<u8> {
        vec![*self as u8]
    }
}

/// Channel to start uploading to
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum UploadChannel {
    Image = 1,
    Gif = 2,
}

impl Arg for UploadChannel {
    const SIZE: usize = 1;
    #[inline(always)]
    fn to_bytes(&self) -> Vec<u8> {
        vec![*self as u8]
    }
}

#[derive(Debug, Clone)]
#[repr(u8)]
pub enum Icon {
    DayClear = 0,
    DayPartlyCloudy = 1,
    DayPartlyRainy = 2,
    NightPartlyCloudy = 3,
    NightClear = 4,
    Cloudy = 5,
    Rainy = 6,
    Snowfall = 7,
    Thunderstorm = 8,
}

impl Icon {
    /// Convert a WMO index into a weather icon, adapting for day and night
    /// Adapted from the list at the bottom of <https://open-meteo.com/en/docs>
    pub fn from_wmo(wmo: u8, is_day: bool) -> Option<Self> {
        match wmo {
            // clear and mainly clear
            0 | 1 => Some(if is_day { Icon::DayClear } else { Icon::NightClear }),

            // partly cloudy
            2 => Some(if is_day { Icon::DayPartlyCloudy } else { Icon::NightPartlyCloudy }),

            // overcast
            3
            // foggy
            | 45 | 48 => Some(Icon::Cloudy),

            // drizzle
            51 | 53 | 55
            // freezing drizzle
            |56 | 57
            // rain
            | 61 | 63 | 65
            // freezing rain
            | 66 | 67 => Some(Icon::Rainy),

            // rain showers
            80..=82 => Some(if is_day { Icon::DayPartlyRainy } else { Icon::Rainy }),


            // snowfall
            | 71 | 73 | 75 | 77
            // snow showers
            | 85 | 86 => Some(Icon::Snowfall),

            // thunderstorm
            95 | 96 | 99 => Some(Icon::Thunderstorm),

            // unknown
            _ => None
        }
    }
}

impl Arg for Icon {
    const SIZE: usize = 1;
    #[inline(always)]
    fn to_bytes(&self) -> Vec<u8> {
        vec![self.clone() as u8]
    }
}

/// Available screen position and offsets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenPosition {
    System(SystemOffset), // up 2
    Time(TimeOffset),     // up 1
    Logo(LogoOffset),     // default
    Battery,              // down 1
}

impl Default for ScreenPosition {
    fn default() -> Self {
        Self::Logo(Default::default())
    }
}

impl ScreenPosition {
    pub const OPTIONS: &'static str = "[ cpu, gpu, download|d, time|t, weather|w, meletrix|m, zoom65|z, image|i, gif|g, battery|b ]";

    /// Convert screen position into directions from the default screen as `[up/down, shift]`
    pub fn to_directions(&self) -> (isize, usize) {
        match self {
            ScreenPosition::System(o) => (-2, *o as usize),
            ScreenPosition::Time(o) => (-1, *o as usize),
            ScreenPosition::Logo(o) => (0, *o as usize),
            ScreenPosition::Battery => (1, 0),
        }
    }
}

impl FromStr for ScreenPosition {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "cpu" => Ok(Self::System(SystemOffset::CpuTemp)),
            "gpu" => Ok(Self::System(SystemOffset::GpuTemp)),
            "download" | "d" => Ok(Self::System(SystemOffset::Download)),
            "time" | "t" => Ok(Self::Time(TimeOffset::Time)),
            "weather" | "w" => Ok(Self::Time(TimeOffset::Weather)),
            "meletrix" | "m" => Ok(Self::Logo(LogoOffset::Meletrix)),
            "zoom65" | "z" => Ok(Self::Logo(LogoOffset::Zoom65)),
            "image" | "i" => Ok(Self::Logo(LogoOffset::Image)),
            "gif" | "g" => Ok(Self::Logo(LogoOffset::Gif)),
            "battery" | "b" => Ok(Self::Battery),
            _ => Err(format!(
                "invalid screen position, must be one of: {}",
                Self::OPTIONS
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum SystemOffset {
    #[default]
    CpuTemp = 0,
    GpuTemp = 1,
    Download = 2,
}

impl SystemOffset {
    /// Convert into a full screen position type
    pub fn pos(&self) -> ScreenPosition {
        ScreenPosition::System(*self)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
#[repr(usize)]
pub enum TimeOffset {
    #[default]
    Time = 0,
    Weather = 1,
}

impl TimeOffset {
    /// Convert into a full screen position type
    pub fn pos(&self) -> ScreenPosition {
        ScreenPosition::Time(*self)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogoOffset {
    #[default]
    Meletrix = 0,
    Zoom65 = 1,
    Image = 2,
    Gif = 3,
}

impl LogoOffset {
    /// Convert into a full screen position type
    pub fn pos(&self) -> ScreenPosition {
        ScreenPosition::Logo(*self)
    }
}
