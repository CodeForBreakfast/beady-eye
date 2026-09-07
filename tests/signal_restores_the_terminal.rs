//! What a signal leaves behind on the terminal.
//!
//! `bdi` puts the terminal on the alternate screen, into raw mode and into
//! reporting the mouse, and puts all of it back from `Drop`. A signal with no
//! handler terminates the process outright, so none of that runs and the
//! shell underneath comes back unusable. These tests kill a real `bdi` on a
//! real pty and read what it wrote on the way out.

use std::time::Duration;

mod terminal;

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::ShimmedTracker;
use terminal::{
    a_home_naming_one_project, a_home_naming_one_project_settled, contains, ENTER_ALTERNATE_SCREEN,
};

/// The pty the tests draw on.
const ROWS: u16 = 40;
const COLS: u16 = 120;

/// The terminal is off the alternate screen from here. Its absence after a
/// signal is the bug.
const LEAVE_ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049l";
/// The mouse modes `EnableMouseCapture` turns on, each paired with the reset
/// that has to follow it. A terminal left reporting the mouse writes an
/// escape sequence into whatever runs next for every cell the pointer
/// crosses, which reaches the reader as input typing itself.
const MOUSE_OFF: [(&str, &[u8]); 5] = [
    ("normal tracking", b"\x1b[?1000l"),
    ("button tracking", b"\x1b[?1002l"),
    ("any-event tracking", b"\x1b[?1003l"),
    ("urxvt coordinates", b"\x1b[?1015l"),
    ("SGR coordinates", b"\x1b[?1006l"),
];

/// Long enough for a process that is going to die to have died.
const LONG_ENOUGH_TO_DIE: Duration = Duration::from_secs(10);

#[test]
fn a_terminate_puts_the_terminal_back() {
    assert_restored_after(libc::SIGTERM, "SIGTERM");
}

#[test]
fn an_interrupt_puts_the_terminal_back() {
    assert_restored_after(libc::SIGINT, "SIGINT");
}

/// A hangup reaches `bdi` when the terminal it is drawing on goes away, which
/// is how a multiplexer reclaims a pane.
#[test]
fn a_hangup_puts_the_terminal_back() {
    assert_restored_after(libc::SIGHUP, "SIGHUP");
}

/// Run `bdi` on a pty until it is drawing, signal it, and assert it handed
/// the terminal back on the way out.
fn assert_restored_after(signal: i32, named: &str) {
    let mut bdi = Driven::bdi(ROWS, COLS, a_home_naming_one_project(named), &[]);
    bdi.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP);

    let restoring = signal_and_read(&mut bdi, signal);

    let said = String::from_utf8_lossy(&restoring);
    assert!(
        contains(&restoring, LEAVE_ALTERNATE_SCREEN),
        "{named} left the terminal on the alternate screen; \
         it wrote {} bytes on the way out: {said:?}",
        restoring.len()
    );
    for (mode, off) in MOUSE_OFF {
        assert!(
            contains(&restoring, off),
            "{named} left the terminal reporting the mouse ({mode}): {said:?}"
        );
    }
}

/// A refresh interval no test here will ever reach, so the only collection is
/// the one `bdi` asks for at startup.
const NOTHING_ELSE_WILL_COLLECT: &str = "[tui]\nrefresh_seconds = 600\n";

/// A `^C` while the trackers are still being read puts the terminal back.
///
/// This is what `bdi` takes on for opening its screen before it has read
/// anything, and the one place that trade can be looked at rather than
/// argued about. During that wait there is now a screen to put back, so the
/// interrupt has to be answered rather than obeyed — and everything that
/// answers it is the same `Drop` the tests above drive, reached at the one
/// moment nothing had ever reached it before.
///
/// `bd` is held for the life of the test, so the collection this interrupts
/// cannot have come back: what is on the screen when the signal lands is the
/// forest with its marks still turning.
#[test]
fn an_interrupt_while_the_first_collection_runs_puts_the_terminal_back() {
    let home =
        a_home_naming_one_project_settled("signal-mid-collection", NOTHING_ELSE_WILL_COLLECT);
    let tracker = ShimmedTracker::beside(&home);
    tracker.hang();

    let mut bdi = Driven::bdi(ROWS, COLS, home, &tracker.environment());
    bdi.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP);
    tracker.wait_until_holding(GIVING_UP);

    let restoring = signal_and_read(&mut bdi, libc::SIGINT);

    let said = String::from_utf8_lossy(&restoring);
    assert!(
        contains(&restoring, LEAVE_ALTERNATE_SCREEN),
        "a SIGINT during the first collection left the terminal on the \
         alternate screen; it wrote {} bytes on the way out: {said:?}",
        restoring.len()
    );
    for (mode, off) in MOUSE_OFF {
        assert!(
            contains(&restoring, off),
            "a SIGINT during the first collection left the terminal \
             reporting the mouse ({mode}): {said:?}"
        );
    }
}

/// Send a signal and collect everything written after it.
fn signal_and_read(bdi: &mut Driven, signal: i32) -> Vec<u8> {
    let before = bdi.mark();
    assert_eq!(
        unsafe { libc::kill(bdi.pid(), signal) },
        0,
        "the signal is ours to send"
    );
    bdi.last_words(before, LONG_ENOUGH_TO_DIE)
}
