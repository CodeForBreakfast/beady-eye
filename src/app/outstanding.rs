//! The reads asked for and not yet come back, and when each one leaves.
//!
//! A read waits out a short window before it goes, so a burst of asks about
//! one project costs one read, and waits behind the read in flight, so a
//! tracker is asked one thing at a time. Whatever drives this, a view or a
//! watcher with no view, decides nothing about either.

use std::sync::mpsc::Sender;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};

use super::{Asked, Awaited, Wanted};

/// How long a read is held after it is asked for before it is sent, so that
/// a burst about one project costs one read rather than one each.
///
/// A producer with nothing to lose by talking — a hook firing per commit, a
/// key held down — says the same thing many times in a moment, and without
/// this each saying is a read. It runs from the first notification and is not
/// reset by the ones after it, so a `^R` held down still gets the read it was
/// pressed for; a resetting window would withhold it for as long as the key
/// was down.
///
/// **It has to stay well under `[tui] unanswered_after_seconds`.** A read
/// waiting out its window is drawn exactly like one a tracker has stopped
/// answering, because `Awaited::unanswered_at` measures from the ask and
/// deliberately not from the send — `Outstanding::came_back` says why that
/// stamp cannot move. The two are kept apart by the config key counting in
/// whole seconds: the shortest wait it can name that is not "immediately" is
/// a second, and this is a fifth of it.
///
/// `a_read_goes_before_its_project_can_be_said_to_have_stopped_being_read`
/// holds that against the shortest patience the key can name, through the
/// predicate rather than by comparing two constants, and on the pair
/// `Outstanding::for_a_run` builds rather than on this constant — so it
/// holds whichever value the window comes to be read from. A
/// `debug_assert!(window < patience)` in `waiting` would cover it too and is
/// not available: the tests that are about the window construct one longer
/// than the patience on purpose.
const WINDOW: TimeDelta = TimeDelta::milliseconds(200);

/// What has been asked for and not yet collected.
///
/// A project is in one of three states here and never in none of them: its
/// read is in flight, or its read is asked for and waiting — out its window,
/// or behind the read in front of it — or it has nothing here at all and is
/// `Armed` to ask again. The last transition is `came_back`'s, and it is what
/// makes the three cover every project: a read that comes back arms, a read
/// that never comes back stays here and is drawn as unanswered.
///
/// A request arriving while a collection is in flight used to be dropped, on
/// the grounds that the collection already running was reading exactly what
/// it would ask for. A refresh that names a project is what ends that: the
/// one running may be reading a different project entirely, and dropping the
/// request would lose the change it was sent for — the failure the inbound
/// channel exists to prevent. So a request waits its turn instead. A whole
/// collection absorbs the single projects it would read anyway, so what waits
/// is never more than one per project.
pub struct Outstanding {
    /// Every read asked for and not yet come back, in the order they will be
    /// served: the one the collector has, then whatever is waiting for it.
    ///
    /// What each names rather than that some read is running — a project line
    /// says for itself whether its own rows are on their way, so the screen
    /// needs to know which projects and not only that some are — and *when
    /// each was asked for*, because that is the only measure of the wait
    /// there is. Nothing downstream can recover it: the collector blocks in
    /// `Command::output()`, which has no deadline of its own, and reports
    /// nothing until it is done, so a tracker hung for an hour and one asked
    /// half a second ago look identical from every side but this one. A read
    /// that has not been sent yet is worse still, because there is nothing to
    /// report from at all.
    ///
    /// One sequence rather than the one in flight beside a stash of what
    /// waits. The question a project line asks is how long its rows have been
    /// on their way, and being sent is a step along that wait rather than the
    /// start of it — so the two belong to one list, ordered by when each was
    /// asked for, which is also the order the collector takes them in.
    awaited: Vec<Awaited>,
    /// How long a read this asks for may go unanswered before the project it
    /// names is reported as having stopped being read. Held here because this
    /// is where a read is asked for, and carried on each one so that whoever
    /// draws it needs nothing else to decide.
    patience: TimeDelta,
    /// How long the read at the front is held before it is sent, so that a
    /// burst of notifications about one project costs one read rather than
    /// one each. See `WINDOW`.
    window: TimeDelta,
    /// Whether the read at the front has gone to the collector. False while
    /// it is waiting out its window, and false again the moment the read it
    /// named comes back.
    sent: bool,
}

impl Outstanding {
    /// Nothing outstanding, at the two waits a run drives with: the patience
    /// its config names, and `WINDOW`.
    ///
    /// The only place production pairs them, so that a guard calling this
    /// holds the relationship between them against the pair a run is built
    /// with rather than against `WINDOW`.
    pub fn for_a_run(patience: TimeDelta) -> Self {
        Self::waiting(patience, WINDOW)
    }

    /// Wait this long on a read from here on, as the config the reader has
    /// just written says.
    ///
    /// The reads already asked for keep the patience they were stamped with,
    /// because that is what the screen has been drawing them against: a read
    /// the reader has been watching for a minute would otherwise be reported
    /// as having stopped answering by an edit that said nothing about it.
    pub fn waits_out(&mut self, patience: TimeDelta) {
        self.patience = patience;
    }

    pub fn waiting(patience: TimeDelta, window: TimeDelta) -> Self {
        Self {
            awaited: Vec::new(),
            patience,
            window,
            sent: false,
        }
    }

    /// Ask for a collection, or keep it until the one running comes back.
    ///
    /// Reports whether what is outstanding is any different for it, which is
    /// not the same as whether a collection started: a request arriving
    /// mid-collection waits its turn, and its project's line says so.
    ///
    /// Asking never sends. Every read waits out its window first, and
    /// `sends` is where the leaving happens — so the screen says a read is
    /// coming at the instant it was asked for, whatever the window then does
    /// about when it goes.
    pub fn ask(&mut self, wanted: Wanted, now: DateTime<Utc>) -> bool {
        if self.awaited.is_empty() {
            self.awaited.push(self.stamped(wanted, now));
            return true;
        }
        self.queue(wanted, now)
    }

    /// Send the read at the front, where its window is out and no other is in
    /// flight.
    ///
    /// One at a time, as it has always been: a collection is dozens of round
    /// trips per project and two at once would double what a tracker is
    /// asked without halving anything.
    pub fn sends(&mut self, ask: &Sender<Asked>, now: DateTime<Utc>) {
        if self.sent {
            return;
        }
        let Some(next) = self.awaited.first() else {
            return;
        };
        if now < next.asked_at + self.window {
            return;
        }
        if ask.send(Asked::Read(next.wanted.clone())).is_err() {
            self.awaited.clear();
            return;
        }
        self.sent = true;
    }

    /// How long until the read at the front leaves, or nothing where none is
    /// waiting to leave: the loop sleeps until this among its other
    /// deadlines, because nothing else is going to wake it for a window
    /// running out.
    pub fn sends_in(&self, now: DateTime<Utc>) -> Option<Duration> {
        if self.sent {
            return None;
        }
        self.awaited.first().map(|next| {
            (next.asked_at + self.window - now)
                .to_std()
                .unwrap_or_default()
        })
    }

    /// Keep a request until its turn, where nothing already waiting covers
    /// it.
    ///
    /// Covered by what is *waiting* rather than by what is in flight: the
    /// collection running may have passed the project before the change was
    /// reported, so a change arriving mid-collection always earns a read of
    /// its own. What it does not earn is a second one.
    fn queue(&mut self, wanted: Wanted, now: DateTime<Utc>) -> bool {
        match wanted {
            // A whole collection reads every project, so it stands in for the
            // single ones waiting with it — and its wait begins when it was
            // asked for, not when the earliest of theirs did. It names every
            // project on the screen, including the ones nothing had asked
            // about, and one instant cannot be true of both: an inherited one
            // would put a wait those projects never had beside their names,
            // and a whole screen of marks saying the reads have stopped is
            // what a reader gets for one project changing.
            //
            // What that costs is the projects it absorbs. Their wait was
            // longer and this understates it, for one patience, after which
            // the mark says what it said before. Understating a wait is what
            // patience is: it is the length of not-saying-yet the project has
            // already decided on, and it is bounded. Overstating one is a
            // claim about a tracker nobody asked.
            Wanted::Everything => {
                if self.queued().any(|it| it.wanted == Wanted::Everything) {
                    return false;
                }
                self.awaited.truncate(self.in_flight());
                self.awaited.push(self.stamped(Wanted::Everything, now));
                true
            }
            // A project reported for again while it waits is the same wait: a
            // project nothing reports for is polled every refresh interval,
            // and the poll goes on naming it for as long as it is uncovered,
            // so a stamp taken from the latest ask would be pushed forward by
            // the very polling that proves nothing has been read.
            Wanted::Project(project) => {
                if self.queued().any(|it| it.wanted.names(&project)) {
                    return false;
                }
                self.awaited
                    .push(self.stamped(Wanted::Project(project), now));
                true
            }
        }
    }

    /// The reads that have not been sent.
    ///
    /// Every one but the first, once the first has gone — and every one of
    /// them while the first is still waiting out its window, which is what
    /// makes the window a debounce at all. A hundred messages about one
    /// project arriving into an idle `bdi` find their own unsent read at the
    /// front and are dropped against it; were the front skipped they would
    /// queue ninety-nine reads behind it.
    fn queued(&self) -> impl Iterator<Item = &Awaited> {
        self.awaited.iter().skip(self.in_flight())
    }

    /// How many reads the collector has: one, or none while the front is
    /// still waiting out its window.
    fn in_flight(&self) -> usize {
        usize::from(self.sent)
    }

    fn stamped(&self, wanted: Wanted, asked_at: DateTime<Utc>) -> Awaited {
        Awaited {
            wanted,
            asked_at,
            patience: self.patience,
        }
    }

    /// Take the read that came back, and say what it read.
    ///
    /// What it read is what arms the projects it covered for their next ask,
    /// which is the only thing that arms them: a read nobody answers arms
    /// nothing, and a project with nothing coming is what the unanswered mark
    /// is for.
    ///
    /// Whatever waited behind it is left to `sends`. The one that reaches the
    /// front keeps the stamp it queued at rather than being stamped again.
    /// Its project's rows have been on their way since the change that wanted
    /// them was reported, and a wait that started over on reaching the front
    /// would tell a reader whose project had been stranded ten minutes behind
    /// a hung tracker that its own tracker had just been asked. Its window
    /// ran out while it waited, so `sends` finds it due and it leaves at once.
    pub fn came_back(&mut self) -> Option<Wanted> {
        if self.awaited.is_empty() {
            return None;
        }
        self.sent = false;
        Some(self.awaited.remove(0).wanted)
    }

    /// Every read outstanding and since when, for the screen to say beside
    /// the projects each of them names.
    pub fn awaited(&self) -> &[Awaited] {
        &self.awaited
    }
}

/// Every property here is one stamp against another: the deadline a queued
/// read crosses is thirty seconds away, and the refresh that re-reports it is
/// thirty seconds apart. So the instants are written by hand, and a test that
/// let the clock supply the difference would be deciding on scheduling jitter
/// rather than on the rule.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::fixtures::{arkham, at_once, ferry, reads, A_LONG_WINDOW, PATIENCE};
    use crate::config::Tui;
    use chrono::TimeZone;
    use pretty_assertions::assert_eq;
    use std::sync::mpsc::{self, Receiver};

    /// `second` seconds into the run, which is how these are written: a
    /// wait is a difference between two of them and the wall clock they
    /// sit on says nothing.
    fn at(second: i64) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 1, 10, 0, 0).unwrap() + TimeDelta::seconds(second)
    }

    /// Asked at `at(0)`, sent, and never answered — so everything after
    /// it waits.
    fn hung_on(project: Wanted) -> (Outstanding, Sender<Asked>, Receiver<Asked>) {
        let (ask, asked) = mpsc::channel();
        let mut outstanding = at_once();
        outstanding.ask(project, at(0));
        outstanding.sends(&ask, at(0));
        (outstanding, ask, asked)
    }

    fn stamps(outstanding: &Outstanding) -> Vec<(Wanted, chrono::DateTime<Utc>)> {
        outstanding
            .awaited()
            .iter()
            .map(|it| (it.wanted.clone(), it.asked_at))
            .collect()
    }

    /// The bead: a project queued behind a tracker that has stopped
    /// answering has to be able to say its own reads have stopped, and
    /// the only thing that can say it is how long the read has been
    /// waiting.
    #[test]
    fn a_read_that_cannot_be_sent_yet_is_stamped_when_it_joins_the_queue() {
        let (mut outstanding, _ask, _asked) = hung_on(arkham());

        outstanding.ask(ferry(), at(5));

        assert_eq!(stamps(&outstanding), [(arkham(), at(0)), (ferry(), at(5))]);
    }

    /// A project with nothing reporting for it is polled every refresh
    /// interval, and the poll goes on reporting it for as long as it is
    /// uncovered — so a queued read is asked for again and again while it
    /// waits. Its wait is how long its rows have been on their way, which
    /// the first of those asks started; a stamp taken from the latest
    /// would be reset by the very polling that proves nothing has been
    /// read, and the mark would never turn.
    #[test]
    fn a_project_reported_for_again_while_it_waits_keeps_the_wait_it_has() {
        let (mut outstanding, _ask, _asked) = hung_on(arkham());

        outstanding.ask(ferry(), at(5));
        outstanding.ask(ferry(), at(35));

        assert_eq!(stamps(&outstanding), [(arkham(), at(0)), (ferry(), at(5))]);
    }

    /// The other side of that rule: a project reported for again while
    /// it is being *read* is a new wait, stamped at its own ask. One that
    /// carried the running read's stamp would be drawn as having stopped
    /// answering the moment it began.
    #[test]
    fn a_project_reported_for_again_while_it_is_read_is_stamped_at_its_own_ask() {
        let (mut outstanding, _ask, _asked) = hung_on(arkham());

        outstanding.ask(arkham(), at(5));

        assert_eq!(stamps(&outstanding), [(arkham(), at(0)), (arkham(), at(5))]);
    }

    /// `codex review` on this change, and it is right: a whole collection
    /// absorbing what waits must not inherit their wait.
    ///
    /// It names every project, including every one nothing had asked
    /// about — so an inherited instant is a wait those projects never had,
    /// and one project changing puts a mark saying the reads have stopped
    /// beside every name on the screen. The first version of this took the
    /// earliest absorbed wait, reasoning only about the projects being
    /// absorbed and never about the ones the collection newly covers.
    ///
    /// What it costs the absorbed ones is a patience: ferry has been
    /// waiting since :05 and this says :40, so its mark goes back from
    /// stopped to turning and returns one patience later. That is the
    /// length of not-saying-yet the project has already chosen, and it is
    /// bounded. The other way round is unbounded and about trackers
    /// nobody asked.
    #[test]
    fn a_whole_collection_absorbing_what_waits_starts_a_wait_of_its_own() {
        let (mut outstanding, _ask, _asked) = hung_on(arkham());

        outstanding.ask(ferry(), at(5));
        outstanding.ask(Wanted::Everything, at(40));

        assert_eq!(
            stamps(&outstanding),
            [(arkham(), at(0)), (Wanted::Everything, at(40))],
            "not :05, which would be every other project's wait too"
        );
    }

    /// The absorption `truncate` could not do before, because before the
    /// window there was no such thing as an unsent read: `ask` sent the
    /// moment the queue was empty, so the front was always in flight and
    /// always had to be kept. No existing test asserts this case because
    /// no existing code could produce it.
    #[test]
    fn a_whole_collection_absorbs_a_read_that_has_not_been_sent() {
        let mut outstanding = Outstanding::waiting(PATIENCE, A_LONG_WINDOW);
        outstanding.ask(arkham(), at(0));

        outstanding.ask(Wanted::Everything, at(1));

        assert_eq!(
            stamps(&outstanding),
            [(Wanted::Everything, at(1))],
            "the whole collection reads arkham anyway, and nothing had gone yet"
        );
    }

    /// And the one in flight is still kept, because the collection
    /// running is not the collection about to be asked for.
    #[test]
    fn a_whole_collection_keeps_the_read_already_in_flight() {
        let (mut outstanding, _ask, _asked) = hung_on(arkham());

        outstanding.ask(Wanted::Everything, at(1));

        assert_eq!(
            stamps(&outstanding),
            [(arkham(), at(0)), (Wanted::Everything, at(1))]
        );
    }

    /// A held-down `^R` gets the read it was pressed for. The window runs
    /// from the first notification and is not reset by the ones after it;
    /// under reset the read would be withheld for as long as the key was
    /// down, which is the one thing that key exists to force.
    #[test]
    fn a_window_is_not_pushed_back_by_the_notifications_that_arrive_in_it() {
        let mut outstanding = Outstanding::waiting(PATIENCE, TimeDelta::seconds(2));
        let (ask, asked) = mpsc::channel();
        outstanding.ask(Wanted::Everything, at(0));

        for pressed in 1..=10 {
            outstanding.ask(Wanted::Everything, at(pressed));
            outstanding.sends(&ask, at(pressed));
        }

        assert_eq!(
            reads(asked.try_iter()),
            [Wanted::Everything],
            "sent two seconds after the first press, not two after the last"
        );
    }

    /// The relationship between the window and the patience, through the
    /// predicate that would get it wrong rather than between two
    /// constants: a read held for the whole of the window goes before the
    /// shortest patience the config key can name has run out, so a
    /// project waiting out a window is never drawn as one whose tracker
    /// has stopped answering.
    ///
    /// It names no window of its own: the pair is the one
    /// `Outstanding::for_a_run` builds, and the read is held for as long
    /// as that pair says its window is. So a window wired to a config key
    /// arrives here as the length of the wait, and this holds the
    /// relationship at whatever value production has come to use.
    #[test]
    fn a_read_goes_before_its_project_can_be_said_to_have_stopped_being_read() {
        let shortest = Tui {
            unanswered_after_seconds: 1,
            ..Tui::default()
        }
        .unanswered_after();
        let (ask, asked) = mpsc::channel();
        let mut outstanding = Outstanding::for_a_run(shortest);
        outstanding.ask(arkham(), at(0));

        let held_for = outstanding
            .sends_in(at(0))
            .expect("the read just asked for is waiting out its window");
        let out =
            at(0) + TimeDelta::from_std(held_for).expect("a window is a length chrono can carry");
        outstanding.sends(&ask, out);

        assert_eq!(
            reads(asked.try_iter()),
            [arkham()],
            "the window was out, so the read went"
        );
        assert!(
            !outstanding.awaited()[0].unanswered_at(out),
            "and arkham was not yet said to have stopped being read"
        );
    }

    /// Reaching the front is not being asked for. A read carries the wait
    /// it has had all along, so a project stranded ten minutes behind a
    /// hung tracker does not tell the reader its tracker was asked a
    /// moment ago the instant that tracker answers.
    #[test]
    fn a_read_that_reaches_the_front_keeps_the_stamp_it_queued_at() {
        let (mut outstanding, _ask, _asked) = hung_on(arkham());
        outstanding.ask(ferry(), at(5));

        outstanding.came_back();

        assert_eq!(stamps(&outstanding), [(ferry(), at(5))]);
    }

    /// Every read outstanding, in the order they will be served, is what
    /// the collector is served from — so the first is the one it is
    /// working on and the rest follow in turn.
    #[test]
    fn the_reads_are_sent_in_the_order_they_were_asked_for() {
        let (mut outstanding, ask, asked) = hung_on(arkham());
        outstanding.ask(ferry(), at(5));
        outstanding.ask(Wanted::Project("harbour".to_string()), at(6));

        outstanding.came_back();
        outstanding.sends(&ask, at(7));
        outstanding.came_back();
        outstanding.sends(&ask, at(8));

        assert_eq!(
            reads(asked.try_iter()),
            [arkham(), ferry(), Wanted::Project("harbour".to_string())]
        );
    }

    /// A collection nothing is waiting for is nothing to take: the loop
    /// answers whatever arrives on the one channel, in whatever order it
    /// arrives, and a snapshot with no ask behind it must leave it as it
    /// was rather than end the run.
    #[test]
    fn a_collection_nothing_asked_for_leaves_the_queue_alone() {
        let (ask, _asked) = mpsc::channel();
        let mut outstanding = at_once();

        assert_eq!(outstanding.came_back(), None);

        outstanding.sends(&ask, at(0));

        assert_eq!(stamps(&outstanding), []);
    }
}
