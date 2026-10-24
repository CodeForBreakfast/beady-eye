//! The loop over the real `Screen`, drawn on a buffer rather than a tty.
//!
//! `Screen` is a `View` that hands each call to `Shown`, and the loop is the
//! only thing that calls it. `Shown`'s own tests reach past every one of those
//! hand-offs, so a `Screen::pasted` that dropped the text would leave them all
//! green. Each test here hands the loop a run of events, lets it go round
//! until they are spent, and reads the last frame it drew.
//!
//! A reading in the middle of a run is a second run that stops there: the
//! loop holds what is on screen between events, and a run handed back halfway
//! would start again on the forest.

use super::fold_keys::collected;
use super::*;
use crate::tui::drive::tests::{
    a_run_reading, at_once, nothing_armed, nothing_watched, polling_every_interval, waiting,
};
use crate::tui::drive::{drive, Event};
use crate::tui::fixtures::A_MOMENT;
use ratatui::backend::TestBackend;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Instant;

impl PutBack for TestBackend {
    fn put_back(&mut self) {}
}

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// What `bd list --all --limit 0 --json` said about one open epic and the four
/// open beads under it, each with the prose it was written with.
const THE_DESCRIBED_SUBTREE: &str =
    include_str!("../../../../tests/fixtures/bd_described_subtree.json");

/// The keys that open that tree and leave the selection on its header: every
/// tree rather than only the staffed ones, since no pane sits on it, back to
/// the first row, down onto the tree, open it, and back up and down again.
const OPEN_THE_TREE: &str = "agjlgj";

/// The tree's header, which is where that walk leaves the selection.
const THE_TREE: &str = "dun-0tp";

const THE_FIRST_BEAD: &str = "dun-0tp.6";

/// A run of `bdi` over `tracker` with `panes` working in its project, on a
/// screen `rows` tall drawn to the config in force.
struct Run {
    tracker: String,
    panes: Vec<String>,
    cfg: Config,
    rows: u16,
}

impl Run {
    fn over(tracker: &str) -> Self {
        Self {
            tracker: tracker.to_string(),
            panes: Vec::new(),
            cfg: the_config_in_force(),
            rows: ROWS,
        }
    }

    fn with_panes(self, panes: &[&str]) -> Self {
        Self {
            panes: panes.iter().map(|pane| pane.to_string()).collect(),
            ..self
        }
    }

    fn collection(&self) -> Event {
        let panes: Vec<&str> = self.panes.iter().map(String::as_str).collect();
        Event::Collected(Box::new(collected(&self.tracker, &panes)))
    }

    /// The screen the loop left after the first collection and each of
    /// `steps` in turn, and what the provider was asked on the way.
    fn after<const N: usize>(&self, steps: [Vec<Event>; N]) -> (Vec<String>, Asking) {
        let (mut screen, asking) = self.screen();
        driven(&mut screen, &waiting(self.events(steps)));
        (drawn(&screen), asking)
    }

    fn events<const N: usize>(&self, steps: [Vec<Event>; N]) -> Vec<Event> {
        std::iter::once(self.collection())
            .chain(steps.into_iter().flatten())
            .collect()
    }

    /// The screen a run opens on: no project read yet.
    fn screen(&self) -> (Screen<TestBackend>, Asking) {
        let awaiting = Snapshot::awaiting(
            vec!["arkham".to_string()],
            Vec::new(),
            "herdr",
            Scope::default(),
            Filter::LiveAgents,
            an_instant(),
        );
        let panes = Asking::default();
        let screen = Screen {
            terminal: Terminal::new(TestBackend::new(COLS, self.rows)).expect("a test backend"),
            shown: Shown::of(
                awaiting,
                Box::new(panes.clone()),
                Box::new(io::sink()),
                Drawing::to(&self.cfg),
                nothing_said(),
                an_instant(),
            ),
        };
        (screen, panes)
    }
}

fn driven(screen: &mut Screen<TestBackend>, events: &Receiver<Event>) {
    let (ask, _asked) = mpsc::channel();
    drive(
        screen,
        events,
        &ask,
        at_once(),
        a_run_reading(nothing_armed()),
        &polling_every_interval(),
        nothing_watched(),
    )
    .expect("the loop runs");
}

fn drawn(screen: &Screen<TestBackend>) -> Vec<String> {
    Painted::read(screen.terminal.backend().buffer()).rows()
}

fn typed(keys: &str) -> Vec<Event> {
    keys.chars()
        .map(|typed| Event::Key(key(KeyCode::Char(typed))))
        .collect()
}

fn pressed<const N: usize>(codes: [KeyCode; N]) -> Vec<Event> {
    codes
        .into_iter()
        .map(|code| Event::Key(key(code)))
        .collect()
}

fn walked() -> Vec<Event> {
    typed(OPEN_THE_TREE)
}

/// The bead the bead window is over, off its border title, or nothing where
/// no window is up. The window's title is the top row of the screen, the id
/// first and how far down the bead the reader is after a `·`.
fn window_over(screen: &[String]) -> Option<String> {
    let (_, title) = screen.first()?.split_once('┌')?;
    let named = title.trim_end().trim_end_matches(['─', '┐']);
    Some(named.split(" · ").next().unwrap_or(named).to_string())
}

fn says(screen: &[String], said: &str) -> bool {
    screen.iter().any(|row| row.contains(said))
}

/// `c`, which shuts the selected node and everything under it, so the search
/// has a fold to open.
const SHUT: &str = "c";

/// A bead in the tree, and its own title, which says its row is drawn.
const THE_SEARCHED_BEAD: &str = "dun-0tp.7";
const THE_SEARCHED_BEADS_TITLE: &str = "phrase.rs is one file";

/// `/` and that bead's id, without the Enter that closes the prompt.
const SEARCH_FOR_IT: &str = "/dun-0tp.7";

/// The id of a bead in the tree, typed into the prompt and searched for: `/`,
/// the id, Enter to close the prompt and Enter to show the bead it landed on.
#[test]
fn an_id_typed_into_the_prompt_takes_the_reader_to_that_bead() {
    let run = Run::over(THE_DESCRIBED_SUBTREE);

    let (shut, _) = run.after([walked(), typed(SHUT)]);
    assert!(
        !says(&shut, THE_SEARCHED_BEADS_TITLE),
        "the bead is drawn with its tree shut, so the search has no fold to \
         open: {shut:#?}"
    );

    let (prompt, _) = run.after([walked(), typed(SHUT), typed(SEARCH_FOR_IT)]);
    assert!(
        prompt
            .last()
            .is_some_and(|foot| foot.contains(SEARCH_FOR_IT)),
        "the characters typed after `/` are not on the foot: {prompt:#?}"
    );

    let (landed, _) = run.after([
        walked(),
        typed(SHUT),
        typed(SEARCH_FOR_IT),
        pressed([KeyCode::Enter, KeyCode::Enter]),
    ]);
    assert_eq!(
        window_over(&landed).as_deref(),
        Some(THE_SEARCHED_BEAD),
        "the search did not leave the selection on the bead it named: \
         {landed:#?}"
    );
}

/// The same id pasted into the prompt, as a terminal sends a paste once `bdi`
/// has asked for it bracketed.
#[test]
fn an_id_pasted_into_the_prompt_takes_the_reader_to_that_bead() {
    let (landed, _) = Run::over(THE_DESCRIBED_SUBTREE).after([
        walked(),
        typed(SHUT),
        typed("/"),
        vec![Event::Pasted(THE_SEARCHED_BEAD.to_string())],
        pressed([KeyCode::Enter, KeyCode::Enter]),
    ]);
    assert_eq!(
        window_over(&landed).as_deref(),
        Some(THE_SEARCHED_BEAD),
        "the pasted id did not reach the prompt: {landed:#?}"
    );
}

/// Down onto the first bead and Enter, which shows it. One motion rather than
/// two: the anomaly the tree's orphaned dependencies raise is drawn between
/// the header and the first bead, and a motion passes over it.
fn the_first_bead_shown() -> Vec<Event> {
    pressed([KeyCode::Char('j'), KeyCode::Enter])
}

/// The arrow the window draws to a bead's parent, which the forest draws too
/// and so can be gone to.
const A_REFERENCE_THAT_CAN_BE_FOLLOWED: &str = "↑";

/// A click on a reference the forest can reach goes to the bead it names.
///
/// `G` takes the window to the end of the first bead, where its sections are:
/// the description is longer than the screen. The row is read off the frame
/// the loop drew rather than worked out, which would run the arithmetic under
/// test a second time.
#[test]
fn a_click_on_a_reference_goes_to_the_bead_it_names() {
    let run = Run::over(THE_DESCRIBED_SUBTREE);
    let (page, _) = run.after([walked(), the_first_bead_shown(), typed("G")]);
    assert_eq!(
        window_over(&page).as_deref(),
        Some(THE_FIRST_BEAD),
        "no window opened over the first bead: {page:#?}"
    );
    let on = the_one_row(&page, A_REFERENCE_THAT_CAN_BE_FOLLOWED);

    let (after, _) = run.after([
        walked(),
        the_first_bead_shown(),
        typed("G"),
        vec![Event::Clicked(on)],
    ]);
    assert_eq!(
        window_over(&after).as_deref(),
        Some(THE_TREE),
        "the click on the parent did not go to it: {after:#?}"
    );
}

#[track_caller]
fn the_one_row(screen: &[String], said: &str) -> u16 {
    let rows: Vec<usize> = (0..screen.len())
        .filter(|row| screen[*row].contains(said))
        .collect();
    let [row] = rows[..] else {
        panic!("{said:?} is drawn on rows {rows:?}, not on one: {screen:#?}");
    };
    u16::try_from(row).expect("a screen this short")
}

/// The window a click on the row saying `said` opens: the click, then Enter.
fn the_window_a_click_opens(said: &str) -> Option<String> {
    let run = Run::over(THE_DESCRIBED_SUBTREE);
    let (forest, _) = run.after([walked()]);
    let on = the_one_row(&forest, said);
    let (opened, _) = run.after([
        walked(),
        vec![Event::Clicked(on)],
        pressed([KeyCode::Enter]),
    ]);
    window_over(&opened)
}

/// The titles the forest draws the tree's header and its first bead under.
const THE_TREES_TITLE: &str = "Every outside program";
const THE_FIRST_BEADS_TITLE: &str = "Change sources";

/// A click on a bead's row, and then Enter: the window that opens names the
/// bead drawn on that row rather than the one the selection was on.
#[test]
fn a_click_on_a_beads_row_selects_the_bead_drawn_there() {
    assert_eq!(
        the_window_a_click_opens(THE_FIRST_BEADS_TITLE).as_deref(),
        Some(THE_FIRST_BEAD),
        "the click landed on the first bead's row, and the window that opened \
         is not that bead's"
    );
}

/// The same with the click landing where the selection already is, which is
/// what makes the test above about the click: a click that moved nothing at
/// all would leave the selection on the tree's header.
#[test]
fn a_click_on_the_row_the_selection_rests_on_opens_that_row() {
    assert_eq!(
        the_window_a_click_opens(THE_TREES_TITLE).as_deref(),
        Some(THE_TREE),
        "the click landed on the tree's header, and the window that opened is \
         not the tree's"
    );
}

/// The word the foot says a copied id with, from `view::phrase`.
const COPIED: &str = "copied";

/// `y` puts the selected bead's id on the foot, and a key the mapping answers
/// with nothing takes it off again for being a press.
#[test]
fn a_key_bound_to_nothing_still_takes_the_copied_id_off_the_foot() {
    let run = Run::over(THE_DESCRIBED_SUBTREE);
    let (said, _) = run.after([walked(), typed("y")]);
    assert!(
        says(&said, COPIED),
        "`y` put nothing on the foot, so there was never a line for the next \
         press to take off: {said:#?}"
    );

    let (after, _) = run.after([walked(), typed("y"), typed("z")]);
    assert!(
        !says(&after, COPIED),
        "the reader has pressed something since, and the foot still says what \
         they copied: {after:#?}"
    );
}

/// A pane working in the project and on no bead, which is the forest's last
/// row.
const THE_PANE: &str = "wT:p7";

/// A tree shorter than the screen leaves the band half of it: the rule and
/// nineteen of the pane's lines on forty rows. What the frame found room for
/// is what the next read of the pane asks for, so the ask says the frame told
/// the band.
#[test]
fn a_short_tree_on_a_tall_screen_shows_half_a_screen_of_the_pane() {
    let (screen, asking) = Run::over("[]").with_panes(&[THE_PANE]).after([typed("G")]);
    assert_eq!(
        asking.reads().last().map(String::as_str),
        Some(format!("{THE_PANE} {}", ROWS / 2 - 1).as_str()),
        "the read did not ask for a band of half the screen: {screen:#?}"
    );
}

/// Once the provider has answered, the band asks for its pane again on its
/// own clock, with nothing else to wake the loop: the one project was read
/// months before the frame, so its age holds the frame far longer than a
/// test waits.
///
/// The channel stays open until the second ask arrives or `A_MOMENT` is up,
/// so a band that forgot its clock fails on the count rather than hanging.
#[test]
fn the_band_asks_for_its_pane_again_on_its_own_clock() {
    let run = Run::over("[]").with_panes(&[THE_PANE]);
    let (mut screen, asking) = run.screen();
    let (to, events) = mpsc::channel();
    for event in run.events([
        typed("G"),
        vec![Event::Tailed(read(THE_PANE, &["rebuilding"]))],
    ]) {
        to.send(event)
            .expect("the loop's end of the channel is open");
    }
    let watching = asking.clone();
    thread::spawn(move || {
        let giving_up = Instant::now() + A_MOMENT;
        while watching.reads().len() < 2 && Instant::now() < giving_up {
            thread::sleep(Duration::from_millis(10));
        }
        drop(to);
    });

    driven(&mut screen, &events);

    assert_eq!(
        asking.reads().len(),
        2,
        "the band asked for its pane {:?} after the provider answered once",
        asking.reads()
    );
}

/// Tab moves the window's ring onto the first bead's parent, and Enter goes
/// there.
fn the_parent_followed() -> Vec<Event> {
    pressed([KeyCode::Tab, KeyCode::Enter])
}

/// Following the parent moves the forest to it and redraws the window there.
#[test]
fn following_a_bead_the_window_names_takes_the_reader_to_it() {
    let run = Run::over(THE_DESCRIBED_SUBTREE);
    let (opened, _) = run.after([walked(), the_first_bead_shown()]);
    assert_eq!(
        window_over(&opened).as_deref(),
        Some(THE_FIRST_BEAD),
        "no window opened over the first bead: {opened:#?}"
    );

    let (followed, _) = run.after([walked(), the_first_bead_shown(), the_parent_followed()]);
    assert_eq!(
        window_over(&followed).as_deref(),
        Some(THE_TREE),
        "the window is not on the bead that was followed: {followed:#?}"
    );
}

/// And `Esc` comes back to the bead it was followed from, rather than out to
/// the forest.
#[test]
fn the_way_back_returns_to_the_bead_the_reference_was_followed_from() {
    let (back, _) = Run::over(THE_DESCRIBED_SUBTREE).after([
        walked(),
        the_first_bead_shown(),
        the_parent_followed(),
        pressed([KeyCode::Esc]),
    ]);
    assert_eq!(
        window_over(&back).as_deref(),
        Some(THE_FIRST_BEAD),
        "the way back did not return to the bead the reference was followed \
         from: {back:#?}"
    );
}

/// A word of the opening lines of the first bead's description that its
/// title stops short of, and one of its last lines. The window is far too
/// short for both at once.
const THE_TOP_OF_THE_BEAD: &str = "shape";
const THE_FOOT_OF_THE_BEAD: &str = "twice";

/// Notches enough to reach the foot of that bead from its top, and more.
const NOTCHES: usize = 40;

/// The wheel over the bead window moves the window down the bead.
#[test]
fn the_wheel_moves_the_bead_window_down_its_bead() {
    let run = Run::over(THE_DESCRIBED_SUBTREE);
    let (at_the_top, _) = run.after([walked(), the_first_bead_shown()]);
    assert!(
        says(&at_the_top, THE_TOP_OF_THE_BEAD) && !says(&at_the_top, THE_FOOT_OF_THE_BEAD),
        "the window opens at the top of its bead, and this one did not: \
         {at_the_top:#?}"
    );

    let (wheeled, _) = run.after([
        walked(),
        the_first_bead_shown(),
        (0..NOTCHES).map(|_| Event::Scrolled(Notch::Down)).collect(),
    ]);
    assert!(
        says(&wheeled, THE_FOOT_OF_THE_BEAD),
        "{NOTCHES} notches of the wheel left the window where it was: \
         {wheeled:#?}"
    );
}

/// The capture with the first bead taken out of it, which is a bead that has
/// left the tracker.
fn without_the_first_bead() -> String {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(THE_DESCRIBED_SUBTREE).expect("a capture of bd list --json");
    let kept: Vec<serde_json::Value> = rows
        .into_iter()
        .filter(|row| row["id"] != THE_FIRST_BEAD)
        .collect();
    assert_eq!(kept.len(), 4, "the capture has to have held that bead");
    serde_json::to_string(&kept).expect("rows serialise")
}

/// A collection that no longer holds the bead the window is on takes the
/// window down: drawn from the selection, it would otherwise show another
/// bead under a title the reader did not open.
#[test]
fn a_collection_that_drops_the_shown_bead_takes_the_window_down() {
    let (after, _) = Run::over(THE_DESCRIBED_SUBTREE).after([
        walked(),
        the_first_bead_shown(),
        vec![Run::over(&without_the_first_bead()).collection()],
    ]);
    assert_eq!(
        window_over(&after),
        None,
        "the tracker no longer holds that bead and a window is still up: \
         {after:#?}"
    );
}

/// And a collection that still holds it leaves the window up, or the test
/// above is met by a window that closes on every collection.
#[test]
fn a_collection_that_keeps_the_shown_bead_leaves_the_window_up() {
    let run = Run::over(THE_DESCRIBED_SUBTREE);
    let (after, _) = run.after([walked(), the_first_bead_shown(), vec![run.collection()]]);
    assert_eq!(
        window_over(&after).as_deref(),
        Some(THE_FIRST_BEAD),
        "the tracker still holds that bead and its window has gone: {after:#?}"
    );
}

/// The whole of the bead window's key row and of the forest's, and a word of
/// each that the other does not say, for the absences: `? keys` is on both.
const THE_WINDOWS_KEYS: &str = "Esc back   ? keys   Tab related   y id";
const THE_FORESTS_KEYS: &str = "a all   ? keys   / find   q quit";
const A_WORD_ONLY_THE_FOREST_SAYS: &str = "q quit";
const A_WORD_ONLY_THE_WINDOW_SAYS: &str = "Tab related";

fn foot(screen: &[String]) -> &str {
    screen.last().expect("a screen with rows on it")
}

/// Opening a bead puts the window's keys on the foot, and leaving it puts the
/// forest's back. The reading before the bead is opened is what makes the
/// two after it mean something.
#[test]
fn the_foot_says_the_windows_keys_while_it_is_up_and_the_forests_when_it_has_gone() {
    let run = Run::over(THE_DESCRIBED_SUBTREE);
    let (forest, _) = run.after([walked()]);
    assert!(
        foot(&forest).contains(THE_FORESTS_KEYS),
        "the forest's own keys are not on its foot: {forest:#?}"
    );

    let (window, _) = run.after([walked(), the_first_bead_shown()]);
    assert!(
        foot(&window).contains(THE_WINDOWS_KEYS),
        "the bead window is up and its keys are not on the foot: {window:#?}"
    );
    assert!(
        !foot(&window).contains(A_WORD_ONLY_THE_FOREST_SAYS),
        "the bead window is up and the forest's keys are still on the foot: \
         {window:#?}"
    );

    let (back, _) = run.after([walked(), the_first_bead_shown(), pressed([KeyCode::Esc])]);
    assert!(
        foot(&back).contains(THE_FORESTS_KEYS),
        "the window has gone and the forest's keys have not come back: {back:#?}"
    );
    assert!(
        !foot(&back).contains(A_WORD_ONLY_THE_WINDOW_SAYS),
        "the window has gone and its keys are still on the foot: {back:#?}"
    );
}

/// What the same command said about a project of six loose roots and two
/// small trees: enough lines for a short screen's forest to scroll by more
/// than a notch.
const THE_LOOSE_ROOTS: &str = include_str!("../../../../tests/fixtures/bulk_loose_roots.json");

/// Nine rows leave the forest five, over nine lines, so the furthest it can
/// scroll is four: one further than a notch goes.
const NINE_ROWS: u16 = 9;

/// The fourth line of the forest, which is on screen before the notch and
/// after it.
const A_ROOT_THE_NOTCH_KEEPS_ON_SCREEN: &str = "dun-b1";

/// How far one notch moves the view for a run whose config says nothing.
/// Written out rather than read off the default, which would agree with any
/// number it was given.
const THREE_LINES: u16 = 3;

/// One notch of the wheel moves the forest three lines down.
///
/// Asserted as the distance one row travelled rather than the row it arrived
/// on, which is the same for every notch big enough to reach the end of the
/// forest.
#[test]
fn a_notch_moves_the_forest_three_lines() {
    let run = Run {
        rows: NINE_ROWS,
        ..Run::over(THE_LOOSE_ROOTS)
    };
    let (before, _) = run.after([typed("agj")]);
    let was = the_one_row(&before, A_ROOT_THE_NOTCH_KEEPS_ON_SCREEN);

    let (after, _) = run.after([typed("agj"), vec![Event::Scrolled(Notch::Down)]]);
    let now = the_one_row(&after, A_ROOT_THE_NOTCH_KEEPS_ON_SCREEN);

    assert_eq!(
        was.checked_sub(now),
        Some(THREE_LINES),
        "one notch moved the view from row {was} to row {now}: {after:#?}"
    );
}
