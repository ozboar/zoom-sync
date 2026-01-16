//! Shared HID protocol implementation for Zoom Tiga-based keyboards.
//!
//! This crate provides the common protocol primitives used by keyboards with
//! the Tiga screen module, including:
//! - Zoom TKL Dyna
//! - Zoom75 Tiga
//!
//! ## Protocol Overview
//!
//! The protocol uses 32-byte HID packets with CRC-16/CCITT-FALSE checksums.
//! Screen size: 320x172 pixels, RGB565 format.

pub mod abi;
pub mod crc;
pub mod types;
pub mod utils;

pub use abi::*;
pub use crc::*;
pub use types::*;
pub use utils::*;

/// Screen width in pixels
pub const SCREEN_WIDTH: u32 = 320;
/// Screen height in pixels
pub const SCREEN_HEIGHT: u32 = 172;
