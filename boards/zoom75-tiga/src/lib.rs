//! High level hidapi abstraction for interacting with Zoom75 Tiga screen modules.
//!
//! This crate provides reverse-engineered bindings to control the Zoom75 Tiga keyboard's
//! built-in display via HID. The protocol uses CRC-16/CCITT-FALSE checksums with a 32-byte
//! packet structure.
//!
//! Screen size: 320x172 pixels, RGB565 format.

use std::sync::{LazyLock, RwLock};

use chrono::{DateTime, Datelike, Local, TimeZone, Timelike};
use hidapi::{HidApi, HidDevice};
use zoom_sync_core::{
    Board, BoardError, BoardInfo, Capabilities, HasGif, HasImage, HasTheme, HasTime, HasWeather,
    Result,
};
use zoom_tiga_protocol::{encode_gif, encode_temperature, Rgb565, ScreenMode, WeatherIcon};

pub use zoom_tiga_protocol::{self as protocol, SCREEN_HEIGHT, SCREEN_WIDTH};

pub mod consts {
    /// USB Vendor ID (TODO: fill in when known)
    pub const VENDOR_ID: u16 = 0;
    /// USB Product ID (TODO: fill in when known)
    pub const PRODUCT_ID: u16 = 0;
    /// HID usage page for the Zoom75 Tiga screen interface
    pub const USAGE_PAGE: u16 = 65376;
    /// HID usage for the Zoom75 Tiga screen interface
    pub const USAGE: u16 = 97;
}

/// Static board info for detection
pub static INFO: BoardInfo = BoardInfo {
    name: "Zoom75 Tiga",
    cli_name: "zoom75-tiga",
    vendor_id: None,
    product_id: None,
    usage_page: Some(consts::USAGE_PAGE),
    usage: Some(consts::USAGE),
    capabilities: Capabilities {
        theme: true,
        time: true,
        weather: true,
        image: true,
        system_info: false,
        screen: false,
        gif: true,
    },
};

/// Lazy handle to hidapi
static API: LazyLock<RwLock<HidApi>> =
    LazyLock::new(|| RwLock::new(HidApi::new().expect("failed to init hidapi")));

/// High level abstraction for managing a Zoom75 Tiga keyboard
pub struct Zoom75Tiga {
    pub device: HidDevice,
    buf: [u8; 64],
}

impl Zoom75Tiga {
    /// Find and open the device for modifications.
    ///
    /// NOTE: This will open any zoom device due to shared usages. Vendor and product IDs are
    ///       required for proper board detection.
    pub fn open() -> Result<Self> {
        API.write().unwrap().refresh_devices()?;
        let api = API.read().unwrap();
        let this = Self {
            device: api
                .device_list()
                .find(|d| {
                    // d.vendor_id() == consts::VENDOR_ID &&
                    // d.product_id() == consts::PRODUCT_ID &&
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
        let packet = protocol::datetime(
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
        let packet = protocol::weather(
            icon,
            encode_temperature(current),
            encode_temperature(high),
            encode_temperature(low),
        );
        self.execute(packet)
    }

    /// Set the screen theme colors.
    pub fn set_theme(&mut self, bg_color: Rgb565, font_color: Rgb565, theme_id: u8) -> Result<()> {
        let packet = protocol::theme(bg_color, font_color, theme_id);
        self.execute(packet)
    }

    /// Send a screen control command.
    pub fn screen_control(&mut self, mode: ScreenMode) -> Result<()> {
        let packet = protocol::screen_control(mode);
        self.execute(packet)
    }

    /// Navigate up in menu.
    pub fn screen_up(&mut self) -> Result<()> {
        self.screen_control(ScreenMode::Up)
    }

    /// Navigate down in menu.
    pub fn screen_down(&mut self) -> Result<()> {
        self.screen_control(ScreenMode::Down)
    }

    /// Enter/select menu item.
    pub fn screen_enter(&mut self) -> Result<()> {
        self.screen_control(ScreenMode::Enter)
    }

    /// Return/back from menu.
    pub fn screen_return(&mut self) -> Result<()> {
        self.screen_control(ScreenMode::Return)
    }

    /// Reset themes, gifs and images.
    pub fn screen_reset(&mut self) -> Result<()> {
        self.screen_control(ScreenMode::Reset)
    }

    /// Upload an image to the keyboard screen.
    ///
    /// The image data should be in RGB565 format.
    /// Screen size is 320x172 pixels.
    pub fn upload_image(&mut self, data: &[u8], cb: &mut dyn FnMut(usize)) -> Result<()> {
        const CHUNK_SIZE: usize = 16;
        let page_count = data.len().div_ceil(CHUNK_SIZE);

        // Send each chunk
        for (page_index, chunk) in data.chunks(CHUNK_SIZE).enumerate() {
            cb(page_index);
            let packet = protocol::image_chunk(page_index as u16, chunk);
            self.execute(packet)?;
        }

        // Send termination packet
        let packet = protocol::image_end();
        self.execute(packet)?;

        cb(page_count);
        Ok(())
    }

    /// Upload a RGB565 animation to the keyboard screen.
    ///
    /// The data should be encoded with frame count, delays, and RGB565 frame data.
    /// Format: [2 bytes frame count BE] + [2 bytes per frame delay BE] + [frame data...]
    pub fn upload_565_animation(&mut self, data: &[u8], cb: &mut dyn FnMut(usize)) -> Result<()> {
        // Animation uses the same upload mechanism as images
        self.upload_image(data, cb)
    }

    /// Clear the GIF/animation from the screen.
    pub fn clear_gif(&mut self) -> Result<()> {
        self.screen_reset()
    }
}

// === Trait Implementations ===

impl Board for Zoom75Tiga {
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

    fn as_gif(&mut self) -> Option<&mut dyn HasGif> {
        Some(self)
    }

    fn as_theme(&mut self) -> Option<&mut dyn HasTheme> {
        Some(self)
    }
}

impl HasTime for Zoom75Tiga {
    fn set_time(&mut self, time: DateTime<Local>, _use_12hr: bool) -> Result<()> {
        // Note: The Zoom75 Tiga uses 24-hour format internally, so we ignore _use_12hr
        Zoom75Tiga::set_time(self, time)
    }
}

impl HasWeather for Zoom75Tiga {
    fn set_weather(
        &mut self,
        wmo: u8,
        is_day: bool,
        current: i16,
        low: i16,
        high: i16,
    ) -> Result<()> {
        let icon = WeatherIcon::from_wmo(wmo, is_day)
            .ok_or(BoardError::CommandFailed("unknown WMO code"))?;
        Zoom75Tiga::set_weather(self, icon, current, low, high)
    }
}

impl HasImage for Zoom75Tiga {
    fn upload_image(&mut self, data: &[u8], progress: &mut dyn FnMut(usize)) -> Result<()> {
        Zoom75Tiga::upload_image(self, data, progress)
    }

    fn clear_image(&mut self) -> Result<()> {
        // Send empty termination to clear
        let packet = protocol::image_end();
        self.execute(packet)
    }
}

impl HasTheme for Zoom75Tiga {
    fn set_theme(&mut self, bg_color: u16, font_color: u16, theme_id: u8) -> Result<()> {
        Zoom75Tiga::set_theme(self, Rgb565(bg_color), Rgb565(font_color), theme_id)
    }
}

impl HasGif for Zoom75Tiga {
    fn upload_gif(&mut self, data: &[u8], progress: &mut dyn FnMut(usize)) -> Result<()> {
        // Re-encode standard GIF to RGB565 format
        let encoded = encode_gif(data, [0, 0, 0], false, |_, _| {})
            .ok_or(BoardError::InvalidMedia("failed to encode gif to rgb565"))?;
        self.upload_565_animation(&encoded, progress)
    }

    fn clear_gif(&mut self) -> Result<()> {
        Zoom75Tiga::clear_gif(self)
    }
}
