//! High level hidapi abstraction for interacting with Zoom TKL Dyna screen modules.
//!
//! This crate provides reverse-engineered bindings to control the Zoom TKL Dyna keyboard's
//! built-in display via HID. The protocol uses CRC-16/CCITT-FALSE checksums with a 32-byte
//! packet structure.
//!
//! Screen size: 320x172 pixels, RGB565 format.

use std::io::Cursor;
use std::sync::atomic::AtomicU16;
use std::sync::{LazyLock, RwLock};

use chrono::{DateTime, Datelike, Local, TimeZone, Timelike};
use hidapi::{HidApi, HidDevice};
use image::codecs::gif::GifDecoder;
use image::AnimationDecoder;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use types::{encode_temperature, Rgb565, ScreenMode, WeatherIcon};
use zoom_sync_core::{
    Board, BoardError, BoardInfo, Capabilities, HasGif, HasImage, HasTheme, HasTime, HasWeather,
    Result,
};

pub mod abi;
pub mod crc;
pub mod types;

pub mod consts {
    /// USB Vendor ID
    pub const VENDOR_ID: u16 = 0x5542;
    /// USB Product ID
    pub const PRODUCT_ID: u16 = 0xC987;
    /// HID usage page for the Zoom TKL Dyna screen interface
    pub const USAGE_PAGE: u16 = 65376;
    /// HID usage for the Zoom TKL Dyna screen interface
    pub const USAGE: u16 = 97;
}

/// Static board info for detection
pub static INFO: BoardInfo = BoardInfo {
    name: "Zoom TKL Dyna",
    cli_name: "zoom-tkl-dyna",
    vendor_id: Some(consts::VENDOR_ID),
    product_id: Some(consts::PRODUCT_ID),
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

/// Screen dimensions
pub const SCREEN_WIDTH: u32 = 320;
pub const SCREEN_HEIGHT: u32 = 172;

/// Encode a raw GIF buffer as RGB565 with frame delays for TKL Dyna.
///
/// Output format:
/// - 2 bytes: frame count (u16 BE)
/// - 2 bytes per frame: delay in centiseconds (u16 BE)
/// - Then: RGB565+alpha data for all frames (concatenated)
///
/// The callback receives (current_frame, total_frames) for progress updates.
pub fn encode_gif(
    gif_data: &[u8],
    background: [u8; 3],
    nearest: bool,
    cb: impl Fn(usize, usize) + Sync,
) -> Option<Vec<u8>> {
    let decoder = GifDecoder::new(Cursor::new(gif_data)).ok()?;
    let frames = decoder.into_frames().collect_frames().ok()?;
    let frame_count = frames.len();
    let [br, bg, bb] = background;

    let filter = if nearest {
        image::imageops::FilterType::Nearest
    } else {
        image::imageops::FilterType::Gaussian
    };

    // Extract delays (in centiseconds) and encode frames
    let completed = AtomicU16::new(1);
    let encoded_frames: Vec<(u16, Vec<u8>)> = frames
        .par_iter()
        .map(|frame| {
            // Get delay in centiseconds
            let delay = frame.delay();
            let (numer, denom) = delay.numer_denom_ms();
            let delay_cs = ((numer / denom) / 10) as u16;

            // Resize and encode frame as RGB565
            let resized =
                image::imageops::resize(frame.buffer(), SCREEN_WIDTH, SCREEN_HEIGHT, filter);
            let buf: Vec<u8> = resized
                .pixels()
                .flat_map(|p| {
                    let [mut r, mut g, mut b, a] = p.0;

                    // Mix alpha values against background
                    let a = a as f64 / 255.0;
                    let ba = 1. - a;
                    r = ((br as f64 * ba) + (r as f64 * a)) as u8;
                    g = ((bg as f64 * ba) + (g as f64 * a)) as u8;
                    b = ((bb as f64 * ba) + (b as f64 * a)) as u8;

                    // Convert into rgb565 pixel type
                    let [x, y] = rgb565::Rgb565::from_rgb888_components(r, g, b).to_rgb565_be();

                    // Extend with hard coded alpha channel
                    [x, y, 0xff]
                })
                .collect();

            let i = completed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            cb(i as usize, frame_count);

            (delay_cs, buf)
        })
        .collect();

    // Build output: header + delays + frame data
    let mut output = Vec::new();

    // Frame count (2 bytes, BE)
    output.extend_from_slice(&(frame_count as u16).to_be_bytes());

    // Frame delays (2 bytes each, BE)
    for (delay, _) in &encoded_frames {
        output.extend_from_slice(&delay.to_be_bytes());
    }

    // Frame data (concatenated)
    for (_, data) in encoded_frames {
        output.extend(data);
    }

    Some(output)
}

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
                    d.vendor_id() == consts::VENDOR_ID
                        && d.product_id() == consts::PRODUCT_ID
                        && d.usage_page() == consts::USAGE_PAGE
                        && d.usage() == consts::USAGE
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
    pub fn set_theme(&mut self, bg_color: Rgb565, font_color: Rgb565, theme_id: u8) -> Result<()> {
        let packet = abi::theme(bg_color, font_color, theme_id);
        self.execute(packet)
    }

    /// Send a screen control command.
    pub fn screen_control(&mut self, mode: ScreenMode) -> Result<()> {
        let packet = abi::screen_control(mode);
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
            let packet = abi::image_chunk(page_index as u16, chunk);
            self.execute(packet)?;
        }

        // Send termination packet
        let packet = abi::image_end();
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

    fn as_gif(&mut self) -> Option<&mut dyn HasGif> {
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

impl HasGif for ZoomTklDyna {
    fn upload_gif(&mut self, data: &[u8], progress: &mut dyn FnMut(usize)) -> Result<()> {
        // Re-encode standard GIF to RGB565 format
        let encoded = encode_gif(data, [0, 0, 0], false, |_, _| {})
            .ok_or(BoardError::InvalidMedia("failed to encode gif to rgb565"))?;
        self.upload_565_animation(&encoded, progress)
    }

    fn clear_gif(&mut self) -> Result<()> {
        ZoomTklDyna::clear_gif(self)
    }
}
