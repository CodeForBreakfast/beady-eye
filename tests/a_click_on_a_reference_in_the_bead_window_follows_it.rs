//! A reference the bead window draws is followed by clicking it, and the rest
//! of the window answers a click the way it did before.
//!
//! `Screen::clicked_bead` is reached through the binary by nothing else: it is
//! a delegation no unit test makes, so it could return a constant and every
//! click over a bead window would do the wrong thing with the suite still
//! green. What is under test is the whole road — xterm's SGR encoding off the
//! wire, `wire::incoming` turning it into a row, and the window being asked
//! what it drew there.
//!
//! No row here is written down. Which row a reference lands on follows from
//! the screen's size, the bead's length and how far the reader has scrolled,
//! and a test that worked one out would be running the arithmetic it exists to
//! check a second time — green whenever both copies are wrong the same way. So
//! every row is read back off the frame `bdi` drew, by [`row_of`], and
//! [`the_rows_read_off_a_frame_are_the_rows_bdi_drew_on`] is what says that
//! reading can be trusted.
//!
//! What says a window is up is the words every window says, `Esc to go back`,
//! rather than the title of a bead: the window is drawn from the *selection*,
//! so one wrongly left up after a click draws over whatever the selection
//! landed on and under that bead's name.

mod terminal;

use std::time::Duration;

use terminal::driver::{clicked_on, Driven, GIVING_UP};
use terminal::shims::ShimmedTracker;
use terminal::{contains, over_the_described_subtree, row_of};
use terminal::{THE_FIRST_BEAD, THE_TREES_HEADER};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// A gap this long between bytes means the frame is drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// Down onto the first bead of the tree and Enter, which shows it, then `G`,
/// which takes the window to the end of the bead. The sections are the last
/// thing on the page and this bead's description is longer than the screen, so
/// without the last of those keys there is no reference on the screen to
/// click.
const SHOW_THE_FIRST_BEAD_WHERE_ITS_SECTIONS_ARE: &[u8] = b"j\rG";

/// `Esc`, which goes back to the bead a reference was followed from.
const BACK: &[u8] = b"\x1b";

/// The arrow the window draws to a bead's parent, which the forest draws too
/// and so can be gone to.
const A_REFERENCE_THAT_CAN_BE_FOLLOWED: &[u8] = "↑".as_bytes();

/// A bead the capture's answer does not hold, which is drawn saying so and
/// which `Tab` passes over.
const A_REFERENCE_THAT_CANNOT: &[u8] = "bdi-rer.9  not in the tracker's answer".as_bytes();

/// The heading over the parent, which is the row immediately above the
/// reference — where a reader aiming at it and missing lands.
const A_ROW_OF_THE_PAGE_THAT_NAMES_NO_BEAD: &[u8] = "PARENT".as_bytes();

/// The project's own line at the head of the forest, which the window is not
/// drawn over.
const A_ROW_THE_WINDOW_IS_NOT_ON: &[u8] = "atlas".as_bytes();

/// The title of the window over the first bead of the capture's tree, and the
/// whole of it: the tree's header is `bdi-0tp`, so the id alone would also be
/// met by the window over its own root. Drawn on the window's top border,
/// which is how this test names that row.
const THE_FIRST_BEADS_WINDOW: &[u8] = "bdi-0tp.6 · Esc to go back".as_bytes();

/// The title of the window over that bead's parent, which is the tree's root.
const ITS_PARENTS_WINDOW: &[u8] = "bdi-0tp · Esc to go back".as_bytes();

/// The part of that title every bead's window says, whichever bead it is on.
/// This is what says a window is *up*.
const A_BEAD_WINDOW: &[u8] = "Esc to go back".as_bytes();

/// The rows two forest lines are drawn on are the rows the harness already
/// says they are.
///
/// Every test below reads a row off a frame and presses on it, so a reading
/// that is quietly wrong makes all of them green by construction — the press
/// lands somewhere nobody chose, and whatever it does there is what gets
/// asserted. These two rows are settled elsewhere: `THE_TREES_HEADER` and
/// `THE_FIRST_BEAD` are where `over_the_described_subtree`'s walk leaves the
/// forest, and `a_click_opens_the_bead_it_lands_on` presses on both of them
/// and says which bead each opens.
#[test]
fn the_rows_read_off_a_frame_are_the_rows_bdi_drew_on() {
    let (mut bdi, _tracker) = over_the_described_subtree("read-back", ROWS, COLS, A_SILENCE);
    let forest = repaint(&mut bdi, ROWS + 1);

    assert_eq!(
        row_of(&forest, "bdi-0tp  Every outside program".as_bytes()),
        Some(THE_TREES_HEADER),
        "the tree's header was read off the frame on another row. The screen \
         it drew: {:?}\n{}",
        String::from_utf8_lossy(&forest),
        bdi.timeline()
    );
    assert_eq!(
        row_of(&forest, ".6       Change sources".as_bytes()),
        Some(THE_FIRST_BEAD),
        "the first bead was read off the frame on another row. The screen it \
         drew: {:?}\n{}",
        String::from_utf8_lossy(&forest),
        bdi.timeline()
    );
}

/// And words drawn twice are read off no row at all.
///
/// Handing back the first is the natural implementation and it hands back a
/// plausible row, so a test built on it presses somewhere nobody chose and
/// asserts whatever that did. This screen is where it would bite: a bead's own
/// row in the forest and the window's reference to that bead say the same
/// words, one over the other.
#[test]
fn words_drawn_twice_are_read_off_no_row() {
    let (bdi, _tracker, page) = at_the_end_of_the_first_beads_page("ambiguous");
    let in_the_forest_and_in_the_window = "bdi-0tp  Every outside program".as_bytes();

    assert!(
        contains(&page, in_the_forest_and_in_the_window),
        "the words are on no row at all, so this says nothing. The screen it \
         drew: {:?}\n{}",
        String::from_utf8_lossy(&page),
        bdi.timeline()
    );
    assert_eq!(
        row_of(&page, in_the_forest_and_in_the_window),
        None,
        "words drawn twice were read off a row. The screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&page),
        bdi.timeline()
    );
}

/// A click on a reference the forest can reach goes to that bead, the way
/// `Enter` on the ring does.
#[test]
fn a_click_on_a_reference_goes_to_the_bead_it_names() {
    let (mut bdi, _tracker, page) = at_the_end_of_the_first_beads_page("followed");
    let on = the_row(&mut bdi, &page, A_REFERENCE_THAT_CAN_BE_FOLLOWED);

    bdi.send(&clicked_on(on));
    bdi.settle(A_SILENCE, GIVING_UP);

    let after = repaint(&mut bdi, ROWS + 2);
    assert!(
        contains(&after, A_BEAD_WINDOW),
        "the click on a reference took the window away rather than following \
         it. The screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&after),
        bdi.timeline()
    );
    assert!(
        contains(&after, ITS_PARENTS_WINDOW),
        "the click on the parent did not go to it. The screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&after),
        bdi.timeline()
    );
}

/// And leaves the way back behind it, so `Esc` returns to the bead the
/// reference was clicked in.
///
/// A click that went without recording where it went from would strand the
/// reader one press from where they were reading, which is worse than not
/// following at all: the keyboard's way out looks as though it should work and
/// lands them somewhere else.
#[test]
fn the_way_back_returns_to_the_bead_a_click_followed_from() {
    let (mut bdi, _tracker, page) = at_the_end_of_the_first_beads_page("returned");
    let on = the_row(&mut bdi, &page, A_REFERENCE_THAT_CAN_BE_FOLLOWED);
    bdi.send(&clicked_on(on));
    bdi.settle(A_SILENCE, GIVING_UP);

    bdi.send(BACK);
    bdi.settle(A_SILENCE, GIVING_UP);

    let back = repaint(&mut bdi, ROWS + 2);
    assert!(
        contains(&back, THE_FIRST_BEADS_WINDOW),
        "the way back did not return to the bead the reference was clicked \
         in. The screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&back),
        bdi.timeline()
    );
}

/// A click on the page that is not on a reference leaves the window exactly as
/// it was — here the heading directly above the parent, which is where a
/// reader aiming at the reference and missing by a row lands.
///
/// This is the failure the bug was worth fixing for. A window that went away
/// on any click would take itself off the screen on the press that was meant
/// to follow something in it.
#[test]
fn a_click_on_the_page_that_names_no_bead_leaves_the_window_alone() {
    let (mut bdi, _tracker, page) = at_the_end_of_the_first_beads_page("missed");
    let on = the_row(&mut bdi, &page, A_ROW_OF_THE_PAGE_THAT_NAMES_NO_BEAD);

    bdi.send(&clicked_on(on));
    bdi.settle(A_SILENCE, GIVING_UP);

    let after = repaint(&mut bdi, ROWS + 2);
    assert!(
        contains(&after, THE_FIRST_BEADS_WINDOW),
        "a click a row off a reference took the window away, or moved it. The \
         screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&after),
        bdi.timeline()
    );
}

/// And so does a click on a reference the forest cannot take the reader to,
/// which is the same answer `Tab` gives by passing over it.
#[test]
fn a_click_on_a_reference_that_cannot_be_followed_leaves_the_window_alone() {
    let (mut bdi, _tracker, page) = at_the_end_of_the_first_beads_page("unreachable");
    let on = the_row(&mut bdi, &page, A_REFERENCE_THAT_CANNOT);

    bdi.send(&clicked_on(on));
    bdi.settle(A_SILENCE, GIVING_UP);

    let after = repaint(&mut bdi, ROWS + 2);
    assert!(
        contains(&after, THE_FIRST_BEADS_WINDOW),
        "a click on a bead the answer does not hold took the window away, or \
         went somewhere. The screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&after),
        bdi.timeline()
    );
}

/// A click off the window takes it away, which is what a click over a bead
/// window has always done and the pointer's only way out of one.
#[test]
fn a_click_off_the_window_takes_it_away() {
    let (mut bdi, _tracker, page) = at_the_end_of_the_first_beads_page("dismissed");
    let on = the_row(&mut bdi, &page, A_ROW_THE_WINDOW_IS_NOT_ON);

    bdi.send(&clicked_on(on));
    bdi.settle(A_SILENCE, GIVING_UP);

    let after = repaint(&mut bdi, ROWS + 2);
    assert!(
        !contains(&after, A_BEAD_WINDOW),
        "a click on the forest round the window left the window up. The \
         screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&after),
        bdi.timeline()
    );
}

/// And so does a click on the window's own border, which is the row the way
/// out is written on.
///
/// Without it a window as tall as the screen has no row off it to click, and a
/// reader holding only a pointer could not leave. The bead window reaches that
/// height on a screen of twenty-four rows or fewer, which is an ordinary
/// terminal.
#[test]
fn a_click_on_the_windows_border_takes_it_away() {
    let (mut bdi, _tracker, page) = at_the_end_of_the_first_beads_page("bordered");
    let on = the_row(&mut bdi, &page, THE_FIRST_BEADS_WINDOW);

    bdi.send(&clicked_on(on));
    bdi.settle(A_SILENCE, GIVING_UP);

    let after = repaint(&mut bdi, ROWS + 2);
    assert!(
        !contains(&after, A_BEAD_WINDOW),
        "a click on the border that says how to leave left the window up. The \
         screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&after),
        bdi.timeline()
    );
}

/// Open the first bead of the tree at the end of its page, where its sections
/// are, and hand back the frame that shows them — so a test that reads a row
/// off it is reading a window that opened.
///
/// The frame is read at the size the clicks are sent at, and no test resizes
/// between the two: the window's rows follow the screen's height, so a row
/// read off one size and pressed at another is a row nobody drew.
#[track_caller]
fn at_the_end_of_the_first_beads_page(named: &str) -> (Driven, ShimmedTracker, Vec<u8>) {
    let (mut bdi, tracker) = over_the_described_subtree(named, ROWS, COLS, A_SILENCE);
    bdi.send(SHOW_THE_FIRST_BEAD_WHERE_ITS_SECTIONS_ARE);
    bdi.settle(A_SILENCE, GIVING_UP);

    let page = repaint(&mut bdi, ROWS + 1);
    assert!(
        contains(&page, THE_FIRST_BEADS_WINDOW),
        "no window opened over the first bead. The screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&page),
        bdi.timeline()
    );
    (bdi, tracker, page)
}

/// The row of a frame something was drawn on, or a failure naming what was
/// looked for and the screen that did not hold it once.
#[track_caller]
fn the_row(bdi: &mut Driven, screen: &[u8], needle: &[u8]) -> u16 {
    row_of(screen, needle).unwrap_or_else(|| {
        panic!(
            "{:?} was drawn on no row of the screen, or on more than one, so \
             there is no row for this test to press. The screen it drew: \
             {:?}\n{}",
            String::from_utf8_lossy(needle),
            String::from_utf8_lossy(screen),
            bdi.timeline()
        )
    })
}

/// The screen as it stands, rather than as it differs from the frame before:
/// a resize is answered by drawing every cell again.
#[track_caller]
fn repaint(bdi: &mut Driven, rows: u16) -> Vec<u8> {
    let repainted = bdi.resize(rows, COLS);
    bdi.answer_to(repainted, GIVING_UP)
}
