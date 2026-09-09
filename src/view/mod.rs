//! Everything between the model and the screen: `bdi`'s own words for what the
//! model found, the cells of one row, the forest those rows are drawn from, and
//! the pane tail beneath it.

use chrono::{DateTime, Utc};

use crate::app::Awaited;
use crate::model::join::BeadKey;

pub mod bindings;
pub mod draw;
pub mod fitted;
pub mod forest;
pub mod lines;
pub mod markdown;
pub mod palette;
pub mod phrase;
pub mod row;
pub mod sgr;
pub mod show;
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

/// Which way one notch of the wheel moves what is on screen.
///
/// Not a `Motion`, because a notch and a keystroke ask for different things
/// even where they point the same way: a motion moves the selection and the
/// view follows it, and a notch moves the view and leaves the selection where
/// the reader put it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notch {
    Up,
    Down,
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
    /// Expand the selected node and everything under it, at every depth.
    ExpandSubtree,
    /// Collapse the selected node and everything under it, at every depth.
    CollapseSubtree,
    /// Let go of every fold set by hand, so the forest rests as `bdi` would
    /// have drawn it for the snapshot it is holding now.
    RestoreDefault,
    /// Show every tree, rather than only those with a live agent.
    ToggleFilter,
    /// Focus the selected bead's pane in herdr.
    Focus,
    /// Show the selected bead whole, as `bd show` would.
    ShowBead,
    /// Move the bead view to the next bead the bead it is showing names.
    NextRelated,
    /// Go back to the forest from the bead view, onto the row it was opened
    /// from.
    Back,
    /// Put the selected bead's id on the terminal's clipboard.
    CopyId,
    /// Show every binding the view answers to.
    ShowBindings,
    /// Open the prompt that takes part of a bead's id or title and goes to a
    /// bead holding it.
    Search,
    /// Go to the next bead matching what was last searched for.
    NextMatch,
    /// Go to the one before it.
    PreviousMatch,
    Refresh,
    Quit,
}

/// One keystroke into the search prompt.
///
/// A vocabulary of its own rather than more `Action`s, because while the
/// prompt is up almost every key is a character of an id rather than a key
/// that does something — and `Action` is the list of keys that do something.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Typing {
    /// One character of the id.
    Character(char),
    /// Take the last character back.
    RubbedOut,
    /// Go to the bead the typed id names.
    Sought,
    /// Leave the prompt, with the selection where it was.
    Abandoned,
}

/// What the reader's last keystroke came to, said at the foot until their
/// next press.
///
/// Feedback on a keystroke rather than a fact about the screen, which is what
/// makes their next press take it off whatever that press turns out to mean.
/// That is the whole of what separates these from a `Notice`, which stands
/// until the thing it is about has changed and no keystroke can dismiss.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Said {
    /// The bead id just put on the terminal's clipboard.
    Copied(String),
    /// The text searched for, which no bead any tree read holds in its id or
    /// its title.
    ///
    /// Said as what `bdi` has read rather than as what exists: a tracker that
    /// refused and a project no collection has reached yet both hold beads no
    /// read has seen, and a flat *nothing matches* would speak for them.
    NothingMatched(String),
    /// The bead a search went to, and its place among the beads matching.
    ///
    /// Said on every landing, not only on an odd one. A search matches part
    /// of an id or part of a title, so the reader has typed a fragment rather
    /// than a name, and neither half of what they are owed is on the row they
    /// land on: the row draws the *shortened* id, and one row cannot say that
    /// eleven others matched.
    ///
    /// That is a change from the exact match this widened, where the row was
    /// the whole answer and the foot only spoke up when a second tracker held
    /// the same id.
    Matched { key: BeadKey, at: usize, of: usize },
}

/// Something true of the view as a whole rather than of any row in it, said
/// at the foot of the screen.
///
/// Two unrelated things produce these: a collection, every refresh, and this
/// process, once at startup before there is anything to collect. The status
/// bar is where they meet, and it is handed them in the order it should give
/// them up, so it draws a notice without knowing which kind it has and a
/// third kind needs no third path to the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// The agent provider is installed and would not answer, so no row can
    /// show an agent. A provider nobody installed is not this: nothing was
    /// lost, so there is nothing to say.
    AgentsUnknown,
    /// The provider answered and named this session among the ones it runs,
    /// and the session would not answer for its panes. Every other session's
    /// seats are drawn; this one's are unknown, and a bead one of them is
    /// working reads as unstaffed until it answers.
    SessionUnanswered(String),
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
    /// The config file has been written and will not load, so `bdi` is
    /// working to the config it had before the edit. Not a fall back to
    /// defaults: what the reader sees is still every project they
    /// configured.
    ///
    /// The only notice here that comes and goes. It goes up on a read that
    /// failed and comes off on the next that did not, whether or not that
    /// one brought anything new — a reader who undoes the edit has fixed the
    /// file, and a screen that went on saying otherwise would leave them
    /// changing a config that was already right.
    ///
    /// Why it would not load is not said. The reader wrote the file a moment
    /// ago and their editor is where the line and the column are; what this
    /// has to tell them is the part they cannot see, which is that `bdi` did
    /// not take it.
    ConfigWouldNotReload,
    /// git could not be run, so the project drawn is named after the
    /// directory its tracker sits at the top of rather than after a remote.
    /// The name is half of every key `bdi` holds, so two readers of one
    /// tracker are looking at differently-named views and neither would
    /// otherwise be told.
    ///
    /// Said only where nothing else named the project — no `BDI_PROJECT` and
    /// no config file — because a name given outright is not a guess. It is
    /// the one notice here whose remedy is a line in a shell profile rather
    /// than a program to install, which is what keeps it from being a warning
    /// readers learn to ignore.
    ProjectNamedWithoutGit,
}

/// How fresh one project's rows are, said beside its name.
///
/// Two things, and both of them are on the line at all times. The mark says
/// how a read of this project is getting on, or how the last one went; the
/// age says how old the rows under the name are. They answer different
/// questions, and a cell that swapped one for the other left the reader
/// watching a mark turn over rows of unknown age.
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
/// width for a read starting or ending — which was the whole of what made
/// the old cell jump.
///
/// The two marks a read outstanding produces answer *how long has it been
/// outstanding*, and neither of them asks whether the collector has reached
/// it yet. A read waiting its turn behind another is on its way as much as
/// one being served, and the reader can do nothing differently for the
/// difference — so `bdi` does not draw one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// A read of this project is outstanding and getting somewhere: it has
    /// been asked for, and not long enough ago for that to be worth saying.
    Collecting,
    /// A read of this project has been outstanding for longer than one may
    /// be: whatever it is waiting on has stopped answering.
    ///
    /// Distinct from `Refused`, which is a collection that came back and said
    /// no. Nothing has come back here and nothing may ever, and the read has
    /// not been given up — see `Awaited::patience`.
    Unanswered,
    /// The last collection read every root of it.
    Read,
    /// The last collection found a root it could not read.
    Refused,
}

impl Mark {
    /// What the mark says with no read of this project outstanding.
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
    /// What to say about one project: how the read of it is getting on or
    /// how the last one went, and when it was last read.
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
    /// Nothing at all for a project never read and with no read outstanding:
    /// there is no row on the screen for the claim to be about.
    ///
    /// `collecting` is the read itself rather than a flag saying there is
    /// one, because a read that has stopped getting anywhere is drawn the
    /// same as one just asked for unless the mark can measure the wait — and
    /// the read is what carries when it was asked for and how long it may go
    /// unanswered.
    pub fn of(
        read_at: Option<DateTime<Utc>>,
        collecting: Option<&Awaited>,
        every_root_read: bool,
        now: DateTime<Utc>,
    ) -> Option<Self> {
        if read_at.is_none() && collecting.is_none() {
            return None;
        }
        Some(Freshness {
            mark: match collecting {
                Some(awaited) if awaited.unanswered_at(now) => Mark::Unanswered,
                Some(_) => Mark::Collecting,
                None => Mark::at_rest(every_root_read),
            },
            read_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Wanted;
    use crate::collect::run::FailureKind;
    use crate::model::join::JoinSource;
    use crate::model::snapshot::TrackerFailure;
    use crate::model::types::testing::an_unreadable;
    use chrono::{TimeDelta, TimeZone};
    use pretty_assertions::assert_eq;

    /// The text says these words.
    ///
    /// The words are written out at the call rather than asked of the code
    /// that produced the text. A test that takes them from `phrase` passes
    /// whatever `phrase` says, the empty string included, so it proves the
    /// words reached the text and nothing about what they are.
    pub(super) fn says(text: &str, words: &str) {
        assert!(
            !words.is_empty(),
            "every text says nothing, so nothing is asserted"
        );
        assert!(text.contains(words), "{text:?} does not say {words:?}");
    }

    /// The text does not say these words. Inverted, the same guard is needed
    /// for the opposite reason: no text leaves nothing out, so an empty
    /// expectation fails whatever the text says.
    pub(super) fn does_not_say(text: &str, words: &str) {
        assert!(
            !words.is_empty(),
            "no text leaves nothing out, so nothing is asserted"
        );
        assert!(!text.contains(words), "{text:?} says {words:?}");
    }

    /// The whole point of the guard: a phrase emptied at source and passed
    /// straight through would satisfy `contains` on every text ever produced.
    #[test]
    #[should_panic(expected = "nothing is asserted")]
    fn nothing_is_not_something_a_text_can_say() {
        says("⚠ agents unknown", "");
    }

    /// And its mirror: no text leaves nothing out, so the inverted form has to
    /// refuse the same expectation for the opposite reason.
    #[test]
    #[should_panic(expected = "nothing is asserted")]
    fn nothing_is_not_something_a_text_can_leave_out() {
        does_not_say("⚠ agents unknown", "");
    }

    /// Every variant of an enum a phrase is drawn from, walked rather than
    /// listed: each arm names the variant after it, so a variant added to the
    /// enum stops these compiling until it has been given a place in the
    /// chain. A roster whose name or comment says it covers an enum is built
    /// from one of these, because a hand-written array literal cannot keep
    /// that promise and reads exactly the same when it has stopped keeping
    /// it.
    ///
    /// The compiler asks; it does not prove. An arm answering `None` early
    /// drops everything after it. Proving it wants `strum`'s `EnumIter`,
    /// which is a dependency for a handful of test rosters, and stable Rust
    /// has no `variant_count`.
    ///
    /// Only enums whose variants stand on their own are here. One carrying
    /// fields has shapes as well as variants, and the values that make a
    /// shape belong beside the test that reads them.
    ///
    /// They sit in `view` rather than in the module that first needed them
    /// because `view::phrase` and `view::tail` roster the same enums, and a
    /// second copy of a chain is the drift the chain exists to catch.
    pub(super) fn every_failure_kind() -> impl Iterator<Item = FailureKind> {
        std::iter::successors(Some(FailureKind::Auth), |kind| match kind {
            FailureKind::Auth => Some(FailureKind::Unavailable),
            FailureKind::Unavailable => Some(FailureKind::Gone),
            FailureKind::Gone => Some(FailureKind::Busy),
            FailureKind::Busy => Some(FailureKind::NotInstalled),
            FailureKind::NotInstalled => Some(FailureKind::Unstartable),
            FailureKind::Unstartable => Some(FailureKind::InstalledUnstartable),
            FailureKind::InstalledUnstartable => Some(FailureKind::Parse),
            FailureKind::Parse => Some(FailureKind::Unsupported),
            FailureKind::Unsupported => Some(FailureKind::UnknownFlag),
            FailureKind::UnknownFlag => None,
        })
    }

    /// Every way a tracker can refuse to be read. See [`every_failure_kind`].
    pub(super) fn every_tracker_failure() -> impl Iterator<Item = TrackerFailure> {
        std::iter::successors(
            Some(TrackerFailure::NoEnvironment),
            |failure| match failure {
                TrackerFailure::NoEnvironment => Some(TrackerFailure::NoCredential),
                TrackerFailure::NoCredential => Some(TrackerFailure::Auth),
                TrackerFailure::Auth => Some(TrackerFailure::Unavailable),
                TrackerFailure::Unavailable => Some(TrackerFailure::NotInstalled),
                TrackerFailure::NotInstalled => Some(TrackerFailure::Unstartable),
                TrackerFailure::Unstartable => Some(TrackerFailure::InstalledUnstartable),
                TrackerFailure::InstalledUnstartable => {
                    Some(TrackerFailure::Parse(an_unreadable()))
                }
                TrackerFailure::Parse(_) => Some(TrackerFailure::UnknownFlag),
                TrackerFailure::UnknownFlag => None,
            },
        )
    }

    /// Every fact said at the foot of the screen. See [`every_failure_kind`].
    pub(super) fn every_notice() -> impl Iterator<Item = Notice> {
        std::iter::successors(Some(Notice::AgentsUnknown), |fact| match fact {
            Notice::AgentsUnknown => Some(Notice::SessionUnanswered("a session".to_string())),
            Notice::SessionUnanswered(_) => Some(Notice::NoInboundChannel),
            Notice::NoInboundChannel => Some(Notice::AnotherBdiHadTheInboundChannel),
            Notice::AnotherBdiHadTheInboundChannel => Some(Notice::ConfigWouldNotReload),
            Notice::ConfigWouldNotReload => Some(Notice::ProjectNamedWithoutGit),
            Notice::ProjectNamedWithoutGit => None,
        })
    }

    /// Everything the foot can say back to a reader's keystroke. See
    /// [`every_failure_kind`].
    pub(super) fn every_said() -> impl Iterator<Item = Said> {
        std::iter::successors(Some(Said::Copied("grv-1".to_string())), |said| match said {
            Said::Copied(_) => Some(Said::NothingMatched("grv-404".to_string())),
            Said::NothingMatched(_) => Some(Said::Matched {
                key: BeadKey {
                    project: "orbital".to_string(),
                    id: "grv-1".to_string(),
                },
                at: 1,
                of: 2,
            }),
            Said::Matched { .. } => None,
        })
    }

    /// Every mark a project's cell can wear. See [`every_failure_kind`].
    pub(super) fn every_mark() -> impl Iterator<Item = Mark> {
        std::iter::successors(Some(Mark::Collecting), |mark| match mark {
            Mark::Collecting => Some(Mark::Unanswered),
            Mark::Unanswered => Some(Mark::Read),
            Mark::Read => Some(Mark::Refused),
            Mark::Refused => None,
        })
    }

    /// Every direction the join can award an agent from. See
    /// [`every_failure_kind`].
    pub(super) fn every_join_source() -> impl Iterator<Item = JoinSource> {
        std::iter::successors(Some(JoinSource::AgentPane), |source| match source {
            JoinSource::AgentPane => Some(JoinSource::DisplayAgent),
            JoinSource::DisplayAgent => None,
        })
    }

    fn at(minute: u32, second: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 30, 10, minute, second)
            .unwrap()
    }

    /// How long the collections below may go unanswered. A round number the
    /// instants are written against, rather than the configured default: what
    /// these assert is that the mark turns on the deadline, not what the
    /// deadline is.
    const PATIENCE: TimeDelta = TimeDelta::seconds(30);

    /// A collection asked for at `asked_at`, of whichever projects — every
    /// test here is about one project and the collection is reading it.
    fn asked_at(asked_at: DateTime<Utc>) -> Awaited {
        Awaited {
            wanted: Wanted::Everything,
            asked_at,
            patience: PATIENCE,
        }
    }

    /// A project the collection in flight is not reading keeps the read it
    /// has. The whole point of moving the indicator off the foot: one
    /// project's collection used to say every project was collecting.
    #[test]
    fn a_project_no_collection_is_reading_says_when_it_was_last_read() {
        assert_eq!(
            Freshness::of(Some(at(22, 14)), None, true, at(22, 20)),
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
            Freshness::of(
                Some(at(22, 14)),
                Some(&asked_at(at(22, 20))),
                true,
                at(22, 20)
            ),
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
    fn a_collection_being_awaited_takes_the_mark_from_the_read_it_is_replacing() {
        assert_eq!(
            Freshness::of(
                Some(at(22, 14)),
                Some(&asked_at(at(22, 20))),
                false,
                at(22, 20)
            )
            .map(|it| it.mark),
            Some(Mark::Collecting)
        );
    }

    /// A project several of whose roots disagree resolves to one mark, and to
    /// the worse of them: the rows in front of the reader are short of the
    /// refused root's.
    #[test]
    fn a_project_with_a_root_that_would_not_read_rests_on_the_refused_mark() {
        assert_eq!(
            Freshness::of(Some(at(22, 14)), None, false, at(22, 20)).map(|it| it.mark),
            Some(Mark::Refused)
        );
    }

    /// The startup frame: every project drawn before any of them has been
    /// read. There is no time to quote, and the collection under way is the
    /// whole of what the line can say.
    #[test]
    fn a_project_never_read_but_being_read_now_still_says_it_is_collecting() {
        assert_eq!(
            Freshness::of(None, Some(&asked_at(at(22, 20))), true, at(22, 20)),
            Some(Freshness {
                mark: Mark::Collecting,
                read_at: None,
            })
        );
    }

    /// The bead. A collection that has stopped answering was drawn exactly
    /// as one asked half a second ago, because nothing measured the wait.
    /// Past the deadline the mark says the tracker has stopped answering
    /// rather than that a collection is under way.
    #[test]
    fn a_collection_that_has_stopped_answering_is_said_to_have_rather_than_to_be_running() {
        let since = at(22, 20);

        assert_eq!(
            Freshness::of(
                Some(at(22, 14)),
                Some(&asked_at(since)),
                true,
                since + PATIENCE
            )
            .map(|it| it.mark),
            Some(Mark::Unanswered)
        );
    }

    /// The other half of the same claim, and the half that keeps the mark
    /// worth reading: a collection still inside the deadline is a collection
    /// under way, however close to it the reader looks.
    #[test]
    fn a_collection_still_inside_the_deadline_is_said_to_be_running() {
        let since = at(22, 20);

        assert_eq!(
            Freshness::of(
                Some(at(22, 14)),
                Some(&asked_at(since)),
                true,
                since + PATIENCE - TimeDelta::milliseconds(1)
            )
            .map(|it| it.mark),
            Some(Mark::Collecting)
        );
    }

    /// The rows on the screen are the last collection's, and a collection
    /// that has stopped answering is the reason they will not be replaced —
    /// so their age matters more than ever, not less. The mark is what
    /// changed; the age is untouched by it.
    #[test]
    fn a_collection_that_has_stopped_answering_still_dates_the_rows_on_the_screen() {
        let since = at(22, 20);

        assert_eq!(
            Freshness::of(
                Some(at(22, 14)),
                Some(&asked_at(since)),
                true,
                since + PATIENCE
            ),
            Some(Freshness {
                mark: Mark::Unanswered,
                read_at: Some(at(22, 14)),
            })
        );
    }

    /// A first collection that never answers: there is no read to date any
    /// rows to, and no rows, but the reader is still told the tracker has
    /// stopped answering. This is the startup frame gone wrong, and it is the
    /// state a reader is likeliest to meet — `bdi` starting against a tracker
    /// that is not there.
    #[test]
    fn a_first_collection_that_never_answers_says_so_over_no_rows_at_all() {
        let since = at(22, 20);

        assert_eq!(
            Freshness::of(None, Some(&asked_at(since)), true, since + PATIENCE),
            Some(Freshness {
                mark: Mark::Unanswered,
                read_at: None,
            })
        );
    }

    /// A collection that has stopped answering says nothing about how the one
    /// before it went. The rows on the screen are that collection's and their
    /// age is what speaks for them; the mark's column belongs to what is
    /// happening now, which is nothing.
    #[test]
    fn a_collection_that_has_stopped_answering_takes_the_mark_from_the_read_it_is_replacing() {
        let since = at(22, 20);

        assert_eq!(
            Freshness::of(
                Some(at(22, 14)),
                Some(&asked_at(since)),
                false,
                since + PATIENCE
            )
            .map(|it| it.mark),
            Some(Mark::Unanswered)
        );
    }

    #[test]
    fn a_project_neither_read_nor_being_read_says_nothing_about_freshness() {
        assert_eq!(Freshness::of(None, None, true, at(22, 20)), None);
    }
}
