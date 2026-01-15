//! High level hidapi abstraction for interacting with zoom65v3 screen modules

use std::sync::{LazyLock, RwLock};

use checksum::checksum;
use chrono::{DateTime, Datelike, Local, TimeZone, Timelike};
use float::DumbFloat16;
use hidapi::{HidApi, HidDevice};
use types::{Icon, ScreenPosition, ScreenTheme, UploadChannel};
use zoom_sync_core::{
    Board, BoardError, BoardInfo, HasGif, HasImage, HasScreen, HasScreenSize, HasSystemInfo,
    HasTime, HasWeather, Result, ScreenGroup, ScreenPosition as CoreScreenPosition,
};

pub mod abi;
pub mod checksum;
pub mod float;
pub mod types;

pub mod consts {
    pub const ZOOM65_VENDOR_ID: u16 = 0x36B5;
    pub const ZOOM65_PRODUCT_ID: u16 = 0x287F;
    pub const ZOOM65_USAGE_PAGE: u16 = 65376;
    pub const ZOOM65_USAGE: u16 = 97;
}

/// Static board info for detection
pub static INFO: BoardInfo = BoardInfo {
    name: "Zoom65 V3",
    cli_name: "zoom65v3",
    vendor_id: Some(consts::ZOOM65_VENDOR_ID),
    product_id: Some(consts::ZOOM65_PRODUCT_ID),
    usage_page: Some(consts::ZOOM65_USAGE_PAGE),
    usage: Some(consts::ZOOM65_USAGE),
};

/// Screen positions for this board
pub static SCREEN_POSITIONS: &[CoreScreenPosition] = &[
    CoreScreenPosition {
        id: "cpu",
        display_name: "CPU Temp",
        group: ScreenGroup::System,
    },
    CoreScreenPosition {
        id: "gpu",
        display_name: "GPU Temp",
        group: ScreenGroup::System,
    },
    CoreScreenPosition {
        id: "download",
        display_name: "Download",
        group: ScreenGroup::System,
    },
    CoreScreenPosition {
        id: "time",
        display_name: "Time",
        group: ScreenGroup::Time,
    },
    CoreScreenPosition {
        id: "weather",
        display_name: "Weather",
        group: ScreenGroup::Time,
    },
    CoreScreenPosition {
        id: "meletrix",
        display_name: "Meletrix",
        group: ScreenGroup::Logo,
    },
    CoreScreenPosition {
        id: "zoom65",
        display_name: "Zoom65",
        group: ScreenGroup::Logo,
    },
    CoreScreenPosition {
        id: "image",
        display_name: "Image",
        group: ScreenGroup::Logo,
    },
    CoreScreenPosition {
        id: "gif",
        display_name: "GIF",
        group: ScreenGroup::Logo,
    },
    CoreScreenPosition {
        id: "battery",
        display_name: "Battery",
        group: ScreenGroup::Battery,
    },
];

/// Screen dimensions
pub const SCREEN_WIDTH: u32 = 110;
pub const SCREEN_HEIGHT: u32 = 110;

/// Lazy handle to hidapi
static API: LazyLock<RwLock<HidApi>> =
    LazyLock::new(|| RwLock::new(HidApi::new().expect("failed to init hidapi")));

/// High level abstraction for managing a zoom65 v3 keyboard
pub struct Zoom65v3 {
    pub device: HidDevice,
    buf: [u8; 64],
}

impl Zoom65v3 {
    /// Find and open the device for modifications
    pub fn open() -> Result<Self> {
        API.write().unwrap().refresh_devices()?;
        let api = API.read().unwrap();
        let this = Self {
            device: api
                .device_list()
                .find(|d| {
                    d.vendor_id() == consts::ZOOM65_VENDOR_ID
                        && d.product_id() == consts::ZOOM65_PRODUCT_ID
                        && d.usage_page() == consts::ZOOM65_USAGE_PAGE
                        && d.usage() == consts::ZOOM65_USAGE
                })
                .ok_or(BoardError::DeviceNotFound)?
                .open_device(&api)?,
            buf: [0u8; 64],
        };

        Ok(this)
    }

    /// Internal method to execute a payload and read the response
    fn execute(&mut self, payload: [u8; 33]) -> Result<Vec<u8>> {
        self.device.write(&payload)?;
        let len = self.device.read(&mut self.buf)?;
        let slice = &self.buf[..len];
        assert!(slice[0] == payload[1]);
        Ok(slice.to_vec())
    }

    /// Set the screen theme. Will reset the screen back to the meletrix logo
    #[inline(always)]
    pub fn screen_theme(&mut self, theme: ScreenTheme) -> Result<()> {
        let res = self.execute(abi::screen_theme(theme))?;
        (res[1] == 1 && res[2] == 1)
            .then_some(())
            .ok_or(BoardError::CommandFailed("device rejected command"))
    }

    /// Increment the screen position
    #[inline(always)]
    pub fn screen_up(&mut self) -> Result<()> {
        let res = self.execute(abi::screen_up())?;
        (res[1] == 1 && res[2] == 1)
            .then_some(())
            .ok_or(BoardError::CommandFailed("device rejected command"))
    }

    /// Decrement the screen position
    #[inline(always)]
    pub fn screen_down(&mut self) -> Result<()> {
        let res = self.execute(abi::screen_down())?;
        (res[1] == 1 && res[2] == 1)
            .then_some(())
            .ok_or(BoardError::CommandFailed("device rejected command"))
    }

    /// Switch the active screen
    #[inline(always)]
    pub fn screen_switch(&mut self) -> Result<()> {
        let res = self.execute(abi::screen_switch())?;
        (res[1] == 1 && res[2] == 1)
            .then_some(())
            .ok_or(BoardError::CommandFailed("device rejected command"))
    }

    /// Reset the screen back to the meletrix logo
    #[inline(always)]
    pub fn reset_screen(&mut self) -> Result<()> {
        let res = self.execute(abi::reset_screen())?;
        (res[1] == 1 && res[2] == 1)
            .then_some(())
            .ok_or(BoardError::CommandFailed("device rejected command"))
    }

    /// Set the screen to a specific position and offset
    pub fn set_screen(&mut self, position: ScreenPosition) -> Result<()> {
        let (y, x) = position.to_directions();

        // Back to default
        self.reset_screen()?;

        // Move screen up or down
        match y {
            y if y < 0 => {
                for _ in 0..y.abs() {
                    self.screen_up()?;
                }
            },
            y if y > 0 => {
                for _ in 0..y.abs() {
                    self.screen_down()?;
                }
            },
            _ => {},
        }

        // Switch screen to offset
        for _ in 0..x {
            self.screen_switch()?;
        }

        Ok(())
    }

    /// Update the keyboards current time.
    /// If 12hr is true, hardcodes the time to 01:00-12:00 for the current day.
    #[inline(always)]
    pub fn set_time<Tz: TimeZone>(&mut self, time: DateTime<Tz>, _12hr: bool) -> Result<()> {
        let res = self.execute(abi::set_time(
            // Provide the current year without the century.
            // This prevents overflows on the year 2256 (meletrix web ui just subtracts 2000)
            (time.year() % 100) as u8,
            time.month() as u8,
            time.day() as u8,
            if _12hr { time.hour12().1 } else { time.hour() } as u8,
            time.minute() as u8,
            time.second() as u8,
        ))?;
        (res[1] == 1 && res[2] == 1)
            .then_some(())
            .ok_or(BoardError::CommandFailed("device rejected command"))
    }

    /// Update the keyboards current weather report
    #[inline(always)]
    pub fn set_weather(&mut self, icon: Icon, current: u8, low: u8, high: u8) -> Result<()> {
        let res = self.execute(abi::set_weather(icon, current, low, high))?;
        (res[1] == 1 && res[2] == 1)
            .then_some(())
            .ok_or(BoardError::CommandFailed("device rejected command"))
    }

    /// Update the keyboards current system info
    #[inline(always)]
    pub fn set_system_info(
        &mut self,
        cpu_temp: u8,
        gpu_temp: u8,
        download_rate: f32,
    ) -> Result<()> {
        let download = DumbFloat16::new(download_rate);
        let res = self.execute(abi::set_system_info(cpu_temp, gpu_temp, download))?;
        (res[1] == 1 && res[2] == 1)
            .then_some(())
            .ok_or(BoardError::CommandFailed("device rejected command"))
    }

    fn upload_media(
        &mut self,
        buf: impl AsRef<[u8]>,
        channel: UploadChannel,
        cb: &mut dyn FnMut(usize),
    ) -> Result<()> {
        let image = buf.as_ref();

        // start upload
        let res = self.execute(abi::upload_start(channel))?;
        if res[1] != 1 || res[2] != 1 {
            return Err(BoardError::CommandFailed("device rejected command"));
        }
        let res = self.execute(abi::upload_length(image.len() as u32))?;
        if res[1] != 1 || res[2] != 1 {
            return Err(BoardError::CommandFailed("device rejected command"));
        }

        for (i, chunk) in image.chunks(24).enumerate() {
            cb(i);

            let chunk_len = chunk.len();
            let mut buf = [0u8; 33];

            // command prefix
            buf[0] = 0x0;
            buf[1] = 88;
            buf[2] = 2 + chunk_len as u8 + 4;

            // chunk index and data
            buf[3] = (i >> 8) as u8;
            buf[4] = (i & 255) as u8;
            buf[5..5 + chunk.len()].copy_from_slice(chunk);

            let mut offset = 3 + 2 + chunk_len;

            // Images are always aligned, but we need to manually align the last chunk of gifs
            if channel == UploadChannel::Gif && i == image.len() / 24 {
                // compute padding for final payload, the checksum needs 32-bit alignment
                let padding = (4 - (image.len() % 24) % 4) % 4;
                buf[2] += padding as u8;
                offset += padding;
            }

            // compute checksum
            let data = &buf[3..offset + 2];
            let crc = checksum(data);
            buf[offset..offset + 4].copy_from_slice(&crc);

            // send payload and read response
            let res = self.execute(buf)?;
            if res[1] != 1 || res[2] != 1 {
                return Err(BoardError::CommandFailed("device rejected command"));
            }
        }

        let res = self.execute(abi::upload_end())?;
        if res[1] != 1 || res[2] != 1 {
            return Err(BoardError::CommandFailed("device rejected command"));
        }

        // TODO: is this required?
        self.reset_screen()?;

        Ok(())
    }

    /// Upload an image to the keyboard. Must be encoded as 110x110 RGBA-3328 raw buffer
    #[inline(always)]
    pub fn upload_image(&mut self, buf: impl AsRef<[u8]>, mut cb: impl FnMut(usize)) -> Result<()> {
        let buf = buf.as_ref();
        if buf.len() != 36300 {
            return Err(BoardError::MediaTooLarge(
                "image must be exactly 36300 bytes",
            ));
        }
        self.upload_media(buf, UploadChannel::Image, &mut cb)
    }

    /// Upload a gif to the keyboard. Must be 111x111.
    #[inline(always)]
    pub fn upload_gif(&mut self, buf: impl AsRef<[u8]>, mut cb: impl FnMut(usize)) -> Result<()> {
        if buf.as_ref().len() >= 1013808 {
            return Err(BoardError::MediaTooLarge("gif exceeds device limit"));
        }
        self.upload_media(buf, UploadChannel::Gif, &mut cb)
    }

    /// Clear the image slot
    #[inline(always)]
    pub fn clear_image(&mut self) -> Result<()> {
        let res = self.execute(abi::delete_image())?;
        (res[1] == 1 && res[2] == 1)
            .then_some(())
            .ok_or(BoardError::CommandFailed("device rejected command"))
    }

    /// Clear the gif slot
    #[inline(always)]
    pub fn clear_gif(&mut self) -> Result<()> {
        let res = self.execute(abi::delete_gif())?;
        (res[1] == 1 && res[2] == 1)
            .then_some(())
            .ok_or(BoardError::CommandFailed("device rejected command"))
    }
}

// === Trait Implementations ===

impl Board for Zoom65v3 {
    fn info(&self) -> &'static BoardInfo {
        &INFO
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

    fn as_screen(&mut self) -> Option<&mut dyn HasScreen> {
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
}

impl HasTime for Zoom65v3 {
    fn set_time(&mut self, time: DateTime<Local>, use_12hr: bool) -> Result<()> {
        Zoom65v3::set_time(self, time, use_12hr)
    }
}

impl HasWeather for Zoom65v3 {
    fn set_weather(&mut self, wmo: u8, is_day: bool, current: i16, low: i16, high: i16) -> Result<()> {
        let icon =
            Icon::from_wmo(wmo, is_day).ok_or(BoardError::CommandFailed("unknown WMO code"))?;
        // Clamp to u8 range for this board's protocol
        Zoom65v3::set_weather(
            self,
            icon,
            current.clamp(0, 255) as u8,
            low.clamp(0, 255) as u8,
            high.clamp(0, 255) as u8,
        )
    }
}

impl HasSystemInfo for Zoom65v3 {
    fn set_system_info(&mut self, cpu: u8, gpu: u8, download: f32) -> Result<()> {
        Zoom65v3::set_system_info(self, cpu, gpu, download)
    }
}

impl HasScreen for Zoom65v3 {
    fn screen_positions(&self) -> &'static [CoreScreenPosition] {
        SCREEN_POSITIONS
    }

    fn set_screen(&mut self, id: &str) -> Result<()> {
        Zoom65v3::set_screen(self, id.parse().map_err(BoardError::InvalidScreenPosition)?)
    }

    fn screen_up(&mut self) -> Result<()> {
        Zoom65v3::screen_up(self)
    }

    fn screen_down(&mut self) -> Result<()> {
        Zoom65v3::screen_down(self)
    }

    fn screen_switch(&mut self) -> Result<()> {
        Zoom65v3::screen_switch(self)
    }

    fn reset_screen(&mut self) -> Result<()> {
        Zoom65v3::reset_screen(self)
    }
}

impl HasScreenSize for Zoom65v3 {
    fn screen_size(&self) -> (u32, u32) {
        (SCREEN_WIDTH, SCREEN_HEIGHT)
    }
}

impl HasImage for Zoom65v3 {
    fn upload_image(&mut self, data: &[u8], progress: &mut dyn FnMut(usize)) -> Result<()> {
        Zoom65v3::upload_image(self, data, progress)
    }

    fn clear_image(&mut self) -> Result<()> {
        Zoom65v3::clear_image(self)
    }
}

impl HasGif for Zoom65v3 {
    fn upload_gif(&mut self, data: &[u8], progress: &mut dyn FnMut(usize)) -> Result<()> {
        Zoom65v3::upload_gif(self, data, progress)
    }

    fn clear_gif(&mut self) -> Result<()> {
        Zoom65v3::clear_gif(self)
    }
}
