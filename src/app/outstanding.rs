//! The reads asked for and not yet come back, and when each one leaves.
//!
//! A read waits out a short window before it goes, so a burst of asks about
//! one project costs one read, and waits behind the read in flight, so a
//! tracker is asked one thing at a time. Whatever drives this, a view or a
//! listener with no view, decides nothing about either.

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
