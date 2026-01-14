//! Feature traits for board capabilities.
//!
//! Boards opt-in to features by implementing these traits and returning
//! `Some(self)` from the corresponding `as_*()` method in the Board trait.

use std::error::Error;

use chrono::{DateTime, Local};

use crate::{ScreenPosition, WeatherIcon};

pub type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

/// Time synchronization capability
pub trait HasTime {
    fn set_time(&mut self, time: DateTime<Local>, use_12hr: bool) -> Result<()>;
}

/// Weather display capability
pub trait HasWeather {
    fn set_weather(&mut self, icon: WeatherIcon, current: u8, low: u8, high: u8) -> Result<()>;
}

/// System info display capability (CPU temp, GPU temp, download speed)
pub trait HasSystemInfo {
    fn set_system_info(&mut self, cpu: u8, gpu: u8, download: f32) -> Result<()>;
}

/// Screen position control capability
pub trait HasScreen {
    fn screen_positions(&self) -> &'static [ScreenPosition];
    fn set_screen(&mut self, position: &ScreenPosition) -> Result<()>;
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
    fn upload_image(&mut self, data: &[u8], progress: &dyn Fn(usize)) -> Result<()>;
    fn clear_image(&mut self) -> Result<()>;
}

/// Animated GIF upload capability
pub trait HasGif {
    fn upload_gif(&mut self, data: &[u8], progress: &dyn Fn(usize)) -> Result<()>;
    fn clear_gif(&mut self) -> Result<()>;
}
