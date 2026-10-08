//! `t` takes the tail away and gives its rows to the forest, and `t` again
//! brings it back with a fresh read of the pane.
//!
//! While the tail is hidden nothing is drawn from a read, so none is made.
//! The shimmed herdr counts every read it is asked for, and the band's
//! interval is short enough that a band still reading would ask dozens of
//! times over the window this test waits.

mod terminal;

use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::{ShimmedHerdr, A_PANE};
use terminal::{a_home_naming_one_project_settled, a_socket_of_its_own, rows_drawn, rows_of};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// `G`, which puts the selection on the last row: the one pane the shimmed
/// herdr reports.
const LAST_ROW: &[u8] = b"G";

const TOGGLE_THE_TAIL: &[u8] = b"t";

/// Part of the heading over the panes working outside every configured
/// project, from `view::phrase`, which says the pane's row is there to land
/// on.
const A_PANE_ROW_HAS_ARRIVED: &[u8] = "every configured project".as_bytes();

/// What is on the pane before the tail is hidden, and what is on it by the
/// time the tail comes back.
const A_BUILD_RUNNING: &str = "rebuilding .#larkspur\n";
const A_BUILD_FINISHED: &str = "rebuilt .#larkspur, generation 541\n";
const WHILE_IT_RUNS: &[u8] = "rebuilding".as_bytes();
const ONCE_IT_IS_DONE: &[u8] = "generation".as_bytes();

/// A band asking twenty times a second.
const ASKING_OFTEN: &str = "\n[tui]\ntail_refresh_millis = 50\n";

/// How long the reads are counted over while the tail is hidden: forty asks'
/// worth for a band that was still reading.
const A_WINDOW: Duration = Duration::from_secs(2);

/// A gap this long between bytes means the frame is drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// The screen drawn whole, by growing the terminal a row and putting it back:
/// a frame drawn as a difference from the one before holds only the cells
/// that moved.
fn the_whole_screen(bdi: &mut Driven) -> Vec<u8> {
    bdi.settle(A_SILENCE, GIVING_UP);
    let grown = bdi.resize(ROWS + 1, COLS);
    bdi.answer_to(grown, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);
    let back = bdi.resize(ROWS, COLS);
    bdi.answer_to(back, GIVING_UP)
}

#[test]
fn t_hides_the_tail_reads_nothing_while_it_is_hidden_and_reads_afresh_to_bring_it_back() {
    let home = a_home_naming_one_project_settled("toggle-tail", ASKING_OFTEN);
    let herdr = ShimmedHerdr::beside(&home);
    herdr.shows(A_BUILD_RUNNING);
    let mut environment = herdr.environment();
    environment.push(a_socket_of_its_own(&home));

    let mut bdi = Driven::bdi(ROWS, COLS, home, &environment);
    bdi.read_until(A_PANE_ROW_HAS_ARRIVED, GIVING_UP);
    bdi.send(LAST_ROW);
    bdi.read_until(WHILE_IT_RUNS, GIVING_UP);
    let shown = the_whole_screen(&mut bdi);
    let named_while_shown = rows_of(&shown, A_PANE.as_bytes()).len();

    bdi.send(TOGGLE_THE_TAIL);
    let hidden = the_whole_screen(&mut bdi);

    assert_eq!(
        rows_of(&hidden, WHILE_IT_RUNS),
        Vec::<u16>::new(),
        "the pane's output is off the screen: {:#?}",
        rows_drawn(&hidden)
    );
    assert_eq!(
        rows_of(&hidden, A_PANE.as_bytes()).len(),
        named_while_shown - 1,
        "the rule naming the pane went with the band, and the forest's row \
         for the pane stayed: {:#?}",
        rows_drawn(&hidden)
    );

    let before = herdr.reads();
    std::thread::sleep(A_WINDOW);
    assert_eq!(
        herdr.reads(),
        before,
        "the pane was read while the tail was hidden\n{}",
        bdi.timeline()
    );

    herdr.shows(A_BUILD_FINISHED);
    bdi.send(TOGGLE_THE_TAIL);
    bdi.read_until(ONCE_IT_IS_DONE, GIVING_UP);
}
