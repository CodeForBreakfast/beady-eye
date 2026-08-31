//! Everything between the model and the screen: `bdi`'s own words for what the
//! model found, the cells of one row, the forest those rows are drawn from, and
//! the pane tail beneath it.

use chrono::{DateTime, Utc};

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

/// How fresh one project's rows are, said beside its name.
///
/// A collection running is the whole answer while it runs: it says the rows
/// are about to be replaced, which is what a reader watching them change
/// needs, and the read behind them is seconds from being superseded anyway.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    Collecting,
    /// When this project's tracker was last read.
    Collected(DateTime<Utc>),
}

impl Freshness {
    /// What to say about one project: when it was last read, and whether the
    /// collection in flight is reading it now.
    ///
    /// One project rather than the screen. A single indicator had to quote
    /// the *oldest* read of any project on it — the weakest claim that was
    /// true of every row — because a refresh naming one project redraws them
    /// all, and the newest read describes only the project it named. That
    /// under-promise was the cost of standing in the foot and speaking for
    /// rows it could not tell apart. An indicator beside a project's own name
    /// speaks for that project's rows alone, so it is exact.
    ///
    /// A read that failed counts as a read. Its trees went down with the
    /// tracker that refused, so none of its rows are on the screen to be
    /// stale — and holding the line back to the last read that *worked*
    /// would date rows nothing came from.
    ///
    /// Nothing at all for a project neither read nor being read: there is no
    /// row on the screen for the claim to be about.
    pub fn of(read_at: Option<DateTime<Utc>>, collecting: bool) -> Option<Self> {
        if collecting {
            return Some(Freshness::Collecting);
        }
        read_at.map(Freshness::Collected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use pretty_assertions::assert_eq;

    fn at(minute: u32, second: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 30, 10, minute, second)
            .unwrap()
    }

    /// A project the collection in flight is not reading keeps the read it
    /// has. The whole point of moving the indicator off the foot: one
    /// project's collection used to say every project was collecting.
    #[test]
    fn a_project_no_collection_is_reading_says_when_it_was_last_read() {
        assert_eq!(
            Freshness::of(Some(at(22, 14)), false),
            Some(Freshness::Collected(at(22, 14)))
        );
    }

    /// A collection running says the rows are about to move, which is what a
    /// reader watching them needs; the read it is about to replace is not.
    #[test]
    fn a_project_being_read_now_says_so_over_the_read_it_is_replacing() {
        assert_eq!(
            Freshness::of(Some(at(22, 14)), true),
            Some(Freshness::Collecting)
        );
    }

    /// The startup frame: every project drawn before any of them has been
    /// read. There is no time to quote, and the collection under way is the
    /// whole of what the line can say.
    #[test]
    fn a_project_never_read_but_being_read_now_still_says_it_is_collecting() {
        assert_eq!(Freshness::of(None, true), Some(Freshness::Collecting));
    }

    #[test]
    fn a_project_neither_read_nor_being_read_says_nothing_about_freshness() {
        assert_eq!(Freshness::of(None, false), None);
    }
}
