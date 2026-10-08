//! A tree shorter than the screen leaves rows free beneath it, and the tail
//! takes them, up to half the screen.
//!
//! Two things have to agree for that to show. The band has to be drawn that
//! tall, and herdr has to be asked for enough of the pane to fill it, since
//! herdr answers a read with the last `--lines` lines of the pane and no
//! more. The shim cuts its answer the same way, so a band asking for six
//! lines draws six, however tall it is.

mod terminal;

use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::ShimmedHerdr;
use terminal::{a_home_naming_one_project, a_socket_of_its_own, row_of, rows_drawn};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// `G`, which puts the selection on the last row: the one pane the shimmed
/// herdr reports.
const LAST_ROW: &[u8] = b"G";

/// Part of the heading over the panes working outside every configured
/// project, from `view::phrase`, which says the collection has come back and
/// the pane's row is there to land on.
const A_PANE_ROW_HAS_ARRIVED: &[u8] = "every configured project".as_bytes();

/// A gap this long between bytes means the frame is drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// The pane's screen: thirty numbered rows, each one word, so each reaches
/// the wire whole.
fn the_panes_screen() -> String {
    (1..=30).map(|n| format!("row-{n:02}\n")).collect()
}

/// The band on a forty-row screen is twenty rows, half of it: the rule and
/// nineteen of the pane's lines. So the oldest line it keeps is the pane's
/// twelfth, and the newest sits on the row above the keys.
#[test]
fn a_short_tree_on_a_tall_screen_shows_half_a_screen_of_the_pane() {
    let home = a_home_naming_one_project("tall-tail");
    let herdr = ShimmedHerdr::beside(&home);
    herdr.shows(&the_panes_screen());
    let mut environment = herdr.environment();
    environment.push(a_socket_of_its_own(&home));

    let mut bdi = Driven::bdi(ROWS, COLS, home, &environment);
    bdi.read_until(A_PANE_ROW_HAS_ARRIVED, GIVING_UP);
    bdi.send(LAST_ROW);
    bdi.read_until(b"row-12", GIVING_UP);

    bdi.settle(A_SILENCE, GIVING_UP);
    let grown = bdi.resize(ROWS + 1, COLS);
    bdi.answer_to(grown, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);
    let back = bdi.resize(ROWS, COLS);
    let screen = bdi.answer_to(back, GIVING_UP);

    assert_eq!(
        row_of(&screen, b"row-12"),
        Some(ROWS / 2),
        "the pane's lines start under the rule that opens the lower half: {:#?}",
        rows_drawn(&screen)
    );
    assert_eq!(
        row_of(&screen, b"row-30"),
        Some(ROWS - 2),
        "the pane's newest line sits on the row above the keys: {:#?}",
        rows_drawn(&screen)
    );
    assert_eq!(
        row_of(&screen, b"row-11"),
        None,
        "nineteen lines, and no more: {:#?}",
        rows_drawn(&screen)
    );
}
