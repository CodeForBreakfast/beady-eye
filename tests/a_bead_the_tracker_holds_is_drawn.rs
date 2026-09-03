//! A bead the tracker holds reaches the screen of a real `bdi`.
//!
//! Every other pty test runs `bdi` against a `HOME` with no tracker, so what
//! it draws is a project whose tracker could not be read, and nothing under
//! it. A test about a row — a freshness state, a fold, a badge, a colour — has
//! had to assert one layer down, at the paint, and say so. This is the test
//! that a tracker can be *answered for* rather than only held up: `bd` on
//! PATH serves a capture, and the binary a reader actually runs draws it.

mod terminal;

use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::ShimmedTracker;
use terminal::{a_home_naming_one_project, contains, ENTER_ALTERNATE_SCREEN};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// A gap this long between bytes means the collection is over and drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// How long a keystroke gets before waiting for it is called stalling.
const LONG_ENOUGH_TO_ANSWER: Duration = Duration::from_secs(10);

/// What `bd list --all --json` said about this project's own tracker.
const THE_TRACKER: &str = include_str!("fixtures/bd_list.json");

/// The one root in that capture: the open epic every other row hangs under,
/// whose own parent the capture does not hold.
const THE_ROOT: &[u8] = "bdi-2bb".as_bytes();
/// One word of its title, *The TUI: one scrollable forest over the snapshot*,
/// that nothing else on the screen says. One word rather than the title,
/// because a repaint reaches the wire a word at a time with a cursor move
/// where each space would be, so the title as written is never in it.
const A_WORD_OF_ITS_TITLE: &[u8] = "scrollable".as_bytes();

/// `a`, which shows every tree rather than only those with a live agent. No
/// pane sits in the temp `HOME`, so without it the one tree here sits behind
/// its project's *no live agent* line and draws no row of its own.
const SHOW_EVERY_TREE: &[u8] = b"a";

#[test]
fn a_bead_the_tracker_holds_is_drawn_as_a_row() {
    let home = a_home_naming_one_project("held");
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_TRACKER);
    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &tracker.environment());
    bdi.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);

    assert_eq!(
        tracker.unanswered(),
        Vec::<String>::new(),
        "bd was asked something the shim had no answer for, so the real bd \
         answered instead and the project was read as having no tracker"
    );
    assert_eq!(
        tracker.direnv_runs(),
        Vec::<String>::new(),
        "a project configured by its path alone was entered with direnv, \
         which a machine with bd and nothing else does not have"
    );

    bdi.send(SHOW_EVERY_TREE);
    bdi.settle(A_SILENCE, LONG_ENOUGH_TO_ANSWER);
    let repainted = bdi.resize(ROWS + 1, COLS);

    let screen = bdi.answer_to(repainted, LONG_ENOUGH_TO_ANSWER);
    assert!(
        contains(&screen, THE_ROOT) && contains(&screen, A_WORD_OF_ITS_TITLE),
        "the root the tracker holds is not on the screen. The screen bdi \
         drew: {:?}\n{}",
        String::from_utf8_lossy(&screen),
        bdi.timeline()
    );
}
