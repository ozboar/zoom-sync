//! Feature traits for board capabilities.
//!
//! Boards opt-in to features by implementing these traits and returning
//! `Some(self)` from the corresponding `as_*()` method in the Board trait.

use chrono::{DateTime, Local};

use crate::ScreenPosition;

/// Errors that can occur during board operations
#[derive(Debug, thiserror::Error)]
pub enum BoardError {
    /// Device was not found
    #[error("device not found")]
    DeviceNotFound,

    /// Command failed on the device
    #[error("command failed: {0}")]
    CommandFailed(&'static str),

    /// Invalid screen position
    #[error("invalid screen position: {0}")]
    InvalidScreenPosition(String),

    /// Invalid media data
    #[error("invalid media: {0}")]
    InvalidMedia(&'static str),

    /// Media too large for device
    #[error("media too large: {0}")]
    MediaTooLarge(&'static str),

    /// HID communication error
    #[error("hid error: {0}")]
    Hid(#[from] hidapi::HidError),

    /// Generic IO error
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, BoardError>;

/// Time synchronization capability
pub trait HasTime {
    fn set_time(&mut self, time: DateTime<Local>, use_12hr: bool) -> Result<()>;
}

/// Weather display capability
pub trait HasWeather {
    /// Set weather display. WMO code is converted to board-specific icon internally.
    /// Temperatures are in Celsius - each board converts to its native format.
    fn set_weather(&mut self, wmo: u8, is_day: bool, current: i16, low: i16, high: i16)
        -> Result<()>;
}

/// System info display capability (CPU temp, GPU temp, download speed)
pub trait HasSystemInfo {
    fn set_system_info(&mut self, cpu: u8, gpu: u8, download: f32) -> Result<()>;
}

/// Screen position control capability
pub trait HasScreen {
    /// Available screen positions for this board
    fn screen_positions(&self) -> &'static [ScreenPosition];
    /// Set screen by position ID (e.g., "cpu", "weather", "gif")
    fn set_screen(&mut self, id: &str) -> Result<()>;
    fn screen_up(&mut self) -> Result<()>;
    fn screen_down(&mut self) -> Result<()>;
    fn screen_switch(&mut self) -> Result<()>;
    fn reset_screen(&mut self) -> Result<()>;
}

/// Screen dimensions - boards with media support should also implement as_screen_size()
pub trait HasScreenSize {
    fn screen_size(&self) -> (u32, u32);
}

/// Static image upload capability
pub trait HasImage {
    fn upload_image(&mut self, data: &[u8], progress: &mut dyn FnMut(usize)) -> Result<()>;
    fn clear_image(&mut self) -> Result<()>;
}

/// Animated GIF upload capability
pub trait HasGif {
    fn upload_gif(&mut self, data: &[u8], progress: &mut dyn FnMut(usize)) -> Result<()>;
    fn clear_gif(&mut self) -> Result<()>;
}

/// Theme customization capability (background color, font color)
pub trait HasTheme {
    /// Set screen theme with RGB565 colors
    /// - bg_color: Background color as RGB565 (16-bit)
    /// - font_color: Font color as RGB565 (16-bit)
    /// - theme_id: Theme preset ID
    fn set_theme(&mut self, bg_color: u16, font_color: u16, theme_id: u8) -> Result<()>;
}
