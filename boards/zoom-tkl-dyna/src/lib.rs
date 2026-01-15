//! High level hidapi abstraction for interacting with Zoom TKL Dyna screen modules.
//!
//! This crate provides reverse-engineered bindings to control the Zoom TKL Dyna keyboard's
//! built-in display via HID. The protocol uses CRC-16/CCITT-FALSE checksums with a 32-byte
//! packet structure.
//!
//! Screen size: 320x172 pixels, RGB565 format.

use std::sync::{LazyLock, RwLock};

use chrono::{DateTime, Datelike, Local, TimeZone, Timelike};
use hidapi::{HidApi, HidDevice};
use types::{encode_temperature, Rgb565, ScreenMode, WeatherIcon};
use zoom_sync_core::{
    Board, BoardError, BoardInfo, HasImage, HasTheme, HasTime, HasWeather, Result,
};

pub mod abi;
pub mod crc;
pub mod types;

pub mod consts {
    /// HID usage page for the Zoom TKL Dyna screen interface
    pub const USAGE_PAGE: u16 = 65376;
    /// HID usage for the Zoom TKL Dyna screen interface
    pub const USAGE: u16 = 97;
}

/// Static board info for detection
pub static INFO: BoardInfo = BoardInfo {
    name: "Zoom TKL Dyna",
    cli_name: "zoom-tkl-dyna",
    vendor_id: None,
    product_id: None,
    usage_page: Some(consts::USAGE_PAGE),
    usage: Some(consts::USAGE),
};

/// Screen dimensions
pub const SCREEN_WIDTH: u32 = 320;
pub const SCREEN_HEIGHT: u32 = 172;

/// Lazy handle to hidapi
static API: LazyLock<RwLock<HidApi>> =
    LazyLock::new(|| RwLock::new(HidApi::new().expect("failed to init hidapi")));

/// High level abstraction for managing a Zoom TKL Dyna keyboard
pub struct ZoomTklDyna {
    pub device: HidDevice,
    buf: [u8; 64],
}

impl ZoomTklDyna {
    /// Find and open the device for modifications
    pub fn open() -> Result<Self> {
        API.write().unwrap().refresh_devices()?;
        let api = API.read().unwrap();
        let this = Self {
            device: api
                .device_list()
                .find(|d| {
                    d.usage_page() == consts::USAGE_PAGE && d.usage() == consts::USAGE
                })
                .ok_or(BoardError::DeviceNotFound)?
                .open_device(&api)?,
            buf: [0u8; 64],
        };

        Ok(this)
    }

    /// Internal method to execute a packet and optionally read the response
    fn execute(&mut self, packet: [u8; 32]) -> Result<()> {
        self.device.write(&packet)?;
        // Read response if available
        let _ = self.device.read_timeout(&mut self.buf, 100);
        Ok(())
    }

    /// Sync the current date and time to the keyboard display.
    pub fn set_time<Tz: TimeZone>(&mut self, time: DateTime<Tz>) -> Result<()> {
        let packet = abi::datetime(
            time.year() as u16,
            time.month() as u8,
            time.day() as u8,
            time.hour() as u8,
            time.minute() as u8,
            time.second() as u8,
            time.weekday().num_days_from_sunday() as u8,
        );
        self.execute(packet)
    }

    /// Update the weather display.
    pub fn set_weather(
        &mut self,
        icon: WeatherIcon,
        current: i16,
        low: i16,
        high: i16,
    ) -> Result<()> {
        let packet = abi::weather(
            icon,
            encode_temperature(current),
            encode_temperature(high),
            encode_temperature(low),
        );
        self.execute(packet)
    }

    /// Set the screen theme colors.
    pub fn set_theme(
        &mut self,
        bg_color: Rgb565,
        font_color: Rgb565,
        theme_id: u8,
    ) -> Result<()> {
        let packet = abi::theme(bg_color, font_color, theme_id);
        self.execute(packet)
    }

    /// Send a screen control command.
    pub fn screen_control(&mut self, mode: ScreenMode) -> Result<()> {
        let packet = abi::screen_control(mode);
        self.execute(packet)
    }

    /// Refresh the screen (mode 2).
    pub fn screen_refresh(&mut self) -> Result<()> {
        self.screen_control(ScreenMode::Refresh)
    }

    /// Next screen/theme (mode 3).
    pub fn screen_next(&mut self) -> Result<()> {
        self.screen_control(ScreenMode::Next)
    }

    /// Previous screen/theme (mode 4).
    pub fn screen_previous(&mut self) -> Result<()> {
        self.screen_control(ScreenMode::Previous)
    }

    /// Upload an image to the keyboard screen.
    ///
    /// The image data should be in RGB565 format with a 452-byte header.
    /// Screen size is 320x172 pixels.
    pub fn upload_image(&mut self, data: &[u8], cb: &mut dyn FnMut(usize)) -> Result<()> {
        const CHUNK_SIZE: usize = 16;
        let page_count = (data.len() + CHUNK_SIZE - 1) / CHUNK_SIZE;

        // Send each chunk
        for (page_index, chunk) in data.chunks(CHUNK_SIZE).enumerate() {
            cb(page_index);
            let packet = abi::image_chunk(page_index as u16, chunk);
            self.execute(packet)?;
        }

        // Send termination packet
        let packet = abi::image_end();
        self.execute(packet)?;

        cb(page_count);
        Ok(())
    }
}

// === Trait Implementations ===

impl Board for ZoomTklDyna {
    fn info(&self) -> &'static BoardInfo {
        &INFO
    }

    fn as_time(&mut self) -> Option<&mut dyn HasTime> {
        Some(self)
    }

    fn as_weather(&mut self) -> Option<&mut dyn HasWeather> {
        Some(self)
    }

    fn as_screen_size(&self) -> Option<(u32, u32)> {
        Some((SCREEN_WIDTH, SCREEN_HEIGHT))
    }

    fn as_image(&mut self) -> Option<&mut dyn HasImage> {
        Some(self)
    }

    fn as_theme(&mut self) -> Option<&mut dyn HasTheme> {
        Some(self)
    }
}

impl HasTime for ZoomTklDyna {
    fn set_time(&mut self, time: DateTime<Local>, _use_12hr: bool) -> Result<()> {
        // Note: The TKL Dyna uses 24-hour format internally, so we ignore _use_12hr
        ZoomTklDyna::set_time(self, time)
    }
}

impl HasWeather for ZoomTklDyna {
    fn set_weather(&mut self, wmo: u8, is_day: bool, current: i16, low: i16, high: i16) -> Result<()> {
        let icon = WeatherIcon::from_wmo(wmo, is_day)
            .ok_or(BoardError::CommandFailed("unknown WMO code"))?;
        ZoomTklDyna::set_weather(self, icon, current, low, high)
    }
}

impl HasImage for ZoomTklDyna {
    fn upload_image(&mut self, data: &[u8], progress: &mut dyn FnMut(usize)) -> Result<()> {
        ZoomTklDyna::upload_image(self, data, progress)
    }

    fn clear_image(&mut self) -> Result<()> {
        // Send empty termination to clear
        let packet = abi::image_end();
        self.execute(packet)
    }
}

impl HasTheme for ZoomTklDyna {
    fn set_theme(&mut self, bg_color: u16, font_color: u16, theme_id: u8) -> Result<()> {
        ZoomTklDyna::set_theme(self, Rgb565(bg_color), Rgb565(font_color), theme_id)
    }
}
