//! A `bdi` draws the seats in every herdr session on the box, not only the
//! one it happens to be in.
//!
//! A box runs several sessions at once, each its own server, and `herdr
//! agent list` answers for one of them. A pane id is minted per session, so
//! the id a bead names is a pane only together with the session holding it —
//! and the tail reads a pane through herdr by id, so a read that named no
//! session would read whichever session `bdi` sits in. A focus is the same
//! hazard with a worse ending: it is the only write `bdi` performs, so one
//! that named no session would move somebody else's terminal. The shim here
//! runs three sessions: one holding nothing, one holding the seat the bead
//! names, and one that will not answer at all.

mod terminal;

use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::{ShimmedHerdr, ShimmedTracker, A_SESSION};
use terminal::{a_home_naming_one_project, a_socket_of_its_own, contains, ENTER_ALTERNATE_SCREEN};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// A gap this long between bytes means the collection is over and drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// The session the seat is in, and the one that will not answer. Neither is
/// the session herdr runs where nothing names one.
const ANOTHER_SESSION: &str = "beacon";
const A_SILENT_SESSION: &str = "standing-agents";

/// The pane the bead names, by id alone — what a seat writes to `agent_pane`.
const THE_SEAT: &str = "w1:p1";

/// What the seat says it is doing, which is what its bead's row shows.
const WHAT_THE_SEAT_SAYS: &str = "re-pointing the dish";

/// One bead, claimed, naming the seat.
fn a_bead_naming_the_seat() -> String {
    format!(
        r#"[{{"id":"atl-1","title":"re-point the dish","status":"in_progress",
            "priority":2,"issue_type":"task","updated_at":"2026-09-03T08:00:00Z",
            "metadata":{{"agent_pane":"{THE_SEAT}"}}}}]"#
    )
}

/// The seat, as the session holding it lists it: in the project's own
/// directory, titled.
fn the_seat_in(home: &std::path::Path) -> String {
    format!(
        r#"{{"result":{{"agents":[{{"pane_id":"{THE_SEAT}","cwd":"{}","agent_status":"working","title":"{WHAT_THE_SEAT_SAYS}"}}]}}}}"#,
        home.display()
    )
}

const NO_PANES: &str = r#"{"result":{"agents":[]}}"#;

/// `G`, which puts the selection on the last row: the bead's, since the one
/// tree holds one bead and no pane is loose under it.
const LAST_ROW: &[u8] = b"G";

/// One word of what the shimmed pane shows, which arrives once the band has
/// read it.
const WHAT_THE_PANE_SHOWS: &[u8] = "generation".as_bytes();

/// `f`, which focuses the selected bead's pane.
const FOCUS: &[u8] = b"f";

/// How long to give the focus to reach the shim, and how often to look.
///
/// The log rather than the screen, because a focus that succeeds need draw
/// nothing: a test that waited for output would be waiting for a frame the
/// product is not obliged to paint, and would pass on a machine slow enough
/// for the write to land inside the silence it settled for.
const FOCUS_ARRIVES: Duration = Duration::from_secs(10);
const LOOKING_AGAIN: Duration = Duration::from_millis(20);

fn what_was_focused(herdr: &ShimmedHerdr) -> Vec<String> {
    let giving_up = std::time::Instant::now() + FOCUS_ARRIVES;
    loop {
        let focused = herdr.focused_panes();
        if !focused.is_empty() || std::time::Instant::now() > giving_up {
            return focused;
        }
        std::thread::sleep(LOOKING_AGAIN);
    }
}

#[test]
fn a_seat_in_another_session_is_drawn_on_its_bead_and_read_from_that_session() {
    let home = a_home_naming_one_project("another-session");
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(&a_bead_naming_the_seat());
    let herdr = ShimmedHerdr::beside(&home);
    herdr.runs(&[A_SESSION, ANOTHER_SESSION, A_SILENT_SESSION]);
    herdr.holds_in(A_SESSION, NO_PANES);
    herdr.holds_in(ANOTHER_SESSION, &the_seat_in(&home));
    let mut environment = tracker.environment();
    environment.extend(herdr.environment());
    environment.push(a_socket_of_its_own(&home));

    let mut bdi = Driven::bdi(ROWS, COLS, home, &environment);
    bdi.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);
    let drawn = bdi.everything();

    assert!(
        contains(&drawn, WHAT_THE_SEAT_SAYS.as_bytes()),
        "the seat in {ANOTHER_SESSION} was not drawn on its bead: {}",
        bdi.timeline()
    );
    assert!(
        contains(&drawn, A_SILENT_SESSION.as_bytes()),
        "the session that would not answer was not named: {}",
        bdi.timeline()
    );

    bdi.send(LAST_ROW);
    bdi.read_until(WHAT_THE_PANE_SHOWS, GIVING_UP);

    let read = herdr.read_panes();
    assert!(
        read.iter()
            .any(|pane| pane == &format!("{ANOTHER_SESSION} {THE_SEAT}")),
        "the band never read the seat from {ANOTHER_SESSION}; it read {read:?}: {}",
        bdi.timeline()
    );
    assert!(
        !read.iter().any(|pane| pane.starts_with(A_SESSION)),
        "the band read a pane from {A_SESSION}, which holds none; it read {read:?}"
    );
}

#[test]
fn a_focus_on_a_seat_in_another_session_is_sent_to_that_session() {
    let home = a_home_naming_one_project("another-session-focus");
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(&a_bead_naming_the_seat());
    let herdr = ShimmedHerdr::beside(&home);
    herdr.runs(&[A_SESSION, ANOTHER_SESSION, A_SILENT_SESSION]);
    herdr.holds_in(A_SESSION, NO_PANES);
    herdr.holds_in(ANOTHER_SESSION, &the_seat_in(&home));
    let mut environment = tracker.environment();
    environment.extend(herdr.environment());
    environment.push(a_socket_of_its_own(&home));

    let mut bdi = Driven::bdi(ROWS, COLS, home, &environment);
    bdi.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);

    bdi.send(LAST_ROW);
    bdi.read_until(WHAT_THE_PANE_SHOWS, GIVING_UP);
    bdi.send(FOCUS);

    let focused = what_was_focused(&herdr);
    assert!(
        focused
            .iter()
            .any(|pane| pane == &format!("{ANOTHER_SESSION} {THE_SEAT}")),
        "the focus never reached {ANOTHER_SESSION}; herdr was asked to focus {focused:?}: {}",
        bdi.timeline()
    );
    assert!(
        !focused.iter().any(|pane| pane.starts_with(A_SESSION)),
        "the focus went to {A_SESSION}, which holds no pane of that id; \
         herdr was asked to focus {focused:?}: {}",
        bdi.timeline()
    );
}
