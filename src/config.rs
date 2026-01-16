//! Configuration file handling for tray mode

use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct Config {
    pub general: GeneralConfig,
    pub refresh: RefreshConfig,
    pub weather: WeatherConfig,
    pub system_info: SystemInfoConfig,
    pub media: MediaConfig,
}

impl Config {
    /// Get the config file path for this platform
    pub fn path() -> Option<PathBuf> {
        ProjectDirs::from("", "", "zoom-sync").map(|dirs| dirs.config_dir().join("config.toml"))
    }

    /// Load config from file, or create default if it doesn't exist
    pub fn load_or_create() -> Result<Self, Box<dyn Error>> {
        let path = Self::path().ok_or("could not determine config directory")?;

        if path.exists() {
            let contents = fs::read_to_string(&path)?;
            let config: Config = toml::from_str(&contents)?;
            Ok(config)
        } else {
            let config = Config::default();
            config.save_with_header()?;
            println!("created default config at {}", path.display());
            Ok(config)
        }
    }

    /// Save config to file
    pub fn save(&self) -> Result<(), Box<dyn Error>> {
        let path = Self::path().ok_or("could not determine config directory")?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let contents = toml::to_string_pretty(self)?;
        fs::write(&path, contents)?;
        Ok(())
    }

    /// Save config with header comments for new files
    pub fn save_with_header(&self) -> Result<(), Box<dyn Error>> {
        let path = Self::path().ok_or("could not determine config directory")?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let header = r#"# zoom-sync configuration file
# https://github.com/ozwaldorf/zoom-sync

"#;
        let contents = toml::to_string_pretty(self)?;
        fs::write(&path, format!("{header}{contents}"))?;
        Ok(())
    }

    /// Reload config from file
    pub fn reload(&mut self) -> Result<(), Box<dyn Error>> {
        let path = Self::path().ok_or("could not determine config directory")?;
        let contents = fs::read_to_string(&path)?;
        *self = toml::from_str(&contents)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    /// Use fahrenheit instead of celsius
    pub fahrenheit: bool,
    /// Use 12-hour time format
    pub use_12hr_time: bool,
    /// Initial screen position on connect (use "reactive" for reactive mode on Linux)
    pub initial_screen: String,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            fahrenheit: false,
            use_12hr_time: false,
            initial_screen: "meletrix".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RefreshConfig {
    /// System info refresh interval
    #[serde(with = "humantime_serde")]
    pub system: Duration,
    /// Weather refresh interval
    #[serde(with = "humantime_serde")]
    pub weather: Duration,
    /// Keyboard reconnection retry interval
    #[serde(with = "humantime_serde")]
    pub retry: Duration,
}

impl Default for RefreshConfig {
    fn default() -> Self {
        Self {
            system: Duration::from_secs(10),
            weather: Duration::from_secs(60 * 60),
            retry: Duration::from_secs(5),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WeatherConfig {
    /// Enable weather updates
    pub enabled: bool,
    /// Manual latitude (optional)
    pub latitude: Option<f64>,
    /// Manual longitude (optional)
    pub longitude: Option<f64>,
}

impl Default for WeatherConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            latitude: None,
            longitude: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SystemInfoConfig {
    /// Enable system info updates
    pub enabled: bool,
    /// CPU temperature sensor label ("auto" for automatic)
    pub cpu_source: String,
    /// GPU device index
    pub gpu_device: u32,
}

impl Default for SystemInfoConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cpu_source: "Package".into(),
            gpu_device: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaConfig {
    /// Background color for transparent images (hex)
    pub background_color: String,
    /// Use nearest neighbor interpolation
    pub use_nearest_neighbor: bool,
    /// Last uploaded image path
    pub last_image: Option<PathBuf>,
    /// Last uploaded GIF path
    pub last_gif: Option<PathBuf>,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            background_color: "#000000".into(),
            use_nearest_neighbor: false,
            last_image: None,
            last_gif: None,
        }
    }
}
