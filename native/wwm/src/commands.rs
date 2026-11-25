//! Commands sent from the hotkey thread to the tiler/focus worker thread.

#[derive(Clone, Copy, Debug)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    /// `true` for the "previous" directions (left/up).
    pub fn is_prev(self) -> bool {
        matches!(self, Direction::Left | Direction::Up)
    }
}

pub enum Command {
    /// A desktop switch just happened: stop flashes and focus the new desktop.
    FocusAfterSwitch,
    /// Move focus to the previous/next tiled window.
    Focus(Direction),
    /// Move the focused window in the tile order.
    Reorder(Direction),
    /// Toggle the focused window between floating and tiled.
    ToggleFloat,
    /// Snap the focused window to a half of its monitor (marks it floating).
    Snap(Direction),
}
