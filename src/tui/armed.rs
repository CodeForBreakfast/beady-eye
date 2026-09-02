//! When a project asks to be read again.
//!
//! Something has to ask for a project nothing else reports changes for, and
//! this is what does. It is armed by the read that came back and disarmed by
//! the ask it makes, so a project is always one or the other — a read on its
//! way, or an ask armed to start one.

use std::time::Duration;

use chrono::{DateTime, Utc};

use super::due::due_after;
use crate::app::Wanted;

/// One project's poll: how long after a read it asks to be read again, and
/// when that next falls due.
///
/// One of these per project, rather than one of them holding a timer per
/// project. What that buys is that there is no clock left here to share: the
/// instant is derived from the read this project's own tracker answered, so
/// nothing short of two projects' reads finishing together can put them back
/// on one schedule, and a simplification that reunited them would have to
/// invent the clock rather than merely stop keeping them apart.
///
/// Three things follow from arming on the read rather than on an interval.
/// Projects stagger themselves and stay staggered. A slow project delays only
/// itself. And a project cannot queue asks behind itself, because the ask it
/// is going to make does not exist until the last one has been answered.
///
/// The mirror of that last one is the hazard, and it is deliberate: a read
/// that never comes back arms nothing, so a hung tracker goes quiet here
/// rather than piling up. What says so on the screen is the read still
/// standing unanswered in `Outstanding`, which is where a wait is measured.
/// Arming anywhere but on a completed read would paper over exactly that.
pub(crate) struct Armed {
    project: String,
    /// How long after a read comes back this project asks for another, or
    /// nothing where it does not poll at all.
    every: Option<Duration>,
    /// When it asks, or nothing while a read it is waiting on is still on its
    /// way — and nothing for good on a project that does not poll, or whose
    /// interval is too long to reach.
    at: Option<DateTime<Utc>>,
}

impl Armed {
    /// A project that polls `every` after each read, or — where that is
    /// nothing — one that leaves being told to whatever else reports for it.
    ///
    /// Disarmed to begin with, because the run asks for every project before
    /// the loop starts: a project's first ask is already on its way when this
    /// is built, and arming here would ask a second time for what is being
    /// read.
    pub(crate) fn polling(project: String, every: Option<Duration>) -> Self {
        Self {
            project,
            every,
            at: None,
        }
    }

    pub(super) fn project(&self) -> &str {
        &self.project
    }

    /// The ask this project is now due to make, where it is due to make one.
    ///
    /// Taking it disarms: what arms this again is the read that comes back,
    /// so an ask nobody answers is an ask never repeated.
    pub(super) fn asks(&mut self, now: DateTime<Utc>) -> Option<Wanted> {
        if !self.at.is_some_and(|at| at <= now) {
            return None;
        }
        self.at = None;
        Some(Wanted::Project(self.project.clone()))
    }

    /// How long until it asks, or nothing where it is not going to ask at
    /// all: a project that does not poll, and one whose read has not come
    /// back yet.
    pub(super) fn asks_in(&self, now: DateTime<Utc>) -> Option<Duration> {
        self.at
            .map(|at| (at - now).to_std().unwrap_or(Duration::ZERO))
    }

    /// A read has come back. Where it read this project, that is when the
    /// next ask is armed from — unless the interval is too long to reach,
    /// which arms nothing, as `due_after` says.
    ///
    /// Whatever asked for it: the socket, the refresh key and this project's
    /// own last ask all arm the next one the same way, which is what makes a
    /// project something keeps reporting for one that never polls — each
    /// report's read pushes the poll out past the interval before it arrives.
    pub(super) fn came_back(&mut self, wanted: &Wanted, at: DateTime<Utc>) {
        if wanted.names(&self.project) {
            self.at = self.every.and_then(|every| due_after(at, every));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::fixtures::{atlas, ferry};

    const EVERY: Duration = Duration::from_secs(30);

    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(seconds, 0).expect("an instant inside the epoch")
    }

    fn polling() -> Armed {
        Armed::polling("atlas".to_string(), Some(EVERY))
    }

    /// The largest whole number of seconds — the unit `refresh_seconds` is
    /// read in — that still lands inside the range an instant can hold, read
    /// off chrono's own last instant rather than quoted from its
    /// documentation. It shrinks as the clock advances, which is why the
    /// answer is checked where the add happens rather than bounded once at
    /// config load.
    fn seconds_to_the_end_of_time(from: DateTime<Utc>) -> u64 {
        (DateTime::<Utc>::MAX_UTC - from)
            .to_std()
            .expect("the end of time is after the epoch")
            .as_secs()
    }

    /// The gap `refresh_seconds` can be given and still be waited out. One
    /// second more is the test below, and the two of them are what pin the
    /// answer to the bound rather than to somewhere past it.
    #[test]
    fn a_project_whose_interval_reaches_the_end_of_time_still_asks() {
        let every = Duration::from_secs(seconds_to_the_end_of_time(at(100)));
        let mut armed = Armed::polling("atlas".to_string(), Some(every));

        armed.came_back(&atlas(), at(100));

        assert_eq!(armed.asks_in(at(100)), Some(every));
    }

    /// A `refresh_seconds` past what an instant can hold is a gap nothing
    /// waits out, rather than a panic on the first read that comes back.
    /// Nothing is what this already says for a project that does not poll,
    /// so the answer is one the caller can already meet.
    #[test]
    fn a_project_whose_interval_outruns_time_asks_no_more() {
        let every = Duration::from_secs(seconds_to_the_end_of_time(at(100)) + 1);
        let mut armed = Armed::polling("atlas".to_string(), Some(every));

        armed.came_back(&atlas(), at(100));

        assert_eq!(armed.asks_in(at(100)), None);
        assert_eq!(armed.asks(at(1_000_000)), None);
    }

    /// The largest `refresh_seconds` the key can hold at all, which is a
    /// thousandfold past the reach an instant has — so what turns it away is
    /// the conversion to an interval rather than the addition. It is the
    /// same answer, reached the other way, and reaching it needs no
    /// arithmetic on `MAX_UTC`: a `u64` full of ones is what a config that
    /// has gone wrong hands over.
    #[test]
    fn a_project_whose_interval_fills_the_key_asks_no_more() {
        let mut armed = Armed::polling("atlas".to_string(), Some(Duration::from_secs(u64::MAX)));

        armed.came_back(&atlas(), at(100));

        assert_eq!(armed.asks_in(at(100)), None);
        assert_eq!(armed.asks(at(1_000_000)), None);
    }

    /// The whole of the design: the interval is a gap after the read rather
    /// than a period the read happens inside.
    #[test]
    fn a_project_asks_again_one_interval_after_the_read_that_answered_it() {
        let mut armed = polling();

        armed.came_back(&atlas(), at(100));

        assert_eq!(armed.asks(at(129)), None, "the interval was not out");
        assert_eq!(armed.asks(at(130)), Some(atlas()));
    }

    /// A run that has read nothing has a read on its way already. Arming at
    /// the start would ask a second time for what is being collected.
    #[test]
    fn a_project_whose_first_read_is_still_coming_asks_for_nothing() {
        let mut armed = polling();

        assert_eq!(armed.asks(at(1_000_000)), None);
        assert_eq!(armed.asks_in(at(0)), None);
    }

    /// Push-only: something else reports this project's changes, so nothing
    /// here ever asks, however many reads come back.
    #[test]
    fn a_project_that_does_not_poll_never_asks_for_itself() {
        let mut armed = Armed::polling("atlas".to_string(), None);

        armed.came_back(&atlas(), at(100));

        assert_eq!(armed.asks_in(at(100)), None);
        assert_eq!(armed.asks(at(1_000_000)), None);
    }

    /// The disarming half of the invariant: one read comes of one ask. A
    /// project that went on asking would queue reads behind itself, which is
    /// the pile-up arming from completion exists to make unrepresentable.
    #[test]
    fn asking_disarms_until_another_read_comes_back() {
        let mut armed = polling();
        armed.came_back(&atlas(), at(100));

        assert_eq!(armed.asks(at(130)), Some(atlas()));

        assert_eq!(armed.asks(at(200)), None, "nothing has answered the ask");
        assert_eq!(armed.asks_in(at(200)), None);

        armed.came_back(&atlas(), at(210));

        assert_eq!(armed.asks(at(240)), Some(atlas()));
    }

    /// The hazard, stated as a test so that a later arm added anywhere but on
    /// a completed read fails here: a tracker that never answers goes quiet,
    /// and is reported as unanswered rather than polled over.
    #[test]
    fn a_project_whose_read_never_comes_back_asks_no_more() {
        let mut armed = polling();
        armed.came_back(&atlas(), at(100));
        armed.asks(at(130));

        assert_eq!(armed.asks(at(1_000_000)), None);
    }

    /// What replaces the coverage suppression the poll used to consult. A
    /// project something keeps reporting for is read on each report, and each
    /// read pushes the poll out past the interval before it arrives — so it
    /// is never polled, without anything having to ask whether it is covered.
    #[test]
    fn a_project_something_keeps_reporting_for_is_not_also_polled() {
        let mut armed = polling();

        for reported_at in [100, 120, 140, 160] {
            armed.came_back(&atlas(), at(reported_at));
            assert_eq!(
                armed.asks(at(reported_at + 20)),
                None,
                "reported again at {}, so the poll is still out at {}",
                reported_at + 20,
                reported_at + 20
            );
        }

        assert_eq!(
            armed.asks(at(190)),
            Some(atlas()),
            "the reports stopped at 160, so the poll comes due one interval after"
        );
    }

    /// The saving a mixed setup gets, which used to be the poll consulting
    /// what had been reported for: the project with a producer is left out of
    /// the poll its neighbour still needs, rather than swept up with it.
    #[test]
    fn a_project_with_a_producer_is_left_out_of_the_poll_its_neighbour_needs() {
        let mut has_one = Armed::polling("atlas".to_string(), None);
        let mut has_none = Armed::polling("ferry".to_string(), Some(EVERY));

        has_one.came_back(&Wanted::Everything, at(100));
        has_none.came_back(&Wanted::Everything, at(100));

        assert_eq!(has_one.asks(at(130)), None);
        assert_eq!(has_none.asks(at(130)), Some(ferry()));
    }

    /// The signal that a live producer has gone quiet is the poll resuming,
    /// so the view degrades to slow rather than to wrong. Nothing detects the
    /// producer's going: the project's last read armed it, and no report has
    /// arrived to push that out.
    #[test]
    fn a_project_the_channel_stops_covering_is_polled_again() {
        let mut armed = polling();

        armed.came_back(&atlas(), at(100));

        assert_eq!(armed.asks(at(130)), Some(atlas()));
    }

    /// A read of everything reads this project too, so it arms this project.
    #[test]
    fn a_read_of_every_project_arms_this_one() {
        let mut armed = polling();

        armed.came_back(&Wanted::Everything, at(100));

        assert_eq!(armed.asks(at(130)), Some(atlas()));
    }

    /// And a read of a different project does not: that is the whole of
    /// per-project independence, and it is settled by what the read named.
    #[test]
    fn a_read_of_another_project_leaves_this_one_as_it_was() {
        let mut armed = polling();
        armed.came_back(&atlas(), at(100));

        armed.came_back(&ferry(), at(120));

        assert_eq!(
            armed.asks(at(130)),
            Some(atlas()),
            "atlas asks 30 after its own read, not 30 after ferry's"
        );
    }

    /// Two projects read at different instants ask at different instants, and
    /// go on doing so. This is what a global tick could not do: it refreshed
    /// the whole configured set together, so every project paid the cascade
    /// at once.
    #[test]
    fn projects_read_at_different_instants_stay_apart() {
        let mut first = Armed::polling("atlas".to_string(), Some(EVERY));
        let mut second = Armed::polling("ferry".to_string(), Some(EVERY));

        first.came_back(&atlas(), at(100));
        second.came_back(&ferry(), at(112));

        assert_eq!(first.asks_in(at(100)), Some(EVERY));
        assert_eq!(second.asks_in(at(100)), Some(Duration::from_secs(42)));
        assert_eq!(first.asks(at(130)), Some(atlas()));
        assert_eq!(second.asks(at(130)), None, "ferry's own read was later");
    }

    /// An ask already overdue is due now rather than negative: the loop asks
    /// this how long to wait, and a wait it cannot express is one it would
    /// have to guess at.
    #[test]
    fn an_ask_already_overdue_says_it_waits_no_longer() {
        let mut armed = polling();
        armed.came_back(&atlas(), at(100));

        assert_eq!(armed.asks_in(at(500)), Some(Duration::ZERO));
    }

    #[test]
    fn a_project_answers_to_its_own_name() {
        assert_eq!(polling().project(), "atlas");
    }
}
