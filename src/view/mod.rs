//! Everything between the model and the screen: `bdi`'s own words for what the
//! model found, the cells of one row, the forest those rows are drawn from, and
//! the pane tail beneath it.

pub mod bindings;
pub mod draw;
pub mod fitted;
pub mod forest;
pub mod lines;
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
    /// Expand every node in the forest, at every depth.
    ExpandAll,
    /// Collapse every node in the forest, at every depth.
    CollapseAll,
    /// Let go of every fold set by hand, so the forest rests as `bdi` would
    /// have drawn it for the snapshot it is holding now.
    RestoreDefault,
    /// Show every tree, rather than only those with a live agent.
    ToggleFilter,
    /// Focus the selected bead's pane in herdr.
    Focus,
    /// Show every binding the view answers to.
    ShowBindings,
    Refresh,
    Quit,
}

/// Something true of the view as a whole rather than of any row in it, said
/// at the foot of the screen.
///
/// Two unrelated things produce these: a collection, every refresh, and this
/// process, once at startup before there is anything to collect. The status
/// bar is where they meet, and it is handed them in the order it should give
/// them up, so it draws a notice without knowing which kind it has and a
/// third kind needs no third path to the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    /// There is no herdr to ask about liveness, so no row can show an agent.
    NoHerdr,
    /// Nothing can tell `bdi` a project has changed, so every project is
    /// polled on the refresh interval and the view is as stale as that.
    NoInboundChannel,
}
