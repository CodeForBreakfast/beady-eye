//! The driver reads what `bdi` writes without being asked to.
//!
//! A terminal a person is sitting at empties itself, whatever the person is
//! doing. A harness that reads only inside a wait does not, so between one
//! wait and the next it puts `bdi` under backpressure no terminal applies:
//! the tty's output queue fills, the next write blocks, and everything `bdi`
//! would have done after drawing does not happen. The size of that queue is
//! the platform's, which is why this reads as a bug on the system it bites
//! on first and is not one. On macOS it is small enough to fill during the
//! first frame — measured 2026-09-02, where
//! `forest_before_the_first_collection` waited ten seconds for a collection
//! `bdi` had not reached the code to begin, and began it 120ms after the
//! master was first read.
//!
//! So this asks the one thing that separates a driver that drains from one
//! that reads on demand: it starts a `bdi` and then waits for nothing at all.
//! What `bdi` wrote in that time was read by nobody who asked for it.

mod terminal;

use std::time::{Duration, Instant};

use terminal::driver::Driven;
use terminal::{a_home_naming_one_project, contains, ENTER_ALTERNATE_SCREEN};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// Long enough that a driver which drains has heard the screen: the harness
/// puts it at tens of milliseconds.
const LONG_ENOUGH: Duration = Duration::from_secs(2);

#[test]
fn what_bdi_wrote_while_no_test_was_waiting_was_read_anyway() {
    let home = a_home_naming_one_project("nobody-waiting");
    let bdi = Driven::bdi(ROWS, COLS, home, &[]);

    let started = Instant::now();
    while started.elapsed() < LONG_ENOUGH {
        std::thread::sleep(Duration::from_millis(50));
    }

    let said = bdi.everything();
    assert!(
        contains(&said, ENTER_ALTERNATE_SCREEN),
        "the driver waited for nothing and so read nothing, leaving bdi to \
         write into a terminal that empties only when a test asks it to. It \
         heard {} bytes in {:?}: {:?}",
        said.len(),
        started.elapsed(),
        String::from_utf8_lossy(&said)
    );
}
