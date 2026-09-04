//! The key bar survives a screen short enough that the tail band is one row.
//!
//! `regions` gives the tail `(LINES + 1).min((rows - 1) / 2)` rows, which is
//! one at a screen height of 4 and of 5 and more everywhere else. A band one
//! row high holds the rule naming the pane and nothing else, because the row
//! a line beneath the rule would go on is the key bar. `draw_tail` holds that
//! with `if room > 0` in each of the two arms that draw a second row.
//!
//! Every other pty test here drives `bdi` at forty rows, where the band is
//! seven and `room` is six, so both guards are met and both are true. What
//! nothing reaches is either of them being false. This is that rule reached
//! the way a reader reaches it: through the binary, on a terminal short
//! enough for the band to run out of room.
//!
//! Both guarded arms are driven, because they are two guards and a test that
//! met one would leave the other reached by nothing. The band draws
//! `Tail::Silent` on a machine with no agent provider, and `Tail::Reading`
//! while herdr is still answering what is on a pane.
//!
//! The forest is left to the tests that are about it. What is asserted here
//! is the one row beneath it.

mod terminal;

use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::{shims_first_with_nothing_called, ShimmedHerdr, ShimmedTracker};
use terminal::{a_home_naming_one_project, a_socket_of_its_own, contains, ENTER_ALTERNATE_SCREEN};

/// The height every other pty test here runs at, and where the band is seven
/// rows. The run that meets the guard starts here and is shortened, because
/// what it has to do first needs the room.
const ROWS: u16 = 40;
const COLS: u16 = 120;

/// A gap this long between bytes means the collection is over and drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// What `bd list --all --json` said about this project's own tracker.
const THE_TRACKER: &str = include_str!("fixtures/bd_list.json");

/// The first hint of the key row, *a all   / find   ? keys   q quit*, and a
/// word nothing else on this screen says — the fixture's titles hold no
/// `all`. One word rather than the row, because the key row is drawn in the
/// terminal's own colour, so its spaces are cells nothing has to write and it
/// reaches the wire a word at a time.
///
/// The first rather than any of them, because a band line is drawn from the
/// left and takes the row from the left: measured with the `Reading` guard
/// disarmed, the line reached column 19 of 35 and `? keys   q quit` was still
/// on the row beyond it. A test naming `quit` would have watched the key bar
/// lose half its hints and called it intact.
const A_WORD_OF_THE_KEY_ROW: &[u8] = "all".as_bytes();

/// The line the band would draw beneath its rule on a machine with no agent
/// provider — one word of *no agent provider · bdi is reading beads alone*,
/// by the same rule as above.
const THE_SILENT_LINE: &[u8] = "provider".as_bytes();

/// The line the band would draw beneath its rule while herdr is still
/// answering, from `view::phrase`. Whole rather than one word, because a band
/// line is drawn in a colour of its own and so writes its spaces.
const THE_READING_LINE: &[u8] = "reading that pane".as_bytes();

/// `G`, which puts the selection on the last row — the one pane the shimmed
/// herdr reports, and the shortest road to a row the tail must read.
const LAST_ROW: &[u8] = b"G";

/// Part of the heading over the panes working outside every configured
/// project, from `view::phrase` — the group the one shimmed pane sits in, and
/// so the group the key above aims at. The collection has to have come back
/// before that key has a row to land on, and `bdi` opens its screen before
/// its first collection returns.
const A_PANE_ROW_HAS_ARRIVED: &[u8] = "no configured project".as_bytes();

/// The shorter of the two heights where the band is one row: a forest of two
/// rows, the band, and the keys.
#[test]
fn a_four_row_screen_draws_the_keys_and_not_the_bands_silent_line() {
    the_silent_band_writes_nothing_over_the_key_bar(4);
}

/// The taller of the two. One row more of forest, and the same band: the
/// division is integer, so the rule is met at both heights rather than
/// skirted at one of them.
#[test]
fn a_five_row_screen_draws_the_keys_and_not_the_bands_silent_line() {
    the_silent_band_writes_nothing_over_the_key_bar(5);
}

/// Run `bdi` on a terminal this many rows high, against a machine with no
/// agent provider, and read the whole of what it draws: the keys are there,
/// and the line the band would have written over them is not.
///
/// Absence is asserted over everything `bdi` said rather than over one frame,
/// because the screen is this size from the moment it opens — there is no
/// frame in the run where the band had room, so the line anywhere in the
/// stream is the line drawn where the keys go.
fn the_silent_band_writes_nothing_over_the_key_bar(rows: u16) {
    let home = a_home_naming_one_project(&format!("short-{rows}"));
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_TRACKER);
    let mut environment = tracker.environment();
    environment.retain(|(key, _)| key != "PATH");
    environment.push(shims_first_with_nothing_called("herdr", &home));
    environment.push(a_socket_of_its_own(&home));

    let mut bdi = Driven::bdi(rows, COLS, home.clone(), &environment);
    bdi.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);
    let drawn = bdi.everything();

    assert!(
        contains(&drawn, A_WORD_OF_THE_KEY_ROW),
        "a {rows}-row screen drew no key bar: {}",
        bdi.timeline()
    );
    assert!(
        !contains(&drawn, THE_SILENT_LINE),
        "a band one row high wrote its line over the key bar of a \
         {rows}-row screen: {}",
        bdi.timeline()
    );
}

/// The other guarded arm: the band while herdr is still being asked what is
/// on the selected pane.
///
/// Started tall and shortened, because the row the key aims at is at the foot
/// of a forest a four-row screen shows two rows of, and the heading that says
/// the collection has come back is drawn only where there is room for it. The
/// read is still held when the screen shrinks, so the band the resize draws
/// is a band saying it is reading.
#[test]
fn a_short_screen_draws_the_keys_and_not_the_bands_reading_line() {
    let home = a_home_naming_one_project("short-reading");
    let herdr = ShimmedHerdr::beside(&home);
    herdr.hang();
    let mut environment = herdr.environment();
    environment.push(a_socket_of_its_own(&home));

    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &environment);
    bdi.read_until(A_PANE_ROW_HAS_ARRIVED, GIVING_UP);
    bdi.send(LAST_ROW);
    herdr.wait_until_holding(GIVING_UP);
    let shortened = bdi.resize(4, COLS);

    let screen = bdi.answer_to(shortened, GIVING_UP);
    assert!(
        herdr.holding(),
        "herdr let go of the read before the screen was read, so the band \
         drawn may be one that had an answer to draw.\n{}",
        bdi.timeline()
    );
    assert!(
        contains(&screen, A_WORD_OF_THE_KEY_ROW),
        "a four-row screen drew no key bar. The screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&screen),
        bdi.timeline()
    );
    assert!(
        !contains(&screen, THE_READING_LINE),
        "a band one row high wrote what it is waiting for over the key bar. \
         The screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&screen),
        bdi.timeline()
    );
}
