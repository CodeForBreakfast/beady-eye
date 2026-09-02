//! `bdi` draws its first frame on a terminal that answers no question.
//!
//! There are two ways to clear the screen before the first frame.
//! `Clear(ClearType::All)` writes `\e[2J` and is done. ratatui's
//! `Terminal::clear` first asks the terminal where its cursor is — `\e[6n` —
//! and waits for the answer, which on a terminal that gives none arrives
//! never: crossterm gives up after two seconds and the error comes back out
//! of `Screen::showing`, so `bdi` has left the screen before drawing on it.
//! Measured on a 120x40 pty for `bdi-2bb.31`: the first frame accounted for
//! 138 of 4800 cells, against 928 with no clear at all. The tidier API did not
//! merely fail to clear; it stopped `bdi` starting.
//!
//! The pty here answers nothing by construction — the test's end of it writes
//! only what a test types, and this test types nothing. Every pty test in the
//! suite runs on that same silence, so the swap turns four of them red, each
//! for a reason that is not this one: *no bd call was held*, *left 4584 of
//! 4800 cells as it found them*. This is the one that goes red saying why.

mod terminal;

use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::{a_home_naming_one_project, contains, ENTER_ALTERNATE_SCREEN};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// A gap this long between bytes means the first frame is drawn, and the
/// collection after it done.
const A_SILENCE: Duration = Duration::from_millis(300);

/// The one project the config names. Its line is drawn from the configured
/// name rather than from anything a tracker said, so it is the first frame.
const THE_PROJECT: &[u8] = "atlas".as_bytes();

/// The one thing crossterm writes to a terminal and waits for the answer to:
/// *where is the cursor*. `Terminal::clear` opens with it, and so does
/// anything that reaches `cursor::position`.
const WHERE_IS_THE_CURSOR: &[u8] = b"\x1b[6n";

#[test]
fn the_first_frame_arrives_without_the_terminal_answering_anything() {
    let home = a_home_naming_one_project("answers-nothing");
    let mut bdi = Driven::bdi(ROWS, COLS, home, &[]);
    bdi.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);

    let said = bdi.everything();
    assert!(
        !contains(&said, WHERE_IS_THE_CURSOR),
        "bdi asked the terminal where its cursor is and waited for the \
         answer. No terminal here gives one, so bdi stalled and then gave \
         up before its first frame — which is what ratatui's \
         `Terminal::clear` does on every terminal that stays silent. Clear \
         with `execute!(Clear(ClearType::All))`, which asks nothing. What \
         bdi wrote: {:?}\n{}",
        String::from_utf8_lossy(&said),
        bdi.timeline()
    );
    assert!(
        contains(&said, THE_PROJECT),
        "bdi opened its screen and never drew the project on it. What bdi \
         wrote: {:?}\n{}",
        String::from_utf8_lossy(&said),
        bdi.timeline()
    );
}
