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
}

pub fn apply_screen(args: &ScreenArgs, board: &mut dyn Board) -> Result<(), Box<dyn Error>> {
    let screen = board
        .as_screen()
        .ok_or("board does not support screen control")?;

    match args {
        ScreenArgs::Screen(pos_id) => {
            let positions = screen.screen_positions();
            let pos = positions.iter().find(|p| p.id == pos_id.0).ok_or_else(|| {
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
    };
    Ok(())
}
