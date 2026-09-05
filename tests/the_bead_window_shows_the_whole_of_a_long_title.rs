//! The window shows the whole of a title too long to fit across it.
//!
//! The unit tests around `said` each drive a width of their own choosing.
//! What a reader gets is the width `offered` hands the window on a terminal
//! of a real size, against a title of the length this tracker's beads
//! actually run to, and only a run on a pty puts those two together.

mod terminal;

use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::{contains, over_the_described_subtree, THE_DESCRIBED_SUBTREE};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// A gap this long between bytes means the frame is drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// A collection is `bd` several times over, so it gets longer than a frame.
const LONG_ENOUGH_TO_COLLECT: Duration = Duration::from_secs(10);

/// `^R`, which asks every project for itself again.
const REFRESH: &[u8] = b"\x12";

/// From the tree's header down onto the first bead, and Enter, which shows
/// it. The same walk `the_bead_window_moves_and_closes_behind_its_bead`
/// makes, and for the same reason: the anomaly the tree's dangling edges
/// raise is drawn between the two rows and a motion passes over it.
const SHOW_THE_FIRST_BEAD: &[u8] = b"j\r";

/// A title from this tracker, long enough that it does not fit across the
/// window. Real rather than invented: the window floors at eighty columns,
/// so a title that wrapped only in a window narrowed to meet it would be a
/// test of the floor. This one is `bdi-2bb.48`'s, and it is the length the
/// beads here run to.
const A_TITLE_THAT_DOES_NOT_FIT: &str = "Thirty-three of thirty-four forest rows are the terminal's default, so the three an agent is on are found by reading rather than by looking";

/// A word from the start of that title, which the forest row and the window
/// both say. It is what tells a screen drawn from this capture from one
/// drawn from the capture it was made out of.
const THE_START_OF_THE_TITLE: &[u8] = "thirty-four".as_bytes();

/// A word from its far end, and specifically one past where the forest row
/// behind the window is cut.
///
/// Not merely a word the window's first row does not reach, which is what
/// this needle looks like and is why it is worth a paragraph. The forest row
/// gets more of a title than the window's first row does, at every width:
/// the forest has the whole terminal less its tree prefix, the window four
/// fifths of it less its border, the bead's glyph and the bead's id. The
/// window is centred, so a strip of that row shows to either side of it. A
/// needle from the middle of the title is therefore already on the screen
/// before the window opens, and the assertion this test exists for passes
/// with the window saying nothing of the title at all. Measured rather than
/// reasoned: the first run of this test failed its guard, with the forest
/// row reading `nobody…` at column 114.
///
/// The capture says this word nowhere else, so a screen saying it is a
/// screen saying the end of that title.
const THE_END_OF_THE_TITLE: &[u8] = "looking".as_bytes();

/// The title of the bead the window is opened on is readable in the window,
/// in full.
///
/// The forest is read first, and that reading is what makes the second one
/// mean something twice over: it says the tracker's new title reached the
/// screen at all, and it says the forest does not already carry the end of
/// it — the window covers the middle of the screen and leaves a forest row
/// showing at either side, so a window that said nothing would otherwise
/// pass.
#[test]
fn the_window_shows_the_whole_of_a_title_too_long_for_it() {
    let (mut bdi, tracker) = over_the_described_subtree("long-title", ROWS, COLS, A_SILENCE);
    tracker.holds(&with_a_title_that_does_not_fit());
    bdi.send(REFRESH);
    bdi.settle(A_SILENCE, LONG_ENOUGH_TO_COLLECT);

    let forest = repaint(&mut bdi, ROWS + 1);
    assert!(
        contains(&forest, THE_START_OF_THE_TITLE),
        "the tracker's new title never reached the screen. The screen it \
         drew: {:?}\n{}",
        String::from_utf8_lossy(&forest),
        bdi.timeline()
    );
    assert!(
        !contains(&forest, THE_END_OF_THE_TITLE),
        "the forest already says the end of that title, so a window that said \
         nothing of it would pass. The screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&forest),
        bdi.timeline()
    );

    bdi.send(SHOW_THE_FIRST_BEAD);
    bdi.settle(A_SILENCE, GIVING_UP);

    let window = repaint(&mut bdi, ROWS);
    assert!(
        contains(&window, THE_END_OF_THE_TITLE),
        "the window cut the title of the bead it was opened on. The screen it \
         drew: {:?}\n{}",
        String::from_utf8_lossy(&window),
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

/// The capture with that title on the bead the window is opened on.
fn with_a_title_that_does_not_fit() -> String {
    let mut rows: Vec<serde_json::Value> =
        serde_json::from_str(THE_DESCRIBED_SUBTREE).expect("a capture of bd list --json");
    let named = rows
        .iter_mut()
        .find(|row| row["id"] == "orb-0tp.6")
        .expect("the capture has to hold the bead the window is opened on");
    named["title"] = serde_json::Value::String(A_TITLE_THAT_DOES_NOT_FIT.to_string());
    serde_json::to_string(&rows).expect("rows serialise")
}
