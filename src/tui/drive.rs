//! Answering one event at a time, until the user quits.
//!
//! Everything that happens to `bdi` reaches here as an `Event` on one
//! channel, and everything the loop does about one it does through `View`.
//! Those two types are the whole of what it knows: it never names a
//! terminal, a forest or a tail, and nothing that produces an event names
//! the loop.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};

use ratatui::crossterm::event::KeyEvent;

use crate::app::{Awaited, Wanted};
use crate::collect::panes::Answer;
use crate::model::snapshot::Snapshot;
use crate::view::{Action, Motion};

use super::keys::action;

/// Everything that reaches the loop.
///
/// A `Snapshot` is large and the other three carry almost nothing, so the
/// collected one is boxed rather than widening every event to its size.
#[cfg_attr(test, derive(Debug, PartialEq))]
pub(super) enum Event {
    Key(KeyEvent),
    /// A left click, on the row of the screen it landed on.
    Clicked(u16),
    /// A wheel notch, as the move it asks the selection to make.
    Scrolled(Motion),
    Resize,
    /// Work has moved on, and what has to be read to see it.
    Changed(Wanted),
    /// A collection has come back.
    Collected(Box<Snapshot>),
    /// herdr has said what is on a pane, or would not say.
    Tailed(Answer),
    /// Something outside has asked `bdi` to stop.
    ///
    /// Its own event rather than a keystroke standing in for one: the loop
    /// answers any key at all by taking the bindings window away, so a
    /// synthesised 'q' arriving while that window is up would close the
    /// window and leave `bdi` running.
    Signalled,
}

/// Every answer herdr gives reaches the loop as one of these, which is the
/// whole of what `collect::panes` knows about the loop: it is handed a
/// `Sender` and told nothing about where it goes.
impl From<Answer> for Event {
    fn from(answer: Answer) -> Self {
        Event::Tailed(answer)
    }
}

/// What the screen has on it.
///
/// The loop holds this rather than the view because it decides what a
/// keystroke means, and while the bindings are up every keystroke means "take
/// them away".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Showing {
    Forest,
    Bindings,
}

/// The rows on the screen and what the user has done to them.
///
/// The seam the loop steers the view across: the loop knows the actions and
/// nothing about rows, and the view knows the rows and only the names of the
/// keys it is handed.
pub(super) trait View {
    /// Show a snapshot just collected, in place of the one on the screen.
    fn collected(&mut self, snapshot: Snapshot);

    /// Say which reads are outstanding, or that none are, reporting whether
    /// the screen has changed.
    ///
    /// The view is told when a read is asked for and told again when one
    /// comes back, so what is on the screen and what the trackers are being
    /// asked are never more than one event apart. It is told *which
    /// projects*, because each project's line says for itself whether its own
    /// rows are about to be replaced — and *when each was asked for*, because
    /// a read that has stopped getting anywhere is drawn exactly like one
    /// just asked for until something measures the wait.
    ///
    /// A sequence rather than the one in flight: a read waiting its turn is
    /// as outstanding as the one being served, and the projects it names have
    /// nothing else on the screen to say so.
    fn collecting(&mut self, awaited: &[Awaited]) -> bool;

    /// How long what is drawn goes on being true with nothing happening, or
    /// nothing where it stays true however long the reader leaves it.
    ///
    /// The loop asks the view rather than deciding for itself, because what
    /// goes stale is what is drawn: an age is a duration and says a different
    /// thing a second later, and a mark part way through turning is a frame
    /// behind by the time the next one is due.
    fn holds_for(&self) -> Option<Duration>;

    /// Take what herdr said about a pane it was asked to read or to focus,
    /// reporting whether the screen has changed.
    fn tailed(&mut self, answer: Answer) -> bool;

    /// Apply one action, reporting whether the screen has changed.
    fn apply(&mut self, action: Action) -> bool;

    /// Select whatever is drawn on one row of the screen, reporting whether
    /// the screen has changed.
    ///
    /// A row is a fact about the frame rather than about the forest, so this
    /// is the loop's other seam: the loop knows where the pointer was and
    /// nothing about what is drawn there.
    fn clicked(&mut self, row: u16) -> bool;

    fn draw(&mut self, showing: Showing) -> anyhow::Result<()>;
}

/// What the loop waited for and got.
enum Waited {
    Event(Event),
    /// What is drawn has stopped being true and nothing has happened: an age
    /// has moved on, or the collecting mark is due its next frame.
    Aged,
}

/// The next thing the loop answers, or nothing where there will never be
/// another.
///
/// Every wait in here is a wait on the one channel: a keystroke, a resize, a
/// project reporting a change and a collection coming back are the same kind
/// of thing to the loop. The one deadline it ever sleeps until is `holds_for`
/// — how long what is drawn goes on being true — because the screen now says
/// things that go stale on their own: an age, and a mark part way through
/// turning. Where it says neither, there is no deadline and the loop waits as
/// long as it has to.
fn wait(events: &Receiver<Event>, holds_for: Option<Duration>) -> Option<Waited> {
    let Some(holds_for) = holds_for else {
        return events.recv().ok().map(Waited::Event);
    };
    match events.recv_timeout(holds_for) {
        Ok(event) => Some(Waited::Event(event)),
        Err(RecvTimeoutError::Timeout) => Some(Waited::Aged),
        Err(RecvTimeoutError::Disconnected) => None,
    }
}

/// Read events until the user quits.
///
/// The collection already in flight is handed in rather than started here.
/// The run's first collection is asked for before the screen opens, so that
/// the first frame this draws already carries the mark saying every project
/// is being read — a forest with no rows and no mark would be a forest that
/// looks read and is not. Where that ask happens is `run`'s business, which
/// is where the order everything starts in is decided.
pub(super) fn drive(
    view: &mut dyn View,
    events: &Receiver<Event>,
    ask: &Sender<Wanted>,
    mut outstanding: Outstanding,
) -> anyhow::Result<()> {
    let mut showing = Showing::Forest;
    view.draw(showing)?;

    while let Some(waited) = wait(events, view.holds_for()) {
        let event = match waited {
            // Nothing has happened and what is drawn is out of date, which is
            // the whole of what makes the mark turn and the ages advance: a
            // collection is dozens of round trips and reports nothing until
            // it is done, and a resting `bdi` whose projects are all reported
            // for polls nothing at all.
            Waited::Aged => {
                view.draw(showing)?;
                continue;
            }
            Waited::Event(event) => event,
        };

        let changed = match event {
            // Any key at all, because a reader who opened the bindings by
            // accident must not have to find the one key that closes them.
            Event::Key(_) if showing == Showing::Bindings => {
                showing = Showing::Forest;
                true
            }
            Event::Key(key) => match action(key) {
                Some(Action::Quit) => return Ok(()),
                Some(Action::ShowBindings) => {
                    showing = Showing::Bindings;
                    true
                }
                Some(Action::Refresh) => {
                    outstanding.ask(ask, Wanted::Everything, Utc::now())
                        && view.collecting(outstanding.awaited())
                }
                Some(action) => view.apply(action),
                None => false,
            },
            // A click or a notch takes the bindings away and does no more,
            // for the same reason a key does: the window is over the forest,
            // so the rows under the pointer are rows nobody can see.
            Event::Clicked(_) | Event::Scrolled(_) if showing == Showing::Bindings => {
                showing = Showing::Forest;
                true
            }
            Event::Clicked(row) => view.clicked(row),
            Event::Scrolled(motion) => view.apply(Action::Move(motion)),
            Event::Resize => true,
            Event::Changed(wanted) => {
                outstanding.ask(ask, wanted, Utc::now()) && view.collecting(outstanding.awaited())
            }
            Event::Collected(snapshot) => {
                outstanding.came_back(ask);
                view.collected(*snapshot);
                // Told after the rows land, and told whatever came of the
                // collection that ended: another may have been waiting behind
                // it, and where none was, a line left saying it was being
                // read would say so over rows that had already arrived.
                view.collecting(outstanding.awaited());
                true
            }
            Event::Tailed(answer) => view.tailed(answer),
            // The same return 'q' takes, and for the same reason: it is
            // returning that drops the screen, and dropping the screen is
            // what hands the terminal back.
            Event::Signalled => return Ok(()),
        };

        if changed {
            view.draw(showing)?;
        }
    }

    Ok(())
}

/// What has been asked for and not yet collected.
///
/// A request arriving while a collection is in flight used to be dropped, on
/// the grounds that the collection already running was reading exactly what
/// it would ask for. A refresh that names a project is what ends that: the
/// one running may be reading a different project entirely, and dropping the
/// request would lose the change it was sent for — the failure the inbound
/// channel exists to prevent. So a request waits its turn instead. A whole
/// collection absorbs the single projects it would read anyway, so what waits
/// is never more than one per project.
pub(super) struct Outstanding {
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
}

impl Outstanding {
    pub(super) fn waiting(patience: TimeDelta) -> Self {
        Self {
            awaited: Vec::new(),
            patience,
        }
    }

    /// Ask for a collection, or keep it until the one running comes back.
    ///
    /// Reports whether what is outstanding is any different for it, which is
    /// not the same as whether a collection started: a request arriving
    /// mid-collection waits its turn, and its project's line says so.
    pub(super) fn ask(&mut self, ask: &Sender<Wanted>, wanted: Wanted, now: DateTime<Utc>) -> bool {
        if self.awaited.is_empty() {
            if ask.send(wanted.clone()).is_err() {
                return false;
            }
            self.awaited.push(self.stamped(wanted, now));
            return true;
        }
        self.queue(wanted, now)
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
                self.awaited.truncate(1);
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

    /// The reads that have not been sent, which is every one but the first.
    fn queued(&self) -> impl Iterator<Item = &Awaited> {
        self.awaited.iter().skip(1)
    }

    fn stamped(&self, wanted: Wanted, asked_at: DateTime<Utc>) -> Awaited {
        Awaited {
            wanted,
            asked_at,
            patience: self.patience,
        }
    }

    /// Take the read that came back, sending whatever waited behind it.
    ///
    /// The one that reaches the front keeps the stamp it queued at rather
    /// than being stamped again. Its project's rows have been on their way
    /// since the change that wanted them was reported, and a wait that
    /// started over on reaching the front would tell a reader whose project
    /// had been stranded ten minutes behind a hung tracker that its own
    /// tracker had just been asked.
    fn came_back(&mut self, ask: &Sender<Wanted>) {
        if self.awaited.is_empty() {
            return;
        }
        self.awaited.remove(0);
        let Some(next) = self.awaited.first() else {
            return;
        };
        if ask.send(next.wanted.clone()).is_err() {
            self.awaited.clear();
        }
    }

    /// Every read outstanding and since when, for the screen to say beside
    /// the projects each of them names.
    pub(super) fn awaited(&self) -> &[Awaited] {
        &self.awaited
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::fixtures::{a_snapshot, atlas, ferry, A_MOMENT, PATIENCE};
    use crate::tui::keys::tests::{control, key};
    use crate::tui::wire::collector;
    use crate::view::phrase;
    use ratatui::crossterm::event::KeyCode;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Instant;

    /// A view that remembers what the loop did to it.
    #[derive(Default)]
    struct Recorder {
        applied: Vec<Action>,
        clicked: Vec<u16>,
        collected: usize,
        /// What the view was told is outstanding, in the order it was told.
        /// What and not how many: a project line answers for its own rows, so
        /// a test that only counted could not tell a refresh of one project
        /// from a refresh of the lot.
        awaited: Vec<Vec<Awaited>>,
        drawn: usize,
        showing: Vec<Showing>,
        /// What a click reports back, for the tests about a click that lands
        /// on no row.
        nothing_under_the_pointer: bool,
    }

    impl Recorder {
        /// Which projects the view was told about, in the order it was told,
        /// with the instants left out. Most of these tests are about which
        /// collections the loop starts and in what order; the ones about the
        /// stamps read `awaited` itself.
        fn collecting(&self) -> Vec<Vec<Wanted>> {
            self.awaited
                .iter()
                .map(|told| told.iter().map(|it| it.wanted.clone()).collect())
                .collect()
        }

        /// When each read the view was told about was asked for, in the order
        /// it was told, flattened: the tests that read this are about how one
        /// stamp compares with another rather than about which telling each
        /// came in.
        fn asked_at(&self) -> Vec<chrono::DateTime<Utc>> {
            self.awaited
                .iter()
                .flat_map(|told| told.iter().map(|it| it.asked_at))
                .collect()
        }
    }

    impl View for Recorder {
        fn collected(&mut self, _snapshot: Snapshot) {
            self.collected += 1;
        }

        fn collecting(&mut self, awaited: &[Awaited]) -> bool {
            self.awaited.push(awaited.to_vec());
            true
        }

        /// The same rule `Shown` keeps, so a loop test is asking the loop
        /// what it asks a real screen: a read outstanding is a frame away
        /// from being out of date, and this view has no ages on it.
        fn holds_for(&self) -> Option<Duration> {
            let told = self.awaited.last()?;
            (!told.is_empty()).then_some(phrase::FRAME)
        }

        fn tailed(&mut self, _answer: Answer) -> bool {
            true
        }

        fn apply(&mut self, action: Action) -> bool {
            self.applied.push(action);
            true
        }

        fn clicked(&mut self, row: u16) -> bool {
            self.clicked.push(row);
            !self.nothing_under_the_pointer
        }

        fn draw(&mut self, showing: Showing) -> anyhow::Result<()> {
            self.drawn += 1;
            self.showing.push(showing);
            Ok(())
        }
    }

    /// An event source holding everything the loop will see, in order.
    fn waiting(events: Vec<Event>) -> Receiver<Event> {
        let (to, from) = mpsc::channel();
        for event in events {
            to.send(event)
                .expect("the loop's end of the channel is open");
        }
        from
    }

    fn typing(keys: [KeyEvent; 4]) -> Receiver<Event> {
        waiting(keys.into_iter().map(Event::Key).collect())
    }

    #[test]
    fn a_keypress_reaches_the_view_as_the_action_it_is_bound_to() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let events = typing([
            key(KeyCode::Char('j')),
            key(KeyCode::Char(' ')),
            key(KeyCode::Char('q')),
            key(KeyCode::Char('k')),
        ]);

        drive(&mut view, &events, &ask, Outstanding::waiting(PATIENCE)).expect("the loop runs");

        assert_eq!(
            view.applied,
            [Action::Move(Motion::NextRow), Action::ToggleFold],
            "q ends the loop, so nothing after it is applied"
        );
    }

    /// `?` puts the bindings up and holds them there, and the next keystroke
    /// takes them away whatever it is — including one nothing is bound to,
    /// which is why the dismissal cannot live in the mapping.
    #[test]
    fn a_question_mark_shows_the_bindings_and_the_next_key_takes_them_away() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let events = typing([
            key(KeyCode::Char('?')),
            key(KeyCode::Char('z')),
            key(KeyCode::Char('?')),
            key(KeyCode::Char('j')),
        ]);

        drive(&mut view, &events, &ask, Outstanding::waiting(PATIENCE)).expect("the loop runs");

        assert_eq!(
            view.showing,
            [
                Showing::Forest,
                Showing::Bindings,
                Showing::Forest,
                Showing::Bindings,
                Showing::Forest,
            ]
        );
        assert!(
            view.applied.is_empty(),
            "j closed the bindings rather than moving the selection: {:?}",
            view.applied
        );
    }

    /// While the bindings are up, `q` is a key like any other: it puts them
    /// away. The forest a reader was looking at is still there to quit from.
    #[test]
    fn quitting_from_the_bindings_takes_two_presses_and_the_first_is_not_lost() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let events = typing([
            key(KeyCode::Char('?')),
            key(KeyCode::Char('q')),
            key(KeyCode::Char('q')),
            key(KeyCode::Char('j')),
        ]);

        drive(&mut view, &events, &ask, Outstanding::waiting(PATIENCE)).expect("the loop runs");

        assert_eq!(
            view.showing,
            [Showing::Forest, Showing::Bindings, Showing::Forest],
            "the second q ended the loop, so nothing was drawn after it"
        );
    }

    /// A collection landing behind the bindings is taken, and the bindings
    /// stay up: a view that vanished under the refresh tick would be one a
    /// reader could not finish reading.
    #[test]
    fn a_collection_arriving_behind_the_bindings_leaves_them_up() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Key(key(KeyCode::Char('?'))),
            Event::Collected(Box::new(a_snapshot())),
        ]);

        drive(&mut view, &events, &ask, Outstanding::waiting(PATIENCE)).expect("the loop runs");

        assert_eq!(view.collected, 1);
        assert_eq!(
            view.showing,
            [Showing::Forest, Showing::Bindings, Showing::Bindings]
        );
    }

    #[test]
    fn a_forced_refresh_asks_for_a_collection_and_the_loop_reads_on() {
        let mut view = Recorder::default();
        let (ask, asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Key(control('r')),
            Event::Key(key(KeyCode::Char('j'))),
        ]);

        drive(&mut view, &events, &ask, Outstanding::waiting(PATIENCE)).expect("the loop runs");

        assert_eq!(asked.try_iter().collect::<Vec<_>>(), [Wanted::Everything]);
        assert_eq!(
            view.applied,
            [Action::Move(Motion::NextRow)],
            "the loop read on rather than waiting for the collection"
        );
    }

    /// The bead this came from: `^R` did nothing a reader could see. The
    /// keystroke asked for a collection and then evaluated to no change at
    /// all, so `view.draw` was never called and the screen stayed
    /// byte-identical for the seconds the collection took.
    #[test]
    fn the_refresh_key_puts_something_on_the_screen_before_the_snapshot_lands() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let events = waiting(vec![Event::Key(control('r'))]);

        drive(&mut view, &events, &ask, Outstanding::waiting(PATIENCE)).expect("the loop runs");

        assert_eq!(
            view.collecting(),
            [vec![Wanted::Everything]],
            "the view was told a collection began, and over which projects"
        );
        assert_eq!(view.drawn, 2, "the first frame, and one for the keystroke");
    }

    /// The timer and the inbound socket start collections nobody pressed a
    /// key for, which is the case the bead calls worse: rows appear and
    /// statuses flip under a reader with nothing to mark that they did.
    #[test]
    fn a_collection_nobody_asked_for_is_still_said_on_the_screen() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let events = waiting(vec![Event::Changed(atlas())]);

        drive(&mut view, &events, &ask, Outstanding::waiting(PATIENCE)).expect("the loop runs");

        assert_eq!(view.collecting(), [vec![atlas()]]);
        assert_eq!(view.drawn, 2);
    }

    // ---- the mark on a collecting project turns ---------------------------

    /// `bdi-7ao.43` drove the loop on a pty at `9cb221d`: with `bd` stubbed to
    /// hang, `bdi` wrote 64 bytes once, at +0.018s, and never again in 35
    /// seconds, while still answering keys in 21 ms. A collection reports
    /// nothing until it is done, so nothing was ever going to redraw the mark
    /// but a deadline of the loop's own.
    #[test]
    fn a_wait_gives_up_when_what_is_drawn_stops_being_true() {
        // An event well after the deadline, so a wait that kept none answers
        // with that instead of never answering at all: a loop that stopped
        // redrawing must fail here rather than hang.
        let (send, events) = mpsc::channel();
        thread::spawn(move || {
            thread::sleep(phrase::FRAME * 8);
            let _ = send.send(Event::Resize);
        });
        let began = Instant::now();

        let waited = wait(&events, Some(phrase::FRAME));

        assert!(matches!(waited, Some(Waited::Aged)));
        assert!(began.elapsed() >= phrase::FRAME, "{:?}", began.elapsed());
    }

    /// And only where something is going to go stale. A screen that says
    /// nothing time can falsify is woken by events alone: this returns the
    /// event rather than a deadline that would have come first.
    #[test]
    fn a_wait_on_a_screen_nothing_can_stale_sleeps_until_an_event() {
        let (send, events) = mpsc::channel();
        thread::spawn(move || {
            thread::sleep(phrase::FRAME * 3);
            let _ = send.send(Event::Resize);
        });

        assert!(matches!(
            wait(&events, None),
            Some(Waited::Event(Event::Resize))
        ));
    }

    /// The deadline running out is not an event: nothing has happened, so the
    /// loop draws the screen as it now is and asks the view nothing.
    #[test]
    fn a_frame_running_out_redraws_the_screen_and_nothing_else() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let (send, events) = mpsc::channel();
        send.send(Event::Changed(atlas()))
            .expect("the loop's end of the channel is open");
        // Long enough for several frames, and bounded so a loop that never
        // turned still ends rather than hanging the suite.
        thread::spawn(move || {
            thread::sleep(phrase::FRAME * 4);
            let _ = send.send(Event::Key(key(KeyCode::Char('q'))));
        });

        drive(&mut view, &events, &ask, Outstanding::waiting(PATIENCE)).expect("the loop runs");

        assert!(
            view.drawn > 2,
            "the first frame, the collection starting, and the mark turning: {}",
            view.drawn
        );
        assert_eq!(view.collecting(), [vec![atlas()]], "no second collection");
        assert_eq!(view.applied, [], "and no action for a frame running out");
    }

    /// A request that arrived mid-collection is sent the moment the one in
    /// flight comes back, from inside `came_back` rather than from an event.
    /// A view told only about the collections an event started would say the
    /// screen was resting while a second one ran.
    #[test]
    fn the_collection_that_starts_behind_another_is_said_too() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Changed(atlas()),
            Event::Changed(ferry()),
            Event::Collected(Box::new(a_snapshot())),
        ]);

        drive(&mut view, &events, &ask, Outstanding::waiting(PATIENCE)).expect("the loop runs");

        assert_eq!(
            view.collecting(),
            [vec![atlas()], vec![atlas(), ferry()], vec![ferry()]],
            "the one the event asked for, then it with ferry waiting behind \
             it, then ferry alone once the first came back"
        );
        assert_eq!(view.collected, 1);
    }

    /// The bead: nothing on the collection path measures the wait, so a
    /// tracker hung for an hour is drawn as one asked half a second ago. The
    /// loop is where the ask happens, so the loop is where it is stamped.
    #[test]
    fn a_collection_is_stamped_with_the_moment_it_was_asked_for() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let before = Utc::now();
        let events = waiting(vec![Event::Changed(atlas())]);

        drive(&mut view, &events, &ask, Outstanding::waiting(PATIENCE)).expect("the loop runs");

        let asked_at = view.asked_at();
        assert_eq!(asked_at.len(), 1, "one collection was started");
        assert!(
            (before..=Utc::now()).contains(&asked_at[0]),
            "the stamp is the moment of the ask: {before} .. {} .. {}",
            asked_at[0],
            Utc::now()
        );
    }

    /// The reason the stamp is taken at the ask and not inherited from
    /// whatever the project was last told about: a project reported for again
    /// while it is being read is a new wait, and one that carried the running
    /// collection's stamp would be drawn as having stopped answering the
    /// moment it began.
    #[test]
    fn a_project_reported_for_again_while_it_is_read_starts_a_wait_of_its_own() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Changed(atlas()),
            Event::Changed(atlas()),
            Event::Collected(Box::new(a_snapshot())),
        ]);

        drive(&mut view, &events, &ask, Outstanding::waiting(PATIENCE)).expect("the loop runs");

        assert_eq!(
            view.collecting(),
            [vec![atlas()], vec![atlas(), atlas()], vec![atlas()]],
            "the one in flight, then it with the second waiting, then the \
             second alone"
        );
        let told = &view.awaited[1];
        assert!(
            told[1].asked_at > told[0].asked_at,
            "the waiting one kept the running one's stamp: {told:?}"
        );
    }

    /// A collection coming back with nothing waiting behind it leaves the
    /// view resting. Saying otherwise would leave a mark turning on the
    /// project's line until the next refresh, over rows that had already
    /// arrived.
    ///
    /// The snapshot arriving is the end of the collection that produced it,
    /// and this is where that is said. The view is told what is in flight
    /// after every collection ends, so nothing it holds has to be unset by
    /// the arrival of rows.
    #[test]
    fn a_collection_with_nothing_behind_it_leaves_the_view_resting() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Changed(atlas()),
            Event::Collected(Box::new(a_snapshot())),
        ]);

        drive(&mut view, &events, &ask, Outstanding::waiting(PATIENCE)).expect("the loop runs");

        assert_eq!(
            view.collecting(),
            [vec![atlas()], vec![]],
            "the one the event asked for, and nothing once it landed"
        );
    }

    /// `bdi-7ao.81`: a request made while a collection is running is kept
    /// rather than sent, and the project it names has to say so. Nothing else
    /// on its line can — the rows under it are the last collection's and look
    /// exactly as they did, so a line that waited for the request to be sent
    /// would draw a resting mark over rows nothing is on its way to replace,
    /// for as long as the collection in front of it takes.
    #[test]
    fn a_request_waiting_its_turn_is_said_beside_the_one_in_flight() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let events = waiting(vec![Event::Changed(atlas()), Event::Changed(ferry())]);

        drive(&mut view, &events, &ask, Outstanding::waiting(PATIENCE)).expect("the loop runs");

        assert_eq!(
            view.collecting(),
            [vec![atlas()], vec![atlas(), ferry()]],
            "the one in flight, then it and the one behind it"
        );
        assert_eq!(view.drawn, 3, "and the screen changed for it");
    }

    #[test]
    fn only_one_collection_runs_at_a_time() {
        let mut view = Recorder::default();
        let (ask, asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Changed(atlas()),
            Event::Changed(ferry()),
            Event::Key(control('r')),
        ]);

        drive(&mut view, &events, &ask, Outstanding::waiting(PATIENCE)).expect("the loop runs");

        assert_eq!(
            asked.try_iter().collect::<Vec<_>>(),
            [atlas()],
            "the two behind it wait for the one in flight to come back"
        );
    }

    /// What a refresh naming a project costs: the collection in flight is no
    /// longer reading what every other request would ask for, so a request
    /// dropped while it runs is a change lost — which is the failure the
    /// inbound channel exists to prevent.
    #[test]
    fn a_change_that_arrived_mid_collection_is_asked_for_when_it_comes_back() {
        let mut view = Recorder::default();
        let (ask, asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Changed(atlas()),
            Event::Changed(ferry()),
            Event::Collected(Box::new(a_snapshot())),
        ]);

        drive(&mut view, &events, &ask, Outstanding::waiting(PATIENCE)).expect("the loop runs");

        assert_eq!(asked.try_iter().collect::<Vec<_>>(), [atlas(), ferry()]);
    }

    /// A project reported for again while it is being read is read again: the
    /// collection in flight may have passed it before the message arrived.
    #[test]
    fn a_project_reported_for_twice_is_read_again_rather_than_deduped() {
        let mut view = Recorder::default();
        let (ask, asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Changed(atlas()),
            Event::Changed(atlas()),
            Event::Collected(Box::new(a_snapshot())),
        ]);

        drive(&mut view, &events, &ask, Outstanding::waiting(PATIENCE)).expect("the loop runs");

        assert_eq!(asked.try_iter().collect::<Vec<_>>(), [atlas(), atlas()]);
    }

    /// Whatever waits behind a collection is bounded by the projects there
    /// are: a whole collection reads them all, so it stands in for every
    /// single project waiting with it.
    #[test]
    fn a_whole_collection_absorbs_the_projects_waiting_beside_it() {
        let mut view = Recorder::default();
        let (ask, asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Changed(atlas()),
            Event::Changed(ferry()),
            Event::Key(control('r')),
            Event::Changed(ferry()),
            Event::Collected(Box::new(a_snapshot())),
            Event::Collected(Box::new(a_snapshot())),
        ]);

        drive(&mut view, &events, &ask, Outstanding::waiting(PATIENCE)).expect("the loop runs");

        assert_eq!(
            asked.try_iter().collect::<Vec<_>>(),
            [atlas(), Wanted::Everything],
            "ferry was going to be read by the whole collection anyway"
        );
    }

    /// What an interval shorter than a collection costs, which is what the
    /// refresh interval is set against: the ticks that pass while a
    /// collection runs collapse into the one waiting behind it, so the work
    /// is one collection per collection rather than one per tick, and a
    /// timer set faster than the tracker can answer cannot pile up.
    #[test]
    fn the_intervals_passing_during_a_collection_cost_one_collection_between_them() {
        let mut view = Recorder::default();
        let (ask, asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Changed(Wanted::Everything),
            Event::Changed(Wanted::Everything),
            Event::Changed(Wanted::Everything),
            Event::Collected(Box::new(a_snapshot())),
            Event::Collected(Box::new(a_snapshot())),
        ]);

        drive(&mut view, &events, &ask, Outstanding::waiting(PATIENCE)).expect("the loop runs");

        assert_eq!(
            asked.try_iter().collect::<Vec<_>>(),
            [Wanted::Everything, Wanted::Everything],
            "three ticks over one collection asked for one more, not two"
        );
    }

    #[test]
    fn a_collection_that_comes_back_reaches_the_view() {
        let mut view = Recorder::default();
        let (ask, asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Changed(atlas()),
            Event::Collected(Box::new(a_snapshot())),
            Event::Changed(ferry()),
        ]);

        drive(&mut view, &events, &ask, Outstanding::waiting(PATIENCE)).expect("the loop runs");

        assert_eq!(view.collected, 1);
        assert_eq!(
            asked.try_iter().collect::<Vec<_>>(),
            [atlas(), ferry()],
            "the collection was over, so the second change asked for its own"
        );
    }

    #[test]
    fn a_collection_that_takes_its_time_does_not_hold_up_the_loop() {
        let (to_the_loop, events) = mpsc::channel();
        let (ask, asked) = mpsc::channel();
        let (release, held) = mpsc::channel::<()>();

        let collecting = to_the_loop.clone();
        let worker = thread::spawn(move || {
            collector(
                Box::new(move |_| {
                    let _ = held.recv();
                    a_snapshot()
                }),
                &asked,
                &collecting,
            );
        });

        // The user forces a refresh, then scrolls while it is outstanding.
        for event in [
            Event::Key(control('r')),
            Event::Key(key(KeyCode::Char('j'))),
            Event::Key(key(KeyCode::Char('q'))),
        ] {
            to_the_loop.send(event).expect("the loop is listening");
        }

        // The outstanding collection holds the channel open, so `q` reaching
        // `Quit` is the only thing that ends this loop. A thread of its own is
        // what lets the wait for that run out, rather than run on forever.
        let (finished, ended) = mpsc::channel();
        let driving = thread::spawn(move || {
            let mut view = Recorder::default();
            let outcome = drive(&mut view, &events, &ask, Outstanding::waiting(PATIENCE));
            let _ = finished.send(());
            (view, outcome)
        });
        ended
            .recv_timeout(A_MOMENT)
            .expect("the loop ended on q with the collection still outstanding");
        let (view, outcome) = driving.join().expect("the loop's thread ends");
        outcome.expect("the loop runs");

        assert_eq!(view.applied, [Action::Move(Motion::NextRow)]);
        assert_eq!(view.collected, 0, "the collection is still outstanding");

        drop(release);
        worker.join().expect("the collector ends with its channels");
    }

    #[test]
    fn a_loop_whose_events_run_out_ends() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();

        drive(
            &mut view,
            &waiting(Vec::new()),
            &ask,
            Outstanding::waiting(PATIENCE),
        )
        .expect("the loop runs");

        assert!(view.applied.is_empty());
    }

    /// The loop ends the way 'q' ends it, so the screen is dropped and the
    /// terminal put back. Nothing else in the loop can do that.
    ///
    /// The signal is waiting in the channel before the loop starts, which is
    /// also the case `run` produces: the signals are taken before the screen
    /// is opened, so one arriving in between is read as the loop's first
    /// event. The resize behind it is what makes a failure show — without it
    /// the channel runs dry and the loop ends whatever the arm does.
    #[test]
    fn a_signal_ends_the_loop() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();

        drive(
            &mut view,
            &waiting(vec![Event::Signalled, Event::Resize]),
            &ask,
            Outstanding::waiting(PATIENCE),
        )
        .expect("the loop runs");

        assert_eq!(
            view.drawn, 1,
            "the first draw and no other: the loop returned rather than \
             going on to the resize behind the signal"
        );
    }

    /// The bindings window swallows any key at all, which is why a signal is
    /// not one. A synthesised 'q' here would take the window away and leave
    /// `bdi` running.
    #[test]
    fn a_signal_ends_the_loop_with_the_bindings_up() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();

        drive(
            &mut view,
            &waiting(vec![
                Event::Key(key(KeyCode::Char('?'))),
                Event::Signalled,
                Event::Resize,
            ]),
            &ask,
            Outstanding::waiting(PATIENCE),
        )
        .expect("the loop runs");

        assert_eq!(
            view.drawn, 2,
            "the first draw and the bindings: the signal ended the run rather \
             than closing the window"
        );
    }

    #[test]
    fn a_resize_redraws_and_nothing_else() {
        let mut view = Recorder::default();
        let (ask, asked) = mpsc::channel();

        drive(
            &mut view,
            &waiting(vec![Event::Resize]),
            &ask,
            Outstanding::waiting(PATIENCE),
        )
        .expect("the loop runs");

        assert!(view.applied.is_empty());
        assert_eq!(asked.try_iter().count(), 0);
        assert_eq!(view.drawn, 2, "the first draw, and the resize");
    }

    /// The screen is drawn for what changed it. A key bound to nothing
    /// changed nothing.
    #[test]
    fn a_key_bound_to_nothing_does_not_redraw() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();

        drive(
            &mut view,
            &waiting(vec![Event::Key(key(KeyCode::Char('z')))]),
            &ask,
            Outstanding::waiting(PATIENCE),
        )
        .expect("the loop runs");

        assert_eq!(view.drawn, 1, "the first draw and no other");
    }

    // ---- the pointer ------------------------------------------------------

    #[test]
    fn a_click_reaches_the_view_as_the_row_it_landed_on() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();

        drive(
            &mut view,
            &waiting(vec![Event::Clicked(9)]),
            &ask,
            Outstanding::waiting(PATIENCE),
        )
        .expect("the loop runs");

        assert_eq!(view.clicked, [9]);
        assert!(view.applied.is_empty(), "a click asks for no action");
        assert_eq!(view.drawn, 2, "the first draw, and the click");
    }

    /// The screen is drawn for what changed it. A click on the tail, the key
    /// row or a blank row past the last line changed nothing.
    #[test]
    fn a_click_that_lands_on_no_row_does_not_redraw() {
        let mut view = Recorder {
            nothing_under_the_pointer: true,
            ..Recorder::default()
        };
        let (ask, _asked) = mpsc::channel();

        drive(
            &mut view,
            &waiting(vec![Event::Clicked(21)]),
            &ask,
            Outstanding::waiting(PATIENCE),
        )
        .expect("the loop runs");

        assert_eq!(view.clicked, [21]);
        assert_eq!(view.drawn, 1, "the first draw and no other");
    }

    /// `bdi` holds no scroll of its own — the window is a pure function of
    /// where the selection sits — so the wheel moves the selection, which is
    /// the only thing the window follows.
    #[test]
    fn a_wheel_notch_moves_the_selection_one_row() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();

        drive(
            &mut view,
            &waiting(vec![
                Event::Scrolled(Motion::PreviousRow),
                Event::Scrolled(Motion::NextRow),
            ]),
            &ask,
            Outstanding::waiting(PATIENCE),
        )
        .expect("the loop runs");

        assert_eq!(
            view.applied,
            [
                Action::Move(Motion::PreviousRow),
                Action::Move(Motion::NextRow)
            ]
        );
        assert!(view.clicked.is_empty(), "a wheel notch points at no row");
    }

    /// The bindings window sits over the forest, so while it is up the rows
    /// under the pointer are rows the reader cannot see. A click takes the
    /// window away and selects nothing, for the same reason any key does.
    #[test]
    fn a_click_over_the_bindings_window_closes_it_and_selects_nothing() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();

        drive(
            &mut view,
            &waiting(vec![
                Event::Key(key(KeyCode::Char('?'))),
                Event::Clicked(9),
                Event::Clicked(9),
            ]),
            &ask,
            Outstanding::waiting(PATIENCE),
        )
        .expect("the loop runs");

        assert_eq!(
            view.clicked,
            [9],
            "the first click took the window away; only the second reached the forest"
        );
        assert_eq!(
            view.showing,
            [
                Showing::Forest,
                Showing::Bindings,
                Showing::Forest,
                Showing::Forest
            ]
        );
    }

    #[test]
    fn a_wheel_notch_over_the_bindings_window_closes_it_and_moves_nothing() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();

        drive(
            &mut view,
            &waiting(vec![
                Event::Key(key(KeyCode::Char('?'))),
                Event::Scrolled(Motion::NextRow),
            ]),
            &ask,
            Outstanding::waiting(PATIENCE),
        )
        .expect("the loop runs");

        assert!(view.applied.is_empty());
        assert_eq!(
            view.showing,
            [Showing::Forest, Showing::Bindings, Showing::Forest]
        );
    }

    /// What waits behind a collection, asked of `Outstanding` directly.
    ///
    /// The loop takes its instants from the clock, and every property here is
    /// about one stamp against another — the deadline a queued read crosses
    /// is thirty seconds away and the refresh that re-reports it is thirty
    /// seconds apart, so a test that let real time supply the difference
    /// would be deciding on scheduling jitter rather than on the rule.
    mod what_waits {
        use super::*;
        use chrono::TimeZone;
        use pretty_assertions::assert_eq;

        /// `second` seconds into the run, which is how these are written: a
        /// wait is a difference between two of them and the wall clock they
        /// sit on says nothing.
        fn at(second: i64) -> chrono::DateTime<Utc> {
            Utc.with_ymd_and_hms(2026, 9, 1, 10, 0, 0).unwrap() + TimeDelta::seconds(second)
        }

        /// Asked at `at(0)` and never answered, so everything after it waits.
        fn hung_on(project: Wanted) -> (Outstanding, Sender<Wanted>, Receiver<Wanted>) {
            let (ask, asked) = mpsc::channel();
            let mut outstanding = Outstanding::waiting(PATIENCE);
            outstanding.ask(&ask, project, at(0));
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
            let (mut outstanding, ask, _asked) = hung_on(atlas());

            outstanding.ask(&ask, ferry(), at(5));

            assert_eq!(stamps(&outstanding), [(atlas(), at(0)), (ferry(), at(5))]);
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
            let (mut outstanding, ask, _asked) = hung_on(atlas());

            outstanding.ask(&ask, ferry(), at(5));
            outstanding.ask(&ask, ferry(), at(35));

            assert_eq!(stamps(&outstanding), [(atlas(), at(0)), (ferry(), at(5))]);
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
            let (mut outstanding, ask, _asked) = hung_on(atlas());

            outstanding.ask(&ask, ferry(), at(5));
            outstanding.ask(&ask, Wanted::Everything, at(40));

            assert_eq!(
                stamps(&outstanding),
                [(atlas(), at(0)), (Wanted::Everything, at(40))],
                "not :05, which would be every other project's wait too"
            );
        }

        /// Reaching the front is not being asked for. A read carries the wait
        /// it has had all along, so a project stranded ten minutes behind a
        /// hung tracker does not tell the reader its tracker was asked a
        /// moment ago the instant that tracker answers.
        #[test]
        fn a_read_that_reaches_the_front_keeps_the_stamp_it_queued_at() {
            let (mut outstanding, ask, _asked) = hung_on(atlas());
            outstanding.ask(&ask, ferry(), at(5));

            outstanding.came_back(&ask);

            assert_eq!(stamps(&outstanding), [(ferry(), at(5))]);
        }

        /// Every read outstanding, in the order they will be served, is what
        /// the collector is served from — so the first is the one it is
        /// working on and the rest follow in turn.
        #[test]
        fn the_reads_are_sent_in_the_order_they_were_asked_for() {
            let (mut outstanding, ask, asked) = hung_on(atlas());
            outstanding.ask(&ask, ferry(), at(5));
            outstanding.ask(&ask, Wanted::Project("harbour".to_string()), at(6));

            outstanding.came_back(&ask);
            outstanding.came_back(&ask);

            assert_eq!(
                asked.try_iter().collect::<Vec<_>>(),
                [atlas(), ferry(), Wanted::Project("harbour".to_string())]
            );
        }

        /// A collection nothing is waiting for is nothing to take: the loop
        /// answers whatever arrives on the one channel, in whatever order it
        /// arrives, and a snapshot with no ask behind it must leave it as it
        /// was rather than end the run.
        #[test]
        fn a_collection_nothing_asked_for_leaves_the_queue_alone() {
            let (ask, _asked) = mpsc::channel();
            let mut outstanding = Outstanding::waiting(PATIENCE);

            outstanding.came_back(&ask);

            assert_eq!(stamps(&outstanding), []);
        }
    }
}
