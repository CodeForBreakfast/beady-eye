//! Everything between the model and the screen: `bdi`'s own words for what the
//! model found, the cells of one row, the forest those rows are drawn from, and
//! the pane tail beneath it.

use std::collections::BTreeMap;

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

/// How fresh what is on the screen is, said at the foot beside the notices.
///
/// A collection running is the whole answer while it runs: it says the rows
/// are about to be replaced, which is what a reader watching them change
/// needs, and the read behind them is seconds from being superseded anyway.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    Collecting,
    /// The oldest read behind anything on the screen.
    Collected(DateTime<Utc>),
}

impl Freshness {
    /// What to say about a snapshot, given whether a collection is in flight.
    ///
    /// The resting answer is the *oldest* of the projects' reads, not the
    /// newest and not the snapshot's own clock. A refresh naming one project
    /// redraws every project, so the newest read describes only the project
    /// it named — and quoting it would tell a reader that rows nothing has
    /// touched for an interval arrived just now. The oldest is the weakest
    /// claim that is true of every row on the screen, so a reader is
    /// under-promised rather than misled.
    ///
    /// A read that failed counts as a read. Its trees went down with the
    /// tracker that refused, so none of its rows are on the screen to be
    /// stale — and holding the view back to the last read that *worked*
    /// would date rows nothing came from.
    ///
    /// Nothing at all where no project has been read: there is no row on the
    /// screen for the claim to be about.
    pub fn of(read_at: &BTreeMap<String, DateTime<Utc>>, collecting: bool) -> Option<Self> {
        if collecting {
            return Some(Freshness::Collecting);
        }
        read_at.values().min().copied().map(Freshness::Collected)
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

    fn read(times: &[(&str, DateTime<Utc>)]) -> BTreeMap<String, DateTime<Utc>> {
        times
            .iter()
            .map(|(project, at)| ((*project).to_string(), *at))
            .collect()
    }

    /// The screen holds rows from every project, so the claim it can make is
    /// the one that is true of all of them. Taking the newest would date the
    /// project a refresh did not name to a read that never saw it.
    #[test]
    fn the_time_shown_is_the_oldest_read_behind_anything_on_the_screen() {
        let read_at = read(&[("orbital", at(52, 9)), ("ferry", at(22, 14))]);

        assert_eq!(
            Freshness::of(&read_at, false),
            Some(Freshness::Collected(at(22, 14)))
        );
    }

    /// A collection running says the rows are about to move, which is what a
    /// reader watching them needs; the read it is about to replace is not.
    #[test]
    fn a_collection_in_flight_is_the_whole_answer() {
        let read_at = read(&[("orbital", at(22, 14))]);

        assert_eq!(Freshness::of(&read_at, true), Some(Freshness::Collecting));
    }

    #[test]
    fn a_screen_no_project_has_been_read_for_says_nothing_about_freshness() {
        assert_eq!(Freshness::of(&BTreeMap::new(), false), None);
    }
}
