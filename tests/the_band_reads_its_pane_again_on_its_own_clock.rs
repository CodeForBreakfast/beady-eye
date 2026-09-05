//! The band reads its pane again on a clock of its own, and shows what
//! changed.
//!
//! Two of the loop's questions are only about that clock — when the next read
//! is due, and asking for it when it is — and nothing has ever reached either
//! through the binary. Every pty test before this one answered every pane read
//! from one file, so a `bdi` that never read again drew exactly the frame a
//! `bdi` that read four times a second drew, and an assertion on what is on
//! the screen could not tell them apart. What makes this a measurement is a
//! shim whose second answer differs from its first.
//!
//! The band's clock is never the only one running. A project that has been
//! read puts an age on its line, the age is a deadline of its own, and the
//! loop reads the pane on every wake whichever deadline woke it — so a `bdi`
//! that had forgotten the band's clock entirely still shows what changed,
//! once a second instead of four times. Every run has a project: a config
//! naming none is refused.
//!
//! So there are two tests here and the second is the one about the clock. The
//! first says the band reads again at all, which is what the shim's second
//! answer makes visible; the second counts how often it asks over a window,
//! which is the only question the ages cannot also answer.

mod terminal;

use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::ShimmedHerdr;
use terminal::{a_home_naming_one_project, a_home_naming_one_project_settled, a_socket_of_its_own};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// `G`, which puts the selection on the last row — the one pane the shimmed
/// herdr reports, and the shortest road to a row the band must read.
const LAST_ROW: &[u8] = b"G";

/// Part of the heading over the panes working outside every configured
/// project, from `view::phrase` — the group the one shimmed pane sits in, and
/// so the group the key above aims at.
const A_PANE_ROW_HAS_ARRIVED: &[u8] = "no configured project".as_bytes();

/// What is on that pane when the band first reads it, and what is on it when
/// the band next looks. One word of each is what the test waits for, because
/// the band draws a pane in the pane's own colours and so reaches the wire a
/// word at a time.
const A_BUILD_RUNNING: &str = "rebuilding .#larkspur\n";
const A_BUILD_FINISHED: &str = "rebuilt .#larkspur, generation 541\n";
const WHILE_IT_RUNS: &[u8] = "rebuilding".as_bytes();
const ONCE_IT_IS_DONE: &[u8] = "generation".as_bytes();

#[test]
fn the_band_shows_what_its_pane_says_next_without_being_asked() {
    let home = a_home_naming_one_project("reread");
    let herdr = ShimmedHerdr::beside(&home);
    herdr.shows(A_BUILD_RUNNING);
    let mut environment = herdr.environment();
    environment.push(a_socket_of_its_own(&home));

    let mut bdi = Driven::bdi(ROWS, COLS, home, &environment);
    bdi.read_until(A_PANE_ROW_HAS_ARRIVED, GIVING_UP);
    bdi.send(LAST_ROW);
    bdi.read_until(WHILE_IT_RUNS, GIVING_UP);

    herdr.shows(A_BUILD_FINISHED);
    bdi.read_until(ONCE_IT_IS_DONE, GIVING_UP);
}

/// A band asking on the projects' clock rather than its own still shows what
/// changed, only late — so the test above is met by a `bdi` that has
/// forgotten it has a clock at all. This is the one that is about the clock.
///
/// Counted rather than timed. How late one answer is is the gap to whatever
/// wakes the loop next, and every deadline the loop holds is in that gap: a
/// project read a moment ago ages in seconds, and the wake that ages it reads
/// the pane too. Measured over ten runs each, that one answer reached the
/// screen 250–252ms after the pane changed with the band's clock in place and
/// 606–855ms without it — two bands that do not overlap here and would meet
/// on a loaded machine, because the second of them is a second minus however
/// long `bdi` took to start.
///
/// How many times the band asked over a window of the test's own choosing has
/// no such phase in it. The interval below is short enough that the band asks
/// dozens of times, and the fastest anything else can wake the loop is the
/// once a second a project's line ages — so the two are an order of magnitude
/// apart rather than a threshold apart.
#[test]
fn the_band_asks_far_more_often_than_anything_else_wakes_the_loop() {
    let home = a_home_naming_one_project_settled("reread-often", ASKING_OFTEN);
    let herdr = ShimmedHerdr::beside(&home);
    herdr.shows(A_BUILD_RUNNING);
    let mut environment = herdr.environment();
    environment.push(a_socket_of_its_own(&home));

    let mut bdi = Driven::bdi(ROWS, COLS, home, &environment);
    bdi.read_until(A_PANE_ROW_HAS_ARRIVED, GIVING_UP);
    bdi.send(LAST_ROW);
    bdi.read_until(WHILE_IT_RUNS, GIVING_UP);

    let before = herdr.reads();
    std::thread::sleep(A_WINDOW);
    let asked = herdr.reads() - before;

    assert!(
        asked >= OFTEN_ENOUGH,
        "the band asked for its pane {asked} times in {A_WINDOW:?}, which is \
         about what a `bdi` reading it only when something else woke the loop \
         would manage. Its interval is the one written in {ASKING_OFTEN:?}.\n{}",
        bdi.timeline()
    );
}

/// A band asking twenty times a second, which is short enough that a window
/// a test can afford to wait holds dozens of asks.
const ASKING_OFTEN: &str = "\n[tui]\ntail_refresh_millis = 50\n";

/// How long the asks are counted over. Two seconds because it has to be
/// several times the one second a project's line takes to age, or the ceiling
/// this test rests on is a number too small to be sure of.
const A_WINDOW: Duration = Duration::from_secs(2);

/// The fewest asks that mean the band kept its own clock. Forty are due in
/// that window; measured on this machine, 34 arrive with the band's clock in
/// place and 2 without it.
///
/// The two figures are not the same kind, which is what makes a number
/// between them an assertion rather than a bet. **2 is a bound the code
/// imposes**: `phrase::holds_for` gives a project's age the age's own unit,
/// that unit is a second for the first minute of any run, and a band with no
/// clock of its own asks only when something wakes the loop — so no machine
/// anywhere gets past it. 34 is this machine's, and a slower one gets fewer.
/// The gap can only widen on a slower host, never close.
const OFTEN_ENOUGH: usize = 10;
