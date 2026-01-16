//! Command and state types for tray-daemon communication

use crate::config::Config;

/// Commands sent from tray menu to the daemon
#[derive(Debug, Clone)]
pub enum TrayCommand {
    /// Set screen to specific position (by ID) and save as default
    SetScreen(&'static str),
    /// Toggle weather updates
    ToggleWeather,
    /// Toggle system info updates
    ToggleSystemInfo,
    /// Toggle 12-hour time format
    Toggle12HrTime,
    /// Toggle fahrenheit/celsius
    ToggleFahrenheit,
    /// Upload pre-encoded image data
    UploadImage(Vec<u8>),
    /// Upload pre-encoded GIF data
    UploadGif(Vec<u8>),
    /// Clear uploaded image
    ClearImage,
    /// Clear uploaded GIF
    ClearGif,
    /// Clear all media
    ClearAllMedia,
    /// Reload config from file
    ReloadConfig,
    /// Quit the application
    Quit,
}

/// Connection status for keyboard
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConnectionStatus {
    #[default]
    Disconnected,
    Connected,
    Reconnecting,
}

impl ConnectionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConnectionStatus::Disconnected => "Disconnected",
            ConnectionStatus::Connected => "Connected",
            ConnectionStatus::Reconnecting => "Reconnecting...",
        }
    }
}

/// State shared from daemon to tray for UI updates
#[derive(Debug, Clone, Default)]
pub struct TrayState {
    pub connection: ConnectionStatus,
    pub current_screen: Option<String>,
    pub config: Config,
    /// Whether reactive mode is currently active (Linux only)
    pub reactive_active: bool,
}
