//! Core Board trait and related types.

use crate::features::{HasGif, HasImage, HasScreen, HasSystemInfo, HasTheme, HasTime, HasWeather};

/// Static information about a board type for detection and CLI
#[derive(Debug, Clone, Copy)]
pub struct BoardInfo {
    pub name: &'static str,
    pub cli_name: &'static str,
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    pub usage_page: Option<u16>,
    pub usage: Option<u16>,
}

/// Screen position for menu building
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScreenPosition {
    pub id: &'static str,
    pub display_name: &'static str,
    pub group: ScreenGroup,
}

/// Screen position grouping for menu organization
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScreenGroup {
    System,
    Time,
    Logo,
    Battery,
}

/// Core board trait - object-safe for `dyn Board`
///
/// Instance methods (`info`, `as_*`) are object-safe.
/// Boards should provide a static `INFO` constant and `open()` method separately.
pub trait Board: Send {
    // === Object-safe instance methods ===

    /// Get board info (instance method for object safety)
    fn info(&self) -> &'static BoardInfo;

    /// Feature opt-in methods - override to return `Some(self)` if feature is supported
    fn as_time(&mut self) -> Option<&mut dyn HasTime> {
        None
    }
    fn as_weather(&mut self) -> Option<&mut dyn HasWeather> {
        None
    }
    fn as_system_info(&mut self) -> Option<&mut dyn HasSystemInfo> {
        None
    }
    fn as_screen(&mut self) -> Option<&mut dyn HasScreen> {
        None
    }
    fn as_screen_size(&self) -> Option<(u32, u32)> {
        None
    }
    fn as_image(&mut self) -> Option<&mut dyn HasImage> {
        None
    }
    fn as_gif(&mut self) -> Option<&mut dyn HasGif> {
        None
    }
    fn as_theme(&mut self) -> Option<&mut dyn HasTheme> {
        None
    }
}
