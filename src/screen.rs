use std::error::Error;

use bpaf::{Bpaf, Parser};
use zoom_sync_core::Board;

/// Screen position ID (string-based for board independence)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenPositionId(pub String);

impl std::str::FromStr for ScreenPositionId {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.to_lowercase()))
    }
}

/// Screen options:
#[derive(Clone, Debug, PartialEq, Eq, Bpaf)]
pub enum ScreenArgs {
    Screen(
        /// Reset and move the screen to a specific position.
        /// [cpu|gpu|download|time|weather|meletrix|zoom65|image|gif|battery]
        #[bpaf(short('s'), long("screen"), argument("POSITION"))]
        ScreenPositionId,
    ),
    /// Move the screen up
    Up,
    /// Move the screen down
    Down,
    /// Switch the screen offset
    Switch,
    #[cfg(target_os = "linux")]
    /// Reactive image/gif mode
    #[bpaf(skip)]
    Reactive,
}

pub fn screen_args_with_reactive() -> impl Parser<ScreenArgs> {
    #[cfg(not(target_os = "linux"))]
    {
        screen_args()
    }

    #[cfg(target_os = "linux")]
    {
        let reactive = bpaf::long("reactive")
            .help("Enable reactive mode, playing gif when typing and image when resting. Requires root permission for reading keypresses via evdev")
            .req_flag(ScreenArgs::Reactive);
        bpaf::construct!([reactive, screen_args()]).group_help("Screen options:")
    }
}

pub fn apply_screen(args: &ScreenArgs, board: &mut dyn Board) -> Result<(), Box<dyn Error>> {
    let screen = board
        .as_screen()
        .ok_or("board does not support screen control")?;

    match args {
        ScreenArgs::Screen(pos_id) => {
            let positions = screen.screen_positions();
            let pos = positions
                .iter()
                .find(|p| p.id == pos_id.0)
                .ok_or_else(|| {
                    let valid: Vec<_> = positions.iter().map(|p| p.id).collect();
                    format!(
                        "invalid screen position '{}'. Valid: {}",
                        pos_id.0,
                        valid.join(", ")
                    )
                })?;
            screen.set_screen(pos.id)?;
        },
        ScreenArgs::Up => screen.screen_up()?,
        ScreenArgs::Down => screen.screen_down()?,
        ScreenArgs::Switch => screen.screen_switch()?,
        #[cfg(target_os = "linux")]
        ScreenArgs::Reactive => return Err("cannot apply reactive gif natively".into()),
    };
    Ok(())
}
