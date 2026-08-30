//! The terminal's lifecycle and the loop that keeps the view live.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use ratatui::crossterm::event::{self, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{DefaultTerminal, Frame};

use crate::collect::changes::{self, Reported, Socket};
use crate::collect::run::RealRunner;
use crate::model::snapshot::Snapshot;
use crate::view::forest::{self, Forest};
use crate::view::tail::{self, Herdr, Panes, Tail};
use crate::view::{draw, Action, Motion};

/// Draw the snapshot until the user quits, re-collecting on a refresh.
///
/// The first collection is made before the alternate screen opens, so the
/// wait happens where the user can still see their own terminal; every one
/// after it runs on a worker thread.
pub fn run(
    refresh: Duration,
    projects: Vec<String>,
    collect: Box<dyn Fn() -> Snapshot + Send>,
) -> anyhow::Result<()> {
    let first = collect();
    // Held, not discarded: the socket comes off the filesystem when this
    // returns, so the run that made it is the run that clears it away.
    let (events, ask, _socket) = wire(refresh, Reported::watching(projects), collect);
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

/// What the screen has on it.
///
/// The bindings take the whole screen rather than sitting over the forest, so
/// that a terminal with room for eight rows spends all eight on them. The loop
/// holds this rather than the view because it decides what a keystroke means,
/// and while the bindings are up every keystroke means "take them away".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Showing {
    Forest,
    Bindings,
}

/// The rows on the screen and what the user has done to them.
///
/// The seam the loop steers the view across: the loop knows the actions and
/// nothing about rows, and the view knows the rows and only the names of the
/// keys it is handed.
trait View {
    /// Show a snapshot just collected, in place of the one on the screen.
    fn collected(&mut self, snapshot: Snapshot);

    /// Apply one action, reporting whether the screen has changed.
    fn apply(&mut self, action: Action) -> bool;

    fn draw(&mut self, showing: Showing) -> anyhow::Result<()>;
}

/// Read events until the user quits.
///
/// Every wait in here is a wait on the one channel: a keystroke, a resize, a
/// project reporting a change and a collection coming back are the same kind
/// of thing to the loop, and none of them is a deadline it sleeps until.
fn drive(view: &mut dyn View, events: &Receiver<Event>, ask: &Sender<()>) -> anyhow::Result<()> {
    let mut showing = Showing::Forest;
    view.draw(showing)?;
    let mut collecting = false;

    while let Ok(event) = events.recv() {
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
            view.draw(showing)?;
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

/// One key a reader can press, and the word for it they can read.
///
/// `control` is a requirement and not an exclusion: a key that does not ask
/// for it answers whatever modifiers are held, which is what the arrows and
/// the letters have always done.
struct Key {
    code: KeyCode,
    control: bool,
    named: &'static str,
}

const fn alone(code: KeyCode, named: &'static str) -> Key {
    Key {
        code,
        control: false,
        named,
    }
}

const fn ctrl(code: char, named: &'static str) -> Key {
    Key {
        code: KeyCode::Char(code),
        control: true,
        named,
    }
}

/// One thing the view does: the keys that ask for it, and what to call it.
struct Binding {
    keys: &'static [Key],
    action: Action,
    /// What pressing it does, for the key bindings view.
    does: &'static str,
    /// Its word in the row under the tail, for the few that earn a permanent
    /// line there.
    hint: Option<&'static str>,
}

/// Every binding there is.
///
/// The mapping and the key bindings view are this table read two ways, so a
/// key is written down once and nothing on screen can disagree with what
/// pressing it does. `^C` is an alias for `q`: raw mode swallows it, and the
/// key everyone reaches for must not be inert.
///
/// The order is least guessable first, because a screen too short for the
/// whole table shows the top of it. A reader who cannot see the arrows will
/// press one anyway; one who cannot see `a` will not work out that the trees
/// they are missing are being filtered.
const BINDINGS: &[Binding] = &[
    Binding {
        keys: &[alone(KeyCode::Enter, "Enter")],
        action: Action::Focus,
        does: "focus the selected bead's pane in herdr",
        hint: Some("focus"),
    },
    Binding {
        keys: &[alone(KeyCode::Char(' '), "Space")],
        action: Action::ToggleFold,
        does: "fold or unfold the selected node",
        hint: None,
    },
    Binding {
        keys: &[alone(KeyCode::Char('a'), "a")],
        action: Action::ToggleFilter,
        does: "show every tree, not only those with a live agent",
        hint: Some("all"),
    },
    Binding {
        keys: &[alone(KeyCode::Char('?'), "?")],
        action: Action::ShowBindings,
        does: "show these key bindings",
        hint: Some("keys"),
    },
    Binding {
        keys: &[alone(KeyCode::Char('q'), "q"), ctrl('c', "^C")],
        action: Action::Quit,
        does: "quit",
        hint: Some("quit"),
    },
    Binding {
        keys: &[ctrl('r', "^R")],
        action: Action::Refresh,
        does: "collect from the trackers again now",
        hint: None,
    },
    Binding {
        keys: &[alone(KeyCode::Down, "Down"), alone(KeyCode::Char('j'), "j")],
        action: Action::Move(Motion::NextRow),
        does: "move down one row",
        hint: None,
    },
    Binding {
        keys: &[alone(KeyCode::Up, "Up"), alone(KeyCode::Char('k'), "k")],
        action: Action::Move(Motion::PreviousRow),
        does: "move up one row",
        hint: None,
    },
    Binding {
        keys: &[
            alone(KeyCode::Right, "Right"),
            alone(KeyCode::Char('l'), "l"),
        ],
        action: Action::ExpandOrChild,
        does: "expand, or move to the first child when it is already expanded",
        hint: None,
    },
    Binding {
        keys: &[alone(KeyCode::Left, "Left"), alone(KeyCode::Char('h'), "h")],
        action: Action::CollapseOrParent,
        does: "collapse, or move to the parent when it is already collapsed",
        hint: None,
    },
    Binding {
        keys: &[ctrl('d', "^D")],
        action: Action::Move(Motion::HalfScreenDown),
        does: "move down half a screen",
        hint: None,
    },
    Binding {
        keys: &[ctrl('u', "^U")],
        action: Action::Move(Motion::HalfScreenUp),
        does: "move up half a screen",
        hint: None,
    },
    Binding {
        keys: &[alone(KeyCode::Char('g'), "g")],
        action: Action::Move(Motion::FirstRow),
        does: "move to the first row",
        hint: None,
    },
    Binding {
        keys: &[alone(KeyCode::Char('G'), "G")],
        action: Action::Move(Motion::LastRow),
        does: "move to the last row",
        hint: None,
    },
];

/// The action a key asks for, or nothing where it is bound to none.
fn action(key: KeyEvent) -> Option<Action> {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);

    BINDINGS
        .iter()
        .find(|binding| {
            binding
                .keys
                .iter()
                .any(|bound| bound.code == key.code && (control || !bound.control))
        })
        .map(|binding| binding.action)
}

/// Every binding named for a reader: the keys to press, and what pressing
/// them does.
fn bindings() -> Vec<(String, &'static str)> {
    BINDINGS
        .iter()
        .map(|binding| {
            (
                binding
                    .keys
                    .iter()
                    .map(|key| key.named)
                    .collect::<Vec<_>>()
                    .join(", "),
                binding.does,
            )
        })
        .collect()
}

/// The row under the tail: the handful of bindings worth a permanent line,
/// each named by the first key that reaches it.
///
/// Naming a key costs more columns than a glyph did, and a row that outgrew a
/// forty-column terminal would lose its last words — which is `q quit`. So the
/// row keeps the keys a reader reaches for and leaves the rest to `?`, which
/// is the one it gains.
fn key_row() -> String {
    BINDINGS
        .iter()
        .filter_map(|binding| {
            let word = binding.hint?;
            Some(format!("{} {word}", binding.keys.first()?.named))
        })
        .collect::<Vec<_>>()
        .join("   ")
}

/// Start everything that produces events, and hand back the loop's ends: the
/// events themselves, the channel a collection is asked for on, and the
/// inbound socket for as long as there is a view to keep live.
fn wire(
    refresh: Duration,
    reported: Reported,
    collect: Box<dyn Fn() -> Snapshot + Send>,
) -> (Receiver<Event>, Sender<()>, Option<Socket>) {
    let (to_the_loop, events) = mpsc::channel();
    let (ask, asked) = mpsc::channel();

    let collecting = to_the_loop.clone();
    thread::spawn(move || collector(collect.as_ref(), &asked, &collecting));

    let typing = to_the_loop.clone();
    thread::spawn(move || keys(&typing));

    let (changed, changes) = mpsc::channel();
    // Opened before the alternate screen, so anything that stopped it is said
    // where the user's own terminal still has it when the view closes.
    let socket = match changes::listen(
        changes::where_writers_find_bdi(),
        &reported,
        changed.clone(),
    ) {
        Ok(socket) => Some(socket),
        Err(refused) => {
            eprintln!("bdi: {refused}");
            None
        }
    };

    let told = to_the_loop.clone();
    thread::spawn(move || {
        report(
            &mut Inbound {
                changes,
                _open: changed,
            },
            &told,
        );
    });

    thread::spawn(move || {
        report(
            &mut Timer {
                every: refresh,
                reported,
            },
            &to_the_loop,
        );
    });

    (events, ask, socket)
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

/// The source for the projects nothing else reports for: it says the work has
/// moved every interval, whether or not it has.
///
/// It stays quiet for as long as every project is being reported for over the
/// inbound channel, because a poll then has nothing to find that a message
/// has not already said. A project the channel stops covering is polled again
/// from the next interval, so a producer going away costs the view its speed
/// rather than its truth. The window is the interval itself: a project
/// reported for more recently than that is one the poll would have found
/// nothing on.
struct Timer {
    every: Duration,
    reported: Reported,
}

impl Changes for Timer {
    fn next(&mut self) {
        loop {
            thread::sleep(self.every);
            if !self.reported.covered(self.every) {
                return;
            }
        }
    }
}

/// The source for the projects something else reports for: it says the work
/// has moved when a writer has said which project it moved in.
struct Inbound {
    changes: Receiver<()>,
    /// Held so the channel never runs out of writers. A source whose last
    /// writer has gone must go quiet, not report as fast as it can.
    _open: Sender<()>,
}

impl Changes for Inbound {
    fn next(&mut self) {
        let _ = self.changes.recv();
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

/// The forest on the alternate screen and the tail beneath it.
///
/// Held apart from the terminal that draws it because the terminal needs a
/// tty and none of this does.
struct Shown {
    forest: Forest,
    panes: Box<dyn Panes>,
    tail: Tail,
    /// The pane the tail on screen was read from, so a selection moving
    /// within it does not spend a herdr call on the answer already drawn.
    tailing: Option<String>,
}

impl Shown {
    fn of(snapshot: Snapshot, panes: Box<dyn Panes>) -> Self {
        let forest = forest::flatten(&snapshot);
        let tail = tail::tail(&forest, panes.as_ref(), tail::LINES);
        let tailing = tail::target(&forest).pane().map(str::to_string);

        Self {
            forest,
            panes,
            tail,
            tailing,
        }
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

    fn collected(&mut self, snapshot: Snapshot) {
        // A refresh keeps the folds and the selection, so the cursor stays on
        // the bead the user put it on however the new snapshot has moved it.
        self.forest.refresh(&snapshot);
        // The refresh tick is when the pane is re-read: the rows it has drawn
        // since the last one are exactly what has moved on.
        self.retail();
    }

    fn apply(&mut self, action: Action) -> bool {
        if action == Action::Focus {
            return self.focus();
        }

        let changed = self.forest.apply(action);
        if changed {
            self.follow();
        }
        changed
    }
}

/// The alternate screen, and what is drawn on it.
///
/// The terminal is on the alternate screen and in raw mode for as long as
/// this lives, so dropping it puts the terminal back however the loop ended.
/// `ratatui::init` hooks panics as well, so a crash does not leave a wedged
/// tty behind either.
struct Screen {
    terminal: DefaultTerminal,
    shown: Shown,
}

impl Screen {
    fn showing(snapshot: Snapshot, panes: Box<dyn Panes>) -> anyhow::Result<Self> {
        let terminal = ratatui::try_init()?;

        Ok(Self {
            terminal,
            shown: Shown::of(snapshot, panes),
        })
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
    draw::draw(frame, frame.area(), forest, &key_row());
    draw::draw_tail(frame, bands.tail, tail);
}

impl Drop for Screen {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

impl View for Screen {
    fn collected(&mut self, snapshot: Snapshot) {
        self.shown.collected(snapshot);
    }

    fn apply(&mut self, action: Action) -> bool {
        self.shown.apply(action)
    }

    fn draw(&mut self, showing: Showing) -> anyhow::Result<()> {
        let (forest, tail) = (&mut self.shown.forest, &self.shown.tail);
        self.terminal.draw(|frame| match showing {
            Showing::Forest => paint(frame, forest, tail),
            Showing::Bindings => draw::key_bindings(frame, frame.area(), &bindings()),
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::run::RunFailure;
    use crate::model::join::BeadKey;
    use crate::model::snapshot::{
        self, Counts, Filter, HerdrState, Node, TrackerFailure, TrackerState, Tree,
    };
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
        showing: Vec<Showing>,
    }

    impl View for Recorder {
        fn collected(&mut self, _snapshot: Snapshot) {
            self.collected += 1;
        }

        fn apply(&mut self, action: Action) -> bool {
            self.applied.push(action);
            true
        }

        fn draw(&mut self, showing: Showing) -> anyhow::Result<()> {
            self.drawn += 1;
            self.showing.push(showing);
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

    /// Every action there is.
    ///
    /// The match is what makes it every one rather than every one anybody
    /// remembered: an action or a motion added to the enums makes it
    /// non-exhaustive, and the compiler names this function until the list
    /// above it has grown too.
    fn every_action() -> Vec<Action> {
        let every = vec![
            Action::Move(Motion::PreviousRow),
            Action::Move(Motion::NextRow),
            Action::Move(Motion::HalfScreenUp),
            Action::Move(Motion::HalfScreenDown),
            Action::Move(Motion::FirstRow),
            Action::Move(Motion::LastRow),
            Action::CollapseOrParent,
            Action::ExpandOrChild,
            Action::ToggleFold,
            Action::ToggleFilter,
            Action::Focus,
            Action::ShowBindings,
            Action::Refresh,
            Action::Quit,
        ];

        for action in &every {
            match action {
                Action::Move(motion) => match motion {
                    Motion::PreviousRow
                    | Motion::NextRow
                    | Motion::HalfScreenUp
                    | Motion::HalfScreenDown
                    | Motion::FirstRow
                    | Motion::LastRow => (),
                },
                Action::CollapseOrParent
                | Action::ExpandOrChild
                | Action::ToggleFold
                | Action::ToggleFilter
                | Action::Focus
                | Action::ShowBindings
                | Action::Refresh
                | Action::Quit => (),
            }
        }

        every
    }

    /// An action no key reaches is one nobody can ask for, and it would have
    /// no line in the key bindings view either.
    ///
    /// `Forest::apply` already fails the build on an action added to the enum
    /// and matched nowhere. This is the other half: one that compiles
    /// everywhere and is still unreachable.
    #[test]
    fn every_action_has_a_key_that_asks_for_it() {
        for action in every_action() {
            assert!(
                BINDINGS.iter().any(|binding| binding.action == action),
                "{action:?} is bound to no key"
            );
        }
    }

    /// The bead this view exists for: a binding added to the table and left
    /// out of the view is the failure being guarded against, so the view is
    /// asserted against the table rather than against a list beside it.
    #[test]
    fn the_key_bindings_view_names_every_binding_the_mapping_holds() {
        let rows = BINDINGS.len() as u16 + 1;
        let mut terminal = Terminal::new(TestBackend::new(100, rows)).expect("a test backend");
        terminal
            .draw(|frame| draw::key_bindings(frame, frame.area(), &bindings()))
            .expect("a draw into memory");
        let buffer = terminal.backend().buffer();
        let drawn: Vec<String> = (0..rows)
            .map(|y| (0..100).map(|x| buffer[(x, y)].symbol()).collect())
            .collect();

        for binding in BINDINGS {
            let named = binding
                .keys
                .iter()
                .map(|bound| bound.named)
                .collect::<Vec<_>>()
                .join(", ");
            assert!(
                drawn
                    .iter()
                    .any(|row| row.contains(&named) && row.contains(binding.does)),
                "{named} ({}) has no line in {drawn:#?}",
                binding.does
            );
        }
    }

    /// The bead's short terminal, decided rather than left to a cut: the way
    /// out, then as many bindings as fit from the top of the table, then a
    /// count of what is missing. Nothing is drawn with its bottom sheared off,
    /// and what survives is the half a reader could not have guessed.
    #[test]
    fn eight_rows_hold_the_way_out_the_keys_worth_most_and_a_count_of_the_rest() {
        let mut terminal = Terminal::new(TestBackend::new(80, 8)).expect("a test backend");
        terminal
            .draw(|frame| draw::key_bindings(frame, frame.area(), &bindings()))
            .expect("a draw into memory");
        let buffer = terminal.backend().buffer();
        let drawn: Vec<String> = (0..8)
            .map(|y| {
                (0..80)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect();

        assert_eq!(
            drawn,
            vec![
                "Key bindings · press any key to close",
                "  Enter     focus the selected bead's pane in herdr",
                "  Space     fold or unfold the selected node",
                "  a         show every tree, not only those with a live agent",
                "  ?         show these key bindings",
                "  q, ^C     quit",
                "  ^R        collect from the trackers again now",
                "  … 8 more bindings · no room on a screen this short",
            ]
        );
    }

    /// Graeme could not tell what `⏎` was, let alone press it. A key named
    /// with anything but the characters on a keyboard is that bug again.
    #[test]
    fn no_key_is_named_with_anything_a_keyboard_does_not_carry() {
        for binding in BINDINGS {
            for bound in binding.keys {
                assert!(!bound.named.is_empty(), "a key with no name");
                assert!(
                    bound
                        .named
                        .chars()
                        .all(|glyph| glyph.is_ascii_graphic() || glyph == ' '),
                    "{:?} is not a name anyone can press",
                    bound.named
                );
            }
        }
    }

    /// The row under the tail said `⏎` while the mapping said `Enter`, which
    /// is the whole of the bug. Both now come off the one table.
    #[test]
    fn the_row_under_the_tail_names_its_keys_as_the_mapping_does() {
        let row = key_row();

        for binding in BINDINGS {
            let Some(word) = binding.hint else { continue };
            let named = binding.keys.first().expect("a key").named;
            assert!(
                row.contains(&format!("{named} {word}")),
                "{named} {word} missing from {row:?}"
            );
        }
        assert!(row.contains("? keys"), "the way to the rest: {row:?}");
    }

    /// A binding added as a match arm rather than to the table would answer a
    /// key the view never mentions. Nothing is left that could do that.
    #[test]
    fn the_mapping_answers_no_key_the_table_does_not_name() {
        let named: Vec<&Key> = BINDINGS.iter().flat_map(|binding| binding.keys).collect();
        let swept = (' '..='~')
            .flat_map(|glyph| [key(KeyCode::Char(glyph)), control(glyph)])
            .chain([
                key(KeyCode::Up),
                key(KeyCode::Down),
                key(KeyCode::Left),
                key(KeyCode::Right),
                key(KeyCode::Enter),
                key(KeyCode::Tab),
                key(KeyCode::Esc),
                key(KeyCode::Backspace),
                key(KeyCode::Home),
                key(KeyCode::End),
            ]);

        for pressed in swept {
            let control = pressed.modifiers.contains(KeyModifiers::CONTROL);
            let expected = named
                .iter()
                .any(|bound| bound.code == pressed.code && (control || !bound.control));
            assert_eq!(
                action(pressed).is_some(),
                expected,
                "the table and the mapping disagree about {pressed:?}"
            );
        }
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
            (key(KeyCode::Char('?')), Action::ShowBindings),
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

        drive(&mut view, &events, &ask).expect("the loop runs");

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

        drive(&mut view, &events, &ask).expect("the loop runs");

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

        drive(&mut view, &events, &ask).expect("the loop runs");

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

    /// A project something is reporting for does not need asking: the poll
    /// would find only what the message has already said.
    #[test]
    fn a_polled_project_goes_quiet_while_something_reports_it() {
        let (to_the_loop, events) = mpsc::channel();
        let reported = Reported::watching(["atlas".to_string()]);

        let producing = reported.clone();
        thread::spawn(move || loop {
            producing.take("atlas");
            thread::sleep(Duration::from_millis(20));
        });
        thread::spawn(move || {
            report(
                &mut Timer {
                    every: Duration::from_secs(1),
                    reported,
                },
                &to_the_loop,
            );
        });

        assert!(
            events.recv_timeout(Duration::from_millis(1500)).is_err(),
            "the writer had said everything a poll would have found"
        );
    }

    /// The signal that a live source has gone quiet: the project is polled
    /// again, so the view degrades to slow rather than to stale.
    #[test]
    fn a_project_the_channel_stops_covering_is_polled_again() {
        let (to_the_loop, events) = mpsc::channel();
        let reported = Reported::watching(["atlas".to_string()]);
        reported.take("atlas");

        thread::spawn(move || {
            report(
                &mut Timer {
                    every: Duration::from_millis(20),
                    reported,
                },
                &to_the_loop,
            );
        });

        assert!(matches!(events.recv_timeout(A_MOMENT), Ok(Event::Changed)));
    }

    /// A channel whose writers have all gone must go quiet. A source that
    /// returned from `next` the moment it had nothing left would report in a
    /// loop and collect without pause.
    #[test]
    fn an_inbound_channel_with_no_writers_left_goes_quiet() {
        let (to_the_loop, events) = mpsc::channel();
        let (changed, changes) = mpsc::channel();
        thread::spawn(move || {
            report(
                &mut Inbound {
                    changes,
                    _open: changed,
                },
                &to_the_loop,
            );
        });

        assert!(events.recv_timeout(Duration::from_millis(100)).is_err());
    }

    #[test]
    fn a_polled_project_is_reported_every_interval() {
        let (to_the_loop, events) = mpsc::channel();
        thread::spawn(move || {
            report(
                &mut Timer {
                    every: Duration::from_millis(20),
                    reported: Reported::default(),
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
                    reported: Reported::default(),
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
        a_grove_of((1..=beads).collect())
    }

    /// The same grove, with its children in the order a tracker would report
    /// them after every one of them changed priority.
    fn a_grove_reordered(beads: usize) -> Snapshot {
        a_grove_of((1..=beads).rev().collect())
    }

    fn a_grove_of(children: Vec<usize>) -> Snapshot {
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
        nodes.extend(children.iter().map(|n| bead(format!("grv-1.{n}"), 1)));

        let tree = Tree {
            project: "grove".to_string(),
            root: "grv-1".to_string(),
            title: "a tree with a great many beads".to_string(),
            counts: Counts {
                total: children.len(),
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

    /// herdr, for a forest whose beads carry no pane. Nothing here asks it
    /// anything; the tail needs one to exist, not to answer.
    struct NoPanes;

    impl Panes for NoPanes {
        fn read(&self, _pane: &str, _lines: u16) -> Result<Vec<String>, RunFailure> {
            Ok(Vec::new())
        }

        fn focus(&self, _pane: &str) -> Result<(), RunFailure> {
            Ok(())
        }
    }

    fn shown(snapshot: Snapshot) -> Shown {
        Shown::of(snapshot, Box::new(NoPanes))
    }

    fn bead(project: &str, id: &str) -> BeadKey {
        BeadKey {
            project: project.to_string(),
            id: id.to_string(),
        }
    }

    /// The band as drawn, not the selected index: a selection restored into a
    /// viewport that scrolled back to the top is the same bug wearing a
    /// different hat, and only the rows show the difference.
    fn forest_band(shown: &mut Shown, width: u16, height: u16) -> Vec<String> {
        let bands = draw::regions(Rect::new(0, 0, width, height));
        let rows = painted(&mut shown.forest, &shown.tail, width, height);
        rows[..bands.forest.height as usize].to_vec()
    }

    #[test]
    fn a_refresh_that_changes_nothing_leaves_the_forest_exactly_as_it_was() {
        let grove = a_grove(30);
        let mut shown = shown(grove.clone());
        forest_band(&mut shown, 60, 24);
        shown.apply(Action::Move(Motion::LastRow));

        let before = forest_band(&mut shown, 60, 24);
        shown.collected(grove);

        assert_eq!(forest_band(&mut shown, 60, 24), before);
    }

    #[test]
    fn a_refresh_that_reorders_around_the_selection_keeps_it_on_its_bead() {
        let mut shown = shown(a_grove(6));
        shown.apply(Action::Move(Motion::LastRow));
        let was = shown.forest.selected_line();

        assert_eq!(shown.forest.selected(), Some(&bead("grove", "grv-1.6")));

        shown.collected(a_grove_reordered(6));

        assert_eq!(shown.forest.selected(), Some(&bead("grove", "grv-1.6")));
        assert_ne!(shown.forest.selected_line(), was);
    }

    /// A quiet grove the live-agent filter hides, drawn ahead of a tree it
    /// keeps, so dropping the filter moves the surviving tree down the
    /// forest rather than leaving it where it already was.
    fn a_hidden_grove_above_a_shown_tree() -> Snapshot {
        let grove = a_grove(6).trees[0].clone();
        let atlas = a_snapshot().trees[0].clone();
        let both = vec![grove, atlas];

        snapshot::refilter(
            &Snapshot {
                collected: both,
                ..a_snapshot_of(Vec::new())
            },
            Filter::LiveAgents,
        )
    }

    #[test]
    fn dropping_the_filter_keeps_the_cursor_on_its_bead() {
        let mut shown = shown(a_hidden_grove_above_a_shown_tree());
        let was = shown.forest.selected_line();

        assert_eq!(shown.forest.selected(), Some(&bead("atlas", "a-1")));

        assert!(shown.apply(Action::ToggleFilter));

        assert_eq!(shown.forest.snapshot().filter, Filter::All);
        assert_eq!(shown.forest.selected(), Some(&bead("atlas", "a-1")));
        assert_ne!(
            shown.forest.selected_line(),
            was,
            "the grove came back above it"
        );
    }

    #[test]
    fn a_refresh_that_changes_nothing_keeps_the_folds_set_by_hand() {
        let grove = a_grove(6);
        let mut shown = shown(grove.clone());
        shown.apply(Action::ToggleFold);
        let folded = forest_band(&mut shown, 60, 24);

        shown.collected(grove);

        assert_eq!(forest_band(&mut shown, 60, 24), folded);
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
