//! The bead window moves down its bead, and goes when its bead does.
//!
//! Nothing reaches either through the binary. No pty test has opened a bead
//! window at all, so `Screen::scroll` and `Screen::bead_still_shown` are
//! delegations a run never makes: both could return a constant, and the
//! wheel over a bead and a collection that took a bead away would both stop
//! working with the suite still green.
//!
//! `bead_still_shown` is driven both ways round. A collection that drops the
//! shown bead must take the window down, and one that keeps it must leave the
//! window up — the two answers are separate tests because a screen that
//! always closed and a screen that never did would each pass the other's.

mod terminal;

use std::time::Duration;

use terminal::driver::{Driven, A_NOTCH_DOWN, GIVING_UP};
use terminal::{contains, over_the_described_subtree, window_over, THE_DESCRIBED_SUBTREE};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// A gap this long between bytes means the frame is drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// A collection is `bd` several times over, so it gets longer than a frame.
const LONG_ENOUGH_TO_COLLECT: Duration = Duration::from_secs(10);

/// From the tree's header, where the walk leaves the selection, down onto the
/// first bead — and Enter, which shows it. One press rather than two: the
/// anomaly the tree's dangling edges raise is drawn between the two rows and
/// a motion passes over it, because nothing about it is a thing to select.
const SHOW_THE_FIRST_BEAD: &[u8] = b"j\r";

/// `^R`, which asks every project for itself again.
const REFRESH: &[u8] = b"\x12";

/// The bead the window this test opens is over. `window_over` says which
/// bead a window is on, and nothing where no window is up — which is the
/// distinction the absence below turns on.
const ITS_BEAD: &str = "dun-0tp.6";

/// A word of the opening lines of that bead's description, and one nothing
/// else on the screen says — the bead's own title is drawn over it and is
/// most of that first sentence, so the word has to be one the title stops
/// short of. One word rather than a phrase, because the window draws its
/// prose in the terminal's own colour, so its spaces are cells nothing has to
/// write and it reaches the wire a word at a time.
const THE_TOP_OF_THE_BEAD: &[u8] = "shape".as_bytes();

/// The same for its last lines, under *Blocked on*. What makes the pair a
/// measurement is that the window is far too short for both to be on it at
/// once: at forty rows it holds thirty lines of a bead that is over forty.
const THE_FOOT_OF_THE_BEAD: &[u8] = "twice".as_bytes();

/// Notches enough to reach the foot of that bead from its top, and more:
/// each is one row, the view stops at the last row of the bead however many
/// arrive, and a test that counted them exactly would be asserting how the
/// window wraps rather than that the wheel moves it.
const NOTCHES: usize = 40;

/// The wheel over the bead window moves the window down the bead.
///
/// Both ends are asserted from a repaint rather than from the difference the
/// notches made, and the reading before the wheel is what makes the reading
/// after it mean something: without it, a window that had shown the foot of
/// the bead all along would pass.
#[test]
fn the_wheel_moves_the_bead_window_down_its_bead() {
    let (mut bdi, _tracker) = over_the_described_subtree("scrolled", ROWS, COLS, A_SILENCE);
    bdi.send(SHOW_THE_FIRST_BEAD);
    bdi.settle(A_SILENCE, GIVING_UP);

    let at_the_top = repaint(&mut bdi, ROWS + 1);
    assert!(
        contains(&at_the_top, THE_TOP_OF_THE_BEAD) && !contains(&at_the_top, THE_FOOT_OF_THE_BEAD),
        "the window opens at the top of its bead, and this one did not. The \
         screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&at_the_top),
        bdi.timeline()
    );

    for _ in 0..NOTCHES {
        bdi.send(A_NOTCH_DOWN);
    }
    bdi.settle(A_SILENCE, GIVING_UP);

    let wheeled = repaint(&mut bdi, ROWS);
    assert!(
        contains(&wheeled, THE_FOOT_OF_THE_BEAD),
        "{NOTCHES} notches of the wheel left the window where it was. The \
         screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&wheeled),
        bdi.timeline()
    );
}

/// A collection that no longer holds the bead the window is on takes the
/// window down: drawn from the selection, it would otherwise show another
/// bead, or nothing, under a title the reader did not open.
#[test]
fn a_collection_that_drops_the_shown_bead_takes_the_window_down() {
    let (mut bdi, tracker) = over_the_described_subtree("dropped", ROWS, COLS, A_SILENCE);
    let up = open_the_first_bead(&mut bdi);

    tracker.holds(&without_the_first_bead());
    bdi.send(REFRESH);
    bdi.settle(A_SILENCE, LONG_ENOUGH_TO_COLLECT);

    let after = repaint(&mut bdi, ROWS);
    assert!(
        window_over(&after).is_none(),
        "the tracker no longer holds that bead and a window is still up. \
         The screen it drew: {:?}\nThe screen before the refresh: {:?}\n{}",
        String::from_utf8_lossy(&after),
        String::from_utf8_lossy(&up),
        bdi.timeline()
    );
}

/// And a collection that still holds it leaves the window up. Without this
/// the test above is met by a window that closes on every collection, which
/// would take the bead the reader is reading away from them at each poll.
#[test]
fn a_collection_that_keeps_the_shown_bead_leaves_the_window_up() {
    let (mut bdi, tracker) = over_the_described_subtree("kept", ROWS, COLS, A_SILENCE);
    let up = open_the_first_bead(&mut bdi);

    tracker.holds(THE_DESCRIBED_SUBTREE);
    bdi.send(REFRESH);
    bdi.settle(A_SILENCE, LONG_ENOUGH_TO_COLLECT);

    let after = repaint(&mut bdi, ROWS);
    assert_eq!(
        window_over(&after).as_deref(),
        Some(ITS_BEAD),
        "the tracker still holds that bead and its window has gone. The \
         screen it drew: {:?}\nThe screen before the refresh: {:?}\n{}",
        String::from_utf8_lossy(&after),
        String::from_utf8_lossy(&up),
        bdi.timeline()
    );
}

/// Show the first bead of the tree, and hand back the screen that proves the
/// window is up — so a test that reads the screen after a collection is
/// reading what the collection did rather than a window that never opened.
#[track_caller]
fn open_the_first_bead(bdi: &mut Driven) -> Vec<u8> {
    bdi.send(SHOW_THE_FIRST_BEAD);
    bdi.settle(A_SILENCE, GIVING_UP);
    let up = repaint(bdi, ROWS + 1);
    assert_eq!(
        window_over(&up).as_deref(),
        Some(ITS_BEAD),
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

/// The capture with the bead the window is opened on taken out of it, which
/// is a bead that has left the tracker — one of the two ways a collection
/// moves the selection off the bead a window is showing.
fn without_the_first_bead() -> String {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(THE_DESCRIBED_SUBTREE).expect("a capture of bd list --json");
    let kept: Vec<serde_json::Value> = rows
        .into_iter()
        .filter(|row| row["id"] != "dun-0tp.6")
        .collect();
    assert_eq!(kept.len(), 4, "the capture has to have held that bead");
    serde_json::to_string(&kept).expect("rows serialise")
}
