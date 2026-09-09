//! The wheel over the forest moves what is on screen and leaves the selection
//! alone, through the binary.
//!
//! What is under test is the whole road: xterm's SGR encoding of a notch off
//! the wire, `wire::incoming` turning it into a scroll, and the forest moving
//! a viewport of its own under a selection that stays where the reader put
//! it. None of it is reachable from a test that drives the forest directly,
//! because the distance a notch travels is `bdi`'s answer to one report from
//! the terminal and nothing else asks the question that way.
//!
//! The screen is nine rows because the forest has to have more lines than it
//! has room for. Nine rows leave the forest five, over nine lines, so the
//! furthest it can scroll is four — one further than a notch goes, which is
//! what makes the distance the notch's rather than the end of the forest's.

mod terminal;

use std::time::Duration;

use terminal::driver::{clicked_on, Driven, A_NOTCH_DOWN, GIVING_UP};
use terminal::{contains, over_the_loose_roots, row_of, rows_of, THE_FIRST_ROOTS_WINDOW};

/// Nine rows and ten draw the same five-row forest band, so the resize that
/// asks for a repaint does not move the rows it is asked about.
const ROWS: u16 = 9;
const COLS: u16 = 120;

/// A gap this long between bytes means the frame is drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// Enter, which shows the bead the selection is on.
const SHOW_BEAD: &[u8] = b"\r";

/// How far one notch moves the view, for a run whose config says nothing —
/// `[tui]`'s `wheel_notch_lines`. Written out rather than read off the
/// default, which would agree with any number it was given.
const THREE_LINES: u16 = 3;

/// The fourth line of the forest, and the first of the six loose roots the
/// capture ends with. Far enough down that it is on screen before the notch
/// and still on screen after it, which is what lets one row say how far the
/// view travelled.
const A_ROOT_THE_NOTCH_KEEPS_ON_SCREEN: &[u8] = b"orb-b1";

/// The window over that root, for a click that lands on it.
const THAT_ROOTS_WINDOW: &str = "orb-b1 · Esc to go back";

/// The root the walk leaves the selection on, which the notch takes off the
/// screen — the whole point of the test that names it.
const THE_SELECTED_ROOT: &[u8] = b"orb-c3";

/// One notch of the wheel moves the forest three lines down.
///
/// Asserted as the distance one row travelled rather than as the row it
/// arrived on: the row it arrives on is the same for every notch big enough
/// to reach the end of the forest, and the distance is the thing chosen.
#[test]
fn a_notch_moves_the_forest_three_lines() {
    let (mut bdi, _tracker) = over_the_loose_roots("wheeled", ROWS, COLS, A_SILENCE);

    let before = repaint(&mut bdi, ROWS + 1);
    let was = row_of(&before, A_ROOT_THE_NOTCH_KEEPS_ON_SCREEN).unwrap_or_else(|| {
        panic!(
            "that root is drawn on no one row before the notch. The screen it \
             drew: {:?}\n{}",
            String::from_utf8_lossy(&before),
            bdi.timeline()
        )
    });

    bdi.send(A_NOTCH_DOWN);
    bdi.settle(A_SILENCE, GIVING_UP);

    let after = repaint(&mut bdi, ROWS);
    let now = row_of(&after, A_ROOT_THE_NOTCH_KEEPS_ON_SCREEN).unwrap_or_else(|| {
        panic!(
            "that root is drawn on no one row after the notch. The screen it \
             drew: {:?}\n{}",
            String::from_utf8_lossy(&after),
            bdi.timeline()
        )
    });

    assert_eq!(
        was.checked_sub(now),
        Some(THREE_LINES),
        "one notch moved the view from row {was} to row {now}. The screen it \
         drew: {:?}\n{}",
        String::from_utf8_lossy(&after),
        bdi.timeline()
    );
}

/// The selection stays on the bead it was on, even where the notch has taken
/// that bead off the screen.
///
/// The window is the instrument, as it is for a click: it is drawn from the
/// selection and titled with the selected bead's id, so it says which bead
/// the selection is on whether or not that bead has a row left to sit on.
/// Three lines take the first root off a five-row band, so this is the
/// off-screen case and not merely the unmoved one.
#[test]
fn a_notch_leaves_the_selection_on_the_bead_it_was_on() {
    let (mut bdi, _tracker) = over_the_loose_roots("kept", ROWS, COLS, A_SILENCE);
    bdi.send(A_NOTCH_DOWN);
    bdi.settle(A_SILENCE, GIVING_UP);

    let scrolled = repaint(&mut bdi, ROWS + 1);
    assert!(
        rows_of(&scrolled, THE_SELECTED_ROOT).is_empty(),
        "the notch left the selected root on screen, so what follows says \
         only that the selection did not move, not that it survived going \
         off the screen. The screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&scrolled),
        bdi.timeline()
    );

    bdi.send(SHOW_BEAD);
    bdi.settle(A_SILENCE, GIVING_UP);

    let opened = repaint(&mut bdi, ROWS);
    assert!(
        contains(&opened, THE_FIRST_ROOTS_WINDOW.as_bytes()),
        "the window that opened after the notch is not the selected root's. \
         The screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&opened),
        bdi.timeline()
    );
}

/// A click after the wheel selects the row under the pointer.
///
/// The inverse the click asks and the skip the drawing takes are one
/// agreement about where a line goes, and a viewport the drawing reads and
/// the inverse does not is exactly how they drift apart. The top row of the
/// band is the one that says so: it is the line the view was scrolled to,
/// and under an inverse still deriving the offset from the selection it would
/// name the first line of the forest instead.
#[test]
fn a_click_after_the_wheel_selects_the_row_under_the_pointer() {
    let (mut bdi, _tracker) = over_the_loose_roots("clicked", ROWS, COLS, A_SILENCE);
    bdi.send(A_NOTCH_DOWN);
    bdi.settle(A_SILENCE, GIVING_UP);

    let scrolled = repaint(&mut bdi, ROWS + 1);
    let top = row_of(&scrolled, A_ROOT_THE_NOTCH_KEEPS_ON_SCREEN).unwrap_or_else(|| {
        panic!(
            "that root is drawn on no one row after the notch. The screen it \
             drew: {:?}\n{}",
            String::from_utf8_lossy(&scrolled),
            bdi.timeline()
        )
    });

    bdi.send(&clicked_on(top));
    bdi.settle(A_SILENCE, GIVING_UP);
    bdi.send(SHOW_BEAD);
    bdi.settle(A_SILENCE, GIVING_UP);

    let opened = repaint(&mut bdi, ROWS);
    assert!(
        contains(&opened, THAT_ROOTS_WINDOW.as_bytes()),
        "the click landed on row {top}, and the window that opened is not the \
         root drawn there. The screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&opened),
        bdi.timeline()
    );
}

/// The screen as it stands, rather than as it differs from the frame before:
/// a resize is answered by drawing every cell again.
#[track_caller]
fn repaint(bdi: &mut Driven, rows: u16) -> Vec<u8> {
    let repainted = bdi.resize(rows, COLS);
    bdi.answer_to(repainted, GIVING_UP)
}
