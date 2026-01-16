//! Board detection and selection logic.

use std::str::FromStr;

use bpaf::Bpaf;
use hidapi::HidApi;
use zoom65v3::{Zoom65v3, INFO as ZOOM65V3_INFO};
use zoom_tkl_dyna::{ZoomTklDyna, INFO as ZOOM_TKL_DYNA_INFO};
use zoom_sync_core::{Board, BoardError, BoardInfo, Capabilities};

/// Supported board types
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Bpaf)]
#[bpaf(fallback(BoardKind::Auto), group_help("Board selection:"))]
pub enum BoardKind {
    /// Auto-detect connected board (default)
    #[default]
    Auto,
    /// Zoom65 V3
    Zoom65v3,
    /// Zoom TKL Dyna
    ZoomTklDyna,
}

impl FromStr for BoardKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "zoom65v3" => Ok(Self::Zoom65v3),
            "zoom-tkl-dyna" | "zoomtkldyna" => Ok(Self::ZoomTklDyna),
            _ => Err(format!(
                "unknown board: {s}. Available: auto, zoom65v3, zoom-tkl-dyna"
            )),
        }
    }
}

impl std::fmt::Display for BoardKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auto => write!(f, "auto"),
            Self::Zoom65v3 => write!(f, "zoom65v3"),
            Self::ZoomTklDyna => write!(f, "zoom-tkl-dyna"),
        }
    }
}

/// Check if a HID device matches the board info
fn matches(device: &hidapi::DeviceInfo, info: &BoardInfo) -> bool {
    info.vendor_id.is_none_or(|vid| device.vendor_id() == vid)
        && info.product_id.is_none_or(|pid| device.product_id() == pid)
        && info.usage_page.is_none_or(|up| device.usage_page() == up)
        && info.usage.is_none_or(|u| device.usage() == u)
}

/// All known board infos for iteration
#[allow(dead_code)]
pub const ALL_BOARDS: &[&BoardInfo] = &[&ZOOM65V3_INFO, &ZOOM_TKL_DYNA_INFO];

impl BoardKind {
    /// Get the static board info without connecting
    pub fn info(&self) -> Option<&'static BoardInfo> {
        match self {
            BoardKind::Auto => None,
            BoardKind::Zoom65v3 => Some(&ZOOM65V3_INFO),
            BoardKind::ZoomTklDyna => Some(&ZOOM_TKL_DYNA_INFO),
        }
    }

    /// Detect which board is connected without opening it
    pub fn detect() -> Option<BoardKind> {
        let api = HidApi::new().ok()?;
        for device in api.device_list() {
            if matches(device, &ZOOM65V3_INFO) {
                return Some(BoardKind::Zoom65v3);
            }
            if matches(device, &ZOOM_TKL_DYNA_INFO) {
                return Some(BoardKind::ZoomTklDyna);
            }
        }
        None
    }

    /// Get capabilities for this board kind.
    /// For Auto, attempts to detect the connected board first.
    pub fn capabilities(&self) -> Capabilities {
        match self {
            BoardKind::Auto => {
                // Try to detect, fall back to union of all capabilities
                Self::detect()
                    .and_then(|k| k.info())
                    .map(|i| i.capabilities)
                    .unwrap_or_else(|| {
                        // Union of all board capabilities when no board detected
                        Capabilities {
                            time: true,
                            weather: true,
                            system_info: true,
                            screen: true,
                            image: true,
                            gif: true,
                            theme: true,
                        }
                    })
            },
            _ => self.info().map(|i| i.capabilities).unwrap_or_default(),
        }
    }

    /// Open the specified board, or auto-detect if Auto
    pub fn as_board(&self) -> Result<Box<dyn Board>, BoardError> {
        match self {
            BoardKind::Auto => {
                // Single HID iteration, check each board's INFO
                // Note: Zoom65v3 is checked first because it has more specific matching
                // (vendor_id + product_id), while ZoomTklDyna only uses usage_page + usage
                let api = HidApi::new()?;
                for device in api.device_list() {
                    if matches(device, &ZOOM65V3_INFO) {
                        return Ok(Box::new(Zoom65v3::open()?));
                    }
                    if matches(device, &ZOOM_TKL_DYNA_INFO) {
                        return Ok(Box::new(ZoomTklDyna::open()?));
                    }
                }
                Err(BoardError::DeviceNotFound)
            },
            BoardKind::Zoom65v3 => Ok(Box::new(Zoom65v3::open()?)),
            BoardKind::ZoomTklDyna => Ok(Box::new(ZoomTklDyna::open()?)),
        }
    }

    /// List all supported board CLI names
    #[allow(dead_code)]
    pub fn supported_boards() -> &'static [&'static str] {
        &["auto", "zoom65v3", "zoom-tkl-dyna"]
    }
}
