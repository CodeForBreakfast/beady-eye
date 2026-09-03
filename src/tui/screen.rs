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
use ratatui::layout::Rect;
use ratatui::{DefaultTerminal, Frame};

use crate::app::Awaited;
use crate::collect::panes::{Answer, Panes};
use crate::model::join::BeadKey;
use crate::model::snapshot::Snapshot;
use crate::view::bindings::key_bindings;
use crate::view::forest::{self, Forest};
use crate::view::phrase;
use crate::view::show::{self, Show};
use crate::view::tail::{self, Tail};
use crate::view::{draw, Action, Freshness, Motion, Notice};

use super::clipboard;
use super::drive::{Showing, View};
use super::due::due_after;
use super::keys::{bindings, key_row};
use super::reload::Reloaded;

/// The forest on the alternate screen and the tail beneath it.
///
/// Held apart from the terminal that draws it because the terminal needs a
/// tty and none of this does.
struct Shown {
    forest: Forest,
    panes: Box<dyn Panes>,
    /// Where the terminal's clipboard is written: the terminal itself, by
    /// escape sequence, so the write travels the way every other byte `bdi`
    /// puts on the screen does and needs no program outside it.
    clipboard: Box<dyn io::Write>,
    tail: Tail,
    /// The pane the band on screen is about, so a selection moving within it
    /// does not spend a call on the provider for the answer already drawn.
    tailing: Option<String>,
    /// Where the band is with the pane read it is waiting on.
    reading: Reading,
    /// How long after the provider answers the pane on the band is asked for
    /// again.
    every: Duration,
    /// When the pane on the band is next asked for, or nothing while a read
    /// of it is out, the band names no pane, or the interval is too long to
    /// reach. Armed by the answer that filled the band and disarmed by the
    /// ask it makes, as a project is.
    due: Option<DateTime<Utc>>,
    /// The id the reader has just put on the clipboard, said at the foot
    /// until their next key or click. Theirs and not the collector's: a
    /// collection landing a moment after the key would otherwise take the
    /// one line that says it fired off before anyone had read it.
    copied: Option<String>,
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
    /// Where the bead view is looking, while one is up. Held here and not on
    /// the forest because it is about a window over the rows, not the rows.
    show: Show,
    /// The bead the view was opened on, so a collection that moves the
    /// selection off it can be told from one that leaves it there.
    viewing: Option<BeadKey>,
    /// What this run of `bdi` cannot do, said at the foot until it can.
    ///
    /// Seeded with what was settled before the first collection, which holds
    /// for the session. A config that will not reload joins them and leaves
    /// again, which is why these are held here rather than beside the
    /// terminal: what the foot says is a fact about the view, and a fact
    /// nothing without a tty could reach would be a fact no test could
    /// either.
    standing: Vec<Notice>,
}

/// Where the band under the forest is with the read it is waiting on.
///
/// At most one is ever out, so what a reader moving faster than the provider
/// answers costs is a run of answers dropped rather than a call per keystroke.
/// An answer is the answer to what was drawn when it was asked for, and what
/// is drawn moves on when the cursor lands on another pane.
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
    fn of(
        snapshot: Snapshot,
        panes: Box<dyn Panes>,
        clipboard: Box<dyn io::Write>,
        every: Duration,
        at_startup: Vec<Notice>,
    ) -> Self {
        let forest = forest::flatten(snapshot);
        let mut shown = Self {
            tail: tail::tail(&forest),
            tailing: tail::target(&forest).pane().map(str::to_string),
            forest,
            panes,
            clipboard,
            reading: Reading::Nothing,
            every,
            due: None,
            copied: None,
            // Nothing in flight until the loop says otherwise. A collection
            // is already running by the time this exists — the run asks for
            // one before it opens the screen — and it reaches this the way
            // every one after it does, through `collecting`.
            collecting: Vec::new(),
            show: Show::default(),
            viewing: None,
            standing: at_startup,
        };
        // Asked for here rather than waited for: the first frame is drawn on
        // the answer to this arriving, not on the provider getting round to it.
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

    /// Put the band on whatever the selection is on now, and ask the provider for
    /// the pane where it is on one.
    ///
    /// Whatever the provider is already answering was asked for the band this
    /// replaces, so it is superseded here, and a read the band it replaces
    /// was due is not due any more.
    fn retail(&mut self) {
        self.tailing = tail::target(&self.forest).pane().map(str::to_string);
        self.tail = tail::tail(&self.forest);
        if self.reading == Reading::Outstanding {
            self.reading = Reading::Superseded;
        }
        self.due = None;
        self.ask();
    }

    /// Ask the provider for the pane the band is waiting on, where it is
    /// waiting on one and it has not been asked already.
    ///
    /// A reader holding an arrow key down moves faster than a slow provider
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
        self.read(pane.clone());
    }

    /// Ask for the pane on the band again where its interval is up. The
    /// rows on the band stand until the answer lands: a band that said it
    /// was reading the pane every time it did would say nothing else.
    fn reread(&mut self, now: DateTime<Utc>) {
        if self.reading != Reading::Nothing || !self.due.is_some_and(|due| due <= now) {
            return;
        }
        if let Some(pane) = self.tailing.clone() {
            self.read(pane);
        }
    }

    /// Ask the provider for a pane. Nothing is due while the answer is on its
    /// way;
    /// what arms the next read is that answer landing.
    fn read(&mut self, pane: String) {
        self.panes.read(&pane, tail::LINES);
        self.reading = Reading::Outstanding;
        self.due = None;
    }

    /// How long until the pane on the band is asked for again, or nothing
    /// where it is not going to be: no pane on the band, a read still out,
    /// or an interval too long to reach.
    fn rereads_in(&self, now: DateTime<Utc>) -> Option<Duration> {
        self.due
            .map(|due| (due - now).to_std().unwrap_or(Duration::ZERO))
    }

    /// Take what a check of the config file found, reporting whether the
    /// screen is any different for it.
    ///
    /// The notice is about whether the file loaded, and nothing else: a read
    /// that brought a config identical to the one in force is a read that
    /// worked, and takes the notice off exactly as one bringing a new config
    /// does. Tying it to whether the config *changed* would leave a reader
    /// who undid a broken edit looking at a foot that still said their
    /// config was broken, with no edit left that would clear it but one
    /// changing the config to something they did not want.
    fn reloaded(&mut self, reloaded: Reloaded) -> bool {
        let broken = match reloaded {
            Reloaded::Untouched => return false,
            Reloaded::Unchanged | Reloaded::Fresh => false,
            Reloaded::Broken => true,
        };
        let said = self.standing.contains(&Notice::ConfigWouldNotReload);
        if said == broken {
            return false;
        }
        if broken {
            self.standing.push(Notice::ConfigWouldNotReload);
        } else {
            self.standing
                .retain(|notice| *notice != Notice::ConfigWouldNotReload);
        }
        true
    }

    /// Take what the provider said, reporting whether the screen is any different
    /// for it.
    fn tailed(&mut self, answer: Answer, now: DateTime<Utc>) -> bool {
        match answer {
            Answer::Read { pane, read } => {
                let superseded = self.reading == Reading::Superseded;
                self.reading = Reading::Nothing;
                if superseded {
                    self.ask();
                    return false;
                }
                let read = tail::read(pane, read);
                let changed = read != self.tail;
                self.tail = read;
                self.due = due_after(now, self.every);
                changed
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

    /// Put the selection on the line the forest drew on one row of `screen`.
    ///
    /// Which line a row holds is a fact about the frame rather than about the
    /// click — the bands divide the screen, and the forest scrolls under its
    /// own — so the geometry is asked here, where the forest is, and the
    /// screen is all the terminal has to say about it.
    fn clicked(&mut self, screen: Rect, row: u16) -> bool {
        let bands = draw::regions(screen);

        match draw::line_at(
            bands.forest,
            self.forest.selected_line(),
            self.forest.lines().len(),
            row,
        ) {
            Some(at) => self.select(at),
            // The tail is an echo of a pane and the key row is a legend.
            // Neither holds anything the selection could sit on, and a row
            // past the last line of the forest holds nothing at all.
            None => false,
        }
    }

    /// Take a copied id off the foot, reporting whether one was on it. The
    /// reader's next press is what takes it off, whatever the press turns
    /// out to mean — it was feedback on a keystroke, not a fact about the
    /// screen.
    fn pressed(&mut self) -> bool {
        self.copied.take().is_some()
    }

    fn collected(&mut self, snapshot: Snapshot) {
        // A refresh keeps the folds and the selection, so the cursor stays on
        // the bead the user put it on however the new snapshot has moved it.
        self.forest.refresh(snapshot);
        // The pane is on the band's own clock. What a collection can do to
        // the band is move the selection off the pane it is showing.
        self.follow();
    }

    fn apply(&mut self, action: Action) -> bool {
        // Asking for the selected bead's pane to be brought to the front
        // changes nothing on this screen, and a row with no pane is a no-op:
        // there is nothing to focus and nothing has gone wrong. What the
        // provider makes of it arrives as an answer like a reading does.
        if action == Action::Focus {
            tail::focus(&self.forest, self.panes.as_ref());
            return false;
        }
        // Showing a bead changes the screen where there is one to show, and
        // starts from its top: the view a reader scrolled to the end of was
        // another bead's. A row with no bead behind it changes nothing.
        if action == Action::ShowBead {
            let opened = show::selected(&self.forest).is_some();
            if opened {
                self.show = Show::default();
                self.viewing = self.selected_bead().cloned();
            }
            return opened;
        }
        if action == Action::CopyId {
            return self.copy_id();
        }

        let changed = self.forest.apply(action);
        self.moved(changed)
    }

    /// Move the bead view, leaving the selection under it where it is.
    fn scroll(&mut self, motion: Motion) -> bool {
        self.show.scroll(motion)
    }

    /// Whether the selection is still on the bead the view was opened on.
    fn bead_still_shown(&self) -> bool {
        self.viewing.is_some() && self.viewing.as_ref() == self.selected_bead()
    }

    /// Put the selected bead's id on the terminal's clipboard and say so at
    /// the foot, where the selection is on a bead. Most rows are beads; on one
    /// that is not there is nothing to copy and nothing has gone wrong, as
    /// with `f`. A write the terminal would not take is not said either:
    /// the foot would be the one line on the screen that was untrue.
    fn copy_id(&mut self) -> bool {
        let Some(key) = self.selected_bead() else {
            return false;
        };
        let id = key.id.clone();
        if clipboard::copy(self.clipboard.as_mut(), &id).is_err() {
            return false;
        }
        self.copied = Some(id);
        true
    }

    /// The bead the selection is on: a bead's own row, or a tree's header,
    /// which carries its root.
    fn selected_bead(&self) -> Option<&BeadKey> {
        self.forest.lines().get(self.forest.selected_line())?.bead()
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
}

impl Screen {
    pub(super) fn showing(
        snapshot: Snapshot,
        panes: Box<dyn Panes>,
        at_startup: Vec<Notice>,
        tail_every: Duration,
    ) -> anyhow::Result<Self> {
        let terminal = ratatui::try_init()?;
        // Built before the mouse is asked for, so that a terminal which
        // refuses is still put back by the `Drop` this now has.
        let screen = Self {
            terminal,
            shown: Shown::of(
                snapshot,
                panes,
                Box::new(io::stdout()),
                tail_every,
                at_startup,
            ),
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
/// it has, and the window over it when one is up — the bindings, or a bead.
///
/// Outside the `terminal.draw` closure so a test backend can drive the whole
/// frame. This is the only place the three bands are agreed on, and `^D` and
/// `^U` are the part of that agreement nothing on screen would show was
/// broken. A window goes on last because it sits over the forest rather
/// than in place of it.
fn paint(
    frame: &mut Frame,
    forest: &mut Forest,
    tail: &Tail,
    over: Over<'_>,
    foot: draw::Foot,
    collecting: &[Awaited],
    now: DateTime<Utc>,
) {
    let bands = draw::regions(frame.area());
    forest.set_half_screen(draw::half_screen(bands.forest));
    draw::draw(frame, frame.area(), forest, collecting, now, foot);
    draw::draw_tail(frame, bands.tail, tail);
    match over {
        Over::Nothing => {}
        Over::Bindings => key_bindings(frame, frame.area(), &bindings()),
        Over::Bead(show) => {
            if let Some(node) = show::selected(forest) {
                show::show(frame, frame.area(), node, show);
            }
        }
    }
}

/// The window over the forest, where one is up, with what drawing it needs:
/// the loop's `Showing`, met by the view the screen holds for it.
enum Over<'a> {
    Nothing,
    Bindings,
    Bead(&'a mut Show),
}

#[cfg(panic = "abort")]
compile_error!(
    "putting the terminal back is `Screen`'s `Drop`, and a build with \
     `panic = \"abort\"` runs no `Drop`: a panic under it leaves the alternate \
     screen up, the terminal in raw mode, and the mouse reporting every move"
);

impl Drop for Screen {
    fn drop(&mut self) {
        // Ahead of the restore, mirroring the order they were turned on in.
        // A terminal left reporting the mouse writes an escape sequence into
        // whatever runs next for every cell the pointer crosses, and there
        // is nothing left running to ask it to stop.
        //
        // Every way the run can end comes through here — the loop returning
        // on 'q', a panic unwinding, and a signal, which the loop answers by
        // returning.
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

    fn tailed(&mut self, answer: Answer, now: DateTime<Utc>) -> bool {
        self.shown.tailed(answer, now)
    }

    fn reread(&mut self, now: DateTime<Utc>) {
        self.shown.reread(now);
    }

    fn reloaded(&mut self, reloaded: Reloaded) -> bool {
        self.shown.reloaded(reloaded)
    }

    fn rereads_in(&self, now: DateTime<Utc>) -> Option<Duration> {
        self.shown.rereads_in(now)
    }

    fn pressed(&mut self) -> bool {
        self.shown.pressed()
    }

    fn apply(&mut self, action: Action) -> bool {
        self.shown.apply(action)
    }

    fn scroll(&mut self, motion: Motion) -> bool {
        self.shown.scroll(motion)
    }

    fn bead_still_shown(&self) -> bool {
        self.shown.bead_still_shown()
    }

    fn clicked(&mut self, row: u16) -> bool {
        let screen = self.terminal.get_frame().area();
        self.shown.clicked(screen, row)
    }

    fn draw(&mut self, showing: Showing, now: DateTime<Utc>) -> anyhow::Result<()> {
        let Shown {
            forest,
            tail,
            show,
            collecting,
            copied,
            standing,
            ..
        } = &mut self.shown;
        let over = match showing {
            Showing::Forest => Over::Nothing,
            Showing::Bindings => Over::Bindings,
            Showing::Bead => Over::Bead(show),
        };
        let foot = draw::Foot {
            standing,
            copied: copied.as_deref(),
            keys: &key_row(),
        };
        self.terminal
            .draw(|frame| paint(frame, forest, tail, over, foot, collecting, now))?;
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
    use crate::model::snapshot::{
        a_provider, Counts, Filter, Node, ProviderState, TrackerState, Tree,
    };
    use crate::model::tree::Link;
    use crate::model::types::{Edge, PaneStatus, Status};
    use crate::tui::fixtures::{a_snapshot, atlas, ferry, reading, PATIENCE};
    use crate::tui::keys::tests::key;
    use crate::tui::keys::{action, BINDINGS};
    use crate::view::bindings::bindings_window;
    use crate::view::painted::{Painted, Run};
    use crate::view::walk::{self, Rows};
    use crate::view::Motion;
    use base64::prelude::{Engine as _, BASE64_STANDARD};
    use chrono::Utc;
    use ratatui::crossterm::event::KeyCode;
    use ratatui::layout::Rect;
    use ratatui::style::{Color, Modifier};
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
            Over::Bindings,
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

    /// A run that found everything it looked for at startup, so the foot
    /// begins with nothing on it but the keys.
    fn nothing_said() -> Vec<Notice> {
        Vec::new()
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
                "  Enter     show the selected bead, or focus its pane from the bead view",
                "  f         focus the selected bead's pane",
                "  Space     fold or unfold the selected node",
                "  a         show every tree, not only those with a live agent",
                "  ?         show these key bindings",
                "  … 15 more bindings · no room on a screen this short",
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
                Over::Bindings,
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
        let alone = screen_of(&mut forest, &tail, 80, 24, Over::Nothing).rows();
        let over = screen_of(&mut forest, &tail, 80, 24, Over::Bindings).rows();
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
                "  Enter     show the selected bead, or focus its pane from the bead view",
                "  f         focus the selected bead's pane",
                "  Space     fold or unfold the selected node",
                "  a         show every tree, not only those with a live agent",
                "  ?         show these key bindings",
                "  q, ^C     quit",
                "  Esc       go back to the forest from the bead view",
                "  ^R        collect from the trackers again now",
                "  E         expand the selected node and everything under it",
                "  C         collapse the selected node and everything under it",
                "  D         restore the default view",
                "  y         copy the selected bead's id to the clipboard",
                "  Down, j   move down one row",
                "  Up, k     move up one row",
                "  Right, l  expand, or move to the first child when it is already expanded",
                "  Left, h   collapse, or move to the parent when it is already collapsed",
                "  ^D, PgDn  move down half a screen",
                "  ^U, PgUp  move up half a screen",
                "  Home, g   move to the first row",
                "  End, G    move to the last row",
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

        assert_eq!(drawn[5], "  q, ^C     quit");
        assert_eq!(
            drawn[0], "  Enter     show the selected bead, o…",
            "a line too long for forty columns, cut with the cut marked"
        );
        assert_eq!(drawn[14], "  Right, l  expand, or move to the fi…");
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
            Over::Nothing,
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
            description: String::new(),
            notes: String::new(),
            owner: None,
            parent: None,
            depends_on: Vec::new(),
            blocks: Vec::new(),
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
            agents: a_provider(ProviderState::Answering),
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

    /// One frame, drawn as `paint` draws it, on a session with nothing to
    /// say at its foot but the keys and whatever the reader just copied.
    fn painted(
        forest: &mut Forest,
        tail: &Tail,
        over: Over<'_>,
        copied: Option<&str>,
        collecting: &[Awaited],
        width: u16,
        height: u16,
    ) -> Painted {
        let keys = key_row();
        let foot = draw::Foot {
            standing: &[],
            copied,
            keys: &keys,
        };
        Painted::drawn_by(width, height, |frame| {
            paint(frame, forest, tail, over, foot, collecting, an_instant());
        })
    }

    fn screen_of(
        forest: &mut Forest,
        tail: &Tail,
        width: u16,
        height: u16,
        over: Over<'_>,
    ) -> Painted {
        painted(forest, tail, over, None, &[], width, height)
    }

    /// The screen with the bead view up, drawn from the view the screen is
    /// holding rather than a fresh one, so a motion between two draws is a
    /// motion of the same view.
    fn bead_view(shown: &mut Shown, width: u16, height: u16) -> Vec<String> {
        let (forest, tail, show) = (&mut shown.forest, &shown.tail, &mut shown.show);
        screen_of(forest, tail, width, height, Over::Bead(show)).rows()
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
        painted(forest, tail, Over::Nothing, None, collecting, width, height)
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
            Over::Nothing,
        )
        .rows();
        forest.apply(Action::Move(Motion::HalfScreenDown));

        assert_eq!(
            forest.selected_line(),
            9,
            "half of the sixteen rows the forest was given, not half the frame"
        );
    }

    /// The provider, remembering what it was asked and answering nothing.
    ///
    /// Nothing here answers, because nothing in `Shown` waits for an answer:
    /// what the provider said arrives through `tailed`, and a test hands it one
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

    /// What the provider said about a pane it read.
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

    /// Press the key at `code` the way the loop presses it: through the
    /// table, so a test here fails when the key is bound to nothing.
    fn press(shown: &mut Shown, code: KeyCode) -> bool {
        let asked = action(key(code)).unwrap_or_else(|| panic!("{code:?} is bound to nothing"));
        shown.apply(asked)
    }

    /// Home from the bottom of a grove taller than the screen: the selection
    /// is on the first row and the band has scrolled back up to draw it.
    #[test]
    fn home_puts_the_selection_on_the_first_row_from_anywhere() {
        let mut shown = shown(a_grove(30));
        to_the_last_row(&mut shown);
        let root = |band: &[String]| band.iter().any(|row| row.contains("grv-1 "));
        assert!(
            !root(&forest_band(&mut shown, 60, 24)),
            "the fixture has to scroll the root off the screen first"
        );

        assert!(press(&mut shown, KeyCode::Home));

        assert_eq!(shown.forest.selected_line(), 0);
        let band = forest_band(&mut shown, 60, 24);
        assert!(root(&band), "{band:#?}");
    }

    /// End from partway down: the selection is on the last row and the band
    /// has scrolled down to draw it.
    #[test]
    fn end_puts_the_selection_on_the_last_row_from_anywhere() {
        let mut shown = shown(a_grove(30));
        let rows = shown.rows();
        walk::until(
            &mut shown,
            |shown| shown.forest.selected_line() >= 3,
            |shown| {
                shown.apply(Action::Move(Motion::NextRow));
            },
            |shown| format!("stopped at row {}", shown.forest.selected_line()),
        );
        let last = |band: &[String]| band.iter().any(|row| row.contains(" .30 "));
        assert!(
            !last(&forest_band(&mut shown, 60, 24)),
            "the fixture has to start with the last row off the screen"
        );

        assert!(press(&mut shown, KeyCode::End));

        assert_eq!(shown.forest.selected_line() + 1, rows);
        let band = forest_band(&mut shown, 60, 24);
        assert!(last(&band), "{band:#?}");
    }

    /// A selection already at the end it was sent to stays where it is.
    #[test]
    fn home_and_end_move_nothing_when_the_selection_is_already_there() {
        let mut shown = shown(a_grove(6));
        press(&mut shown, KeyCode::Home);
        let at_home = shown.forest.selected_line();

        assert!(!press(&mut shown, KeyCode::Home));
        assert_eq!(shown.forest.selected_line(), at_home);

        press(&mut shown, KeyCode::End);
        let at_end = shown.forest.selected_line();

        assert!(!press(&mut shown, KeyCode::End));
        assert_eq!(shown.forest.selected_line(), at_end);
    }

    /// The provider refusing to bring a pane to the front.
    fn refused(pane: &str) -> Answer {
        Answer::Focused {
            pane: pane.to_string(),
            focused: Err(RunFailure {
                kind: FailureKind::Gone,
                program: "a provider".to_string(),
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

    /// How long the band waits after an answer before asking for its pane
    /// again, for every screen here. Never slept: the tests that are about
    /// it hand the screen instants a chosen distance apart.
    const EVERY: Duration = Duration::from_millis(250);

    fn shown(snapshot: Snapshot) -> Shown {
        Shown::of(
            snapshot,
            Box::new(Asking::default()),
            Box::new(io::sink()),
            EVERY,
            nothing_said(),
        )
    }

    /// The same, with the record of what the provider was asked kept beside it.
    fn shown_asking(snapshot: Snapshot) -> (Shown, Asking) {
        shown_asking_every(snapshot, EVERY)
    }

    /// The same again, waiting a gap of the caller's choosing rather than the
    /// one every other screen here waits.
    fn shown_asking_every(snapshot: Snapshot, every: Duration) -> (Shown, Asking) {
        let panes = Asking::default();
        (
            Shown::of(
                snapshot,
                Box::new(panes.clone()),
                Box::new(io::sink()),
                every,
                nothing_said(),
            ),
            panes,
        )
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
        let rows = screen_of(&mut shown.forest, &shown.tail, width, height, Over::Nothing).rows();
        rows[..bands.forest.height as usize].to_vec()
    }

    /// The pane on the row a staffed grove opens with.
    const A_SELECTED_PANE: &str = "w:p0";

    /// The same grove with every bead saying something of itself, so the
    /// bead view has something to show that no row of the forest carries.
    fn a_described_grove(beads: usize) -> Snapshot {
        let mut snapshot = a_grove(beads);
        for tree in &mut snapshot.collected {
            let tree = Arc::make_mut(tree);
            for node in &mut tree.beads {
                node.description = format!("what {} is about", node.id);
            }
        }
        snapshot.trees = snapshot.collected.clone();
        snapshot
    }

    /// The rows inside the bead window's border, trimmed, with the forest
    /// around it left out.
    fn bead_window_inner(shown: &mut Shown, width: u16, height: u16) -> Vec<String> {
        let rows = bead_view(shown, width, height);
        let window = window_of(&rows);
        let (x, y) = (window.x as usize, window.y as usize);
        rows[y + 1..y + window.height as usize - 1]
            .iter()
            .map(|row| {
                row.chars()
                    .skip(x + 1)
                    .take(window.width as usize - 2)
                    .collect::<String>()
                    .trim()
                    .to_string()
            })
            .collect()
    }

    /// The bead window's rectangle, read off the screen by its corners.
    fn bead_window(shown: &mut Shown, width: u16, height: u16) -> Rect {
        window_of(&bead_view(shown, width, height))
    }

    /// The bead window's rectangle on the screen, by its corners: the title
    /// row's `┌` and `┐`, and the `└` beneath the first in the rows below.
    /// The forest's own `└──` connectors sit elsewhere on their rows.
    fn window_of(rows: &[String]) -> Rect {
        let y = rows
            .iter()
            .position(|row| row.contains("Esc to go back"))
            .expect("the bead window's title is on the screen");
        let top: Vec<char> = rows[y].chars().collect();
        let x = top.iter().position(|c| *c == '┌').unwrap();
        let right = top.iter().rposition(|c| *c == '┐').unwrap();
        let bottom = rows[y + 1..]
            .iter()
            .position(|row| row.chars().nth(x) == Some('└'))
            .map(|n| y + 1 + n)
            .expect("the bead window's bottom edge is on the screen");
        Rect::new(
            x as u16,
            y as u16,
            (right - x + 1) as u16,
            (bottom - y + 1) as u16,
        )
    }

    /// A grove whose first bead's description runs to `lines` lines, so the
    /// bead is taller than any window a screen in these tests offers.
    fn a_grove_with_a_tall_bead(lines: usize) -> Snapshot {
        let mut snapshot = a_described_grove(6);
        let tree = Arc::make_mut(&mut snapshot.collected[0]);
        tree.beads[0].description = (1..=lines)
            .map(|n| format!("line {n} of the description"))
            .collect::<Vec<_>>()
            .join("\n");
        snapshot.trees = snapshot.collected.clone();
        snapshot
    }

    /// A grove whose first bead's description carries a heading, an item, a
    /// code span and emphasis.
    fn a_grove_with_a_marked_up_bead() -> Snapshot {
        let mut snapshot = a_described_grove(6);
        let tree = Arc::make_mut(&mut snapshot.collected[0]);
        tree.beads[0].description = "## Shape\n\n- keep `wrap` *soft*".to_string();
        snapshot.trees = snapshot.collected.clone();
        snapshot
    }

    /// `bdi-2bb.45`: the description is rendered as markdown in the window's
    /// own styling — a heading bold, an item behind a bullet, a code span in
    /// a tone of its own, emphasis italic — as the screen paints it.
    #[test]
    fn the_bead_window_styles_the_descriptions_markdown() {
        let mut shown = shown(a_grove_with_a_marked_up_bead());
        assert!(shown.apply(Action::ShowBead));
        let (forest, tail, show) = (&mut shown.forest, &shown.tail, &mut shown.show);
        let painted = screen_of(forest, tail, 80, 24, Over::Bead(show));

        let runs: Vec<Run> = (0..24).flat_map(|y| painted.row(y)).collect();
        let run_of = |said: &str| {
            runs.iter()
                .find(|run| run.said == said)
                .unwrap_or_else(|| panic!("{said:?} is drawn in a run of its own: {runs:#?}"))
                .clone()
        };
        assert!(run_of("Shape").style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(run_of("wrap").style.fg, Some(Color::Cyan));
        assert!(run_of("soft").style.add_modifier.contains(Modifier::ITALIC));
        assert!(
            painted.rows().iter().any(|row| row.contains("• keep")),
            "{:#?}",
            painted.rows()
        );
    }

    /// `bdi-2bb.44`: the window follows the terminal rather than stopping at
    /// eighty columns. Both sides are four fifths of the screen's, centred, so
    /// a bigger terminal gets a bigger window and not the same box in the
    /// middle of a bigger forest.
    #[test]
    fn the_bead_window_is_four_fifths_of_the_screen_on_either_side() {
        let mut shown = shown(a_grove_with_a_tall_bead(100));
        assert!(shown.apply(Action::ShowBead));

        assert_eq!(bead_window(&mut shown, 120, 40), Rect::new(12, 4, 96, 32));
        assert_eq!(bead_window(&mut shown, 200, 60), Rect::new(20, 6, 160, 48));
    }

    /// A bead shorter than four fifths of the screen keeps a window its own
    /// height: the room is offered, not filled with nothing.
    #[test]
    fn a_short_bead_keeps_a_short_window_on_a_tall_screen() {
        let mut shown = shown(a_described_grove(6));
        assert!(shown.apply(Action::ShowBead));

        let window = bead_window(&mut shown, 200, 60);
        assert_eq!(window.width, 160);
        assert!(window.height < 48, "{window:?}");
        assert_eq!(
            bead_window_inner(&mut shown, 200, 60).len() + 2,
            window.height as usize,
            "the window is as tall as the bead and its border"
        );
    }

    /// A bead taller than four fifths of a big screen fills that height and
    /// scrolls for the rest, the title saying so, the same as on a small one.
    #[test]
    fn a_bead_taller_than_the_window_scrolls_by_motion_on_a_big_screen() {
        let mut shown = shown(a_grove_with_a_tall_bead(100));
        assert!(shown.apply(Action::ShowBead));

        let rows = bead_view(&mut shown, 200, 60);
        assert!(
            rows[6].contains("j, k to scroll"),
            "the title says the bead scrolls: {:?}",
            rows[6]
        );
        let top = bead_window_inner(&mut shown, 200, 60);
        assert_eq!(top.len(), 46);

        assert!(shown.scroll(Motion::NextRow));
        let down_one = bead_window_inner(&mut shown, 200, 60);
        assert_eq!(down_one[0], top[1]);

        assert!(shown.scroll(Motion::LastRow));
        let bottom = bead_window_inner(&mut shown, 200, 60);
        assert_eq!(bottom[45], "line 100 of the description");
        assert!(!shown.scroll(Motion::NextRow), "nothing below the last row");
    }

    /// The bead, with a bead row selected: Enter shows what `bd show` would
    /// say of it, from the rows `bdi` already holds — the fields the bead
    /// lists, and nothing a row of the forest could have carried.
    #[test]
    fn enter_on_a_bead_row_shows_the_bead() {
        let mut shown = shown(a_described_grove(6));
        assert_eq!(cursor(&shown), Some(&bead("grove", "grv-1")));

        assert!(shown.apply(Action::ShowBead));

        let inner = bead_window_inner(&mut shown, 80, 24);
        assert_eq!(inner[0], "◐ grv-1  a bead in the grove");
        assert!(inner.contains(&"DESCRIPTION".to_string()), "{inner:#?}");
        assert!(
            inner.contains(&"what grv-1 is about".to_string()),
            "{inner:#?}"
        );
    }

    /// A project's line is not a bead, so there is nothing to show: Enter
    /// there does nothing and says nothing, the same as before.
    #[test]
    fn enter_on_a_row_that_is_not_a_bead_does_nothing() {
        let mut shown = shown(a_described_grove(6));
        assert!(shown.select(0), "onto the project's own line");
        assert_eq!(cursor(&shown), None);
        let before = forest_band(&mut shown, 80, 24);

        assert!(!shown.apply(Action::ShowBead));

        assert_eq!(forest_band(&mut shown, 80, 24), before);
    }

    /// Leaving the view puts the reader back on the row it was opened from,
    /// with the forest exactly as they left it: the motions that moved the
    /// bead moved nothing under it.
    #[test]
    fn leaving_the_bead_view_puts_the_reader_back_on_the_same_row() {
        let mut shown = shown(a_described_grove(30));
        forest_band(&mut shown, 80, 12);
        for _ in 0..4 {
            assert!(shown.apply(Action::Move(Motion::NextRow)));
        }
        let at = shown.forest.selected_line();
        let on = cursor(&shown).cloned();
        let before = forest_band(&mut shown, 80, 12);

        assert!(shown.apply(Action::ShowBead));
        bead_view(&mut shown, 80, 5);
        assert!(shown.scroll(Motion::NextRow), "the view had rows to scroll");
        assert!(shown.scroll(Motion::LastRow));
        bead_view(&mut shown, 80, 5);

        assert_eq!(shown.forest.selected_line(), at);
        assert_eq!(cursor(&shown).cloned(), on);
        assert_eq!(forest_band(&mut shown, 80, 12), before);
    }

    /// A motion while the bead is up moves the bead, and a motion that would
    /// move it nowhere reports no change: the row under it is not what the
    /// key is about.
    #[test]
    fn a_motion_in_the_bead_view_moves_the_bead_and_not_the_selection() {
        let mut shown = shown(a_described_grove(6));
        let at = shown.forest.selected_line();
        assert!(shown.apply(Action::ShowBead));
        let top = bead_window_inner(&mut shown, 80, 6);

        assert!(shown.scroll(Motion::NextRow));
        let down_one = bead_window_inner(&mut shown, 80, 6);

        assert_ne!(down_one, top);
        assert_eq!(down_one[0], top[1]);
        assert_eq!(shown.forest.selected_line(), at);
        assert!(shown.scroll(Motion::PreviousRow), "back to the top");
        assert!(
            !shown.scroll(Motion::PreviousRow),
            "and nowhere further, so nothing to redraw"
        );
        assert_eq!(shown.forest.selected_line(), at);
    }

    /// Opening the view again starts it from the top: a reader who scrolled
    /// one bead to the end and opened another wants the other's start.
    #[test]
    fn opening_the_bead_view_starts_it_at_the_top() {
        let mut shown = shown(a_described_grove(6));
        assert!(shown.apply(Action::ShowBead));
        bead_view(&mut shown, 80, 6);
        assert!(shown.scroll(Motion::LastRow));

        assert!(shown.apply(Action::Move(Motion::NextRow)));
        assert!(shown.apply(Action::ShowBead));

        let inner = bead_window_inner(&mut shown, 80, 6);
        assert_eq!(inner[0], "◐ grv-1.1  a bead in the grove");
    }

    /// A collection that leaves the selection on its bead leaves the view
    /// on it too, however the rows around it moved.
    #[test]
    fn a_collection_that_keeps_the_selection_on_its_bead_keeps_the_view_on_it() {
        let mut shown = shown(a_described_grove(6));
        shown.apply(Action::ExpandOrChild);
        shown.apply(Action::Move(Motion::LastRow));
        assert_eq!(cursor(&shown), Some(&bead("grove", "grv-1.6")));
        assert!(shown.apply(Action::ShowBead));

        let mut reordered = a_grove_reordered(6);
        for tree in &mut reordered.collected {
            let tree = Arc::make_mut(tree);
            for node in &mut tree.beads {
                node.description = format!("what {} is about", node.id);
            }
        }
        reordered.trees = reordered.collected.clone();
        shown.collected(reordered);

        assert_eq!(cursor(&shown), Some(&bead("grove", "grv-1.6")));
        assert!(shown.bead_still_shown());
        assert!(
            bead_window_inner(&mut shown, 80, 24).contains(&"what grv-1.6 is about".to_string())
        );
    }

    /// A collection that drops the bead the view was opened on moves the
    /// selection onto the nearest forebear that survived — which is another
    /// bead, and not the one the reader opened. The view says it is no
    /// longer on its bead, so the loop can take it down rather than draw
    /// the forebear under the title the reader opened.
    #[test]
    fn a_collection_that_drops_the_bead_the_view_is_on_says_the_view_is_off_it() {
        let mut shown = shown(a_described_grove(6));
        shown.apply(Action::ExpandOrChild);
        shown.apply(Action::Move(Motion::LastRow));
        assert_eq!(cursor(&shown), Some(&bead("grove", "grv-1.6")));
        assert!(shown.apply(Action::ShowBead));

        shown.collected(a_described_grove(3));

        assert_ne!(cursor(&shown), Some(&bead("grove", "grv-1.6")));
        assert!(!shown.bead_still_shown());
    }

    /// `Clear` is what stops the trees showing between the rows. Every row
    /// inside the border is asserted to hold nothing of the forest.
    #[test]
    fn no_forest_shows_through_the_bead_window() {
        let mut shown = shown(a_described_grove(30));
        assert!(shown.apply(Action::ShowBead));

        let rows = bead_view(&mut shown, 80, 24);
        let inside = rows.iter().filter(|row| row.starts_with('│'));

        for row in inside {
            assert!(
                !row.contains("grv-1."),
                "a row of the forest shows through the window: {row:?}"
            );
        }
    }

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

    /// A click selects the line the forest drew on that row. Which line that
    /// is is a question about the screen rather than about the click: a
    /// forest scrolled under its window draws a different line on the same
    /// row, and a click that answered from the row alone would select the
    /// wrong bead on every screen the forest has outgrown.
    #[test]
    fn a_click_selects_the_line_the_forest_drew_on_that_row() {
        let screen = Rect::new(0, 0, 60, 24);
        let band = draw::regions(screen).forest;
        let mut shown = shown(a_grove(30));

        assert!(shown.clicked(screen, band.y + 3));
        assert_eq!(shown.forest.selected_line(), 3);

        to_the_last_row(&mut shown);
        let last = shown.forest.selected_line();
        assert!(
            last + 1 > band.height as usize,
            "the grove has to outgrow the band for the scroll to be the \
             difference between the two clicks"
        );
        assert!(shown.clicked(screen, band.y));
        assert_eq!(
            shown.forest.selected_line(),
            last + 1 - band.height as usize,
            "the last line is on the band's last row, so its first row holds \
             the line a bandful before it"
        );
    }

    /// The tail is an echo of a pane and the key row is a legend. A click on
    /// either holds nothing the selection could sit on, and leaves it where
    /// it was.
    #[test]
    fn a_click_beneath_the_forest_selects_nothing() {
        let screen = Rect::new(0, 0, 60, 24);
        let bands = draw::regions(screen);
        let mut shown = shown(a_grove(30));
        shown.clicked(screen, bands.forest.y + 3);
        let selected = shown.forest.selected_line();

        for row in [bands.tail.y, bands.keys.y] {
            assert!(!shown.clicked(screen, row), "the click at row {row} moved");
            assert_eq!(shown.forest.selected_line(), selected);
        }
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

    /// The screen opens on what the band has to say while the provider is still
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
    /// the provider and the band says so on its own.
    #[test]
    fn a_row_with_no_pane_asks_the_provider_nothing() {
        let (shown, panes) = shown_asking(a_grove(6));

        assert_eq!(shown.tail, Tail::Silent(phrase::no_agent_to_tail()));
        assert!(panes.reads().is_empty());
    }

    #[test]
    fn what_was_read_for_the_pane_selected_is_what_the_band_shows() {
        let (mut shown, _) = shown_asking(a_staffed_grove(6));

        assert!(shown.tailed(read(A_SELECTED_PANE, &["rebuilt .#thinkpad"]), an_instant()));

        assert_eq!(
            shown.tail,
            Tail::Pane {
                pane: A_SELECTED_PANE.to_string(),
                lines: vec!["rebuilt .#thinkpad".to_string()],
            }
        );
    }

    /// A reader holding an arrow key down moves faster than a slow provider
    /// answers. Every move asks for the pane it landed on, but only one
    /// question is ever out, so what a run of keystrokes costs is one
    /// call — not one per key — and the band goes on naming the pane it is
    /// waiting for.
    ///
    /// The tail falling behind the selection is the price of the keyboard
    /// never doing so, and it is a decision rather than an accident: the loop
    /// answers every one of these moves at once, and the rows under the
    /// forest arrive when the cursor rests long enough for an answer to land.
    #[test]
    fn moving_faster_than_the_provider_answers_costs_one_read_at_a_time() {
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
            !shown.tailed(
                read(A_SELECTED_PANE, &["nothing should reach the screen"]),
                an_instant()
            ),
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
            "and that pane is what the provider is asked for next"
        );
    }

    /// The pane is read on the band's own clock and not on the collection's:
    /// a collection landing under the same pane leaves the rows on the band
    /// standing and asks the provider for nothing.
    ///
    /// It used to be the other way — the collection was the tick the pane
    /// was re-read on, and a read out when it landed was superseded so the
    /// band would not show the pane as it was. The band's own interval is
    /// what bounds that now, and it is far shorter than a collection's.
    #[test]
    fn a_collection_landing_under_the_pane_leaves_the_band_to_its_own_clock() {
        let grove = a_staffed_grove(6);
        let (mut shown, panes) = shown_asking(grove.clone());
        let opened_on = panes.reads();
        assert!(shown.tailed(read(A_SELECTED_PANE, &["what it says"]), an_instant()));

        shown.collected(grove);

        assert_eq!(
            panes.reads(),
            opened_on,
            "the collection asked the provider for nothing"
        );
        assert_eq!(
            shown.tail,
            Tail::Pane {
                pane: A_SELECTED_PANE.to_string(),
                lines: vec!["what it says".to_string()],
            },
            "and the rows on the band stand"
        );
        assert_eq!(
            shown.rereads_in(an_instant()),
            Some(EVERY),
            "the band's own clock is what reads the pane next"
        );
    }

    /// The bead: the pane is asked for again one interval after the answer
    /// that filled the band, and not before. Nothing on the band changes for
    /// the ask — the rows stand until the answer lands — because a band that
    /// said *reading that pane* four times a second would be a band nobody
    /// could read.
    #[test]
    fn the_pane_is_read_again_one_interval_after_the_answer_that_filled_the_band() {
        let (mut shown, panes) = shown_asking(a_staffed_grove(6));
        let opened_on = panes.reads();
        let answered = an_instant();
        shown.tailed(read(A_SELECTED_PANE, &["what it says"]), answered);
        let filled = shown.tail.clone();

        assert_eq!(shown.rereads_in(answered), Some(EVERY));
        shown.reread(answered + EVERY / 2);
        assert_eq!(panes.reads(), opened_on, "half an interval in, not yet");
        assert_eq!(
            shown.rereads_in(answered + EVERY / 2),
            Some(EVERY / 2),
            "and half is what is left to wait"
        );

        shown.reread(answered + EVERY);

        assert_eq!(
            panes.reads(),
            [
                opened_on,
                vec![format!("{A_SELECTED_PANE} {}", tail::LINES)]
            ]
            .concat(),
            "the interval came round and the pane was asked for"
        );
        assert_eq!(
            shown.tail, filled,
            "the rows stand while the answer is on its way"
        );
        assert_eq!(
            shown.rereads_in(answered + EVERY),
            None,
            "nothing is due while a read is out"
        );
    }

    /// A pane that has said nothing new since the last read is the common
    /// case four times a second, and it is not a change to the screen.
    #[test]
    fn an_answer_that_repeats_the_rows_on_the_band_changes_nothing() {
        let (mut shown, _) = shown_asking(a_staffed_grove(6));
        let answered = an_instant();
        assert!(shown.tailed(read(A_SELECTED_PANE, &["what it says"]), answered));
        shown.reread(answered + EVERY);

        let again = answered + EVERY * 2;
        assert!(
            !shown.tailed(read(A_SELECTED_PANE, &["what it says"]), again),
            "the same rows again are not a change"
        );
        assert_eq!(
            shown.rereads_in(again),
            Some(EVERY),
            "and the answer still arms the read after it"
        );
    }

    /// The answer to that read arms the next, so a pane the cursor rests on
    /// is read for as long as it rests there.
    #[test]
    fn every_answer_arms_the_read_after_it() {
        let (mut shown, panes) = shown_asking(a_staffed_grove(6));
        let answered = an_instant();
        shown.tailed(read(A_SELECTED_PANE, &["first"]), answered);
        shown.reread(answered + EVERY);
        let asked_twice = panes.reads();

        let again = answered + EVERY * 2;
        assert!(shown.tailed(read(A_SELECTED_PANE, &["second"]), again));
        shown.reread(again + EVERY);

        assert_eq!(panes.reads().len(), asked_twice.len() + 1);
        assert_eq!(
            shown.tail,
            Tail::Pane {
                pane: A_SELECTED_PANE.to_string(),
                lines: vec!["second".to_string()],
            }
        );
    }

    /// The largest whole number of milliseconds — the unit
    /// `tail_refresh_millis` is read in — that still lands inside the range
    /// an instant can hold, read off chrono's own last instant rather than
    /// quoted from its documentation. It shrinks as the clock advances,
    /// which is why the answer is checked where the add happens rather than
    /// bounded once at config load.
    fn millis_to_the_end_of_time(from: DateTime<Utc>) -> u64 {
        u64::try_from(
            (DateTime::<Utc>::MAX_UTC - from)
                .to_std()
                .expect("the end of time is after this instant")
                .as_millis(),
        )
        .expect("a gap an instant can hold counts in milliseconds a u64 holds")
    }

    #[test]
    fn a_config_that_would_not_load_is_said_at_the_foot() {
        let mut shown = shown(a_staffed_grove(6));

        assert!(shown.reloaded(Reloaded::Broken), "the foot has changed");
        assert_eq!(shown.standing, [Notice::ConfigWouldNotReload]);
    }

    /// The sequence a reader who breaks their config and undoes the edit
    /// walks: the file is back to exactly what `bdi` is already working to,
    /// so nothing reloads — and the notice has to come off all the same. A
    /// foot that cleared it only on a config that *changed* would leave them
    /// with no edit that clears it but one changing the config to something
    /// they did not want.
    #[test]
    fn undoing_a_broken_edit_takes_the_notice_off_though_nothing_reloaded() {
        let mut shown = shown(a_staffed_grove(6));
        shown.reloaded(Reloaded::Broken);

        assert!(shown.reloaded(Reloaded::Unchanged), "the foot has changed");
        assert_eq!(shown.standing, []);
    }

    #[test]
    fn a_config_that_loads_again_takes_the_notice_off() {
        let mut shown = shown(a_staffed_grove(6));
        shown.reloaded(Reloaded::Broken);

        assert!(shown.reloaded(Reloaded::Fresh), "the foot has changed");
        assert_eq!(shown.standing, []);
    }

    /// A check that read nothing says nothing, which is what leaves the
    /// notice up between one check and the next: the file is still broken and
    /// nobody has written it since.
    #[test]
    fn a_check_that_read_nothing_leaves_the_foot_as_it_was() {
        let mut shown = shown(a_staffed_grove(6));
        shown.reloaded(Reloaded::Broken);

        assert!(!shown.reloaded(Reloaded::Untouched), "nothing has changed");
        assert_eq!(shown.standing, [Notice::ConfigWouldNotReload]);
    }

    /// Being told again what the foot already says is not a change. The loop
    /// draws on this, and a check every couple of seconds that always
    /// reported one would redraw the screen for ever.
    #[test]
    fn a_foot_told_again_what_it_already_says_has_not_changed() {
        let mut shown = shown(a_staffed_grove(6));

        assert!(!shown.reloaded(Reloaded::Unchanged), "nothing to take off");
        shown.reloaded(Reloaded::Broken);
        assert!(!shown.reloaded(Reloaded::Broken), "already said");
        assert_eq!(shown.standing, [Notice::ConfigWouldNotReload]);
    }

    /// What was settled before the first collection stands whatever the
    /// config does: the notice that comes and goes is the only one that does.
    #[test]
    fn a_config_notice_leaves_the_notices_settled_at_startup_alone() {
        let mut shown = Shown::of(
            a_staffed_grove(6),
            Box::new(Asking::default()),
            Box::new(io::sink()),
            EVERY,
            vec![Notice::NoInboundChannel],
        );

        shown.reloaded(Reloaded::Broken);
        assert_eq!(
            shown.standing,
            [Notice::NoInboundChannel, Notice::ConfigWouldNotReload]
        );

        shown.reloaded(Reloaded::Fresh);
        assert_eq!(shown.standing, [Notice::NoInboundChannel]);
    }

    /// The gap `tail_refresh_millis` can be given and still be waited out.
    /// One millisecond more is the test below, and the two of them are what
    /// pin the answer to the bound rather than to somewhere past it.
    #[test]
    fn a_band_whose_interval_reaches_the_end_of_time_still_rereads() {
        let every = Duration::from_millis(millis_to_the_end_of_time(an_instant()));
        let (mut shown, _) = shown_asking_every(a_staffed_grove(6), every);

        shown.tailed(read(A_SELECTED_PANE, &["what it says"]), an_instant());

        assert_eq!(shown.rereads_in(an_instant()), Some(every));
    }

    /// A `tail_refresh_millis` past what an instant can hold is a gap
    /// nothing waits out, rather than a panic on the first answer the
    /// provider gives. Nothing is what this already says for a band with no
    /// pane on it, so the answer is one the caller can already meet.
    #[test]
    fn a_band_whose_interval_outruns_time_rereads_no_more() {
        let every = Duration::from_millis(millis_to_the_end_of_time(an_instant()) + 1);
        let (mut shown, panes) = shown_asking_every(a_staffed_grove(6), every);

        shown.tailed(read(A_SELECTED_PANE, &["what it says"]), an_instant());
        let asked_once = panes.reads();

        assert_eq!(shown.rereads_in(an_instant()), None);
        shown.reread(an_instant() + EVERY * 10);

        assert_eq!(panes.reads(), asked_once);
    }

    /// A row with no pane has nothing to read again, so the band sets no
    /// deadline and the loop is not woken for it.
    #[test]
    fn a_band_with_no_pane_is_never_due_to_read_one() {
        let (mut shown, panes) = shown_asking(a_grove(6));

        assert_eq!(shown.rereads_in(an_instant()), None);
        shown.reread(an_instant() + EVERY * 10);

        assert!(panes.reads().is_empty());
    }

    /// The selection leaving the pane disarms the read that was due for it:
    /// the pane it lands on is asked for at once, and that answer is what
    /// arms the next.
    #[test]
    fn leaving_the_pane_for_another_disarms_the_read_that_was_due_for_it() {
        let (mut shown, panes) = shown_asking(a_staffed_grove(6));
        let answered = an_instant();
        shown.tailed(read(A_SELECTED_PANE, &["what it says"]), answered);
        assert_eq!(shown.rereads_in(answered), Some(EVERY));

        shown.apply(Action::Move(Motion::NextRow));
        let Tail::Reading { pane: landed_on } = shown.tail.clone() else {
            panic!("the row below is another agent's pane: {:?}", shown.tail);
        };
        assert_ne!(landed_on, A_SELECTED_PANE);

        assert_eq!(
            panes.reads().last(),
            Some(&format!("{landed_on} {}", tail::LINES)),
            "the pane landed on is asked for at once"
        );
        assert_eq!(
            shown.rereads_in(answered),
            None,
            "and nothing is due while that read is out"
        );
        let asked = panes.reads();
        shown.reread(answered + EVERY);
        assert_eq!(
            panes.reads(),
            asked,
            "the first pane's interval coming round asks for nothing"
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

        shown.tailed(
            read(A_SELECTED_PANE, &["about the pane left behind"]),
            an_instant(),
        );
        assert!(shown.tailed(
            read(&resting_on, &["about the pane rested on"]),
            an_instant()
        ));

        assert_eq!(
            shown.tail,
            Tail::Pane {
                pane: resting_on,
                lines: vec!["about the pane rested on".to_string()],
            }
        );
    }

    /// `⏎` asks for the pane and nothing on this screen changes for it:
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

        assert!(shown.tailed(refused(A_SELECTED_PANE), an_instant()));

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

        assert!(!shown.tailed(refused(A_SELECTED_PANE), an_instant()));

        assert_eq!(shown.tail, was);
    }

    /// What the terminal was handed for its clipboard, byte for byte.
    #[derive(Clone, Default)]
    struct Clipboard(Arc<Mutex<Vec<u8>>>);

    impl Clipboard {
        fn written(&self) -> Vec<u8> {
            self.0.lock().expect("no test panics holding this").clone()
        }
    }

    impl io::Write for Clipboard {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .expect("no test panics holding this")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// The same, with what was written to the terminal's clipboard kept
    /// beside it.
    fn shown_copying(snapshot: Snapshot) -> (Shown, Clipboard) {
        let clipboard = Clipboard::default();
        (
            Shown::of(
                snapshot,
                Box::new(Asking::default()),
                Box::new(clipboard.clone()),
                EVERY,
                nothing_said(),
            ),
            clipboard,
        )
    }

    /// The text one OSC 52 sequence carries, where the bytes are exactly one
    /// such sequence and nothing else. Decoded rather than matched, so the
    /// test says what the terminal will paste and not what `bdi` wrote.
    fn on_the_clipboard(written: &[u8]) -> String {
        let payload = written
            .strip_prefix(b"\x1b]52;c;")
            .and_then(|rest| rest.strip_suffix(b"\x07"))
            .unwrap_or_else(|| panic!("not one OSC 52 sequence and nothing else: {written:?}"));
        String::from_utf8(BASE64_STANDARD.decode(payload).expect("base64")).expect("utf-8")
    }

    /// `y` on a bead row puts that bead's id — the id alone, exactly as `bd`
    /// takes it — on the terminal's clipboard, and writes nothing else.
    #[test]
    fn y_on_a_bead_row_puts_its_id_on_the_clipboard_and_nothing_else() {
        let (mut shown, clipboard) = shown_copying(a_grove(6));
        assert_eq!(cursor(&shown), Some(&bead("grove", "grv-1")));

        press(&mut shown, KeyCode::Char('y'));

        assert_eq!(on_the_clipboard(&clipboard.written()), "grv-1");
    }

    /// A project line, a group and the hidden-trees line name no bead, so
    /// there is nothing to copy and nothing has gone wrong — the same as
    /// Enter there.
    #[test]
    fn y_on_a_row_that_is_not_a_bead_writes_nothing() {
        let (mut shown, clipboard) = shown_copying(a_hidden_grove_above_a_shown_tree());
        shown.apply(Action::Move(Motion::LastRow));
        assert_eq!(cursor(&shown), None, "the hidden-trees line names no bead");

        assert!(!shown.apply(Action::CopyId));

        assert_eq!(clipboard.written(), b"");
    }

    /// The row at the foot of a frame of this screen, as drawn.
    fn foot_of(shown: &mut Shown, width: u16, height: u16) -> String {
        painted(
            &mut shown.forest,
            &shown.tail,
            Over::Nothing,
            shown.copied.as_deref(),
            &[],
            width,
            height,
        )
        .rows()
        .pop()
        .expect("a screen with rows on it")
    }

    /// Nothing else on the screen changes for a copy, so the foot is where a
    /// reader learns the key fired — and which id it took, on a row whose id
    /// may be abbreviated.
    #[test]
    fn after_y_the_foot_says_which_id_was_copied() {
        let (mut shown, _) = shown_copying(a_grove(6));

        assert!(shown.apply(Action::CopyId), "the foot has changed");

        assert!(
            foot_of(&mut shown, 80, 24).contains("copied grv-1"),
            "{:?}",
            foot_of(&mut shown, 80, 24)
        );
    }

    /// The reader's next press takes it off, whatever the press turns out to
    /// mean: it was feedback on a keystroke, not a fact about the screen. The
    /// screen has changed for that alone, so the press is one to redraw on.
    #[test]
    fn the_readers_next_press_takes_the_copied_id_off_the_foot() {
        let (mut shown, _) = shown_copying(a_grove(6));
        shown.apply(Action::CopyId);

        assert!(shown.pressed(), "the foot has changed");

        assert!(!foot_of(&mut shown, 80, 24).contains("copied"));
    }

    /// With nothing on the foot to take off, a press changes nothing here,
    /// so a key bound to nothing goes on costing no redraw.
    #[test]
    fn a_press_with_nothing_copied_changes_nothing() {
        assert!(!shown(a_grove(6)).pressed());
    }

    /// What happens behind the reader's back is not the reader's next
    /// action: a collection landing a moment after `y` would otherwise take
    /// the feedback off before anyone had read it.
    #[test]
    fn a_collection_landing_leaves_the_copied_id_on_the_foot() {
        let (mut shown, _) = shown_copying(a_grove(6));
        shown.apply(Action::CopyId);

        shown.collected(a_grove(6));

        assert!(foot_of(&mut shown, 80, 24).contains("copied grv-1"));
    }

    /// A terminal that will not take the write.
    struct Refusing;

    impl io::Write for Refusing {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// A foot that said *copied* over a write that never reached the terminal
    /// would be the one line on the screen that was untrue.
    #[test]
    fn a_write_the_terminal_refused_is_not_reported_as_copied() {
        let mut shown = Shown::of(
            a_grove(6),
            Box::new(Asking::default()),
            Box::new(Refusing),
            EVERY,
            nothing_said(),
        );

        assert!(!shown.apply(Action::CopyId));

        assert!(!foot_of(&mut shown, 80, 24).contains("copied"));
    }

    #[test]
    fn a_frame_puts_the_tail_in_the_band_reserved_for_it() {
        let mut forest = an_open_grove(30);
        let tail = Tail::Pane {
            pane: "w:p1".to_string(),
            lines: vec!["rebuilt .#thinkpad".to_string()],
        };

        let rows = screen_of(&mut forest, &tail, 40, 12, Over::Nothing).rows();
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
