use std::error::Error;

use bpaf::Bpaf;
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
        /// Zoom65v3:
        /// [cpu|gpu|download|time|weather|meletrix|zoom65|image|gif|battery]
        /// Zoom TKL DYNA:
        /// [up|down|enter|return]
        #[bpaf(short('s'), long("screen"), argument("POSITION"))]
        ScreenPositionId,
    ),
    /// Move the screen up
    Up,
    /// Move the screen down
    Down,
    /// Switch the screen offset
    Switch,
    /// Reset the screen to default position
    Reset,
}

pub fn apply_screen(args: &ScreenArgs, board: &mut dyn Board) -> Result<(), Box<dyn Error>> {
    match args {
        ScreenArgs::Screen(pos_id) => {
            let screen = board
                .as_screen_pos()
                .ok_or("board does not support setting screen position")?;
            let positions = screen.screen_positions();
            let pos = positions
                .iter()
                .find(|p| p.as_id() == pos_id.0)
                .ok_or_else(|| {
                    let valid: Vec<_> = positions.iter().map(|p| p.as_id()).collect();
                    format!(
                        "invalid screen position '{}'. Valid: {}",
                        pos_id.0,
                        valid.join(", ")
                    )
                })?;
            screen.set_screen(pos.as_id())?;
        },
        ScreenArgs::Up => {
            board
                .as_screen_nav()
                .ok_or("board does not support screen navigation")?
                .screen_up()?;
        },
        ScreenArgs::Down => {
            board
                .as_screen_nav()
                .ok_or("board does not support screen navigation")?
                .screen_down()?;
        },
        ScreenArgs::Switch => {
            board
                .as_screen_nav()
                .ok_or("board does not support screen navigation")?
                .screen_switch()?;
        },
        ScreenArgs::Reset => {
            board
                .as_screen_nav()
                .ok_or("board does not support screen navigation")?
                .screen_reset()?;
        },
    };
    Ok(())
}
