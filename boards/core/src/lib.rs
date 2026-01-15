//! Core traits and types for zoom-sync board abstraction.
//!
//! This crate provides:
//! - Feature traits (`HasTime`, `HasWeather`, etc.) that boards can implement
//! - The `Board` trait with `as_*()` methods for feature discovery
//! - Common types like `BoardInfo`, `ScreenPosition`, `WeatherIcon`

mod board;
mod features;

pub use board::{Board, BoardInfo, ScreenGroup, ScreenPosition, WeatherIcon};
pub use features::{
    HasGif, HasImage, HasScreen, HasScreenSize, HasSystemInfo, HasTime, HasWeather, Result,
};
