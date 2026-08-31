//! Whether the loop answers the keyboard while a tracker is being read.
//!
//! `bdi` reads trackers on a worker thread and the loop draws what comes back,
//! so a keystroke during a collection is answered at once. That is a claim
//! about which thread does what, and nothing but a terminal `bdi` is typed at
//! can hold it: run the binary and read what it printed and a loop that
//! blocked for four seconds and one that never blocked leave the same output.
//!
//! Nothing here is asserted against a clock. The tracker is held outright
//! rather than made slow, so "the key was answered before the collection came
//! back" is an ordering that holds at any speed — it cannot go red on a loaded
//! machine, only on a `bdi` that stopped answering.

mod terminal;

use std::time::Duration;

use beady_eye::view::phrase::FRAME;
use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::ShimmedTracker;
use terminal::{a_home_naming_one_project_read_without_direnv, contains};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// A gap this long between bytes means the frame is over. `bdi` writes a
/// frame in one burst.
const A_SILENCE: Duration = Duration::from_millis(300);

/// The same, for a screen with a collection running on it. There is no long
/// quiet to wait for then: the project being read wears a mark that turns,
/// and `bdi` redraws it every `FRAME`. Half a frame still lands between two
/// bursts, which is all this wait was ever for — that what arrives next is an
/// answer to the keystroke rather than the tail of the frame before it.
fn between_frames() -> Duration {
    FRAME / 2
}

/// How long a keystroke gets before waiting for it is called stalling.
/// Nothing is asserted against it: it is what a stalled loop is reported in
/// rather than waited on for a minute. A key measures 3 to 15 ms healthy, and
/// the slowest bounded path on the loop is a wedged herdr's 2 s pane read, so
/// this is hundreds of times what a working `bdi` needs.
const LONG_ENOUGH_TO_ANSWER: Duration = Duration::from_secs(10);

/// How long a `^R` gets to reach `bd`. A collection asks its tracker within a
/// credential command of being asked for, so this too is only what a failure
/// is reported in rather than waited on.
const LONG_ENOUGH_TO_ASK: Duration = Duration::from_secs(10);

/// `^R`, which asks for a collection of every project.
const REFRESH: &[u8] = b"\x12";
/// `?`, which puts the key bindings up over the forest. The keystroke to test
/// with, because it redraws whatever the forest holds — a movement key on a
/// screen with one unreadable project has nowhere to move to, and would report
/// a live loop as a stalled one.
const SHOW_BINDINGS: &[u8] = b"?";
/// The first line of the bindings window, from `view::bindings`. Asserting on
/// it rather than on "some bytes arrived" is what keeps this test honest once
/// something on the screen animates: then bytes arrive on their own, and only
/// their content says the key was acted on.
const BINDINGS_OPENED: &[u8] = "Key bindings".as_bytes();

#[test]
fn a_keystroke_is_answered_while_a_collection_is_outstanding() {
    let home = a_home_naming_one_project_read_without_direnv("outstanding");
    let tracker = ShimmedTracker::beside(&home);
    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &tracker.environment());
    bdi.read_until(terminal::ENTER_ALTERNATE_SCREEN, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);

    tracker.hang();
    bdi.send(REFRESH);
    tracker.wait_until_holding(LONG_ENOUGH_TO_ASK);
    bdi.settle(between_frames(), GIVING_UP);
    let asked = bdi.send(SHOW_BINDINGS);

    let answer = bdi.answer_to(asked, LONG_ENOUGH_TO_ANSWER);
    assert!(
        contains(&answer, BINDINGS_OPENED),
        "bdi did not answer a keystroke while its tracker was being read, \
         so the loop waits on collections rather than drawing what they \
         send back. It wrote {} bytes after the key: {:?}\n{}",
        answer.len(),
        String::from_utf8_lossy(&answer),
        bdi.timeline()
    );
}
