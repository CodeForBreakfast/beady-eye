//! A bead the window names is followed to, and the way back returns.
//!
//! What every assertion here names is the words a window says rather than the
//! title of a bead. The window is drawn from the *selection*, so one wrongly
//! left up after a jump draws over whatever the selection landed on and under
//! that bead's name — a test looking for the bead it came from to be gone
//! finds it gone, executes the line it is about, cannot observe it, and
//! passes.

mod terminal;

use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::{over_the_described_subtree, window_over, THE_DESCRIBED_SUBTREE};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// A gap this long between bytes means the frame is drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// Down onto the first bead of the tree and Enter, which shows it.
const SHOW_THE_FIRST_BEAD: &[u8] = b"j\r";

/// `^R`, which asks every project for itself again.
const REFRESH: &[u8] = b"\x12";

/// A collection is `bd` several times over, so it gets longer than a frame.
const LONG_ENOUGH_TO_COLLECT: Duration = Duration::from_secs(10);

/// `Tab`, which moves the window on to the next bead this one names.
const NEXT_BEAD_NAMED: &[u8] = b"\t";

/// `Enter`, which goes to the bead the window is on.
const FOLLOW: &[u8] = b"\r";

/// `Esc`, which goes back to the bead the reader followed from.
const BACK: &[u8] = b"\x1b";

/// The bead the walk lands on, which the window opens over.
const THE_FIRST_BEAD: &str = "dun-0tp.6";

/// Its parent, which is the tree's root and the bead `Tab` puts the ring on.
const ITS_PARENT: &str = "dun-0tp";

/// A collection landing after a follow leaves the window up, on the bead the
/// reader followed to.
///
/// `bead_still_shown` is consulted only when a collection lands, so a follow
/// read without one never asks the guard. The window closing here is a reader
/// dropped into the forest at the next poll, several seconds after the press
/// that looked as though it had worked.
#[test]
fn a_collection_after_a_follow_leaves_the_window_on_the_bead_followed_to() {
    let (mut bdi, tracker) = over_the_described_subtree("polled", ROWS, COLS, A_SILENCE);
    open_the_first_bead(&mut bdi);
    bdi.send(NEXT_BEAD_NAMED);
    bdi.send(FOLLOW);
    bdi.settle(A_SILENCE, GIVING_UP);

    tracker.holds(THE_DESCRIBED_SUBTREE);
    bdi.send(REFRESH);
    bdi.settle(A_SILENCE, LONG_ENOUGH_TO_COLLECT);

    let after = repaint(&mut bdi, ROWS);
    assert_eq!(
        window_over(&after).as_deref(),
        Some(ITS_PARENT),
        "a collection took the window down after a follow, or moved it off \
         the bead that was followed to. The screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&after),
        bdi.timeline()
    );
}

/// The way back from a follow returns to the bead the window was opened on,
/// and a second `Esc` leads out of the view from there. A reader who pressed
/// `Esc` twice would otherwise still be in a window.
#[test]
fn the_way_back_leaves_the_view_from_the_bead_the_window_was_opened_on() {
    let (mut bdi, _tracker) = over_the_described_subtree("left", ROWS, COLS, A_SILENCE);
    open_the_first_bead(&mut bdi);
    bdi.send(NEXT_BEAD_NAMED);
    bdi.send(FOLLOW);
    bdi.settle(A_SILENCE, GIVING_UP);
    bdi.send(BACK);
    bdi.settle(A_SILENCE, GIVING_UP);

    bdi.send(BACK);
    bdi.settle(A_SILENCE, GIVING_UP);

    let out = repaint(&mut bdi, ROWS);
    assert!(
        window_over(&out).is_none(),
        "a second way back left a window up. The screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&out),
        bdi.timeline()
    );
}

/// Show the first bead of the tree, and hand back the screen that proves the
/// window is up — so a test that reads the screen after a press is reading
/// what the press did rather than a window that never opened.
#[track_caller]
fn open_the_first_bead(bdi: &mut Driven) -> Vec<u8> {
    bdi.send(SHOW_THE_FIRST_BEAD);
    bdi.settle(A_SILENCE, GIVING_UP);
    let up = repaint(bdi, ROWS + 1);
    assert_eq!(
        window_over(&up).as_deref(),
        Some(THE_FIRST_BEAD),
        "no window opened over the first bead. The screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&up),
        bdi.timeline()
    );
    up
}

/// The screen as it stands, rather than as it differs from the frame before:
/// a resize is answered by drawing every cell again.
#[track_caller]
fn repaint(bdi: &mut Driven, rows: u16) -> Vec<u8> {
    let repainted = bdi.resize(rows, COLS);
    bdi.answer_to(repainted, GIVING_UP)
}
