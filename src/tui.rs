//! The terminal's lifecycle and the loop that keeps the view live.

use std::collections::{BTreeSet, VecDeque};
use std::io;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    MouseButton, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::{DefaultTerminal, Frame};

use crate::app::Wanted;
use crate::collect::changes::{self, Reported, Socket, Uncovered};
use crate::collect::panes::{Herdr, Panes};
use crate::collect::run::RealRunner;
use crate::model::snapshot::Snapshot;
use crate::view::bindings::key_bindings;
use crate::view::forest::{self, Forest};
use crate::view::tail::{self, Tail};
use crate::view::{draw, Action, Motion, Notice};

/// Draw the snapshot until the user quits, re-collecting on a refresh.
///
/// The first collection is made before the alternate screen opens, so the
/// wait happens where the user can still see their own terminal; every one
/// after it runs on a worker thread.
pub fn run(
    refresh: Duration,
    projects: Vec<String>,
    mut collect: Box<dyn FnMut(&Wanted) -> Snapshot + Send>,
) -> anyhow::Result<()> {
    let first = collect(&Wanted::Everything);
    // Held, not discarded: the socket comes off the filesystem when this
    // returns, so the run that made it is the run that clears it away.
    let (events, ask, _socket, at_startup) = wire(refresh, Reported::watching(projects), collect);
    let mut screen = Screen::showing(first, Box::new(Herdr::new(RealRunner)), at_startup)?;

    drive(&mut screen, &events, &ask)
}

/// Everything that reaches the loop.
///
/// A `Snapshot` is large and the other three carry almost nothing, so the
/// collected one is boxed rather than widening every event to its size.
#[cfg_attr(test, derive(Debug, PartialEq))]
enum Event {
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
}

/// What the screen has on it.
///
/// The loop holds this rather than the view because it decides what a
/// keystroke means, and while the bindings are up every keystroke means "take
/// them away".
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

    /// Select whatever is drawn on one row of the screen, reporting whether
    /// the screen has changed.
    ///
    /// A row is a fact about the frame rather than about the forest, so this
    /// is the loop's other seam: the loop knows where the pointer was and
    /// nothing about what is drawn there.
    fn clicked(&mut self, row: u16) -> bool;

    fn draw(&mut self, showing: Showing) -> anyhow::Result<()>;
}

/// Read events until the user quits.
///
/// Every wait in here is a wait on the one channel: a keystroke, a resize, a
/// project reporting a change and a collection coming back are the same kind
/// of thing to the loop, and none of them is a deadline it sleeps until.
fn drive(
    view: &mut dyn View,
    events: &Receiver<Event>,
    ask: &Sender<Wanted>,
) -> anyhow::Result<()> {
    let mut showing = Showing::Forest;
    view.draw(showing)?;
    let mut outstanding = Outstanding::default();

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
                    outstanding.ask(ask, Wanted::Everything);
                    false
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
                outstanding.ask(ask, wanted);
                false
            }
            Event::Collected(snapshot) => {
                outstanding.came_back(ask);
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
#[derive(Default)]
struct Outstanding {
    collecting: bool,
    everything: bool,
    projects: BTreeSet<String>,
}

impl Outstanding {
    /// Ask for a collection, or keep it until the one running comes back.
    fn ask(&mut self, ask: &Sender<Wanted>, wanted: Wanted) {
        if !self.collecting {
            self.collecting = ask.send(wanted).is_ok();
            return;
        }
        match wanted {
            Wanted::Everything => {
                self.everything = true;
                self.projects.clear();
            }
            Wanted::Project(project) => {
                if !self.everything {
                    self.projects.insert(project);
                }
            }
        }
    }

    /// Take the collection that came back, asking for whatever waited behind
    /// it.
    fn came_back(&mut self, ask: &Sender<Wanted>) {
        self.collecting = false;
        let next = if std::mem::take(&mut self.everything) {
            Some(Wanted::Everything)
        } else {
            self.projects.pop_first().map(Wanted::Project)
        };
        if let Some(next) = next {
            self.ask(ask, next);
        }
    }
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

/// The inbound channel, or nothing and the two things said in its place.
///
/// Both are deliberate and neither is a copy of the other. The notice is what
/// a reader needs while they are looking: the view is polled rather than
/// reported, so it is only ever as fresh as the refresh interval, and no row
/// above the foot could show that. The stderr line is what they need
/// afterwards: it names the path and the error underneath it, which is the
/// actionable half — another `bdi` holding the socket is closed by hand — and
/// the half no phrase may carry. It is written before the alternate screen
/// opens, so it is still on the primary screen when the view tears down, and
/// it can be redirected to a file where a notice never can.
fn inbound(opened: Result<Socket, changes::Refused>) -> (Option<Socket>, Option<Notice>) {
    match opened {
        Ok(socket) => (Some(socket), None),
        Err(refused) => {
            eprintln!("bdi: {refused}");
            (None, Some(Notice::NoInboundChannel))
        }
    }
}

/// Start everything that produces events, and hand back the loop's ends: the
/// events themselves, the channel a collection is asked for on, the inbound
/// socket for as long as there is a view to keep live, and whatever this run
/// of `bdi` has to say about itself.
fn wire(
    refresh: Duration,
    reported: Reported,
    collect: Box<dyn FnMut(&Wanted) -> Snapshot + Send>,
) -> (Receiver<Event>, Sender<Wanted>, Option<Socket>, Vec<Notice>) {
    let (to_the_loop, events) = mpsc::channel();
    let (ask, asked) = mpsc::channel();

    let collecting = to_the_loop.clone();
    thread::spawn(move || collector(collect, &asked, &collecting));

    let typing = to_the_loop.clone();
    thread::spawn(move || keys(&typing));

    let (changed, changes) = mpsc::channel();
    let (socket, refused) = inbound(changes::listen(
        changes::where_writers_find_bdi(),
        &reported,
        changed.clone(),
    ));

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
                due: VecDeque::new(),
            },
            &to_the_loop,
        );
    });

    (events, ask, socket, refused.into_iter().collect())
}

/// What tells `bdi` that a project's work has moved on.
///
/// A tracker that can report its own changes is subscribed to; one that
/// cannot is timed, and a timer is only a duller way of being told. Each
/// source runs on its own thread and blocks there, so the loop never sleeps
/// until a deadline of its own.
trait Changes: Send {
    /// Block until there is something to collect for, and say what reading it
    /// takes. Nothing, where the source has no more to report.
    fn next(&mut self) -> Option<Wanted>;
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
    /// What the last interval found uncovered and has not yet reported. One
    /// report is one collection, so several projects are handed over one at
    /// a time.
    due: VecDeque<String>,
}

impl Changes for Timer {
    fn next(&mut self) -> Option<Wanted> {
        loop {
            if let Some(project) = self.due.pop_front() {
                return Some(Wanted::Project(project));
            }
            thread::sleep(self.every);
            match self.reported.uncovered(self.every) {
                Uncovered::Everything => return Some(Wanted::Everything),
                Uncovered::These(projects) => self.due = projects.into(),
                Uncovered::Nothing => {}
            }
        }
    }
}

/// The source for the projects something else reports for: it says the work
/// has moved when a writer has said which project it moved in.
struct Inbound {
    changes: Receiver<String>,
    /// Held so the channel never runs out of writers. A source whose last
    /// writer has gone must go quiet, not report as fast as it can.
    _open: Sender<String>,
}

impl Changes for Inbound {
    fn next(&mut self) -> Option<Wanted> {
        self.changes.recv().ok().map(Wanted::Project)
    }
}

/// Report one project's changes until the loop stops listening.
fn report(source: &mut dyn Changes, to: &Sender<Event>) {
    while let Some(wanted) = source.next() {
        if to.send(Event::Changed(wanted)).is_err() {
            return;
        }
    }
}

/// Collect on demand, off the UI thread.
///
/// One collection is dozens of remote round trips per project, and the view
/// has to stay under the user's hands throughout.
fn collector(
    mut collect: Box<dyn FnMut(&Wanted) -> Snapshot + Send>,
    asked: &Receiver<Wanted>,
    to: &Sender<Event>,
) {
    while let Ok(wanted) = asked.recv() {
        if to
            .send(Event::Collected(Box::new(collect(&wanted))))
            .is_err()
        {
            return;
        }
    }
}

/// Read the terminal until it has nothing more to say.
fn keys(to: &Sender<Event>) {
    while let Ok(read) = event::read() {
        let Some(event) = incoming(read) else {
            continue;
        };
        if to.send(event).is_err() {
            return;
        }
    }
}

/// What the loop is told about one thing the terminal reported, where it is
/// told anything at all.
///
/// A key that is only being released is not a keystroke; on terminals that
/// report releases at all, taking both would act on every binding twice.
///
/// Capture turns on far more than the two gestures the forest answers.
/// Crossterm asks for any-event tracking, so the terminal reports every cell
/// the pointer crosses whether a button is down or not, and a reader dragging
/// across the screen produces hundreds. They are dropped here, on the thread
/// that reads them, because the loop must stay under the user's hands: a
/// wedged loop is a `^C` that never reaches the Quit mapping and a terminal
/// left in raw mode.
///
/// So of the pointer only two things are answered — a left click, which names
/// a row, and a wheel notch, which moves the selection. A release, a drag,
/// bare motion, the other two buttons and the horizontal wheel are each
/// dropped: none of them names a row the reader is asking for, and
/// right-click in a herdr pane belongs to herdr's own menu.
fn incoming(read: event::Event) -> Option<Event> {
    match read {
        event::Event::Key(key) if key.kind == KeyEventKind::Press => Some(Event::Key(key)),
        event::Event::Resize(..) => Some(Event::Resize),
        event::Event::Mouse(mouse) => match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => Some(Event::Clicked(mouse.row)),
            MouseEventKind::ScrollUp => Some(Event::Scrolled(Motion::PreviousRow)),
            MouseEventKind::ScrollDown => Some(Event::Scrolled(Motion::NextRow)),
            _ => None,
        },
        _ => None,
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

    /// Report a selection that may have moved, taking the tail with it where
    /// it did. Every way of moving the selection goes through here, so no
    /// route to it can leave the tail behind showing another bead's pane.
    fn moved(&mut self, changed: bool) -> bool {
        if changed {
            self.follow();
        }
        changed
    }

    /// Put the selection on one line of the forest.
    fn select(&mut self, at: usize) -> bool {
        let changed = self.forest.select_line(at);
        self.moved(changed)
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
        self.moved(changed)
    }
}

/// The alternate screen, and what is drawn on it.
///
/// The terminal is on the alternate screen, in raw mode and reporting the
/// mouse for as long as this lives, so dropping it puts the terminal back
/// however the loop ended. `ratatui::init` hooks panics as well, and `Drop`
/// runs as the panic unwinds, so a crash does not leave a wedged tty behind
/// either.
struct Screen {
    terminal: DefaultTerminal,
    shown: Shown,
    /// What this run of `bdi` could not do, settled before the first
    /// collection and true until the session ends.
    at_startup: Vec<Notice>,
}

impl Screen {
    fn showing(
        snapshot: Snapshot,
        panes: Box<dyn Panes>,
        at_startup: Vec<Notice>,
    ) -> anyhow::Result<Self> {
        let terminal = ratatui::try_init()?;
        // Built before the mouse is asked for, so that a terminal which
        // refuses is still put back by the `Drop` this now has.
        let screen = Self {
            terminal,
            shown: Shown::of(snapshot, panes),
            at_startup,
        };

        // Capture costs the reader the terminal's own mouse: while `bdi` is
        // up, dragging over the window no longer selects text in it. It is
        // taken anyway and unconditionally, because in the terminal this is
        // read in it costs less than it looks. herdr owns the mouse above
        // the pane and keeps its copy mode, which selects by keyboard; kitty
        // keeps its shift-drag, which bypasses whatever the application
        // grabbed. So what is given up is drag-selection inside one pane,
        // and what is bought is the pointer working at all.
        execute!(io::stdout(), EnableMouseCapture)?;

        Ok(screen)
    }
}

/// One frame: the forest, the tail beneath it, the height the forest is told
/// it has, and the bindings window when one is up.
///
/// Outside the `terminal.draw` closure so a test backend can drive the whole
/// frame. This is the only place the three bands are agreed on, and `^D` and
/// `^U` are the part of that agreement nothing on screen would show was
/// broken. The bindings go on last because they sit over the forest rather
/// than in place of it.
fn paint(
    frame: &mut Frame,
    forest: &mut Forest,
    tail: &Tail,
    showing: Showing,
    at_startup: &[Notice],
) {
    let bands = draw::regions(frame.area());
    forest.set_half_screen(draw::half_screen(bands.forest));
    draw::draw(frame, frame.area(), forest, at_startup, &key_row());
    draw::draw_tail(frame, bands.tail, tail);
    if showing == Showing::Bindings {
        key_bindings(frame, frame.area(), &bindings());
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        // Ahead of the restore, mirroring the order they were turned on in.
        // A session that ended with the mouse still captured would leave the
        // reader a window whose pointer does nothing and no program left to
        // ask for it back.
        let _ = execute!(io::stdout(), DisableMouseCapture);
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

    fn clicked(&mut self, row: u16) -> bool {
        let bands = draw::regions(self.terminal.get_frame().area());
        let forest = &self.shown.forest;

        match draw::line_at(
            bands.forest,
            forest.selected_line(),
            forest.lines().len(),
            row,
        ) {
            Some(at) => self.shown.select(at),
            // The tail is an echo of a pane and the key row is a legend.
            // Neither holds anything the selection could sit on, and a row
            // past the last line of the forest holds nothing at all.
            None => false,
        }
    }

    fn draw(&mut self, showing: Showing) -> anyhow::Result<()> {
        let (forest, tail) = (&mut self.shown.forest, &self.shown.tail);
        let at_startup = &self.at_startup;
        self.terminal
            .draw(|frame| paint(frame, forest, tail, showing, at_startup))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::run::RunFailure;
    use crate::model::join::{AgentRef, BeadKey, JoinSource};
    use crate::model::snapshot::{
        self, Counts, Filter, HerdrState, Node, TrackerFailure, TrackerState, Tree,
    };
    use crate::model::types::{PaneStatus, Status};
    use crate::view::bindings::bindings_window;
    use crate::view::Motion;
    use chrono::Utc;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use ratatui::widgets::Block;
    use ratatui::Terminal;

    /// Long enough that a thread which was going to report has, and short
    /// enough that a test waiting in vain is not a hang.
    const A_MOMENT: Duration = Duration::from_secs(5);

    /// What the bindings window has inside its border, one string per row,
    /// with the forest it sits over left out. Trailing blanks are trimmed, so
    /// anything the window failed to clear survives into the assertion.
    fn window_inner(width: u16, height: u16) -> Vec<String> {
        let mut forest = forest::flatten(&a_grove(30));
        let screen = painted(
            &mut forest,
            &Tail::Silent("nothing to tail"),
            width,
            height,
            Showing::Bindings,
        );
        let inner =
            Block::bordered().inner(bindings_window(Rect::new(0, 0, width, height), &bindings()));

        (inner.y..inner.y + inner.height)
            .map(|y| {
                screen[y as usize]
                    .chars()
                    .skip(inner.x as usize)
                    .take(inner.width as usize)
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    /// A view that remembers what the loop did to it.
    #[derive(Default)]
    struct Recorder {
        applied: Vec<Action>,
        clicked: Vec<u16>,
        collected: usize,
        drawn: usize,
        showing: Vec<Showing>,
        /// What a click reports back, for the tests about a click that lands
        /// on no row.
        nothing_under_the_pointer: bool,
    }

    impl View for Recorder {
        fn collected(&mut self, _snapshot: Snapshot) {
            self.collected += 1;
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

    /// A source that reports only when the test says so.
    struct OnCue(Receiver<Wanted>);

    impl Changes for OnCue {
        fn next(&mut self) -> Option<Wanted> {
            // A test that has finished with this source drops the cue, and
            // the source goes quiet.
            self.0.recv().ok()
        }
    }

    /// A timer with nothing yet due, as one starts.
    fn polling(every: Duration, reported: Reported) -> Timer {
        Timer {
            every,
            reported,
            due: VecDeque::new(),
        }
    }

    fn atlas() -> Wanted {
        Wanted::Project("atlas".to_string())
    }

    fn ferry() -> Wanted {
        Wanted::Project("ferry".to_string())
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
            unconfigured: Vec::new(),
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
        let drawn = window_inner(100, BINDINGS.len() as u16 + 2);

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

    /// The bead's short terminal, decided rather than left to a cut: as many
    /// bindings as fit from the top of the table, then a count of what is
    /// missing. Nothing is drawn with its bottom sheared off, and what
    /// survives is the half a reader could not have guessed.
    ///
    /// The way out is the window's title rather than a row, so it is checked
    /// on the border above these.
    #[test]
    fn eight_rows_hold_the_keys_worth_most_and_a_count_of_the_rest() {
        assert_eq!(
            window_inner(80, 8),
            vec![
                "  Enter     focus the selected bead's pane in herdr",
                "  Space     fold or unfold the selected node",
                "  a         show every tree, not only those with a live agent",
                "  ?         show these key bindings",
                "  q, ^C     quit",
                "  … 9 more bindings · no room on a screen this short",
            ]
        );
    }

    /// A reader who cannot see how to leave is stuck in a view they may have
    /// opened by accident, so the way out is the one thing a window too short
    /// for any binding at all still carries.
    #[test]
    fn the_way_out_is_the_windows_title_however_short_the_screen() {
        for height in [8, 24] {
            let window = bindings_window(Rect::new(0, 0, 80, height), &bindings());
            let mut forest = forest::flatten(&a_grove(30));
            let screen = painted(
                &mut forest,
                &Tail::Silent("nothing to tail"),
                80,
                height,
                Showing::Bindings,
            );

            assert!(
                screen[window.y as usize].contains("Key bindings · press any key to close"),
                "no way out on the window's own top row at {height} rows: {:?}",
                screen[window.y as usize]
            );
        }
    }

    /// The bead: a reader opens `?` to look up the key for the row they are
    /// on, so that row has to still be on the screen. The full-screen view
    /// this replaced took the whole forest away.
    #[test]
    fn the_forest_is_still_drawn_around_the_bindings_window() {
        let mut forest = forest::flatten(&a_grove(30));
        let tail = Tail::Silent("nothing to tail");
        let alone = painted(&mut forest, &tail, 80, 24, Showing::Forest);
        let over = painted(&mut forest, &tail, 80, 24, Showing::Bindings);
        let window = bindings_window(Rect::new(0, 0, 80, 24), &bindings());

        assert!(
            window.height < 24 && window.width < 80,
            "a window the size of the screen is the view this replaced: {window:?}"
        );

        let beside = window.x as usize;
        assert!(
            beside > 0,
            "no forest is left beside a window flush to the edge"
        );

        for (n, (row, was)) in over.iter().zip(alone.iter()).enumerate() {
            if !(window.y..window.y + window.height).contains(&(n as u16)) {
                assert_eq!(
                    row, was,
                    "row {n} is above or below the window and changed anyway"
                );
                continue;
            }
            let left: String = row.chars().take(beside).collect();
            assert_eq!(
                left,
                was.chars().take(beside).collect::<String>(),
                "the forest beside the window on row {n}"
            );
        }
    }

    /// `Clear` is what stops the trees showing between the bindings. Every
    /// row is asserted whole, so forest text surviving in the columns a
    /// shorter binding does not reach is a failure rather than a trim.
    #[test]
    fn no_forest_shows_through_the_bindings_window() {
        assert_eq!(
            window_inner(80, 24),
            vec![
                "  Enter     focus the selected bead's pane in herdr",
                "  Space     fold or unfold the selected node",
                "  a         show every tree, not only those with a live agent",
                "  ?         show these key bindings",
                "  q, ^C     quit",
                "  ^R        collect from the trackers again now",
                "  Down, j   move down one row",
                "  Up, k     move up one row",
                "  Right, l  expand, or move to the first child when it is already expanded",
                "  Left, h   collapse, or move to the parent when it is already collapsed",
                "  ^D        move down half a screen",
                "  ^U        move up half a screen",
                "  g         move to the first row",
                "  G         move to the last row",
            ]
        );
    }

    /// A window two columns narrower than the screen it sits in, on the
    /// narrowest screen anyone uses. `q, ^C quit` is the line that must
    /// survive: a reader who cannot find it is stuck.
    #[test]
    fn a_forty_column_screen_keeps_the_keys_and_cuts_only_what_it_must() {
        let window = bindings_window(Rect::new(0, 0, 40, 24), &bindings());
        assert_eq!(
            window.width, 40,
            "a window wider than the screen has to clamp"
        );

        let drawn = window_inner(40, 24);

        assert_eq!(drawn[4], "  q, ^C     quit");
        assert_eq!(
            drawn[8], "  Right, l  expand, or move to the fi…",
            "the one line too long for forty columns, cut with the cut marked"
        );
        assert_eq!(drawn.len(), BINDINGS.len(), "a narrow screen loses no rows");
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

        assert_eq!(asked.try_iter().collect::<Vec<_>>(), [Wanted::Everything]);
        assert_eq!(
            view.applied,
            [Action::Move(Motion::NextRow)],
            "the loop read on rather than waiting for the collection"
        );
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

        drive(&mut view, &events, &ask).expect("the loop runs");

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

        drive(&mut view, &events, &ask).expect("the loop runs");

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

        drive(&mut view, &events, &ask).expect("the loop runs");

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

        drive(&mut view, &events, &ask).expect("the loop runs");

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

        drive(&mut view, &events, &ask).expect("the loop runs");

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

        drive(&mut view, &events, &ask).expect("the loop runs");

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

        cue.send(atlas()).expect("the source is listening");

        assert_eq!(
            events.recv_timeout(A_MOMENT).ok(),
            Some(Event::Changed(atlas())),
            "the loop was told which project moved, not just that something did"
        );
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
            report(&mut polling(Duration::from_secs(1), reported), &to_the_loop);
        });

        assert!(
            events.recv_timeout(Duration::from_millis(1500)).is_err(),
            "the writer had said everything a poll would have found"
        );
    }

    /// The saving a mixed setup gets from a refresh being nameable: the
    /// project with a producer is left out of the poll its neighbour still
    /// needs, rather than swept up with it every interval.
    #[test]
    fn a_poll_names_only_the_projects_nothing_is_reporting_for() {
        let (to_the_loop, events) = mpsc::channel();
        let reported = Reported::watching(["atlas".to_string(), "ferry".to_string()]);

        let producing = reported.clone();
        thread::spawn(move || loop {
            producing.take("atlas");
            thread::sleep(Duration::from_millis(20));
        });
        thread::spawn(move || {
            report(
                &mut polling(Duration::from_millis(300), reported),
                &to_the_loop,
            );
        });

        assert_eq!(
            events.recv_timeout(A_MOMENT).ok(),
            Some(Event::Changed(ferry())),
            "atlas is being reported for, so the poll has only ferry to find"
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
                &mut polling(Duration::from_millis(20), reported),
                &to_the_loop,
            );
        });

        assert_eq!(
            events.recv_timeout(A_MOMENT).ok(),
            Some(Event::Changed(Wanted::Everything)),
            "a poll knows nothing about where the work moved, so it reads everywhere"
        );
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
                &mut polling(Duration::from_millis(20), Reported::default()),
                &to_the_loop,
            );
        });

        for reported in 1..=2 {
            assert!(
                matches!(events.recv_timeout(A_MOMENT), Ok(Event::Changed(_))),
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
                &mut polling(Duration::from_millis(1), Reported::default()),
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

    // ---- the pointer ------------------------------------------------------

    fn moused(kind: MouseEventKind, row: u16) -> event::Event {
        event::Event::Mouse(event::MouseEvent {
            kind,
            column: 17,
            row,
            modifiers: KeyModifiers::NONE,
        })
    }

    /// The row every table entry below is reported on, so that what a test
    /// asserts is the kind and not the arithmetic.
    const ON_ROW: u16 = 9;

    /// Every kind of mouse event a captured terminal reports, and what the
    /// reader makes of it.
    ///
    /// The match is what makes it every one rather than every one anybody
    /// remembered: a kind added to crossterm's enum makes it non-exhaustive,
    /// and the compiler names this function until the list above it has been
    /// decided too.
    fn every_mouse_kind() -> Vec<(MouseEventKind, Option<Event>)> {
        let every = vec![
            (
                MouseEventKind::Down(MouseButton::Left),
                Some(Event::Clicked(ON_ROW)),
            ),
            (MouseEventKind::Down(MouseButton::Right), None),
            (MouseEventKind::Down(MouseButton::Middle), None),
            (MouseEventKind::Up(MouseButton::Left), None),
            (MouseEventKind::Up(MouseButton::Right), None),
            (MouseEventKind::Up(MouseButton::Middle), None),
            (MouseEventKind::Drag(MouseButton::Left), None),
            (MouseEventKind::Drag(MouseButton::Right), None),
            (MouseEventKind::Drag(MouseButton::Middle), None),
            (MouseEventKind::Moved, None),
            (
                MouseEventKind::ScrollUp,
                Some(Event::Scrolled(Motion::PreviousRow)),
            ),
            (
                MouseEventKind::ScrollDown,
                Some(Event::Scrolled(Motion::NextRow)),
            ),
            (MouseEventKind::ScrollLeft, None),
            (MouseEventKind::ScrollRight, None),
        ];

        for (kind, _) in &every {
            match kind {
                MouseEventKind::Down(button)
                | MouseEventKind::Up(button)
                | MouseEventKind::Drag(button) => match button {
                    MouseButton::Left | MouseButton::Right | MouseButton::Middle => (),
                },
                MouseEventKind::Moved
                | MouseEventKind::ScrollUp
                | MouseEventKind::ScrollDown
                | MouseEventKind::ScrollLeft
                | MouseEventKind::ScrollRight => (),
            }
        }

        every
    }

    /// The bead this arm exists for: capture turns on every report the
    /// terminal can make, and each one is answered or dropped because it was
    /// decided, not because it fell through a gap.
    #[test]
    fn every_kind_of_mouse_report_is_answered_or_dropped_on_purpose() {
        for (kind, wanted) in every_mouse_kind() {
            assert_eq!(incoming(moused(kind, ON_ROW)), wanted, "{kind:?}");
        }
    }

    /// Motion is the flood: capture asks for a report on every cell the
    /// pointer crosses, and the loop must never be handed one. Dropping it
    /// here costs a match arm on a thread that is not the loop.
    #[test]
    fn a_pointer_moving_over_the_screen_reaches_the_loop_not_at_all() {
        let flood: Vec<Option<Event>> = (0..500)
            .map(|row| incoming(moused(MouseEventKind::Moved, row % 24)))
            .collect();

        assert!(flood.iter().all(Option::is_none));
    }

    /// A click names a row and nothing else. Every band spans the width of
    /// the screen, so the column the pointer was in names no other row.
    #[test]
    fn a_click_is_read_as_the_row_it_landed_on() {
        for row in [0, 9, 23, u16::MAX] {
            assert_eq!(
                incoming(moused(MouseEventKind::Down(MouseButton::Left), row)),
                Some(Event::Clicked(row)),
                "row {row}"
            );
        }
    }

    /// The rule that was there before the mouse was: a key only being
    /// released is not a keystroke, and nothing else the terminal reports is
    /// one either.
    #[test]
    fn a_key_release_a_focus_change_and_a_resize_are_read_as_they_were() {
        let pressed = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE);
        let released = KeyEvent::new_with_kind(
            KeyCode::Char('j'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );

        assert_eq!(
            incoming(event::Event::Key(pressed)),
            Some(Event::Key(pressed))
        );
        assert_eq!(incoming(event::Event::Key(released)), None);
        assert_eq!(incoming(event::Event::Resize(80, 24)), Some(Event::Resize));
        assert_eq!(incoming(event::Event::FocusGained), None);
        assert_eq!(incoming(event::Event::FocusLost), None);
    }

    #[test]
    fn a_click_reaches_the_view_as_the_row_it_landed_on() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();

        drive(&mut view, &waiting(vec![Event::Clicked(9)]), &ask).expect("the loop runs");

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

        drive(&mut view, &waiting(vec![Event::Clicked(21)]), &ask).expect("the loop runs");

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
        )
        .expect("the loop runs");

        assert!(view.applied.is_empty());
        assert_eq!(
            view.showing,
            [Showing::Forest, Showing::Bindings, Showing::Forest]
        );
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
            unconfigured: Vec::new(),
            conflicts: Vec::new(),
            collected: trees,
        }
    }

    /// A directory of this test's own, so a test that binds a socket does not
    /// collide with another run of the suite.
    fn a_socket_path(named: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("bdi-{named}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a directory to put the socket in");
        dir.join("beady-eye").join("changes.sock")
    }

    /// The bead's own case: the failure that used to reach only stderr now
    /// also reaches the view, as a notice that outlives the moment it was
    /// printed in. The stderr line is kept — see `inbound` — so this asserts
    /// the notice and leaves the printing to the test harness to capture.
    #[test]
    fn a_channel_that_would_not_open_leaves_a_notice_behind_it() {
        let (socket, notice) = inbound(Err(changes::Refused::NoRuntimeDirectory));

        assert!(socket.is_none());
        assert_eq!(notice, Some(Notice::NoInboundChannel));
    }

    /// A session with a working channel has nothing to say about itself, and
    /// a foot that warned anyway would teach a reader to ignore it.
    #[test]
    fn a_channel_that_opens_says_nothing() {
        let (changed, _changes) = mpsc::channel();
        let at = a_socket_path("inbound");

        let (socket, notice) = inbound(changes::listen(
            Some(at),
            &Reported::watching(["atlas".to_string()]),
            changed,
        ));

        assert!(socket.is_some());
        assert_eq!(notice, None);
    }

    fn painted(
        forest: &mut Forest,
        tail: &Tail,
        width: u16,
        height: u16,
        showing: Showing,
    ) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test backend");
        terminal
            .draw(|frame| paint(frame, forest, tail, showing, &[]))
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
        let mut forest = an_open_grove(30);

        forest.apply(Action::Move(Motion::HalfScreenDown));
        assert_eq!(
            forest.selected_line(),
            10,
            "the forest's own default, until a frame has been drawn"
        );

        let mut forest = an_open_grove(30);
        painted(
            &mut forest,
            &Tail::Silent("nothing to tail"),
            60,
            24,
            Showing::Forest,
        );
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

    /// The grove with its root opened by hand. Nobody is working in it and
    /// nothing is wrong with it, so the fold default rests it as its header,
    /// and what these tests are about is the rows under it.
    /// The grove with its tree open, which is how it rests: every bead in it
    /// is ready and the fold opens down to ready work. Asserted rather than
    /// toggled, because a toggle over a tree already open shuts it and takes
    /// every row under the header with it.
    fn an_open_grove(beads: usize) -> forest::Forest {
        let forest = forest::flatten(&a_grove(beads));
        assert_eq!(
            forest.lines()[0].folded,
            Some(true),
            "the grove's beads are ready, so its tree rests open"
        );
        forest
    }

    fn shown(snapshot: Snapshot) -> Shown {
        Shown::of(snapshot, Box::new(NoPanes))
    }

    /// Where the cursor is, by the bead its line carries. `Forest` does not
    /// answer this: a tree's header line carries its root, so a caller that
    /// read the key without telling a header from a bead would tail the root
    /// of whatever tree the cursor's header stands for.
    fn cursor(shown: &Shown) -> Option<&BeadKey> {
        let forest = &shown.forest;
        forest.lines()[forest.selected_line()].bead()
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
        let rows = painted(
            &mut shown.forest,
            &shown.tail,
            width,
            height,
            Showing::Forest,
        );
        rows[..bands.forest.height as usize].to_vec()
    }

    /// The same grove with a live agent on every bead, so that moving the
    /// selection changes which pane the tail is reading.
    fn a_staffed_grove(beads: usize) -> Snapshot {
        let mut snapshot = a_grove(beads);
        for tree in snapshot
            .trees
            .iter_mut()
            .chain(snapshot.collected.iter_mut())
        {
            for (at, node) in tree.nodes.iter_mut().enumerate() {
                node.agent = Some(AgentRef {
                    pane: format!("w:p{at}"),
                    pane_status: PaneStatus::Working,
                    title: None,
                    source: JoinSource::AgentPane,
                });
            }
        }
        snapshot
    }

    /// A click is another way to move the selection and not another kind of
    /// selection. Where a keystroke would have taken the tail, a click that
    /// lands on the same row has to take it to the same place — the tail
    /// going stale under the pointer is a screen that quietly disagrees with
    /// itself.
    #[test]
    fn a_click_takes_the_tail_with_it_exactly_as_a_keystroke_does() {
        let mut by_key = shown(a_staffed_grove(6));
        by_key.apply(Action::Move(Motion::LastRow));

        let mut by_click = shown(a_staffed_grove(6));
        let at_rest = by_click.tailing.clone();
        assert!(by_click.select(by_key.forest.selected_line()));

        assert_ne!(
            by_key.tailing, at_rest,
            "the fixture has to move the tail at all"
        );
        assert_eq!(by_click.tailing, by_key.tailing);
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
        shown.apply(Action::ExpandOrChild);
        shown.apply(Action::Move(Motion::LastRow));
        let was = shown.forest.selected_line();

        assert_eq!(cursor(&shown), Some(&bead("grove", "grv-1.6")));

        shown.collected(a_grove_reordered(6));

        assert_eq!(cursor(&shown), Some(&bead("grove", "grv-1.6")));
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

        assert_eq!(cursor(&shown), Some(&bead("atlas", "a-1")));

        assert!(shown.apply(Action::ToggleFilter));

        assert_eq!(shown.forest.snapshot().filter, Filter::All);
        assert_eq!(cursor(&shown), Some(&bead("atlas", "a-1")));
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
        let mut forest = an_open_grove(30);
        let tail = Tail::Pane {
            pane: "w:p1".to_string(),
            lines: vec!["rebuilt .#thinkpad".to_string()],
        };

        let rows = painted(&mut forest, &tail, 40, 12, Showing::Forest);
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
