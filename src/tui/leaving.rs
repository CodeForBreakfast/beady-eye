//! A deadline on leaving, whatever the terminal is doing.
//!
//! Putting the screen back is a write, and so is any frame the loop is in the
//! middle of. A terminal nobody drains blocks a write for ever, so a `bdi`
//! told to quit on one never gets as far as exiting — and a terminating
//! signal cannot end it either, because `bdi` answers those rather than dying
//! of them.

use std::thread;
use std::time::Duration;

/// How long `bdi` has to leave once told to. Putting the screen back takes
/// milliseconds, so a run still here at this point is waiting on its terminal.
const LEAVING_WITHIN: Duration = Duration::from_secs(3);

/// Exit at the deadline if nothing else has, leaving the screen as it is.
///
/// What `bdi` queued for the terminal is discarded first. Darwin's close of a
/// terminal waits for its output queue to empty, and the last close happens
/// inside exit, where nothing can interrupt it: a process left there sits in
/// `ps`'s `E` state for as long as the terminal goes undrained.
pub(super) fn leave_by_the_deadline() {
    thread::spawn(|| {
        thread::sleep(LEAVING_WITHIN);
        unsafe {
            libc::tcflush(libc::STDOUT_FILENO, libc::TCOFLUSH);
            libc::_exit(libc::EXIT_FAILURE);
        }
    });
}

/// Wait for the terminal to take everything written to it, under the
/// deadline.
///
/// Waited for here so that the wait the last close would make in exit has
/// nothing left to wait for, and so that this wait, unlike that one, is one
/// the deadline can still end.
pub(super) fn hand_over_what_was_written() {
    unsafe { libc::tcdrain(libc::STDOUT_FILENO) };
}
