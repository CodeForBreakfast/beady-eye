//! What `bdi` puts on the alternate screen.
//!
//! `Shown` is the rows and the band beneath them, held apart from the
//! terminal because none of it needs a tty; `Screen` is the terminal, and
//! putting it back however the run ends is what its `Drop` is for. They are
//! one module because the drawing reaches into the rows it draws.

use std::io;
use std::time::Duration;

use chrono::{DateTime, Utc};
use ratatui::crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{Clear, ClearType};
use ratatui::{DefaultTerminal, Frame};

use crate::app::Awaited;
use crate::collect::panes::{Answer, Panes};
use crate::model::snapshot::Snapshot;
use crate::view::bindings::key_bindings;
use crate::view::forest::{self, Forest};
use crate::view::phrase;
use crate::view::tail::{self, Tail};
use crate::view::{draw, Action, Freshness, Notice};

use super::drive::{Showing, View};
use super::keys::{bindings, key_row};

/// The forest on the alternate screen and the tail beneath it.
///
/// Held apart from the terminal that draws it because the terminal needs a
/// tty and none of this does.
struct Shown {
    forest: Forest,
    panes: Box<dyn Panes>,
    tail: Tail,
    /// The pane the band on screen is about, so a selection moving within it
    /// does not spend a herdr call on the answer already drawn.
    tailing: Option<String>,
    /// Where the band is with the pane read it is waiting on.
    reading: Reading,
    /// What the collection in flight is reading and when it was asked for,
    /// where one is running. Every project line it names says its own rows
    /// are about to be replaced, and the rest of the screen carries on saying
    /// how stale it is.
    ///
    /// The instant is what tells a collection under way from one that has
    /// stopped answering. It is stamped where the ask happens rather than
    /// here, because being told a collection is running is not the same as
    /// one having started, and only the loop knows which it is telling us.
    ///
    /// Held here rather than on the lines because it changes without the
    /// snapshot changing: a collection starts and ends between two
    /// flattenings, and the mark on a line it names turns several times
    /// inside one of them.
    collecting: Vec<Awaited>,
}

/// Where the band under the forest is with the read it is waiting on.
///
/// At most one is ever out, so what a reader moving faster than herdr answers
/// costs is a run of answers dropped rather than a herdr call per keystroke.
/// An answer is the answer to what was drawn when it was asked for, and what
/// is drawn moves on: the cursor onto another pane, and a collection coming
/// back under the same one, which is the refresh tick the pane is re-read on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reading {
    /// Nothing is out, so the pane the band names is the pane to ask for.
    Nothing,
    /// A read is out, and its answer is the answer to what is drawn.
    Outstanding,
    /// A read is out, and what is drawn moved on after it was asked. Its
    /// answer is the answer to nothing on the screen, so it is dropped and
    /// the pane the band names now is asked for in its place.
    Superseded,
}

impl Shown {
    fn of(snapshot: Snapshot, panes: Box<dyn Panes>) -> Self {
        let forest = forest::flatten(snapshot);
        let mut shown = Self {
            tail: tail::tail(&forest),
            tailing: tail::target(&forest).pane().map(str::to_string),
            forest,
            panes,
            reading: Reading::Nothing,
            // Nothing in flight until the loop says otherwise. A collection
            // is already running by the time this exists — the run asks for
            // one before it opens the screen — and it reaches this the way
            // every one after it does, through `collecting`.
            collecting: Vec::new(),
        };
        // Asked for here rather than waited for: the first frame is drawn on
        // the answer to this arriving, not on herdr getting round to it.
        shown.ask();
        shown
    }

    /// Say what the collection in flight is reading, or that none is,
    /// reporting whether the screen is any different for it.
    ///
    /// Being told again what it already says is not a change: a collection
    /// coming back with another waiting behind it names the same projects as
    /// often as not, and a redraw that puts the same frame back is a redraw
    /// for nothing.
    fn collecting(&mut self, awaited: &[Awaited]) -> bool {
        let changed = self.collecting != awaited;
        self.collecting = awaited.to_vec();
        changed
    }

    /// How long what is drawn goes on being true with nothing happening.
    ///
    /// The soonest deadline any project line on the screen sets. Each of them
    /// says two things — a mark, and how old its rows are — and a collection
    /// in flight does not silence the ages: they go on ticking under the mark
    /// that is turning, so a project read a moment ago is due a redraw well
    /// inside the frame.
    ///
    /// The ages are asked of `read_at` rather than of the lines, so a project
    /// drawn under any rule the forest has is covered by it. A project that is
    /// read and not drawn costs a redraw nobody sees, which is cheaper than
    /// the line that quietly stops being true.
    ///
    /// The mark is asked once of the collection, where the ages are asked one
    /// project at a time. Which projects a collection names decides which
    /// *lines* turn, and cannot decide when the screen is next due: every line
    /// a collection names turns on the one clock, so a frame falls due as soon
    /// as any of them is turning. Asking it per project would be a second way
    /// of reaching the same answer — and a collection running before anything
    /// it names has come back has no project in `read_at` to be asked about
    /// anyway, which is the startup frame.
    fn holds_for(&self, now: DateTime<Utc>) -> Option<Duration> {
        let snapshot = self.forest.snapshot();
        let ageing = snapshot
            .read_at
            .values()
            .filter_map(|at| Freshness::of(Some(*at), None, true, now));
        let turning = self
            .collecting
            .iter()
            .filter_map(|awaited| Freshness::of(None, Some(awaited), true, now));

        turning
            .into_iter()
            .chain(ageing)
            .filter_map(|how_fresh| phrase::holds_for(how_fresh, now))
            .min()
    }

    /// Put the band on whatever the selection is on now, and ask herdr for
    /// the pane where it is on one.
    ///
    /// Whatever herdr is already answering was asked for the band this
    /// replaces, so it is superseded here whether or not the pane has
    /// changed: a collection coming back is the refresh tick the pane is
    /// re-read on, and an answer read before it would be the pane as it was
    /// rather than as it is.
    fn retail(&mut self) {
        self.tailing = tail::target(&self.forest).pane().map(str::to_string);
        self.tail = tail::tail(&self.forest);
        if self.reading == Reading::Outstanding {
            self.reading = Reading::Superseded;
        }
        self.ask();
    }

    /// Ask herdr for the pane the band is waiting on, where it is waiting on
    /// one and herdr has not been asked already.
    ///
    /// A reader holding an arrow key down moves faster than a slow herdr
    /// answers, so every answer arrives about a screen they have already left
    /// and is dropped; the band goes on naming the pane it is waiting for
    /// until the cursor rests long enough for one answer to land. That is the
    /// trade the bound is spent on: the tail can fall behind the selection,
    /// and the keyboard never does.
    fn ask(&mut self) {
        if self.reading != Reading::Nothing {
            return;
        }
        let Tail::Reading { pane } = &self.tail else {
            return;
        };
        let pane = pane.clone();
        self.panes.read(&pane, tail::LINES);
        self.reading = Reading::Outstanding;
    }

    /// Take what herdr said, reporting whether the screen is any different
    /// for it.
    fn tailed(&mut self, answer: Answer) -> bool {
        match answer {
            Answer::Read { pane, read } => {
                let superseded = self.reading == Reading::Superseded;
                self.reading = Reading::Nothing;
                if superseded {
                    self.ask();
                    return false;
                }
                self.tail = tail::read(pane, read);
                true
            }
            // A focus that would not come says so where the tail is, and only
            // while the tail is still that pane's: the rule above the band
            // names the pane, so a refusal about one the reader has left
            // would be drawn as a refusal about the one they are on.
            Answer::Focused { pane, focused } => match tail::focused(focused) {
                Some(said) if self.tailing.as_deref() == Some(pane.as_str()) => {
                    self.tail = said;
                    true
                }
                _ => false,
            },
        }
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

    fn collected(&mut self, snapshot: Snapshot) {
        // A refresh keeps the folds and the selection, so the cursor stays on
        // the bead the user put it on however the new snapshot has moved it.
        self.forest.refresh(snapshot);
        // The refresh tick is when the pane is re-read: the rows it has drawn
        // since the last one are exactly what has moved on.
        self.retail();
    }

    fn apply(&mut self, action: Action) -> bool {
        // Asking for the selected bead's pane to be brought to the front
        // changes nothing on this screen, and a row with no pane is a no-op:
        // there is nothing to focus and nothing has gone wrong. What herdr
        // makes of it arrives as an answer like a reading does.
        if action == Action::Focus {
            tail::focus(&self.forest, self.panes.as_ref());
            return false;
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
/// either. A signal arrives as an event the loop returns on, so it is the
/// same drop again rather than a second way of restoring anything.
pub(super) struct Screen {
    terminal: DefaultTerminal,
    shown: Shown,
    /// What this run of `bdi` could not do, settled before the first
    /// collection and true until the session ends.
    at_startup: Vec<Notice>,
}

impl Screen {
    pub(super) fn showing(
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

        // Nothing above has erased anything: entering the alternate screen is
        // the terminal's business, and the first frame writes only the cells
        // it has ink for, so every cell it leaves blank keeps whatever the
        // terminal was showing there. Erasing here is what makes the screen
        // `bdi`'s.
        //
        // Not `terminal.clear()`, which opens by asking the terminal where
        // the cursor is and waiting for the reply.
        execute!(io::stdout(), Clear(ClearType::All))?;

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
    collecting: &[Awaited],
    now: DateTime<Utc>,
) {
    let bands = draw::regions(frame.area());
    forest.set_half_screen(draw::half_screen(bands.forest));
    draw::draw(
        frame,
        frame.area(),
        forest,
        at_startup,
        collecting,
        now,
        &key_row(),
    );
    draw::draw_tail(frame, bands.tail, tail);
    if showing == Showing::Bindings {
        key_bindings(frame, frame.area(), &bindings());
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        // Ahead of the restore, mirroring the order they were turned on in.
        // A terminal left reporting the mouse writes an escape sequence into
        // whatever runs next for every cell the pointer crosses, and there
        // is nothing left running to ask it to stop.
        //
        // Every way the run can end comes through here — the loop returning
        // on 'q', a panic unwinding, and a signal, which the loop answers by
        // returning. All three rest on the build unwinding: a profile that
        // sets `panic = "abort"` runs no `Drop` at all and would take this
        // with it, leaving exactly the terminal described above.
        let _ = execute!(io::stdout(), DisableMouseCapture);
        ratatui::restore();
    }
}

impl View for Screen {
    fn collected(&mut self, snapshot: Snapshot) {
        self.shown.collected(snapshot);
    }

    fn collecting(&mut self, awaited: &[Awaited]) -> bool {
        self.shown.collecting(awaited)
    }

    fn holds_for(&self, drawn_at: DateTime<Utc>) -> Option<Duration> {
        self.shown.holds_for(drawn_at)
    }

    fn tailed(&mut self, answer: Answer) -> bool {
        self.shown.tailed(answer)
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

    fn draw(&mut self, showing: Showing, now: DateTime<Utc>) -> anyhow::Result<()> {
        let (forest, tail) = (&mut self.shown.forest, &self.shown.tail);
        let at_startup = &self.at_startup;
        let collecting = self.shown.collecting.as_slice();
        self.terminal
            .draw(|frame| paint(frame, forest, tail, showing, at_startup, collecting, now))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Wanted;
    use crate::collect::run::{FailureKind, RunFailure};
    use crate::config::Scope;
    use crate::model::join::{AgentRef, BeadKey, JoinSource};
    use crate::model::snapshot::{Counts, Filter, HerdrState, Node, TrackerState, Tree};
    use crate::model::tree::Link;
    use crate::model::types::{Edge, PaneStatus, Status};
    use crate::tui::fixtures::{a_snapshot, atlas, ferry, reading, PATIENCE};
    use crate::tui::keys::BINDINGS;
    use crate::view::bindings::bindings_window;
    use crate::view::painted::Painted;
    use crate::view::walk::{self, Rows};
    use crate::view::Motion;
    use chrono::Utc;
    use ratatui::layout::Rect;
    use ratatui::widgets::Block;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    /// What the bindings window has inside its border, one string per row,
    /// with the forest it sits over left out. Trailing blanks are trimmed, so
    /// anything the window failed to clear survives into the assertion.
    fn window_inner(width: u16, height: u16) -> Vec<String> {
        let mut forest = forest::flatten(a_grove(30));
        let screen = screen_of(
            &mut forest,
            &Tail::Silent("nothing to tail"),
            width,
            height,
            Showing::Bindings,
        )
        .rows();
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

    /// The instant a test frame is drawn at. Nothing in these fixtures has
    /// been read, so no line quotes it; it is here because a frame is always
    /// drawn at some instant and a test must not pick a moving one.
    fn an_instant() -> DateTime<Utc> {
        use chrono::TimeZone;
        Utc.with_ymd_and_hms(2026, 8, 30, 10, 22, 14).unwrap()
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
                "  … 12 more bindings · no room on a screen this short",
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
            let mut forest = forest::flatten(a_grove(30));
            let screen = screen_of(
                &mut forest,
                &Tail::Silent("nothing to tail"),
                80,
                height,
                Showing::Bindings,
            )
            .rows();

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
        let mut forest = forest::flatten(a_grove(30));
        let tail = Tail::Silent("nothing to tail");
        let alone = screen_of(&mut forest, &tail, 80, 24, Showing::Forest).rows();
        let over = screen_of(&mut forest, &tail, 80, 24, Showing::Bindings).rows();
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
                "  E         expand the selected node and everything under it",
                "  C         collapse the selected node and everything under it",
                "  D         restore the default view",
                "  Down, j   move down one row",
                "  Up, k     move up one row",
                "  Right, l  expand, or move to the first child when it is already expanded",
                "  Left, h   collapse, or move to the parent when it is already collapsed",
                "  ^D, PgDn  move down half a screen",
                "  ^U, PgUp  move up half a screen",
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
            drawn[11], "  Right, l  expand, or move to the fi…",
            "the one line too long for forty columns, cut with the cut marked"
        );
        assert_eq!(drawn.len(), BINDINGS.len(), "a narrow screen loses no rows");
    }

    /// The row is cut from its own end, so one that outgrew the narrowest
    /// screen anyone uses would lose `q quit` — which is what a reader who
    /// cannot get out is looking for. `bdi-2bb.12` cut `^R` from it to make
    /// room for keys named in words, and it has been full to the column ever
    /// since: expanding and collapsing the whole forest and restoring the
    /// default are named in `?` alone for exactly that reason.
    #[test]
    fn the_row_under_the_tail_is_drawn_whole_on_the_narrowest_screen() {
        let mut forest = an_open_grove(30);
        let screen = screen_of(
            &mut forest,
            &Tail::Silent("nothing to tail"),
            40,
            24,
            Showing::Forest,
        )
        .rows();

        assert_eq!(
            screen.last().expect("a screen with rows on it").trim_end(),
            key_row()
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
        let bead = |id: String| Node {
            id,
            title: "a bead in the grove".to_string(),
            status: Status::InProgress,
            issue_type: "task".to_string(),
            priority: 2,
            ready: true,
            blocked_by: Vec::new(),
            started_at: None,
            closed_at: None,
            badges: Vec::new(),
            agent: None,
            anomalies: Vec::new(),
        };

        let mut beads = vec![bead("grv-1".to_string())];
        beads.extend(children.iter().map(|n| bead(format!("grv-1.{n}"))));
        let mut links = vec![(1..beads.len())
            .map(|bead| Link {
                bead,
                edge: Edge::ParentChild,
                first: true,
            })
            .collect()];
        links.resize(beads.len(), Vec::new());

        let tree = Arc::new(Tree {
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
            beads,
            children: links,
            dangling: Vec::new(),
            cycles: Vec::new(),
        });

        Snapshot {
            filter: Filter::All,
            trees: vec![Arc::clone(&tree)],
            collected: vec![tree],
            ..a_snapshot_of(Vec::new())
        }
    }

    fn a_snapshot_of(trees: Vec<Arc<Tree>>) -> Snapshot {
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
            read_at: BTreeMap::new(),
            collected: trees,
            projects: vec!["grove".to_string()],
            scope: Scope::default(),
        }
    }

    fn screen_of(
        forest: &mut Forest,
        tail: &Tail,
        width: u16,
        height: u16,
        showing: Showing,
    ) -> Painted {
        Painted::drawn_by(width, height, |frame| {
            paint(frame, forest, tail, showing, &[], &[], an_instant());
        })
    }

    /// The same screen with a collection in flight, so a project line the
    /// collection names says what it is doing.
    fn screen_collecting(
        forest: &mut Forest,
        tail: &Tail,
        collecting: &[Awaited],
        width: u16,
        height: u16,
    ) -> Painted {
        Painted::drawn_by(width, height, |frame| {
            paint(
                frame,
                forest,
                tail,
                Showing::Forest,
                &[],
                collecting,
                an_instant(),
            );
        })
    }

    /// `bdi-7ao.51`, on the screen a reader is looking at rather than in any
    /// one of the cells beneath it: a collection that has stopped answering is
    /// drawn as having stopped, where one still under way turns.
    ///
    /// The two are asserted together and from one fixture, because the defect
    /// was never that either state drew wrongly — it was that they drew the
    /// same, and a test that only knew what a hung tracker looks like could
    /// not have told.
    #[test]
    fn a_screen_says_which_of_its_projects_have_stopped_answering() {
        let mut snapshot = a_grove(2);
        snapshot.read_at.insert(
            "grove".to_string(),
            an_instant() - chrono::TimeDelta::seconds(30),
        );
        let mark = |asked_at| {
            let mut forest = forest::flatten(snapshot.clone());
            let row = screen_collecting(
                &mut forest,
                &Tail::Silent("nothing to tail"),
                &[reading(Wanted::Everything, asked_at)],
                60,
                10,
            )
            .rows()[0]
                .clone();
            assert!(row.contains("grove"), "the project's own line: {row:?}");
            row
        };

        assert!(
            mark(an_instant() - PATIENCE).contains("⠿ 30s ago"),
            "a collection past its patience: {:?}",
            mark(an_instant() - PATIENCE)
        );
        assert!(
            !mark(an_instant()).contains('⠿'),
            "a collection just asked for: {:?}",
            mark(an_instant())
        );
    }

    /// Nothing on screen shows that `^D` moved by the wrong amount, so the
    /// frame is what says the forest was told how tall it is.
    #[test]
    fn a_frame_tells_the_forest_how_far_a_half_screen_is() {
        let mut forest = an_open_grove(30);

        forest.apply(Action::Move(Motion::HalfScreenDown));
        assert_eq!(
            forest.selected_line(),
            11,
            "the forest's own default, from the first root, until a frame has been drawn"
        );

        let mut forest = an_open_grove(30);
        screen_of(
            &mut forest,
            &Tail::Silent("nothing to tail"),
            60,
            24,
            Showing::Forest,
        )
        .rows();
        forest.apply(Action::Move(Motion::HalfScreenDown));

        assert_eq!(
            forest.selected_line(),
            9,
            "half of the sixteen rows the forest was given, not half the frame"
        );
    }

    /// herdr, remembering what it was asked and answering nothing.
    ///
    /// Nothing here answers, because nothing in `Shown` waits for an answer:
    /// what herdr said arrives through `tailed`, and a test hands it one
    /// itself, whenever it likes and about whichever pane it likes.
    #[derive(Clone, Default)]
    struct Asking {
        reads: Arc<Mutex<Vec<String>>>,
        focuses: Arc<Mutex<Vec<String>>>,
    }

    impl Asking {
        fn reads(&self) -> Vec<String> {
            self.reads
                .lock()
                .expect("no test panics holding this")
                .clone()
        }

        fn focuses(&self) -> Vec<String> {
            self.focuses
                .lock()
                .expect("no test panics holding this")
                .clone()
        }
    }

    impl Panes for Asking {
        fn read(&self, pane: &str, lines: u16) {
            self.reads
                .lock()
                .expect("no test panics holding this")
                .push(format!("{pane} {lines}"));
        }

        fn focus(&self, pane: &str) {
            self.focuses
                .lock()
                .expect("no test panics holding this")
                .push(pane.to_string());
        }
    }

    /// What herdr said about a pane it read.
    fn read(pane: &str, lines: &[&str]) -> Answer {
        Answer::Read {
            pane: pane.to_string(),
            read: Ok(lines.iter().map(|line| (*line).to_string()).collect()),
        }
    }

    impl Rows for Shown {
        fn rows(&self) -> usize {
            self.forest.rows()
        }
    }

    /// Press down until the selection is on the last row, and say how many
    /// presses moved it.
    fn to_the_last_row(shown: &mut Shown) -> usize {
        let rows = shown.rows();
        let from = shown.forest.selected_line();

        let moved = walk::until(
            shown,
            |shown| shown.forest.selected_line() + 1 == rows,
            |shown| {
                shown.apply(Action::Move(Motion::NextRow));
            },
            |shown| {
                format!(
                    "a walk from row {from} of {rows} stopped at row {}",
                    shown.forest.selected_line()
                )
            },
        );

        assert_eq!(
            moved,
            rows - 1 - from,
            "a walk from row {from} of {rows} reaches the last row in one press per row after it"
        );
        moved
    }

    /// herdr refusing to bring a pane to the front.
    fn refused(pane: &str) -> Answer {
        Answer::Focused {
            pane: pane.to_string(),
            focused: Err(RunFailure {
                kind: FailureKind::Gone,
                program: "herdr".to_string(),
                detail: "the test said so".to_string(),
            }),
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
        let forest = forest::flatten(a_grove(beads));
        assert_eq!(
            forest.lines()[0].folded,
            Some(true),
            "the grove's beads are ready, so its tree rests open"
        );
        forest
    }

    fn shown(snapshot: Snapshot) -> Shown {
        Shown::of(snapshot, Box::new(Asking::default()))
    }

    /// The same, with the record of what herdr was asked kept beside it.
    fn shown_asking(snapshot: Snapshot) -> (Shown, Asking) {
        let panes = Asking::default();
        (Shown::of(snapshot, Box::new(panes.clone())), panes)
    }

    /// The screen opens on a snapshot already collected, so there is nothing
    /// to say is running. Opening on a collection would set a mark turning on
    /// every project line that only the next refresh could take off.
    #[test]
    fn a_screen_opens_over_a_collection_that_has_already_finished() {
        assert_eq!(shown(a_snapshot()).collecting, []);
    }

    /// What is being read and since when, not whether something is: each
    /// project line answers for its own rows, so the screen is told the
    /// collection's own `Wanted` and asks that which projects it names — and
    /// the instant beside it, so a line can say the tracker has stopped
    /// answering rather than that a collection is under way.
    #[test]
    fn a_screen_holds_what_the_collection_in_flight_is_reading_and_since_when() {
        let mut shown = shown(a_snapshot());
        let reading_atlas = reading(atlas(), an_instant());

        shown.collecting(std::slice::from_ref(&reading_atlas));
        assert_eq!(shown.collecting, [reading_atlas]);

        shown.collecting(&[]);
        assert_eq!(shown.collecting, []);
    }

    /// The loop draws on a change and this is what it asks. Told again what
    /// it already says, the screen is no different and a redraw would put
    /// the same frame back — which a collection queued behind another does
    /// whenever the two name the same projects.
    #[test]
    fn a_screen_told_again_what_it_already_says_reports_no_change() {
        let mut shown = shown(a_snapshot());
        let at = an_instant();

        assert!(
            shown.collecting(&[reading(atlas(), at)]),
            "None to one project"
        );
        assert!(
            !shown.collecting(&[reading(atlas(), at)]),
            "the same collection again"
        );
        assert!(
            shown.collecting(&[reading(ferry(), at)]),
            "one project to another"
        );
        assert!(shown.collecting(&[]), "and back to nothing running");
        assert!(!shown.collecting(&[]), "which is also said only once");
    }

    /// A fresh collection of the projects the last one named is not the same
    /// collection, whatever it names: its wait starts over, and a line that
    /// went on counting from the first ask would report the tracker as having
    /// stopped answering seconds into a collection that had only just begun.
    #[test]
    fn a_screen_told_of_a_new_collection_of_the_same_projects_reports_a_change() {
        let mut shown = shown(a_snapshot());
        let at = an_instant();

        assert!(shown.collecting(&[reading(atlas(), at)]));
        assert!(shown.collecting(&[reading(atlas(), at + chrono::TimeDelta::seconds(1))]));
    }

    /// `codex review` on this change, and it is right: the foot said
    /// `collected 10:21:44`, which is true for as long as it is on the
    /// screen. An age is not. A `bdi` whose projects are all reported for
    /// polls nothing, so with no work moving it would sit saying `0s ago`
    /// for as long as the reader left it.
    #[test]
    fn a_screen_at_rest_over_a_read_goes_stale_and_says_when() {
        let mut snapshot = a_snapshot();
        let read = an_instant();
        snapshot.read_at.insert("orbital".to_string(), read);

        let shown = shown(snapshot);

        assert_eq!(
            shown.holds_for(read + chrono::TimeDelta::milliseconds(250)),
            Some(Duration::from_millis(750)),
            "the second it is saying is three quarters over"
        );
    }

    /// The soonest of them, because the newest read is the one whose words
    /// change first and the line must not be wrong in between.
    #[test]
    fn a_screen_holds_only_as_long_as_its_newest_read_does() {
        let mut snapshot = a_snapshot();
        let read = an_instant();
        snapshot.read_at.insert("ferry".to_string(), read);
        snapshot
            .read_at
            .insert("orbital".to_string(), read - chrono::TimeDelta::hours(2));

        let shown = shown(snapshot);

        assert_eq!(
            shown.holds_for(read + chrono::TimeDelta::seconds(4)),
            Some(Duration::from_secs(1)),
            "the four-second-old read, not the two-hour-old one"
        );
    }

    /// With nothing read there is no age on the screen, so the mark turning
    /// is the whole of what expires.
    #[test]
    fn a_screen_with_a_collection_on_it_and_no_age_holds_for_one_frame() {
        let mut shown = shown(a_snapshot());
        shown.collecting(&[reading(atlas(), an_instant())]);

        assert_eq!(shown.holds_for(an_instant()), Some(phrase::FRAME));
    }

    /// The other end of the same case, and the one a reviewer reads as the
    /// deadline failing to fire: a first collection that never answers leaves
    /// a still mark over no rows, and there is nothing in that cell for time
    /// alone to falsify. What carried the screen across the deadline was the
    /// turning mark, which was still running up to it.
    #[test]
    fn a_screen_whose_first_collection_stopped_answering_has_nothing_left_to_expire() {
        let mut shown = shown(a_snapshot());
        let asked_at = an_instant();
        shown.collecting(&[reading(atlas(), asked_at)]);

        assert_eq!(
            shown.holds_for(asked_at + chrono::TimeDelta::seconds(30)),
            None
        );
    }

    /// And with rows on the screen, a hung tracker comes off the frame clock
    /// and onto the age's. The mark has stopped moving, so redrawing it 12
    /// times a second for however long the tracker stays quiet would be
    /// wakeups for a screen that cannot change.
    #[test]
    fn a_hung_tracker_is_redrawn_for_its_rows_age_rather_than_for_a_still_mark() {
        let mut snapshot = a_snapshot();
        let read = an_instant();
        snapshot.read_at.insert("atlas".to_string(), read);
        let mut shown = shown(snapshot);
        shown.collecting(&[reading(atlas(), read)]);

        assert_eq!(
            shown.holds_for(read + chrono::TimeDelta::seconds(30)),
            Some(Duration::from_secs(1)),
            "the age turning over, not the frame the mark has stopped on"
        );
    }

    /// `bdi-7ao.58`: the age stays on the line under the turning mark, so it
    /// goes on expiring under it. A screen that held for a whole frame
    /// whenever a collection ran would leave `0s ago` on it into its second
    /// second — the mark turning the whole time and saying nothing about the
    /// age beside it.
    #[test]
    fn a_collection_in_flight_does_not_stop_the_ages_beneath_it_running_out() {
        let mut snapshot = a_snapshot();
        let read = an_instant();
        snapshot.read_at.insert("atlas".to_string(), read);
        let mut shown = shown(snapshot);
        shown.collecting(&[reading(atlas(), read)]);

        assert_eq!(
            shown.holds_for(read + chrono::TimeDelta::milliseconds(970)),
            Some(Duration::from_millis(30)),
            "the age turns over inside the frame the mark is on"
        );
    }

    /// And the other way: an age that is not going to change for another day
    /// leaves the turning mark deciding when the screen is next due.
    #[test]
    fn a_collection_over_day_old_rows_is_redrawn_for_the_mark_rather_than_the_age() {
        let mut snapshot = a_snapshot();
        let read = an_instant();
        let now = read + chrono::TimeDelta::seconds(86_400);
        snapshot.read_at.insert("atlas".to_string(), read);
        let mut shown = shown(snapshot);
        shown.collecting(&[reading(atlas(), now)]);

        assert_eq!(shown.holds_for(now), Some(phrase::FRAME));
    }

    /// Nothing read and nothing running: there is no age on the screen, so
    /// there is nothing for time alone to falsify and no reason to wake.
    #[test]
    fn a_screen_no_project_has_been_read_for_stays_true_however_long_it_is_left() {
        assert_eq!(shown(a_snapshot()).holds_for(an_instant()), None);
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
        let rows = screen_of(
            &mut shown.forest,
            &shown.tail,
            width,
            height,
            Showing::Forest,
        )
        .rows();
        rows[..bands.forest.height as usize].to_vec()
    }

    /// The pane on the row a staffed grove opens with.
    const A_SELECTED_PANE: &str = "w:p0";

    /// The same grove with a live agent on every bead, so that moving the
    /// selection changes which pane the tail is reading.
    fn a_staffed_grove(beads: usize) -> Snapshot {
        let mut snapshot = a_grove(beads);
        for tree in &mut snapshot.collected {
            let tree = Arc::make_mut(tree);
            for (at, node) in tree.beads.iter_mut().enumerate() {
                node.agent = Some(AgentRef {
                    pane: format!("w:p{at}"),
                    pane_status: PaneStatus::Working,
                    title: None,
                    source: JoinSource::AgentPane,
                });
            }
        }
        snapshot.trees = snapshot.collected.clone();
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

        let mut snapshot = Snapshot {
            collected: both,
            projects: vec!["grove".to_string(), "atlas".to_string()],
            ..a_snapshot_of(Vec::new())
        };
        snapshot.refilter(Filter::LiveAgents);
        snapshot
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

    /// The screen opens on what the band has to say while herdr is still
    /// answering, and asks for the reading rather than waiting on it. The
    /// startup path used to pay the whole of `PATIENCE` before the first
    /// frame, on a screen that had nothing to announce it with.
    #[test]
    fn the_screen_opens_naming_the_pane_it_is_waiting_on() {
        let (shown, panes) = shown_asking(a_staffed_grove(6));

        assert_eq!(
            shown.tail,
            Tail::Reading {
                pane: A_SELECTED_PANE.to_string()
            }
        );
        assert_eq!(
            panes.reads(),
            [format!("{A_SELECTED_PANE} {}", tail::LINES)],
            "the pane is asked for exactly the lines the band has room for"
        );
    }

    /// A bead nobody is working names no pane, so there is nothing to ask
    /// herdr and the band says so on its own.
    #[test]
    fn a_row_with_no_pane_asks_herdr_nothing() {
        let (shown, panes) = shown_asking(a_grove(6));

        assert_eq!(shown.tail, Tail::Silent(phrase::no_agent_to_tail()));
        assert!(panes.reads().is_empty());
    }

    #[test]
    fn what_herdr_read_for_the_pane_selected_is_what_the_band_shows() {
        let (mut shown, _) = shown_asking(a_staffed_grove(6));

        assert!(shown.tailed(read(A_SELECTED_PANE, &["rebuilt .#thinkpad"])));

        assert_eq!(
            shown.tail,
            Tail::Pane {
                pane: A_SELECTED_PANE.to_string(),
                lines: vec!["rebuilt .#thinkpad".to_string()],
            }
        );
    }

    /// A reader holding an arrow key down moves faster than a slow herdr
    /// answers. Every move asks for the pane it landed on, but only one
    /// question is ever out, so what a run of keystrokes costs is one herdr
    /// call — not one per key — and the band goes on naming the pane it is
    /// waiting for.
    ///
    /// The tail falling behind the selection is the price of the keyboard
    /// never doing so, and it is a decision rather than an accident: the loop
    /// answers every one of these moves at once, and the rows under the
    /// forest arrive when the cursor rests long enough for an answer to land.
    #[test]
    fn moving_faster_than_herdr_answers_costs_one_read_at_a_time() {
        let (mut shown, panes) = shown_asking(a_staffed_grove(6));
        let opened_on = panes.reads();

        let moved = to_the_last_row(&mut shown);

        assert!(moved > 1, "the forest has rows to move down");
        assert_eq!(
            panes.reads(),
            opened_on,
            "a read was already out, so none of the moves asked for another"
        );
        let resting_on = match &shown.tail {
            Tail::Reading { pane } => pane.clone(),
            other => panic!("the band is still waiting on a pane: {other:?}"),
        };
        assert_ne!(
            resting_on, A_SELECTED_PANE,
            "the selection left the pane the read is out for"
        );

        // The answer to the read started before any of that arrives, about a
        // pane the reader is nowhere near.
        assert!(
            !shown.tailed(read(A_SELECTED_PANE, &["nothing should reach the screen"])),
            "an answer about a pane the selection has left changes no screen"
        );

        assert_eq!(
            shown.tail,
            Tail::Reading {
                pane: resting_on.clone()
            },
            "the band still names the pane the cursor is on"
        );
        assert_eq!(
            panes.reads(),
            [opened_on, vec![format!("{resting_on} {}", tail::LINES)]].concat(),
            "and that pane is what herdr is asked for next"
        );
    }

    /// The refresh tick is when the pane is re-read, and a read already out
    /// when the collection lands was asked for before it. Taking that one as
    /// the refresh's reading would draw the pane as it was rather than as it
    /// is, and ask for nothing further until the tick after — which on a
    /// herdr slow enough to overlap every tick is a band that stops keeping
    /// up altogether while the cursor sits still.
    #[test]
    fn a_collection_landing_mid_read_reads_the_pane_again() {
        let grove = a_staffed_grove(6);
        let (mut shown, panes) = shown_asking(grove.clone());
        let opened_on = panes.reads();

        shown.collected(grove);
        assert_eq!(
            panes.reads(),
            opened_on,
            "a read was already out, so the refresh asked for no second one"
        );

        assert!(
            !shown.tailed(read(
                A_SELECTED_PANE,
                &["what the pane said before the refresh"]
            )),
            "the answer to a question asked before the refresh is not the refresh's answer"
        );
        assert_eq!(
            shown.tail,
            Tail::Reading {
                pane: A_SELECTED_PANE.to_string()
            }
        );
        assert_eq!(
            panes.reads(),
            [
                opened_on,
                vec![format!("{A_SELECTED_PANE} {}", tail::LINES)]
            ]
            .concat(),
            "so the pane is asked for again once herdr is free"
        );

        assert!(shown.tailed(read(A_SELECTED_PANE, &["what it says now"])));
        assert_eq!(
            shown.tail,
            Tail::Pane {
                pane: A_SELECTED_PANE.to_string(),
                lines: vec!["what it says now".to_string()],
            }
        );
    }

    /// The read the answer above asked for lands where the cursor is resting,
    /// so a tail that falls behind catches up rather than stopping.
    #[test]
    fn the_tail_catches_up_once_the_cursor_rests() {
        let (mut shown, _) = shown_asking(a_staffed_grove(6));
        to_the_last_row(&mut shown);
        let resting_on = match &shown.tail {
            Tail::Reading { pane } => pane.clone(),
            other => panic!("the band is still waiting on a pane: {other:?}"),
        };

        shown.tailed(read(A_SELECTED_PANE, &["about the pane left behind"]));
        assert!(shown.tailed(read(&resting_on, &["about the pane rested on"])));

        assert_eq!(
            shown.tail,
            Tail::Pane {
                pane: resting_on,
                lines: vec!["about the pane rested on".to_string()],
            }
        );
    }

    /// `⏎` asks herdr for the pane and nothing on this screen changes for it:
    /// what changes is which terminal pane is in front of the reader.
    #[test]
    fn enter_asks_for_the_pane_and_draws_nothing() {
        let (mut shown, panes) = shown_asking(a_staffed_grove(6));

        assert!(!shown.apply(Action::Focus));

        assert_eq!(panes.focuses(), [A_SELECTED_PANE]);
    }

    #[test]
    fn a_pane_that_will_not_come_to_the_front_says_so_where_the_tail_is() {
        let (mut shown, _) = shown_asking(a_staffed_grove(6));

        assert!(shown.tailed(refused(A_SELECTED_PANE)));

        assert_eq!(
            shown.tail,
            Tail::Silent(phrase::pane_unreadable(FailureKind::Gone))
        );
    }

    /// The rule above the band names the pane the band is about, so a refusal
    /// about a pane the reader has left would read as a refusal about the one
    /// they are on. It is dropped instead.
    #[test]
    fn a_focus_refused_for_a_pane_the_reader_has_left_is_not_drawn_over_the_one_they_are_on() {
        let (mut shown, _) = shown_asking(a_staffed_grove(6));
        assert!(shown.apply(Action::Move(Motion::NextRow)));
        let was = shown.tail.clone();

        assert!(!shown.tailed(refused(A_SELECTED_PANE)));

        assert_eq!(shown.tail, was);
    }

    #[test]
    fn a_frame_puts_the_tail_in_the_band_reserved_for_it() {
        let mut forest = an_open_grove(30);
        let tail = Tail::Pane {
            pane: "w:p1".to_string(),
            lines: vec!["rebuilt .#thinkpad".to_string()],
        };

        let rows = screen_of(&mut forest, &tail, 40, 12, Showing::Forest).rows();
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
