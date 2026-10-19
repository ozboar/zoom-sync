//! Board detection and selection logic.

use bpaf::Bpaf;
use hidapi::HidApi;
use zoom65v3::{Zoom65v3, INFO as ZOOM65V3_INFO};
use zoom75_tiga::{Zoom75Tiga, INFO as ZOOM75_TIGA_INFO};
use zoom_sync_core::{Board, BoardError, BoardInfo, Capabilities};
use zoom_tkl_dyna::{ZoomTklDyna, INFO as ZOOM_TKL_DYNA_INFO};

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
    /// Zoom75 Tiga
    #[bpaf(long("zoom75-tiga"))]
    Zoom75Tiga,
}

/// Check if a HID device matches the board info
fn matches(device: &hidapi::DeviceInfo, info: &BoardInfo) -> bool {
    info.vendor_id.is_none_or(|vid| device.vendor_id() == vid)
        && info.product_id.is_none_or(|pid| device.product_id() == pid)
        && info.usage_page.is_none_or(|up| device.usage_page() == up)
        && info.usage.is_none_or(|u| device.usage() == u)
}

impl BoardKind {
    /// Get the static board info without connecting
    pub const fn info(&self) -> Option<&'static BoardInfo> {
        match self {
            BoardKind::Auto => None,
            BoardKind::Zoom65v3 => Some(&ZOOM65V3_INFO),
            BoardKind::ZoomTklDyna => Some(&ZOOM_TKL_DYNA_INFO),
            BoardKind::Zoom75Tiga => Some(&ZOOM75_TIGA_INFO),
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
            if matches(device, &ZOOM75_TIGA_INFO) {
                return Some(BoardKind::Zoom75Tiga);
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
                            screen_pos: true,
                            screen_nav: true,
                            image: true,
                            gif: true,
                            theme: true,
                        }
                    })
            },
            _ => self.info().map(|i| i.capabilities).unwrap_or_default(),
        }
    }

    /// Open the specified board. Errors on BoardKind::Auto
    pub fn as_board(self) -> Result<Box<dyn Board>, BoardError> {
        match self {
            BoardKind::Zoom65v3 => Ok(Box::new(Zoom65v3::open()?)),
            BoardKind::ZoomTklDyna => Ok(Box::new(ZoomTklDyna::open()?)),
            BoardKind::Zoom75Tiga => Ok(Box::new(Zoom75Tiga::open()?)),
            BoardKind::Auto => {
                if let Some(detected) = Self::detect() {
                    detected.as_board()
                } else {
                    Err(BoardError::DeviceNotFound)
                }
            },
        }
    }
}
