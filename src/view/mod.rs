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

/// The one way to ask what a widget drew, reachable from every test that
/// renders to a buffer — `view/` and `tui/` alike, which is why it sits here
/// rather than inside the module that first needed it.
#[cfg(test)]
pub(crate) mod painted;

/// The one bounded walk over the rows on screen, reachable from every test
/// that drives the selection — `view/` and `tui/` alike, which is why it sits
/// here rather than inside the module that first needed it.
#[cfg(test)]
pub(crate) mod walk;

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
    /// The same loss, with the one cause a reader can do something about:
    /// another `bdi` had the channel when this one asked for it. Closing that
    /// one frees the path but gives this run nothing, since the socket is
    /// asked for once and never again — the line on the primary screen is
    /// where the whole remedy is said, because it takes two steps and the
    /// foot has room for neither.
    ///
    /// Said in the tense of the refusal rather than as a claim about a
    /// process that is still running. `bdi` asks for the socket once, at
    /// startup, and nothing re-checks — so all this ever reports is what was
    /// true then, and the holder may have gone since.
    AnotherBdiHadTheInboundChannel,
}

/// How fresh one project's rows are, said beside its name.
///
/// Two things, and both of them are on the line at all times. The mark says
/// what the collection is doing or how the last one went; the age says how
/// old the rows under the name are. They answer different questions, and a
/// cell that swapped one for the other left the reader watching a mark turn
/// over rows of unknown age.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Freshness {
    pub mark: Mark,
    /// When this project's tracker was last read, where it ever has been.
    ///
    /// Absent only while the first collection of it is still running: there
    /// is no read to date the rows to, and no rows either.
    pub read_at: Option<DateTime<Utc>>,
}

/// What the mark beside a project's name says.
///
/// One column in every state, so the cell beside the name does not change
/// width for a collection starting or ending — which was the whole of what
/// made the old cell jump.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// A collection is reading this project now.
    Collecting,
    /// The last collection read every root of it.
    Read,
    /// The last collection found a root it could not read.
    Refused,
}

impl Mark {
    /// What the mark says with no collection reading this project.
    ///
    /// A project several of whose roots disagree resolves to one mark, and it
    /// resolves to the worse of them: the rows in front of the reader are
    /// short of the refused root's, and a mark saying the collection went
    /// well would be a claim about work that is not on the screen.
    fn at_rest(every_root_read: bool) -> Self {
        if every_root_read {
            Mark::Read
        } else {
            Mark::Refused
        }
    }
}

impl Freshness {
    /// What to say about one project: how the collection of it went or is
    /// going, and when it was last read.
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
    /// would date rows nothing came from. The mark is what says the read
    /// failed.
    ///
    /// Nothing at all for a project neither read nor being read: there is no
    /// row on the screen for the claim to be about.
    pub fn of(
        read_at: Option<DateTime<Utc>>,
        collecting: bool,
        every_root_read: bool,
    ) -> Option<Self> {
        if read_at.is_none() && !collecting {
            return None;
        }
        Some(Freshness {
            mark: if collecting {
                Mark::Collecting
            } else {
                Mark::at_rest(every_root_read)
            },
            read_at,
        })
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
            Freshness::of(Some(at(22, 14)), false, true),
            Some(Freshness {
                mark: Mark::Read,
                read_at: Some(at(22, 14)),
            })
        );
    }

    /// The bead: the rows on the screen during a collection are the previous
    /// collection's rows, and their age is the only thing saying so. A cell
    /// that gave the age up for the mark left a reader watching a mark turn
    /// over rows of unknown age.
    #[test]
    fn a_project_being_read_now_keeps_the_age_of_the_rows_still_on_the_screen() {
        assert_eq!(
            Freshness::of(Some(at(22, 14)), true, true),
            Some(Freshness {
                mark: Mark::Collecting,
                read_at: Some(at(22, 14)),
            })
        );
    }

    /// A collection reading this project is the whole of what the mark says
    /// while it runs. How the one before it went is about rows that are
    /// seconds from being replaced.
    #[test]
    fn a_collection_in_flight_takes_the_mark_from_the_read_it_is_replacing() {
        assert_eq!(
            Freshness::of(Some(at(22, 14)), true, false).map(|it| it.mark),
            Some(Mark::Collecting)
        );
    }

    /// A project several of whose roots disagree resolves to one mark, and to
    /// the worse of them: the rows in front of the reader are short of the
    /// refused root's.
    #[test]
    fn a_project_with_a_root_that_would_not_read_rests_on_the_refused_mark() {
        assert_eq!(
            Freshness::of(Some(at(22, 14)), false, false).map(|it| it.mark),
            Some(Mark::Refused)
        );
    }

    /// The startup frame: every project drawn before any of them has been
    /// read. There is no time to quote, and the collection under way is the
    /// whole of what the line can say.
    #[test]
    fn a_project_never_read_but_being_read_now_still_says_it_is_collecting() {
        assert_eq!(
            Freshness::of(None, true, true),
            Some(Freshness {
                mark: Mark::Collecting,
                read_at: None,
            })
        );
    }

    #[test]
    fn a_project_neither_read_nor_being_read_says_nothing_about_freshness() {
        assert_eq!(Freshness::of(None, false, true), None);
    }
}
