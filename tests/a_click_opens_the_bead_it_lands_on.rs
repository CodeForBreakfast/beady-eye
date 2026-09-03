//! A click on a bead's row puts the selection there, through the binary.
//!
//! Every key the pty tests type is a key: no test has ever sent `bdi` a mouse
//! report, so `Screen::clicked` — the one method of the adapter that is more
//! than a delegation — has been reached by nothing a reader does. What is
//! under test is the whole road: xterm's SGR encoding off the wire,
//! `wire::incoming` turning it into a row, and the screen being asked which
//! line the forest drew there.
//!
//! The bead window is the instrument rather than the subject. A selection
//! that has moved and one that has not draw the same rows in different
//! colours, and a colour reaches the wire as a difference from the frame
//! before; the window's title is the selected bead's id, drawn in a span of
//! its own, and says outright which row the click landed on.

mod terminal;

use std::time::Duration;

use terminal::driver::{clicked_on, GIVING_UP};
use terminal::{contains, over_the_described_subtree, THE_FIRST_BEAD, THE_TREES_HEADER};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// A gap this long between bytes means the frame is drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// Enter, which shows the bead the selection is on.
const SHOW_BEAD: &[u8] = b"\r";

/// The title of the window over the first bead of the capture's tree, from
/// `view::show`. Whole rather than one word, because the title is drawn in a
/// span of its own and so writes its own spaces — and whole is the point: the
/// tree's header is `bdi-0tp`, so a needle of the id alone would also be met
/// by the window this test exists to prove was *not* opened.
const THE_FIRST_BEADS_WINDOW: &str = "bdi-0tp.6 · Esc to go back";

/// The same for the tree's header, which is the row the walk leaves the
/// selection on.
const THE_TREES_WINDOW: &str = "bdi-0tp · Esc to go back";

/// A click on a bead's row, and then Enter: the window that opens names the
/// bead drawn on that row rather than the one the selection was on.
#[test]
fn a_click_on_a_beads_row_selects_the_bead_drawn_there() {
    assert_eq!(
        the_window_a_click_opens("clicked-bead", THE_FIRST_BEAD),
        Some(THE_FIRST_BEADS_WINDOW),
        "the click landed on the first bead's row, and the window that opened \
         is not that bead's"
    );
}

/// The same walk with the click landing where the selection already is.
///
/// It is what makes the test above about the click: a `clicked` that moved
/// nothing at all would leave the selection on the tree's header, and this
/// says which window that is.
#[test]
fn a_click_on_the_row_the_selection_rests_on_opens_that_row() {
    assert_eq!(
        the_window_a_click_opens("clicked-header", THE_TREES_HEADER),
        Some(THE_TREES_WINDOW),
        "the click landed on the tree's header, and the window that opened is \
         not the tree's"
    );
}

/// Open a `bdi` over the described subtree, click that row, show whatever the
/// selection is on now, and say which of the two windows the screen holds.
///
/// A `bdi` of its own for each row rather than two clicks over one, because a
/// click over the bead view takes the view down and does no more: a second
/// click would be spent closing the first window rather than selecting
/// anything.
///
/// The screen is read from a repaint rather than from the difference the
/// window made, so what is asserted is the screen as it stands.
#[track_caller]
fn the_window_a_click_opens(named: &str, row: u16) -> Option<&'static str> {
    let (mut bdi, _tracker) = over_the_described_subtree(named, ROWS, COLS, A_SILENCE);
    bdi.send(&clicked_on(row));
    bdi.settle(A_SILENCE, GIVING_UP);
    bdi.send(SHOW_BEAD);
    bdi.settle(A_SILENCE, GIVING_UP);

    let repainted = bdi.resize(ROWS + 1, COLS);
    let screen = bdi.answer_to(repainted, GIVING_UP);
    [THE_FIRST_BEADS_WINDOW, THE_TREES_WINDOW]
        .into_iter()
        .find(|window| contains(&screen, window.as_bytes()))
}
