//! HID transport shared by Tiga-based keyboards.

use std::time::{Duration, Instant};

use hidapi::HidDevice;
use zoom_sync_core::{BoardError, Result};

use crate::abi::{self, cmd};
use crate::utils::{media_header, FRAME_LEN};

/// Bytes of media data carried by each upload page
const PAGE_SIZE: usize = 16;
/// How long to wait for a reply to regular commands
const COMMAND_TIMEOUT: Duration = Duration::from_millis(100);
/// How long to wait for an upload page to be acknowledged before resending
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(1);
/// Number of times an upload page is sent before giving up
const UPLOAD_ATTEMPTS: usize = 3;

/// An open Tiga screen module
pub struct TigaDevice {
    pub device: HidDevice,
    buf: [u8; 64],
}

impl TigaDevice {
    /// Wrap an opened hid device, sending the same info query the web driver sends on connect.
    pub fn new(device: HidDevice) -> Result<Self> {
        let mut this = Self {
            device,
            buf: [0u8; 64],
        };
        this.execute(abi::info())?;
        Ok(this)
    }

    /// Wait for a reply to `command`, discarding unrelated input reports.
    /// Returns false if the timeout elapsed without a reply.
    fn wait_for(&mut self, command: u8, timeout: Duration) -> Result<bool> {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(false);
            }
            let len = self
                .device
                .read_timeout(&mut self.buf, remaining.as_millis().max(1) as i32)?;
            if len == 0 {
                return Ok(false);
            }
            if len > 9 && self.buf[0] == 0x1C && self.buf[9] == command {
                return Ok(true);
            }
        }
    }

    /// Discard any pending input reports.
    fn drain(&mut self) -> Result<()> {
        while self.device.read_timeout(&mut self.buf, 0)? > 0 {}
        Ok(())
    }

    /// Send a packet and consume its reply, if any
    pub fn execute(&mut self, packet: [u8; 33]) -> Result<()> {
        self.device.write(&packet)?;
        self.wait_for(packet[10], COMMAND_TIMEOUT)?;
        Ok(())
    }

    /// Send an upload packet, resending until the device acknowledges it
    fn execute_upload(&mut self, packet: [u8; 33]) -> Result<()> {
        for _ in 0..UPLOAD_ATTEMPTS {
            self.device.write(&packet)?;
            if self.wait_for(cmd::IMAGE, UPLOAD_TIMEOUT)? {
                return Ok(());
            }
        }
        Err(BoardError::CommandFailed(
            "device did not acknowledge upload",
        ))
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
        if frames.is_empty() || !frames.len().is_multiple_of(FRAME_LEN) {
            return Err(BoardError::InvalidMedia(
                "data is not a whole number of frames",
            ));
        }

        let header = media_header();
        let mut data = Vec::with_capacity(header.len() + frames.len());
        data.extend_from_slice(&header);
        data.extend_from_slice(frames);

        self.drain()?;
        let page_count = data.len().div_ceil(PAGE_SIZE);
        for (page_index, chunk) in data.chunks(PAGE_SIZE).enumerate() {
            cb(page_index);
            // The page index wraps past u16::MAX, matching the web driver
            self.execute_upload(abi::image_chunk(page_index as u16, chunk))?;
        }
        self.execute_upload(abi::image_end())?;

        cb(page_count);
        Ok(())
    }
}
