//! Core Board trait and related types.

use crate::{
    HasGif, HasImage, HasScreenNavigation, HasScreenPositions, HasSystemInfo, HasTheme, HasTime,
    HasWeather,
};

/// Static capability flags for a board (compile-time known)
#[derive(Debug, Clone, Copy, Default)]
pub struct Capabilities {
    pub time: bool,
    pub weather: bool,
    pub system_info: bool,
    pub screen_pos: bool,
    pub screen_nav: bool,
    pub image: bool,
    pub gif: bool,
    pub theme: bool,
}

/// Static information about a board type for detection and CLI
#[derive(Debug, Clone, Copy)]
pub struct BoardInfo {
    pub name: &'static str,
    pub cli_name: &'static str,
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    pub usage_page: Option<u16>,
    pub usage: Option<u16>,
    pub capabilities: Capabilities,
}

/// Unified enum representing all screen positions across all devices
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScreenPosition {
    // Zoom65v3
    Cpu,
    Gpu,
    Download,
    Time,
    Weather,
    Meletrix,
    Zoom,
    Image,
    Gif,
    Battery,
    // Add additional board specific positions here
}

impl ScreenPosition {
    /// Get the string identifier for this screen position
    pub fn as_id(&self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Gpu => "gpu",
            Self::Download => "download",
            Self::Time => "time",
            Self::Weather => "weather",
            Self::Meletrix => "meletrix",
            Self::Zoom => "zoom",
            Self::Image => "image",
            Self::Gif => "gif",
            Self::Battery => "battery",
        }
    }
}

/// Core board trait - object-safe for `dyn Board`
///
/// Instance methods (`info`, `as_*`) are object-safe.
/// Boards should provide a static `INFO` constant and `open()` method separately.
pub trait Board: Send {
    /// Get board info (instance method for object safety)
    fn info(&self) -> &'static BoardInfo;

    // Screen navigation features
    fn as_screen_size(&self) -> Option<(u32, u32)> {
        None
    }
    fn as_screen_nav(&mut self) -> Option<&mut dyn HasScreenNavigation> {
        None
    }
    fn as_screen_pos(&mut self) -> Option<&mut dyn HasScreenPositions> {
        None
    }

    // Media features
    fn as_image(&mut self) -> Option<&mut dyn HasImage> {
        None
    }
    fn as_gif(&mut self) -> Option<&mut dyn HasGif> {
        None
    }

    // Information features
    fn as_time(&mut self) -> Option<&mut dyn HasTime> {
        None
    }
    fn as_weather(&mut self) -> Option<&mut dyn HasWeather> {
        None
    }
    fn as_system_info(&mut self) -> Option<&mut dyn HasSystemInfo> {
        None
    }

    fn as_theme(&mut self) -> Option<&mut dyn HasTheme> {
        None
    }
}
