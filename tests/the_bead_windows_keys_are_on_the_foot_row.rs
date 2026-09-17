//! The foot row says the bead window's keys while the window is up, and the
//! forest's the rest of the time.
//!
//! Which row the foot is handed is decided in `Screen::draw`, off the loop's
//! `Showing`, and no unit test reaches it: the test helper that draws a frame
//! hands the foot a row of its own, so both arms of that match could name the
//! same function with the suite still green. This is the choice reached the
//! way a reader reaches it, through the binary, by opening a bead and leaving
//! it again.
//!
//! The row is read off the foot rather than out of the stream. A word of it
//! found anywhere on the screen would also be found in a bead's prose, and
//! the assertion that matters is an absence: the forest's keys are *gone*
//! while the window is up, which is a claim about one row rather than about
//! the screen.

mod terminal;

use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::{over_the_described_subtree, rows_drawn};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// A gap this long between bytes means the frame is drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// From the tree's header down onto the first bead, and Enter, which shows
/// it. One motion rather than two: the anomaly the tree's dangling edges
/// raise is drawn between the two rows and a motion passes over it.
const SHOW_THE_FIRST_BEAD: &[u8] = b"j\r";

/// `Esc`, which takes the window down.
const LEAVE_THE_WINDOW: &[u8] = b"\x1b";

/// The whole of the bead window's row, from `keys::bead_key_rows`.
const THE_WINDOWS_KEYS: &str = "Esc back   ? keys   Tab related   y id";

/// The whole of the forest's, from `keys::key_row`.
const THE_FORESTS_KEYS: &str = "a all   ? keys   / find   q quit";

/// A word of the forest's row that the window's does not say, for the
/// absence. `? keys` is on both rows, so a test asserting the forest's whole
/// row had gone would pass on a foot that still said half of it.
const A_WORD_ONLY_THE_FOREST_SAYS: &str = "q quit";

/// And the same the other way round.
const A_WORD_ONLY_THE_WINDOW_SAYS: &str = "Tab related";

/// Opening a bead puts the window's keys on the foot; leaving it puts the
/// forest's back.
///
/// The reading before the bead is opened is what makes the two after it mean
/// something: without it, a foot that had said the window's keys from the
/// first frame would pass.
#[test]
fn the_foot_says_the_windows_keys_while_it_is_up_and_the_forests_when_it_has_gone() {
    let (mut bdi, _tracker) = over_the_described_subtree("window-keys", ROWS, COLS, A_SILENCE);

    let forest = foot_of(&mut bdi, ROWS + 1);
    assert!(
        forest.contains(THE_FORESTS_KEYS),
        "the forest's own keys are not on its foot: {forest:?}\n{}",
        bdi.timeline()
    );

    bdi.send(SHOW_THE_FIRST_BEAD);
    bdi.settle(A_SILENCE, GIVING_UP);

    let window = foot_of(&mut bdi, ROWS);
    assert!(
        window.contains(THE_WINDOWS_KEYS),
        "the bead window is up and its keys are not on the foot: \
         {window:?}\n{}",
        bdi.timeline()
    );
    assert!(
        !window.contains(A_WORD_ONLY_THE_FOREST_SAYS),
        "the bead window is up and the forest's keys are still on the foot: \
         {window:?}\n{}",
        bdi.timeline()
    );

    bdi.send(LEAVE_THE_WINDOW);
    bdi.settle(A_SILENCE, GIVING_UP);

    let back = foot_of(&mut bdi, ROWS + 1);
    assert!(
        back.contains(THE_FORESTS_KEYS),
        "the window has gone and the forest's keys have not come back: \
         {back:?}\n{}",
        bdi.timeline()
    );
    assert!(
        !back.contains(A_WORD_ONLY_THE_WINDOW_SAYS),
        "the window has gone and its keys are still on the foot: \
         {back:?}\n{}",
        bdi.timeline()
    );
}

/// The row at the foot of a frame `bdi` was made to repaint whole: a frame
/// drawn as the difference from the one before holds only the cells that
/// moved, and the foot is a row that often has not.
#[track_caller]
fn foot_of(bdi: &mut Driven, rows: u16) -> String {
    let repainted = bdi.resize(rows, COLS);
    let screen = bdi.answer_to(repainted, GIVING_UP);
    rows_drawn(&screen).pop().expect("a screen with rows on it")
}
