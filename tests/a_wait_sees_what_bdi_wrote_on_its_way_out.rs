//! What a wait sees when `bdi` has written its last words and gone.
//!
//! A wait takes its view from what the driver has heard, and what the driver
//! has heard is what its drain thread has appended. So a wait that reads only
//! that view reads it as fresh as the scheduler last happened to run the
//! drain — and on an exited `bdi` that is the difference between reporting the
//! error it exited with and reporting that it never said anything. The second
//! is a false red, and it is a false red in the one place a reader goes to
//! find out why `bdi` died.
//!
//! So a wait empties the pty itself before it looks. On a `bdi` that has been
//! reaped that is exact rather than a guess: a dead process writes nothing
//! more, so a pty a read finds empty has given up the whole of what it wrote.
//!
//! The drain is held back here because on an unloaded machine it never falls
//! behind, and a test that cannot reach the state it is about is green for a
//! reason of its own. Half a second is far longer than the glance a wait takes
//! between looks, so the pty holds `bdi`'s last words across one for certain
//! rather than when the machine happens to be busy.

mod terminal;

use std::time::Duration;

use terminal::a_home_whose_config_does_not_parse;
use terminal::driver::{Driven, GIVING_UP};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// Long enough that the pty holds what `bdi` wrote across a wait's glance,
/// which is [`A_GLANCE`](terminal::driver) at 50ms.
const FURTHER_BEHIND_THAN_A_GLANCE: Duration = Duration::from_millis(500);

/// What `bdi` says to a config it cannot read, from `cli`. Written out rather
/// than asked of `bdi`, so a message changed by hand is a test to change by
/// hand — and it is the whole point here that these words are on the wire and
/// nothing has picked them up.
const WHAT_IT_SAYS_ON_THE_WAY_OUT: &[u8] = b"TOML parse error";

#[test]
fn a_wait_reads_what_bdi_wrote_before_it_exited_however_far_behind_the_drain_is() {
    let home = a_home_whose_config_does_not_parse("last-words");
    let mut bdi =
        Driven::bdi_with_the_drain_held_back(ROWS, COLS, home, &[], FURTHER_BEHIND_THAN_A_GLANCE);

    bdi.read_until(WHAT_IT_SAYS_ON_THE_WAY_OUT, GIVING_UP);
}
