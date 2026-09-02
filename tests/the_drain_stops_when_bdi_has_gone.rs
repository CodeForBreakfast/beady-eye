//! What the drain does once there is nothing left to drain.
//!
//! The driver reads the terminal on a thread of its own so `bdi` is never
//! blocked writing into one nobody is reading. That thread has to know when to
//! stop, and the deadline it is given — the `Driven` being dropped — comes too
//! late: a pty hangs up when its last slave handle closes, which is `bdi`'s own
//! stdio and so `bdi` gone, and `poll` reports a hangup whatever it was asked
//! to wait for. So a loop that polled and read on regardless would come
//! straight back every time round, with nothing to read and nothing to wait
//! for, and spin a core for as long as the test held the `Driven`.
//!
//! A core spent on nothing is not only waste. It is contention, and the tests
//! it contends with are the ones that measure `bdi` against a clock.

mod terminal;

use std::time::{Duration, Instant};

use terminal::a_home_whose_config_does_not_parse;
use terminal::driver::{Driven, GIVING_UP};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// What `bdi` says to a config it cannot read, so the wait below ends on a
/// `bdi` that has already gone rather than on a deadline.
const WHAT_IT_SAYS_ON_THE_WAY_OUT: &[u8] = b"TOML parse error";

/// Long enough for the drain to notice a hangup, which takes it one glance.
const LONG_ENOUGH_TO_NOTICE: Duration = Duration::from_secs(2);

#[test]
fn the_drain_gives_up_once_the_terminal_has_hung_up() {
    let home = a_home_whose_config_does_not_parse("hung-up");
    let mut bdi = Driven::bdi(ROWS, COLS, home, &[]);
    bdi.read_until(WHAT_IT_SAYS_ON_THE_WAY_OUT, GIVING_UP);

    let giving_up = Instant::now() + LONG_ENOUGH_TO_NOTICE;
    while bdi.still_reading() && Instant::now() < giving_up {
        std::thread::sleep(Duration::from_millis(50));
    }

    assert!(
        !bdi.still_reading(),
        "the drain was still reading a terminal bdi had hung up {LONG_ENOUGH_TO_NOTICE:?} \
         earlier, which is a poll that returns at once and a core spent on nothing"
    );
}
