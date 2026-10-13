//! A `bdi` told to quit leaves even when its terminal has stopped taking
//! what it writes.
//!
//! A terminal nobody drains blocks the next write, and putting the screen back
//! is a write. So a `bdi` told to quit on such a terminal never got as far as
//! exiting, and a terminating signal could not end it either, because `bdi`
//! answers those rather than dying of them.

use std::time::Duration;

mod terminal;

use terminal::driver::{Driven, GIVING_UP};
use terminal::{a_home_naming_one_project, ENTER_ALTERNATE_SCREEN};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// Long enough for the loop to have taken the resize and be blocked drawing
/// the frame it asks for, so the signal lands on a loop that cannot read it.
/// Too short, and the signal is read first and this tests the same leaving as
/// the quit below.
const BLOCKED_DRAWING: Duration = Duration::from_secs(1);

#[test]
fn a_terminate_ends_a_bdi_blocked_drawing() {
    let mut bdi = Driven::bdi(
        ROWS,
        COLS,
        a_home_naming_one_project("blocked-drawing"),
        &[],
    );
    bdi.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP);
    let _stopped = bdi.stop_taking_output();
    bdi.resize(ROWS, COLS + 1);
    std::thread::sleep(BLOCKED_DRAWING);

    let before = bdi.mark();
    assert_eq!(
        unsafe { libc::kill(bdi.pid(), libc::SIGTERM) },
        0,
        "the signal is ours to send"
    );
    bdi.last_words(before, GIVING_UP);

    assert!(
        bdi.exited().is_some(),
        "a SIGTERM left bdi running, blocked writing to a terminal that \
         takes nothing. {}",
        bdi.timeline()
    );
}

#[test]
fn a_quit_ends_a_bdi_whose_terminal_takes_nothing() {
    let mut bdi = Driven::bdi(
        ROWS,
        COLS,
        a_home_naming_one_project("quit-undrained"),
        &[],
    );
    bdi.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP);
    let _stopped = bdi.stop_taking_output();

    let before = bdi.send(b"q");
    bdi.last_words(before, GIVING_UP);

    assert!(
        bdi.exited().is_some(),
        "q left bdi running, blocked putting the screen back on a terminal \
         that takes nothing. {}",
        bdi.timeline()
    );
}
