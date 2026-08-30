//! Everything between the model and the screen: `bdi`'s own words for what the
//! model found, the cells of one row, the forest those rows are drawn from, and
//! the pane tail beneath it.

pub mod draw;
pub mod forest;
pub mod phrase;
pub mod row;
pub mod tail;

/// Where a keystroke moves the selection.
///
/// Named by the motion rather than the key: `bdi`'s bindings are vim-like, but
/// the view knows nothing about which key produced a motion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    PreviousRow,
    NextRow,
    HalfScreenUp,
    HalfScreenDown,
    FirstRow,
    LastRow,
}

/// What the user asked the view to do, in the view's own terms.
///
/// The seam between the loop that reads keys and the forest that changes state:
/// each side names this type and neither names the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Move(Motion),
    /// Collapse the selected node, or move to its parent when it is already
    /// collapsed.
    CollapseOrParent,
    /// Expand the selected node, or move to its first child when it is already
    /// expanded.
    ExpandOrChild,
    ToggleFold,
    /// Show every tree, rather than only those with a live agent.
    ToggleFilter,
    /// Focus the selected bead's pane in herdr.
    Focus,
    Refresh,
    Quit,
}
