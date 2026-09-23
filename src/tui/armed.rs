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

/// One disarmed `Armed` per project a config names.
///
/// Handed to the loop rather than worked out by one, because how long a
/// project waits is the command line's as much as the config's: `--poll` and
/// `--no-poll` overrule what each project's own key says, and that is
/// settled where `bdi` is run. The loop asks this again whenever the reader
/// writes a config, so the set of projects that poll is the set the file
/// names.
pub(crate) type Arming = Box<dyn Fn(&crate::config::Config) -> Vec<Armed>>;

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
    /// How long a project that does not poll is taken to be current after
    /// something last vouched for it, or nothing where it is never said to
    /// have lapsed.
    covered_for: Option<Duration>,
    /// When something last vouched for its rows: a read of it coming back,
    /// or a word from something covering it.
    vouched_at: Option<DateTime<Utc>>,
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
            covered_for: None,
            vouched_at: None,
        }
    }

    /// This project, said to have lapsed once `covered_for` has passed with
    /// nothing vouching for it, where it does not poll.
    pub(crate) fn lapsing_after(self, covered_for: Duration) -> Self {
        Self {
            covered_for: Some(covered_for),
            ..self
        }
    }

    pub(super) fn project(&self) -> &str {
        &self.project
    }

    /// This project polling as the config the reader has just written says,
    /// still due when it already was.
    ///
    /// How often it asks is the file's and takes effect at once — a reader
    /// who turns a project's `poll` off has said something about this run,
    /// not about the next one. When it next asks is not the file's: that
    /// deadline was armed by this project's last read, and an edit anywhere
    /// in the file must not push out an ask that was already due.
    ///
    /// A project the edit stopped polling has no deadline left at all, so it
    /// asks no more rather than once more.
    pub(super) fn still_due(self, named: Armed) -> Armed {
        Armed {
            at: named.every.and(self.at),
            vouched_at: self.vouched_at,
            ..named
        }
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
            self.vouched_at = Some(at);
        }
    }

    /// Something outside says it covers this project and nothing in it has
    /// moved, which says of its rows what a read that found nothing would
    /// have said. So the poll is pushed out from here as a read's return
    /// would push it.
    ///
    /// Only where an ask is armed. A read still on its way arms the next ask
    /// when it comes back, which is later than this word.
    pub(super) fn covered(&mut self, at: DateTime<Utc>) {
        if self.at.is_some() {
            self.at = self.every.and_then(|every| due_after(at, every));
        }
        self.vouched_at = Some(at);
    }

    /// Whether nothing has vouched for this project's rows for longer than
    /// it may go. Only a project that does not poll can lapse: a polled one
    /// is kept current by its own poll, and that is the reader's choice
    /// rather than a producer failing where nobody can see.
    pub(super) fn lapsed(&self, now: DateTime<Utc>) -> bool {
        self.lapses_at().is_some_and(|lapses| lapses <= now)
    }

    /// How long until it lapses, or nothing where it is not going to: it
    /// polls, nothing has vouched for it yet, or it has lapsed already.
    pub(super) fn lapses_in(&self, now: DateTime<Utc>) -> Option<Duration> {
        self.lapses_at()
            .and_then(|lapses| (lapses - now).to_std().ok())
            .filter(|wait| !wait.is_zero())
    }

    fn lapses_at(&self) -> Option<DateTime<Utc>> {
        if self.every.is_some() {
            return None;
        }
        due_after(self.vouched_at?, self.covered_for?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::fixtures::{arkham, ferry};

    const EVERY: Duration = Duration::from_secs(30);

    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(seconds, 0).expect("an instant inside the epoch")
    }

    fn polling() -> Armed {
        Armed::polling("arkham".to_string(), Some(EVERY))
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
        let mut armed = Armed::polling("arkham".to_string(), Some(every));

        armed.came_back(&arkham(), at(100));

        assert_eq!(armed.asks_in(at(100)), Some(every));
    }

    /// A `refresh_seconds` past what an instant can hold is a gap nothing
    /// waits out, rather than a panic on the first read that comes back.
    /// Nothing is what this already says for a project that does not poll,
    /// so the answer is one the caller can already meet.
    #[test]
    fn a_project_whose_interval_outruns_time_asks_no_more() {
        let every = Duration::from_secs(seconds_to_the_end_of_time(at(100)) + 1);
        let mut armed = Armed::polling("arkham".to_string(), Some(every));

        armed.came_back(&arkham(), at(100));

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
        let mut armed = Armed::polling("arkham".to_string(), Some(Duration::from_secs(u64::MAX)));

        armed.came_back(&arkham(), at(100));

        assert_eq!(armed.asks_in(at(100)), None);
        assert_eq!(armed.asks(at(1_000_000)), None);
    }

    /// The whole of the design: the interval is a gap after the read rather
    /// than a period the read happens inside.
    #[test]
    fn a_project_asks_again_one_interval_after_the_read_that_answered_it() {
        let mut armed = polling();

        armed.came_back(&arkham(), at(100));

        assert_eq!(armed.asks(at(129)), None, "the interval was not out");
        assert_eq!(armed.asks(at(130)), Some(arkham()));
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
        let mut armed = Armed::polling("arkham".to_string(), None);

        armed.came_back(&arkham(), at(100));

        assert_eq!(armed.asks_in(at(100)), None);
        assert_eq!(armed.asks(at(1_000_000)), None);
    }

    /// The disarming half of the invariant: one read comes of one ask. A
    /// project that went on asking would queue reads behind itself, which is
    /// the pile-up arming from completion exists to make unrepresentable.
    #[test]
    fn asking_disarms_until_another_read_comes_back() {
        let mut armed = polling();
        armed.came_back(&arkham(), at(100));

        assert_eq!(armed.asks(at(130)), Some(arkham()));

        assert_eq!(armed.asks(at(200)), None, "nothing has answered the ask");
        assert_eq!(armed.asks_in(at(200)), None);

        armed.came_back(&arkham(), at(210));

        assert_eq!(armed.asks(at(240)), Some(arkham()));
    }

    /// The hazard, stated as a test so that a later arm added anywhere but on
    /// a completed read fails here: a tracker that never answers goes quiet,
    /// and is reported as unanswered rather than polled over.
    #[test]
    fn a_project_whose_read_never_comes_back_asks_no_more() {
        let mut armed = polling();
        armed.came_back(&arkham(), at(100));
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
            armed.came_back(&arkham(), at(reported_at));
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
            Some(arkham()),
            "the reports stopped at 160, so the poll comes due one interval after"
        );
    }

    /// The saving a mixed setup gets, which used to be the poll consulting
    /// what had been reported for: the project with a producer is left out of
    /// the poll its neighbour still needs, rather than swept up with it.
    #[test]
    fn a_project_with_a_producer_is_left_out_of_the_poll_its_neighbour_needs() {
        let mut has_one = Armed::polling("arkham".to_string(), None);
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

        armed.came_back(&arkham(), at(100));

        assert_eq!(armed.asks(at(130)), Some(arkham()));
    }

    /// What a producer's word buys a quiet project. Each report used to push
    /// the poll out only by way of the read it caused, so a producer with
    /// nothing to report let the poll come due however alive it was. Saying
    /// it covers the project pushes the poll out with no read at all.
    #[test]
    fn a_project_something_says_it_covers_is_not_polled_while_it_says_so() {
        let mut armed = polling();
        armed.came_back(&arkham(), at(100));

        for covered_at in [120, 140, 160] {
            armed.covered(at(covered_at));
            assert_eq!(
                armed.asks(at(covered_at + 20)),
                None,
                "covered again at {covered_at}, so the poll is still out"
            );
        }

        assert_eq!(
            armed.asks(at(190)),
            Some(arkham()),
            "the word stopped at 160, so the poll comes due one interval after"
        );
    }

    /// A read on its way arms the poll when it comes back, and that is later
    /// than any word heard while it was out. Arming on the word as well would
    /// be a second ask behind one nobody has answered yet.
    #[test]
    fn a_word_heard_while_a_read_is_out_arms_nothing() {
        let mut armed = polling();

        armed.covered(at(100));

        assert_eq!(armed.asks_in(at(100)), None);
        assert_eq!(armed.asks(at(1_000_000)), None);
    }

    /// A project that does not poll has no poll to push out.
    #[test]
    fn a_word_arms_nothing_for_a_project_that_does_not_poll() {
        let mut armed = Armed::polling("arkham".to_string(), None);
        armed.came_back(&arkham(), at(100));

        armed.covered(at(120));

        assert_eq!(armed.asks(at(1_000_000)), None);
    }

    const COVERED_FOR: Duration = Duration::from_secs(60);

    fn not_polling() -> Armed {
        Armed::polling("arkham".to_string(), None).lapsing_after(COVERED_FOR)
    }

    /// With no poll behind it, a project whose producer has died shows rows
    /// exactly as current-looking as one whose producer is covering it. So
    /// once nothing has vouched for it for the term, it says it has lapsed.
    #[test]
    fn a_project_that_does_not_poll_lapses_a_term_after_its_last_read() {
        let mut armed = not_polling();

        armed.came_back(&arkham(), at(100));

        assert!(!armed.lapsed(at(159)), "the term was not out");
        assert_eq!(armed.lapses_in(at(100)), Some(COVERED_FOR));
        assert!(armed.lapsed(at(160)));
        assert_eq!(armed.lapses_in(at(160)), None, "and lapsed is not a wait");
    }

    /// The word is what keeps a quiet project from lapsing, which is the whole
    /// of what `covered` is for.
    #[test]
    fn a_project_something_says_it_covers_does_not_lapse_while_it_says_so() {
        let mut armed = not_polling();
        armed.came_back(&arkham(), at(100));

        for covered_at in [140, 180, 220] {
            armed.covered(at(covered_at));
            assert!(
                !armed.lapsed(at(covered_at + 40)),
                "covered at {covered_at}"
            );
        }

        assert!(armed.lapsed(at(280)), "the word stopped at 220");
    }

    /// Unlike the poll, a word heard while a read is out counts: it is a
    /// claim about the rows now, and a read that never comes back is drawn
    /// unanswered whatever this says.
    #[test]
    fn a_word_heard_before_the_first_read_is_back_still_vouches() {
        let mut armed = not_polling();

        armed.covered(at(100));

        assert!(!armed.lapsed(at(159)));
        assert!(armed.lapsed(at(160)));
    }

    /// Nothing is claimed about a project nothing has read yet: the read that
    /// started the run is on its way, and the screen already says so.
    #[test]
    fn a_project_nothing_has_read_or_covered_has_not_lapsed() {
        let armed = not_polling();

        assert!(!armed.lapsed(at(1_000_000)));
        assert_eq!(armed.lapses_in(at(0)), None);
    }

    /// A polled project is kept current by its own poll, which is the
    /// operator's choice and not a producer failing quietly.
    #[test]
    fn a_polled_project_never_lapses() {
        let mut armed = polling().lapsing_after(COVERED_FOR);

        armed.came_back(&arkham(), at(100));

        assert!(!armed.lapsed(at(1_000_000)));
        assert_eq!(armed.lapses_in(at(100)), None);
    }

    /// The word is a claim about now, so an edit to the config keeps it: a
    /// project covered a moment ago is not lapsed by the reader saving a file.
    #[test]
    fn a_config_the_reader_writes_keeps_the_last_word() {
        let mut standing = not_polling();
        standing.came_back(&arkham(), at(100));

        let named = standing.still_due(not_polling());

        assert!(!named.lapsed(at(159)));
        assert!(named.lapsed(at(160)));
    }

    /// A read of everything reads this project too, so it arms this project.
    #[test]
    fn a_read_of_every_project_arms_this_one() {
        let mut armed = polling();

        armed.came_back(&Wanted::Everything, at(100));

        assert_eq!(armed.asks(at(130)), Some(arkham()));
    }

    /// And a read of a different project does not: that is the whole of
    /// per-project independence, and it is settled by what the read named.
    #[test]
    fn a_read_of_another_project_leaves_this_one_as_it_was() {
        let mut armed = polling();
        armed.came_back(&arkham(), at(100));

        armed.came_back(&ferry(), at(120));

        assert_eq!(
            armed.asks(at(130)),
            Some(arkham()),
            "arkham asks 30 after its own read, not 30 after ferry's"
        );
    }

    /// Two projects read at different instants ask at different instants, and
    /// go on doing so. This is what a global tick could not do: it refreshed
    /// the whole configured set together, so every project paid the cascade
    /// at once.
    #[test]
    fn projects_read_at_different_instants_stay_apart() {
        let mut first = Armed::polling("arkham".to_string(), Some(EVERY));
        let mut second = Armed::polling("ferry".to_string(), Some(EVERY));

        first.came_back(&arkham(), at(100));
        second.came_back(&ferry(), at(112));

        assert_eq!(first.asks_in(at(100)), Some(EVERY));
        assert_eq!(second.asks_in(at(100)), Some(Duration::from_secs(42)));
        assert_eq!(first.asks(at(130)), Some(arkham()));
        assert_eq!(second.asks(at(130)), None, "ferry's own read was later");
    }

    /// An ask already overdue is due now rather than negative: the loop asks
    /// this how long to wait, and a wait it cannot express is one it would
    /// have to guess at.
    #[test]
    fn an_ask_already_overdue_says_it_waits_no_longer() {
        let mut armed = polling();
        armed.came_back(&arkham(), at(100));

        assert_eq!(armed.asks_in(at(500)), Some(Duration::ZERO));
    }

    #[test]
    fn a_project_answers_to_its_own_name() {
        assert_eq!(polling().project(), "arkham");
    }
}
