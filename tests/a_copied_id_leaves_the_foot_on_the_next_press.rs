//! What the reader copied leaves the foot when they next press something.
//!
//! It was feedback on a keystroke rather than a fact about the screen, so the
//! next press takes it off whatever that press turns out to mean — and the
//! press that proves it has to be one the mapping answers with nothing, since
//! a key that did something would take the line off by redrawing anyway.
//!
//! The loop asks the screen on every press and the screen forgets what was
//! copied. Nothing has ever asked it through the binary: no pty test has
//! copied an id, so `Screen::pressed` could return a constant and the line
//! would stand under every key a reader typed after it, for the rest of the
//! run.

mod terminal;

use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::{contains, over_the_described_subtree};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// A gap this long between bytes means the frame is drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// `y`, which puts the selected bead's id on the terminal's clipboard and
/// says so at the foot.
const COPY_ID: &[u8] = b"y";

/// A key the mapping answers with nothing, so what it takes off the foot it
/// takes off for being a press and for no other reason.
const A_KEY_BOUND_TO_NOTHING: &[u8] = b"z";

/// The word the foot says it with, from `view::phrase`. The word alone rather
/// than the line, because the id it names is on a row of the forest too, and
/// the two are the same letters.
const COPIED: &[u8] = "copied".as_bytes();

#[test]
fn a_key_bound_to_nothing_still_takes_the_copied_id_off_the_foot() {
    let (mut bdi, _tracker) = over_the_described_subtree("copied", ROWS, COLS, A_SILENCE);

    bdi.send(COPY_ID);
    bdi.settle(A_SILENCE, GIVING_UP);
    let said = repaint(&mut bdi, ROWS + 1);
    assert!(
        contains(&said, COPIED),
        "the selection is on a bead and `y` put nothing on the foot, so there \
         was never a line for the next press to take off. The screen it drew: \
         {:?}\n{}",
        String::from_utf8_lossy(&said),
        bdi.timeline()
    );

    bdi.send(A_KEY_BOUND_TO_NOTHING);
    bdi.settle(A_SILENCE, GIVING_UP);
    let after = repaint(&mut bdi, ROWS);
    assert!(
        !contains(&after, COPIED),
        "the reader has pressed something since, and the foot still says what \
         they copied. The screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&after),
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
