//! What the driver does with a key typed before `bdi` has taken the terminal.
//!
//! Until `bdi` puts the terminal into raw mode, the pty's line discipline
//! holds every key until a newline that never comes, so a key typed then is
//! not late but gone: nothing answers it, and a test waiting for the answer
//! waits its whole deadline and then reports that nothing arrived. Which is
//! true and says nothing about why — bdi-7ao.52 lost a test to it, and wrote
//! the fact down where the next test file would never read it.
//!
//! So the driver refuses the key instead. Whether the terminal is in raw mode
//! is not on the wire, but what follows it is: `bdi` enters raw mode and then
//! opens the alternate screen, so a driver that has read the alternate screen
//! knows the line discipline is out of the way, and one that has not does not
//! type. This types before it has read anything, deliberately, and asks for
//! the refusal — at once, and naming what it should have waited for.

mod terminal;

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::{Duration, Instant};

use terminal::a_home_naming_one_project;
use terminal::driver::Driven;

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// `?`, which would put the key bindings up — had it arrived.
const SHOW_BINDINGS: &[u8] = b"?";

/// Long enough to tell a refusal from a wait: the driver's own deadline is a
/// minute, and a refusal is a comparison against what has been read.
const A_REFUSAL_TAKES: Duration = Duration::from_secs(10);

#[test]
fn a_key_typed_before_the_first_frame_is_refused_at_once() {
    let home = a_home_naming_one_project("typed-early");
    let mut bdi = Driven::bdi(ROWS, COLS, home, &[]);

    let typed_at = Instant::now();
    let typed = catch_unwind(AssertUnwindSafe(|| bdi.send(SHOW_BINDINGS)));
    let took = typed_at.elapsed();

    let refusal = typed
        .err()
        .expect("the driver typed at a bdi that had not opened its screen, and said nothing");
    let refusal = refusal
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| refusal.downcast_ref::<&str>().map(|said| said.to_string()))
        .expect("the refusal is a message");
    assert!(
        refusal.contains("ENTER_ALTERNATE_SCREEN"),
        "the refusal does not say what to wait for: {refusal}"
    );
    assert!(
        took < A_REFUSAL_TAKES,
        "the driver took {took:?} to refuse the key, which is a wait rather than a refusal"
    );
}
