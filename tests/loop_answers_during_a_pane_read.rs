//! Whether the loop answers while herdr is being asked what is on a pane.
//!
//! The tail under the forest is one herdr call per selection, and the loop
//! reads keys on the same thread it draws on. So a herdr that is slow rather
//! than absent is a dead keyboard: every cursor move costs the whole read, and
//! `q` and `^C` queue behind it with the alternate screen still up. That is a
//! claim about which thread does what, and only a terminal `bdi` is typed at
//! can hold it.
//!
//! Nothing here waits for the screen to fall silent. `bdi` redraws a project's
//! freshness on its own while a collection runs, so silence is not a state a
//! live `bdi` reaches; every wait below is for a thing to be said.

mod terminal;

use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::ShimmedHerdr;
use terminal::{a_home_naming_one_project, contains};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// A gap this long between bytes means the frame is over. `bdi` writes a
/// frame in one burst.
const A_SILENCE: Duration = Duration::from_millis(300);

/// `G`, which puts the selection on the last row — the one pane the shimmed
/// herdr reports, and the shortest road to a row the tail must read.
const LAST_ROW: &[u8] = b"G";
/// `?`, which puts the key bindings up over the forest. The keystroke to test
/// with, because it redraws whatever the forest holds.
const SHOW_BINDINGS: &[u8] = b"?";
/// The first line of the bindings window, from `view::bindings`.
const BINDINGS_OPENED: &[u8] = "Key bindings".as_bytes();
/// What the band says under the rule while herdr is still answering, from
/// `view::phrase`. Written out rather than asked of `bdi`, so that a phrase
/// changed by hand is a test to change by hand.
const BEING_READ: &[u8] = "reading that pane".as_bytes();
/// Part of the heading over the panes working outside every configured
/// project, from `view::phrase` — the group the one shimmed pane sits in, and
/// so the group the key below aims at.
///
/// Waited for rather than the screen falling quiet, and waited for rather
/// than the first frame. Two things have to have happened before a key means
/// what these tests need it to mean: the terminal has to be in raw mode, so
/// the key is passed on rather than held by the line discipline until a
/// newline that never comes; and the collection has to have come back, so
/// there is a pane row to land on. `bdi` opens its screen before its first
/// collection returns, so the first frame satisfies only the first of those
/// and this line satisfies both.
const A_PANE_ROW_HAS_ARRIVED: &[u8] = "no configured project".as_bytes();
/// One word of what the shimmed herdr says is on the pane, from
/// `ShimmedHerdr`, and a word that is on the screen nowhere else.
///
/// One word and not the line: a repaint after a clear writes only the cells
/// that differ from blank, and a pane's own text is drawn in the terminal's
/// own colour — so its spaces are cells nothing has to write, and the line
/// reaches the wire a word at a time with a cursor move between each.
const WHAT_THE_PANE_SAID: &[u8] = ".#thinkpad,".as_bytes();

/// The band under the forest says what it is waiting for while it waits.
///
/// A `bdi` that waits on herdr inside the keystroke has nothing to say in the
/// meantime — it is not drawing — and once its own patience runs out it says
/// the pane could not be read, which is a different sentence. So this needs no
/// clock: the words below reach the screen only from a band drawn while the
/// read is still outstanding.
#[test]
fn the_band_says_it_is_reading_while_the_read_is_held() {
    let home = a_home_naming_one_project("pane-read-held");
    let herdr = ShimmedHerdr::beside(&home);
    herdr.hang();

    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &herdr.environment());
    bdi.read_until(A_PANE_ROW_HAS_ARRIVED, GIVING_UP);
    bdi.send(LAST_ROW);
    herdr.wait_until_holding(GIVING_UP);
    let repainted = bdi.resize(ROWS + 1, COLS);

    let screen = bdi.answer_to(repainted, GIVING_UP);
    assert!(
        contains(&screen, BEING_READ),
        "the band under the forest does not say it is reading while the read \
         is held, so `bdi` waits on herdr rather than drawing what it is \
         waiting for. The screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&screen),
        bdi.timeline()
    );
    assert!(
        herdr.holding(),
        "herdr let go of the read before the screen was read, so the band \
         above may be one drawn after the answer landed.\n{}",
        bdi.timeline()
    );
}

/// What the pane said reaches the band.
///
/// herdr's answer arrives as an event now, so between it and the screen there
/// is a road: `Event::Tailed`, the loop, `Shown::tailed`, and a redraw that
/// only happens where the view says the screen changed. Nothing but a running
/// `bdi` travels the whole of it, and a break anywhere along it leaves a band
/// that says it is reading and never says anything else.
#[test]
fn what_the_pane_said_reaches_the_band() {
    let home = a_home_naming_one_project("pane-read-answered");
    let herdr = ShimmedHerdr::beside(&home);

    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &herdr.environment());
    bdi.read_until(A_PANE_ROW_HAS_ARRIVED, GIVING_UP);
    bdi.send(LAST_ROW);
    // herdr answers a read in milliseconds and nothing else on this screen
    // animates, so a quiet this long is the answer having landed and been
    // drawn. A `bdi` that never went quiet would fail here rather than pass.
    bdi.settle(A_SILENCE, GIVING_UP);
    let repainted = bdi.resize(ROWS + 1, COLS);

    let screen = bdi.answer_to(repainted, GIVING_UP);
    assert!(
        contains(&screen, WHAT_THE_PANE_SAID),
        "what the pane said never reached the band. The screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&screen),
        bdi.timeline()
    );
}

/// A keystroke arriving while a pane read is outstanding is answered at once.
///
/// Asserted against a clock, which the collection's twin deliberately is not,
/// and it cannot be otherwise: a `bdi` that waits on herdr inside the
/// keystroke waits a bounded two seconds and then carries on, so it answers
/// every key eventually and no ordering tells the two apart. What the bound
/// below is, is the gap between them. A key measures 3 to 15 ms on a loop that
/// does not wait, and the whole of `PATIENCE` on one that does; a second is
/// sixty times the first and half the second.
#[test]
fn a_keystroke_is_answered_while_a_pane_read_is_outstanding() {
    let home = a_home_naming_one_project("pane-read-outstanding");
    let herdr = ShimmedHerdr::beside(&home);

    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &herdr.environment());
    bdi.read_until(terminal::ENTER_ALTERNATE_SCREEN, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);

    // Held only now, so the read this keystroke starts is the one outstanding
    // — a hold set before `bdi` started would be spent on the startup read.
    herdr.hang();
    bdi.send(LAST_ROW);
    herdr.wait_until_holding(GIVING_UP);
    let asked = bdi.send(SHOW_BINDINGS);

    let answer = bdi.answer_to(asked, ANSWERED_AT_ONCE);
    assert!(
        contains(&answer, BINDINGS_OPENED),
        "bdi did not answer a keystroke within {ANSWERED_AT_ONCE:?} while a \
         pane read was outstanding, so the loop waits on herdr rather than \
         drawing what it sends back. It wrote {} bytes after the key: {:?}\n{}",
        answer.len(),
        String::from_utf8_lossy(&answer),
        bdi.timeline()
    );
}

/// How long a keystroke gets before the loop is called blocked. See the test
/// that uses it for why this one is a number rather than an ordering.
const ANSWERED_AT_ONCE: Duration = Duration::from_secs(1);
