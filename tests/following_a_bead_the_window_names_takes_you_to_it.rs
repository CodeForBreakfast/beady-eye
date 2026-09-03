//! A bead the window names is followed to, and the way back returns.
//!
//! Nothing reaches `Screen::follow` or `Screen::retrace` through the binary
//! otherwise: both are delegations a run never makes under the unit tests, so
//! each could return a constant and `Tab`, `Enter` and `Esc` would all stop
//! working over a bead window with the suite still green.
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
use terminal::{contains, over_the_described_subtree, THE_DESCRIBED_SUBTREE};

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

/// The title of the window over the bead the walk lands on, and the whole of
/// it: the tree's header is `bdi-0tp`, so the id alone would also be met by
/// the window over the row above.
const THE_FIRST_BEADS_WINDOW: &[u8] = "bdi-0tp.6 · Esc to go back".as_bytes();

/// The title of the window over that bead's parent, which is the tree's root
/// and the bead `Tab` puts the ring on.
const ITS_PARENTS_WINDOW: &[u8] = "bdi-0tp · Esc to go back".as_bytes();

/// The part of that title every bead's window says, whichever bead it is on.
/// This is what says a window is *up*.
const A_BEAD_WINDOW: &[u8] = "Esc to go back".as_bytes();

/// The keys the title offers once a bead this one names can be gone to. Drawn
/// only where a press would do something, so it is also what says the forest
/// answered that the parent is reachable.
const THE_KEYS_THAT_FOLLOW: &[u8] = "Tab, Enter to follow".as_bytes();

/// Following the parent moves the forest to it and redraws the window there.
///
/// The window is drawn from the selection, so the second reading is two
/// claims at once: the selection moved, and the window survived the move.
/// `bead_still_shown` closes the window when the selection leaves the bead it
/// was opened on, and it cannot tell a reader's deliberate jump from a
/// collection dragging them off — so a follow that did not say where it had
/// gone would close the window on the very press that asked for the bead.
#[test]
fn following_a_bead_the_window_names_takes_the_reader_to_it() {
    let (mut bdi, _tracker) = over_the_described_subtree("followed", ROWS, COLS, A_SILENCE);
    let opened = open_the_first_bead(&mut bdi);
    assert!(
        contains(&opened, THE_KEYS_THAT_FOLLOW),
        "the window offers no key that follows a bead, so there is nothing \
         for this test to press. The screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&opened),
        bdi.timeline()
    );

    bdi.send(NEXT_BEAD_NAMED);
    bdi.send(FOLLOW);
    bdi.settle(A_SILENCE, GIVING_UP);

    let followed = repaint(&mut bdi, ROWS);
    assert!(
        contains(&followed, A_BEAD_WINDOW),
        "the window closed on the press that asked for the bead. The screen \
         it drew: {:?}\nThe screen before the press: {:?}\n{}",
        String::from_utf8_lossy(&followed),
        String::from_utf8_lossy(&opened),
        bdi.timeline()
    );
    assert!(
        contains(&followed, ITS_PARENTS_WINDOW),
        "the window stayed up and is not on the bead that was followed. The \
         screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&followed),
        bdi.timeline()
    );
}

/// A collection landing after a follow leaves the window up, on the bead the
/// reader followed to.
///
/// This is the one that reaches the guard. `bead_still_shown` is consulted
/// only when a collection lands, so the test above can follow a bead and read
/// the screen without the guard ever being asked — measured, by making the
/// follow forget to say where it had gone: that turned exactly one test red
/// and neither of the pty tests was it. The window closing here is a reader
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
    assert!(
        contains(&after, ITS_PARENTS_WINDOW),
        "a collection took the window down after a follow, or moved it off \
         the bead that was followed to. The screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&after),
        bdi.timeline()
    );
}

/// And `Esc` comes back to the bead it was followed from, rather than out to
/// the forest. Without this the test above is met by a jump nobody can undo,
/// which is a reader stranded one press from where they were reading.
#[test]
fn the_way_back_returns_to_the_bead_the_reference_was_followed_from() {
    let (mut bdi, _tracker) = over_the_described_subtree("returned", ROWS, COLS, A_SILENCE);
    open_the_first_bead(&mut bdi);
    bdi.send(NEXT_BEAD_NAMED);
    bdi.send(FOLLOW);
    bdi.settle(A_SILENCE, GIVING_UP);

    bdi.send(BACK);
    bdi.settle(A_SILENCE, GIVING_UP);

    let back = repaint(&mut bdi, ROWS);
    assert!(
        contains(&back, THE_FIRST_BEADS_WINDOW),
        "the way back did not return to the bead the reference was followed \
         from. The screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&back),
        bdi.timeline()
    );
}

/// And once more takes the window down, because the bead the window was
/// opened on is where the way back leads out of the view. A reader who
/// pressed `Esc` twice would otherwise still be in a window.
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
        !contains(&out, A_BEAD_WINDOW),
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
    assert!(
        contains(&up, THE_FIRST_BEADS_WINDOW),
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
