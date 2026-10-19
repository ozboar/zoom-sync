//! High level hidapi abstraction for interacting with Zoom TKL Dyna screen modules.
//!
//! This crate provides reverse-engineered bindings to control the Zoom TKL Dyna keyboard's
//! built-in display via HID. The protocol uses CRC-16/CCITT-FALSE checksums with a 32-byte
//! packet structure.
//!
//! Screen size: 320x172 pixels, RGB565 format.

use std::io::Cursor;
use std::sync::{LazyLock, RwLock};
use std::time::Duration;

use chrono::{DateTime, Datelike, Local, TimeZone, Timelike};
use hidapi::{HidApi, HidDevice};
use image::codecs::gif::GifDecoder;
use image::{imageops, AnimationDecoder};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use types::{encode_temperature, Rgb565, ScreenMode, WeatherIcon};
use zoom_sync_core::{
    Board, BoardError, BoardInfo, Capabilities, HasGif, HasImage, HasScreenNavigation,
    HasSystemInfo, HasTheme, HasTime, HasWeather, Result,
};

pub mod abi;
pub mod crc;
pub mod types;

pub mod consts {
    /// USB Vendor ID
    pub const VENDOR_ID: u16 = 0x5542;
    /// USB Product ID
    pub const PRODUCT_ID: u16 = 0xC987;
    /// HID usage page
    pub const USAGE_PAGE: u16 = 65376;
    /// HID usage
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
        system_info: true,
        screen_pos: false,
        screen_nav: true,
        gif: true,
    },
};

/// Screen dimensions
pub const SCREEN_WIDTH: u32 = 320;
pub const SCREEN_HEIGHT: u32 = 172;

/// Size of the header prepended to uploaded media
const MEDIA_HEADER_LEN: usize = 452;
/// Bytes of media data carried by each upload page
const PAGE_SIZE: usize = 16;

/// Media upload header. Contents are static regardless of frame count.
fn media_header() -> [u8; MEDIA_HEADER_LEN] {
    let mut header = [0u8; MEDIA_HEADER_LEN];
    header[360..364].copy_from_slice(&(MEDIA_HEADER_LEN as u32).to_le_bytes());
    header[400] = 1;
    header[403] = 1;
    header
}

/// Convert a screen sized RGBA frame into little-endian RGB565, ignoring alpha.
fn encode_frame(frame: &image::RgbaImage) -> Vec<u8> {
    frame
        .pixels()
        .flat_map(|p| {
            let [r, g, b, _] = p.0;
            Rgb565::from_rgb(r, g, b).to_le_bytes()
        })
        .collect()
}

/// Re-encode a GIF as concatenated screen sized RGB565 little-endian frames.
///
/// Frames larger than the screen are cropped from the top left corner, smaller frames are
/// resized to fit. Frame delays are not supported by the device.
pub fn encode_gif(gif_data: &[u8]) -> Option<Vec<u8>> {
    let frames = GifDecoder::new(Cursor::new(gif_data))
        .ok()?
        .into_frames()
        .collect_frames()
        .ok()?;
    let encoded: Vec<Vec<u8>> = frames
        .par_iter()
        .map(|frame| {
            let buf = frame.buffer();
            let frame = if buf.width() >= SCREEN_WIDTH && buf.height() >= SCREEN_HEIGHT {
                imageops::crop_imm(buf, 0, 0, SCREEN_WIDTH, SCREEN_HEIGHT).to_image()
            } else {
                imageops::resize(
                    buf,
                    SCREEN_WIDTH,
                    SCREEN_HEIGHT,
                    imageops::FilterType::Gaussian,
                )
            };
            encode_frame(&frame)
        })
        .collect();
    Some(encoded.concat())
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
        let mut this = Self {
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

        // Same info query the web driver sends on connect
        this.execute(abi::info())?;

        Ok(this)
    }

    /// Internal method to execute a packet and optionally read the response
    fn execute(&mut self, packet: [u8; 33]) -> Result<()> {
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

    /// Update the system info display
    pub fn set_system_info(
        &mut self,
        cpu_temp: u8,
        gpu_temp: u16,
        download_rate: f32,
        gpu_fan_speed: u16,
    ) -> Result<()> {
        let mut gpu_fan_speed_adjusted = gpu_fan_speed;
        if gpu_fan_speed >= 10000 {
            eprintln!("warning: actual fan speed at {gpu_fan_speed}. clamping to 9999");
            gpu_fan_speed_adjusted = 9999;
        }

        let packet =
            abi::set_system_info(cpu_temp, gpu_temp, download_rate, gpu_fan_speed_adjusted);
        self.execute(packet)
    }

    /// Set the screen theme colors.
    /// For TKL Dyna, this sets the reactive typing color themeing built into the screen.
    pub fn set_theme(&mut self, bg_color: Rgb565, font_color: Rgb565, theme_id: u8) -> Result<()> {
        let packet = abi::theme(bg_color, font_color, theme_id);
        self.execute(packet)
    }

    /// Send a screen navigation command.
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

    /// Restart the screen module.
    pub fn restart_screen(&mut self) -> Result<()> {
        self.execute(abi::restart())?;
        std::thread::sleep(Duration::from_millis(100));
        self.execute(abi::info())
    }

    /// Upload media to the keyboard screen.
    ///
    /// `frames` is one or more concatenated 320x172 frames of little-endian RGB565 pixels.
    /// The callback receives the current page index.
    pub fn upload_media(&mut self, frames: &[u8], cb: &mut dyn FnMut(usize)) -> Result<()> {
        let frame_len = (SCREEN_WIDTH * SCREEN_HEIGHT * 2) as usize;
        if frames.is_empty() || !frames.len().is_multiple_of(frame_len) {
            return Err(BoardError::InvalidMedia(
                "data is not a whole number of frames",
            ));
        }

        let mut data = Vec::with_capacity(MEDIA_HEADER_LEN + frames.len());
        data.extend_from_slice(&media_header());
        data.extend_from_slice(frames);

        let page_count = data.len().div_ceil(PAGE_SIZE);
        for (page_index, chunk) in data.chunks(PAGE_SIZE).enumerate() {
            cb(page_index);
            // The page index wraps past u16::MAX, matching the web driver
            self.execute(abi::image_chunk(page_index as u16, chunk))?;
        }
        self.execute(abi::image_end())?;

        cb(page_count);
        Ok(())
    }
}

// === Trait Implementations ===

impl Board for ZoomTklDyna {
    fn info(&self) -> &'static BoardInfo {
        &INFO
    }

    fn as_theme(&mut self) -> Option<&mut dyn HasTheme> {
        Some(self)
    }

    fn as_screen_size(&self) -> Option<(u32, u32)> {
        Some((SCREEN_WIDTH, SCREEN_HEIGHT))
    }

    fn as_screen_nav(&mut self) -> Option<&mut dyn HasScreenNavigation> {
        Some(self)
    }

    fn as_image(&mut self) -> Option<&mut dyn HasImage> {
        Some(self)
    }

    fn as_gif(&mut self) -> Option<&mut dyn HasGif> {
        Some(self)
    }

    fn as_time(&mut self) -> Option<&mut dyn HasTime> {
        Some(self)
    }

    fn as_weather(&mut self) -> Option<&mut dyn HasWeather> {
        Some(self)
    }

    fn as_system_info(&mut self) -> Option<&mut dyn HasSystemInfo> {
        Some(self)
    }
}

impl HasTheme for ZoomTklDyna {
    fn set_theme(&mut self, bg_color: u16, font_color: u16, theme_id: u8) -> Result<()> {
        ZoomTklDyna::set_theme(self, Rgb565(bg_color), Rgb565(font_color), theme_id)
    }
}

impl HasScreenNavigation for ZoomTklDyna {
    fn screen_up(&mut self) -> Result<()> {
        ZoomTklDyna::screen_up(self)
    }
    fn screen_down(&mut self) -> Result<()> {
        ZoomTklDyna::screen_down(self)
    }
    fn screen_switch(&mut self) -> Result<()> {
        ZoomTklDyna::screen_enter(self)
    }
    fn screen_reset(&mut self) -> Result<()> {
        ZoomTklDyna::screen_return(self)
    }
}

impl HasImage for ZoomTklDyna {
    fn upload_image(&mut self, data: &[u8], progress: &mut dyn FnMut(usize)) -> Result<()> {
        // Input is big-endian RGB565 with a trailing alpha byte per pixel
        if data.len() != (SCREEN_WIDTH * SCREEN_HEIGHT * 3) as usize {
            return Err(BoardError::InvalidMedia("image does not match screen size"));
        }
        let frame: Vec<u8> = data.chunks_exact(3).flat_map(|p| [p[1], p[0]]).collect();
        self.upload_media(&frame, progress)
    }

    fn clear_image(&mut self) -> Result<()> {
        // Send empty termination to clear
        let packet = abi::image_end();
        self.execute(packet)
    }
}

impl HasGif for ZoomTklDyna {
    fn upload_gif(&mut self, data: &[u8], progress: &mut dyn FnMut(usize)) -> Result<()> {
        let encoded =
            encode_gif(data).ok_or(BoardError::InvalidMedia("failed to encode gif to rgb565"))?;
        self.upload_media(&encoded, progress)
    }

    fn clear_gif(&mut self) -> Result<()> {
        self.restart_screen()
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
        let icon = WeatherIcon::from_wmo(wmo, is_day);
        ZoomTklDyna::set_weather(self, icon, current, low, high)
    }
}

impl HasSystemInfo for ZoomTklDyna {
    fn set_system_info(&mut self, cpu: u8, gpu: u32, download: f32, fan_rpm: u32) -> Result<()> {
        ZoomTklDyna::set_system_info(
            self,
            cpu,
            gpu.try_into().unwrap_or(u16::MAX),
            download,
            fan_rpm.try_into().unwrap_or(u16::MAX),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_header_matches_reference() {
        let header = media_header();
        assert_eq!(&header[360..364], &[196, 1, 0, 0]);
        assert_eq!(header[400], 1);
        assert_eq!(header[403], 1);
        assert_eq!(header.iter().filter(|&&b| b != 0).count(), 4);
    }

    #[test]
    fn frame_is_little_endian() {
        let frame = image::RgbaImage::from_pixel(1, 1, [255, 0, 0, 255].into());
        assert_eq!(encode_frame(&frame), vec![0x00, 0xF8]);
    }
}
