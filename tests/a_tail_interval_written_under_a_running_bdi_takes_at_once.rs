//! `[tui] tail_refresh_millis` written under a running `bdi` is the interval
//! the band asks on from then on.
//!
//! **Counted rather than timed, because a count has no phase in it.** How
//! late one answer is is the gap to whatever wakes the loop next, and every
//! deadline the loop holds is in that gap — so a duration read here is a
//! reading of this machine's load, and the bound a test could assert on it
//! would be a bet on the slowest host CI ever runs on. How many times the
//! band asked over a window of the test's own choosing is not: at 50ms it is
//! due 40 asks in two seconds, and the ceiling on the other side is imposed
//! by the code rather than by the hardware.
//!
//! `the_band_reads_its_pane_again_on_its_own_clock` is the neighbouring test
//! and this is deliberately not it. That one starts on an interval short
//! enough that the ages cannot be told from the band's own clock, and its
//! whole difficulty is separating the two. This one starts on an interval no
//! run outlives, where the band asks for its pane once and is not due again
//! inside the test — so the question is not *how often* but *whether the edit
//! reaches the band at all*. Measured against `204dea35`, the tree before
//! this change: **0 asks in the window after the reader wrote the short
//! interval.**
//!
//! So there are two counts and the first is the control. Without it the test
//! says only that the band asks often, which is what it would do if the
//! interval had been short all along and the edit had changed nothing.

mod terminal;

use std::path::{Path, PathBuf};
use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::ShimmedHerdr;
use terminal::{a_socket_of_its_own, contains};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// `G`, which puts the selection on the last row — the one pane the shimmed
/// herdr reports, and the shortest road to a row the band must read.
const LAST_ROW: &[u8] = b"G";

/// Part of the heading over the panes working outside every configured
/// project, which is the group the one shimmed pane sits in.
const A_PANE_ROW_HAS_ARRIVED: &[u8] = "no configured project".as_bytes();

/// What is on the pane, and the word of it the band draws that this waits
/// for. The band draws a pane in the pane's own colours, so its rows reach
/// the wire a word at a time.
const A_BUILD_RUNNING: &str = "rebuilding .#thinkpad\n";
const WHILE_IT_RUNS: &[u8] = "rebuilding".as_bytes();

/// An interval no run of this test outlives, so the only asks counted under
/// it are the ones something other than the band's clock produced.
const ASKING_ALMOST_NEVER: &str = "\n[tui]\ntail_refresh_millis = 3600000\n";

/// The interval the reader writes, twenty asks a second.
const ASKING_OFTEN: &str = "\n[tui]\ntail_refresh_millis = 50\n";

/// How long the asks are counted over, on both sides of the edit. Two seconds
/// because it has to be several times the one second a project's line takes
/// to age, or the control's ceiling is a number too small to be sure of.
const A_WINDOW: Duration = Duration::from_secs(2);

/// The most asks an interval nothing reaches can produce over that window.
/// Nothing wakes the band inside it — `reread` asks only where its own due
/// date has passed, and this one has not — so the ceiling is what a selection
/// moving or a collection landing produces, and this test does neither. Not
/// zero, because a bound that names the number it saw is a bound that fails
/// on the first machine that behaves slightly differently for a reason the
/// test is not about.
const A_FEW: usize = 5;

/// The fewest asks that mean the band took the reader's interval. Forty are
/// due in the window, and the same interval over the same window is what
/// `the_band_reads_its_pane_again_on_its_own_clock` sets this number from. A
/// slower host manages fewer of the forty and none of the five, so the gap
/// between the two bounds can only widen.
const OFTEN_ENOUGH: usize = 10;

/// Long enough for the check to fall due and for the frame that follows it.
/// `tui::reload::CHECKED_EVERY` is a constant rather than a config setting —
/// a setting would have to be read out of the file it governs — so nothing
/// here can shorten it, and this is a wait rather than a measurement.
const A_RELOAD: Duration = Duration::from_secs(6);

/// A `HOME` whose config names one project and carries `settings`.
fn a_home_settled(named: &str, settings: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("bdi-{named}-{}", std::process::id()));
    std::fs::create_dir_all(home.join(".config/beady-eye")).expect("the directory is ours to make");
    settling(&home, settings);
    home
}

/// The config rewritten to carry `settings`, as a reader editing the file
/// leaves it.
fn settling(home: &Path, settings: &str) {
    std::fs::write(
        home.join(".config/beady-eye/config.toml"),
        format!(
            "[[projects]]\nname = \"atlas\"\npath = \"{}\"\n{settings}",
            home.display()
        ),
    )
    .expect("the config is ours to write");
}

/// How many times the band asked for its pane over `A_WINDOW`.
fn asks_over_the_window(herdr: &ShimmedHerdr) -> usize {
    let before = herdr.reads();
    std::thread::sleep(A_WINDOW);
    herdr.reads() - before
}

#[test]
fn a_band_takes_the_interval_the_reader_writes_without_a_restart() {
    let home = a_home_settled("tail-interval", ASKING_ALMOST_NEVER);
    let herdr = ShimmedHerdr::beside(&home);
    herdr.shows(A_BUILD_RUNNING);
    let mut environment = herdr.environment();
    environment.push(a_socket_of_its_own(&home));

    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &environment);
    bdi.read_until(A_PANE_ROW_HAS_ARRIVED, GIVING_UP);
    bdi.send(LAST_ROW);
    bdi.read_until(WHILE_IT_RUNS, GIVING_UP);

    let before = asks_over_the_window(&herdr);
    assert!(
        before <= A_FEW,
        "the run opened on an interval nothing reaches, so the band asked \
         {before} times in {A_WINDOW:?} — every one of them something other \
         than its own clock waking the loop\n{}",
        bdi.timeline()
    );

    settling(&home, ASKING_OFTEN);
    std::thread::sleep(A_RELOAD);

    let after = asks_over_the_window(&herdr);
    assert!(
        after >= OFTEN_ENOUGH,
        "the band asked {after} times in {A_WINDOW:?} after the reader wrote \
         {ASKING_OFTEN:?}, which is about what it managed on the interval \
         they replaced\n{}",
        bdi.timeline()
    );
    assert!(
        contains(&bdi.everything(), WHILE_IT_RUNS),
        "and it is still the band, still showing the pane it was reading\n{}",
        bdi.timeline()
    );
}
