//! The terminal's lifecycle and the loop that keeps the view live.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use ratatui::crossterm::event::{self, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{DefaultTerminal, Frame};

use crate::collect::run::RealRunner;
use crate::model::snapshot::{self, Filter, Snapshot};
use crate::view::forest::{self, Forest};
use crate::view::tail::{self, Herdr, Panes, Tail};
use crate::view::{draw, Action, Motion};

/// Draw the snapshot until the user quits, re-collecting on a refresh.
///
/// The first collection is made before the alternate screen opens, so the
/// wait happens where the user can still see their own terminal; every one
/// after it runs on a worker thread.
pub fn run(refresh: Duration, collect: Box<dyn Fn() -> Snapshot + Send>) -> anyhow::Result<()> {
    let first = collect();
    let (events, ask) = wire(refresh, collect);
    let mut screen = Screen::showing(first, Box::new(Herdr::new(RealRunner)))?;

    drive(&mut screen, &events, &ask)
}

/// Everything that reaches the loop.
///
/// A `Snapshot` is large and the other three carry almost nothing, so the
/// collected one is boxed rather than widening every event to its size.
enum Event {
    Key(KeyEvent),
    Resize,
    /// A project's work has moved on.
    Changed,
    /// A collection has come back.
    Collected(Box<Snapshot>),
}

/// The rows on the screen and what the user has done to them.
///
/// The seam the loop steers the view across: the loop knows the actions and
/// nothing about rows, and the view knows the rows and nothing about keys.
trait View {
    /// Show a snapshot just collected, in place of the one on the screen.
    fn collected(&mut self, snapshot: Snapshot);

    /// Apply one action, reporting whether the screen has changed.
    fn apply(&mut self, action: Action) -> bool;

    fn draw(&mut self) -> anyhow::Result<()>;
}

/// Read events until the user quits.
///
/// Every wait in here is a wait on the one channel: a keystroke, a resize, a
/// project reporting a change and a collection coming back are the same kind
/// of thing to the loop, and none of them is a deadline it sleeps until.
fn drive(view: &mut dyn View, events: &Receiver<Event>, ask: &Sender<()>) -> anyhow::Result<()> {
    view.draw()?;
    let mut collecting = false;

    while let Ok(event) = events.recv() {
        let changed = match event {
            Event::Key(key) => match action(key) {
                Some(Action::Quit) => return Ok(()),
                Some(Action::Refresh) => {
                    collecting = request(ask, collecting);
                    false
                }
                Some(action) => view.apply(action),
                None => false,
            },
            Event::Resize => true,
            Event::Changed => {
                collecting = request(ask, collecting);
                false
            }
            Event::Collected(snapshot) => {
                collecting = false;
                view.collected(*snapshot);
                true
            }
        };

        if changed {
            view.draw()?;
        }
    }

    Ok(())
}

/// Ask for a collection, reporting whether one is now running.
///
/// A request made while one is in flight is dropped: the collection already
/// running is reading exactly what this one would ask for.
fn request(ask: &Sender<()>, collecting: bool) -> bool {
    collecting || ask.send(()).is_ok()
}

/// The action a key asks for, or nothing where it is bound to none.
///
/// The bindings are vim-like, with the arrows, `space`, `⏎`, `a`, `^R` and
/// `q` alongside them. `^C` is an alias for `q`: raw mode swallows it, and
/// the key everyone reaches for must not be inert.
fn action(key: KeyEvent) -> Option<Action> {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => Some(Action::Move(Motion::NextRow)),
        KeyCode::Char('k') | KeyCode::Up => Some(Action::Move(Motion::PreviousRow)),
        KeyCode::Char('h') | KeyCode::Left => Some(Action::CollapseOrParent),
        KeyCode::Char('l') | KeyCode::Right => Some(Action::ExpandOrChild),
        KeyCode::Char('g') => Some(Action::Move(Motion::FirstRow)),
        KeyCode::Char('G') => Some(Action::Move(Motion::LastRow)),
        KeyCode::Char('d') if control => Some(Action::Move(Motion::HalfScreenDown)),
        KeyCode::Char('u') if control => Some(Action::Move(Motion::HalfScreenUp)),
        KeyCode::Char(' ') => Some(Action::ToggleFold),
        KeyCode::Enter => Some(Action::Focus),
        KeyCode::Char('a') => Some(Action::ToggleFilter),
        KeyCode::Char('r') if control => Some(Action::Refresh),
        KeyCode::Char('c') if control => Some(Action::Quit),
        KeyCode::Char('q') => Some(Action::Quit),
        _ => None,
    }
}

/// Start everything that produces events, and hand back the loop's ends: the
/// events themselves, and the channel a collection is asked for on.
fn wire(
    refresh: Duration,
    collect: Box<dyn Fn() -> Snapshot + Send>,
) -> (Receiver<Event>, Sender<()>) {
    let (to_the_loop, events) = mpsc::channel();
    let (ask, asked) = mpsc::channel();

    let collecting = to_the_loop.clone();
    thread::spawn(move || collector(collect.as_ref(), &asked, &collecting));

    let typing = to_the_loop.clone();
    thread::spawn(move || keys(&typing));

    thread::spawn(move || report(&mut Timer { every: refresh }, &to_the_loop));

    (events, ask)
}

/// What tells `bdi` that a project's work has moved on.
///
/// A tracker that can report its own changes is subscribed to; one that
/// cannot is timed, and a timer is only a duller way of being told. Each
/// source runs on its own thread and blocks there, so the loop never sleeps
/// until a deadline of its own.
trait Changes: Send {
    /// Block until there is something to collect for.
    fn next(&mut self);
}

/// The source for a project nothing else reports for: it says the work has
/// moved every interval, whether or not it has.
#[derive(Clone, Copy)]
struct Timer {
    every: Duration,
}

impl Changes for Timer {
    fn next(&mut self) {
        thread::sleep(self.every);
    }
}

/// Report one project's changes until the loop stops listening.
fn report(source: &mut dyn Changes, to: &Sender<Event>) {
    loop {
        source.next();
        if to.send(Event::Changed).is_err() {
            return;
        }
    }
}

/// Collect on demand, off the UI thread.
///
/// One collection is dozens of remote round trips per project, and the view
/// has to stay under the user's hands throughout.
fn collector(collect: &dyn Fn() -> Snapshot, asked: &Receiver<()>, to: &Sender<Event>) {
    while asked.recv().is_ok() {
        if to.send(Event::Collected(Box::new(collect()))).is_err() {
            return;
        }
    }
}

/// Read the terminal until it has nothing more to say.
///
/// A key that is only being released is not a keystroke; on terminals that
/// report releases at all, taking both would act on every binding twice.
fn keys(to: &Sender<Event>) {
    while let Ok(read) = event::read() {
        let event = match read {
            event::Event::Key(key) if key.kind == KeyEventKind::Press => Event::Key(key),
            event::Event::Resize(..) => Event::Resize,
            _ => continue,
        };
        if to.send(event).is_err() {
            return;
        }
    }
}

/// The forest on the alternate screen, and the snapshot it was flattened
/// from.
///
/// The terminal is on the alternate screen and in raw mode for as long as
/// this lives, so dropping it puts the terminal back however the loop ended.
/// `ratatui::init` hooks panics as well, so a crash does not leave a wedged
/// tty behind either.
struct Screen {
    terminal: DefaultTerminal,
    snapshot: Snapshot,
    forest: Forest,
    filter: Filter,
    panes: Box<dyn Panes>,
    tail: Tail,
    /// The pane the tail on screen was read from, so a selection moving
    /// within it does not spend a herdr call on the answer already drawn.
    tailing: Option<String>,
}

impl Screen {
    fn showing(snapshot: Snapshot, panes: Box<dyn Panes>) -> anyhow::Result<Self> {
        let terminal = ratatui::try_init()?;
        let forest = forest::flatten(&snapshot);
        let tail = tail::tail(&forest, panes.as_ref(), tail::LINES);
        let tailing = tail::target(&forest).pane().map(str::to_string);

        Ok(Self {
            terminal,
            filter: snapshot.filter,
            snapshot,
            forest,
            panes,
            tail,
            tailing,
        })
    }

    /// Read the tail for whatever the selection is on now.
    fn retail(&mut self) {
        self.tailing = tail::target(&self.forest).pane().map(str::to_string);
        self.tail = tail::tail(&self.forest, self.panes.as_ref(), tail::LINES);
    }

    /// Follow the selection, where it has left the pane the tail is showing.
    fn follow(&mut self) {
        if tail::moved_on(&self.forest, self.tailing.as_deref()) {
            self.retail();
        }
    }

    /// Bring the selected bead's pane to the front, reporting whether that
    /// changed the screen. A row with no pane is a no-op: there is nothing to
    /// focus and nothing has gone wrong.
    fn focus(&mut self) -> bool {
        match tail::focus(&self.forest, self.panes.as_ref()) {
            None => false,
            Some(said) => {
                self.tail = said;
                true
            }
        }
    }
}

/// One frame: the forest, the tail beneath it, and the height the forest is
/// told it has.
///
/// Outside the `terminal.draw` closure so a test backend can drive the whole
/// frame. This is the only place the three bands are agreed on, and `^D` and
/// `^U` are the part of that agreement nothing on screen would show was
/// broken.
fn paint(frame: &mut Frame, forest: &mut Forest, tail: &Tail) {
    let bands = draw::regions(frame.area());
    forest.set_half_screen(draw::half_screen(bands.forest));
    draw::draw(frame, frame.area(), forest);
    draw::draw_tail(frame, bands.tail, tail);
}

impl Drop for Screen {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

impl View for Screen {
    fn collected(&mut self, snapshot: Snapshot) {
        self.snapshot = snapshot;
        self.forest = forest::flatten(&self.snapshot);
        // The refresh tick is when the pane is re-read: the rows it has drawn
        // since the last one are exactly what has moved on.
        self.retail();
    }

    fn apply(&mut self, action: Action) -> bool {
        if action == Action::Focus {
            return self.focus();
        }

        // Which trees show is a display choice over what was collected, so
        // the filter re-reads the snapshot in hand rather than the trackers.
        if action == Action::ToggleFilter {
            self.filter = match self.filter {
                Filter::LiveAgents => Filter::All,
                Filter::All => Filter::LiveAgents,
            };
            self.snapshot = snapshot::refilter(&self.snapshot, self.filter);
            self.forest = forest::flatten(&self.snapshot);
            self.follow();
            return true;
        }

        let changed = self.forest.apply(action);
        if changed {
            self.follow();
        }
        changed
    }

    fn draw(&mut self) -> anyhow::Result<()> {
        let (forest, tail) = (&mut self.forest, &self.tail);
        self.terminal.draw(|frame| paint(frame, forest, tail))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::snapshot::{Counts, HerdrState, Node, TrackerFailure, TrackerState, Tree};
    use crate::model::types::Status;
    use crate::view::Motion;
    use chrono::Utc;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use ratatui::Terminal;

    /// Long enough that a thread which was going to report has, and short
    /// enough that a test waiting in vain is not a hang.
    const A_MOMENT: Duration = Duration::from_secs(5);

    /// A view that remembers what the loop did to it.
    #[derive(Default)]
    struct Recorder {
        applied: Vec<Action>,
        collected: usize,
        drawn: usize,
    }

    impl View for Recorder {
        fn collected(&mut self, _snapshot: Snapshot) {
            self.collected += 1;
        }

        fn apply(&mut self, action: Action) -> bool {
            self.applied.push(action);
            true
        }

        fn draw(&mut self) -> anyhow::Result<()> {
            self.drawn += 1;
            Ok(())
        }
    }

    /// A source that reports only when the test says so.
    struct OnCue(Receiver<()>);

    impl Changes for OnCue {
        fn next(&mut self) {
            // A test that has finished with this source drops the cue, and
            // the thread's next send fails and ends it.
            let _ = self.0.recv();
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn control(code: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(code), KeyModifiers::CONTROL)
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

    /// A snapshot of one unremarkable tree. Nothing the loop does depends on
    /// what is in one, only on when it arrives.
    fn a_snapshot() -> Snapshot {
        let tree = Tree {
            project: "atlas".to_string(),
            root: "a-1".to_string(),
            title: "the only tree there is".to_string(),
            ..Tree::tracker_unreachable("atlas", "a-1", TrackerFailure::Unavailable)
        };

        Snapshot {
            generated_at: Utc::now(),
            herdr: HerdrState::Ok,
            filter: Filter::LiveAgents,
            trees: vec![tree.clone()],
            hidden_trees: Vec::new(),
            failed_projects: Vec::new(),
            unattributed: Vec::new(),
            conflicts: Vec::new(),
            collected: vec![tree],
        }
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

        drive(&mut view, &events, &ask).expect("the loop runs");

        assert_eq!(
            view.applied,
            [Action::Move(Motion::NextRow), Action::ToggleFold],
            "q ends the loop, so nothing after it is applied"
        );
    }

    #[test]
    fn every_binding_reaches_the_action_it_names() {
        let bound = [
            (key(KeyCode::Char('j')), Action::Move(Motion::NextRow)),
            (key(KeyCode::Down), Action::Move(Motion::NextRow)),
            (key(KeyCode::Char('k')), Action::Move(Motion::PreviousRow)),
            (key(KeyCode::Up), Action::Move(Motion::PreviousRow)),
            (key(KeyCode::Char('h')), Action::CollapseOrParent),
            (key(KeyCode::Left), Action::CollapseOrParent),
            (key(KeyCode::Char('l')), Action::ExpandOrChild),
            (key(KeyCode::Right), Action::ExpandOrChild),
            (key(KeyCode::Char('g')), Action::Move(Motion::FirstRow)),
            (key(KeyCode::Char('G')), Action::Move(Motion::LastRow)),
            (control('d'), Action::Move(Motion::HalfScreenDown)),
            (control('u'), Action::Move(Motion::HalfScreenUp)),
            (key(KeyCode::Char(' ')), Action::ToggleFold),
            (key(KeyCode::Enter), Action::Focus),
            (key(KeyCode::Char('a')), Action::ToggleFilter),
            (control('r'), Action::Refresh),
            (key(KeyCode::Char('q')), Action::Quit),
            (control('c'), Action::Quit),
        ];

        for (pressed, expected) in bound {
            assert_eq!(action(pressed), Some(expected), "for {pressed:?}");
        }
    }

    /// The letters that carry a binding only under control carry none on
    /// their own, and a key nothing is bound to asks for nothing.
    #[test]
    fn a_key_bound_to_nothing_asks_for_nothing() {
        for pressed in [
            key(KeyCode::Char('d')),
            key(KeyCode::Char('u')),
            key(KeyCode::Char('r')),
            key(KeyCode::Char('c')),
            key(KeyCode::Char('z')),
            key(KeyCode::Tab),
        ] {
            assert_eq!(action(pressed), None, "for {pressed:?}");
        }
    }

    #[test]
    fn a_forced_refresh_asks_for_a_collection_and_the_loop_reads_on() {
        let mut view = Recorder::default();
        let (ask, asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Key(control('r')),
            Event::Key(key(KeyCode::Char('j'))),
        ]);

        drive(&mut view, &events, &ask).expect("the loop runs");

        assert_eq!(asked.try_iter().count(), 1);
        assert_eq!(
            view.applied,
            [Action::Move(Motion::NextRow)],
            "the loop read on rather than waiting for the collection"
        );
    }

    #[test]
    fn a_change_arriving_while_one_is_in_flight_does_not_stack_another() {
        let mut view = Recorder::default();
        let (ask, asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Changed,
            Event::Changed,
            Event::Key(control('r')),
        ]);

        drive(&mut view, &events, &ask).expect("the loop runs");

        assert_eq!(
            asked.try_iter().count(),
            1,
            "the collection in flight is already reading what these would ask for"
        );
    }

    #[test]
    fn a_collection_that_comes_back_reaches_the_view() {
        let mut view = Recorder::default();
        let (ask, asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Changed,
            Event::Collected(Box::new(a_snapshot())),
            Event::Changed,
        ]);

        drive(&mut view, &events, &ask).expect("the loop runs");

        assert_eq!(view.collected, 1);
        assert_eq!(
            asked.try_iter().count(),
            2,
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
                &move || {
                    let _ = held.recv();
                    a_snapshot()
                },
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

        let mut view = Recorder::default();
        drive(&mut view, &events, &ask).expect("the loop runs");

        assert_eq!(view.applied, [Action::Move(Motion::NextRow)]);
        assert_eq!(view.collected, 0, "the collection is still outstanding");

        drop(release);
        drop(ask);
        worker.join().expect("the collector ends with its channels");
    }

    #[test]
    fn a_project_is_reported_by_whatever_reports_its_changes() {
        let (to_the_loop, events) = mpsc::channel();
        let (cue, cued) = mpsc::channel();
        thread::spawn(move || report(&mut OnCue(cued), &to_the_loop));

        assert!(
            events.recv_timeout(Duration::from_millis(100)).is_err(),
            "nothing was reported until the source said so"
        );

        cue.send(()).expect("the source is listening");

        assert!(matches!(events.recv_timeout(A_MOMENT), Ok(Event::Changed)));
    }

    #[test]
    fn a_timed_project_is_reported_every_interval() {
        let (to_the_loop, events) = mpsc::channel();
        thread::spawn(move || {
            report(
                &mut Timer {
                    every: Duration::from_millis(20),
                },
                &to_the_loop,
            );
        });

        for reported in 1..=2 {
            assert!(
                matches!(events.recv_timeout(A_MOMENT), Ok(Event::Changed)),
                "the timer stopped after {reported} report(s)"
            );
        }
    }

    /// The loop's threads are the loop's: each ends when the loop stops
    /// listening, rather than outliving the screen it was drawing for.
    #[test]
    fn a_reporter_ends_when_the_loop_stops_listening() {
        let (to_the_loop, events) = mpsc::channel();
        let reporter = thread::spawn(move || {
            report(
                &mut Timer {
                    every: Duration::from_millis(1),
                },
                &to_the_loop,
            );
        });

        drop(events);

        assert!(reporter.join().is_ok());
    }

    #[test]
    fn a_loop_whose_events_run_out_ends() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();

        drive(&mut view, &waiting(Vec::new()), &ask).expect("the loop runs");

        assert!(view.applied.is_empty());
    }

    #[test]
    fn a_resize_redraws_and_nothing_else() {
        let mut view = Recorder::default();
        let (ask, asked) = mpsc::channel();

        drive(&mut view, &waiting(vec![Event::Resize]), &ask).expect("the loop runs");

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
        )
        .expect("the loop runs");

        assert_eq!(view.drawn, 1, "the first draw and no other");
    }

    /// A tree with enough beads under it that a half-screen motion has room
    /// to land somewhere that says how far it moved.
    fn a_grove(beads: usize) -> Snapshot {
        let bead = |id: String, depth: u16| Node {
            id,
            title: "a bead in the grove".to_string(),
            status: Status::InProgress,
            issue_type: "task".to_string(),
            priority: 2,
            depth,
            edge: None,
            ready: true,
            blocked_by: Vec::new(),
            started_at: None,
            closed_at: None,
            badges: Vec::new(),
            agent: None,
            anomalies: Vec::new(),
            truncated: false,
        };

        let mut nodes = vec![bead("grv-1".to_string(), 0)];
        nodes.extend((1..=beads).map(|n| bead(format!("grv-1.{n}"), 1)));

        let tree = Tree {
            project: "grove".to_string(),
            root: "grv-1".to_string(),
            title: "a tree with a great many beads".to_string(),
            counts: Counts {
                total: beads,
                closed: 0,
                live_agents: 0,
                anomalies: 0,
            },
            tracker: TrackerState::Ok,
            nodes,
            dangling: Vec::new(),
            unreachable: Vec::new(),
        };

        Snapshot {
            filter: Filter::All,
            trees: vec![tree.clone()],
            collected: vec![tree],
            ..a_snapshot_of(Vec::new())
        }
    }

    fn a_snapshot_of(trees: Vec<Tree>) -> Snapshot {
        Snapshot {
            generated_at: Utc::now(),
            herdr: HerdrState::Ok,
            filter: Filter::LiveAgents,
            trees: trees.clone(),
            hidden_trees: Vec::new(),
            failed_projects: Vec::new(),
            unattributed: Vec::new(),
            conflicts: Vec::new(),
            collected: trees,
        }
    }

    fn painted(forest: &mut Forest, tail: &Tail, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test backend");
        terminal
            .draw(|frame| paint(frame, forest, tail))
            .expect("a draw into memory");
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
            .collect()
    }

    /// Nothing on screen shows that `^D` moved by the wrong amount, so the
    /// frame is what says the forest was told how tall it is.
    #[test]
    fn a_frame_tells_the_forest_how_far_a_half_screen_is() {
        let mut forest = forest::flatten(&a_grove(30));

        forest.apply(Action::Move(Motion::HalfScreenDown));
        assert_eq!(
            forest.selected_line(),
            10,
            "the forest's own default, until a frame has been drawn"
        );

        let mut forest = forest::flatten(&a_grove(30));
        painted(&mut forest, &Tail::Silent("nothing to tail"), 60, 24);
        forest.apply(Action::Move(Motion::HalfScreenDown));

        assert_eq!(
            forest.selected_line(),
            8,
            "half of the sixteen rows the forest was given, not half the frame"
        );
    }

    #[test]
    fn a_frame_puts_the_tail_in_the_band_reserved_for_it() {
        let mut forest = forest::flatten(&a_grove(30));
        let tail = Tail::Pane {
            pane: "w:p1".to_string(),
            lines: vec!["rebuilt .#thinkpad".to_string()],
        };

        let rows = painted(&mut forest, &tail, 40, 12);
        let bands = draw::regions(Rect::new(0, 0, 40, 12));

        assert!(
            rows[bands.tail.y as usize].contains("w:p1"),
            "the rule naming the pane opens the tail's band: {rows:?}"
        );
        assert!(rows[bands.tail.y as usize + 1].starts_with("  rebuilt .#thinkpad"));
        assert!(
            rows[bands.tail.y as usize - 1].contains("a bead in the grove"),
            "the row above the tail is still the forest's"
        );
        assert!(rows[bands.keys.y as usize].contains("q quit"));
    }
}
