//! A bead whose pane now holds another agent's session says so on the screen
//! of a real `bdi`, on the bead's own row and in the conflicts beneath the
//! forest, rather than drawing a claim that no seat ever registered.

mod terminal;

use std::time::{Duration, Instant};

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::{ShimmedHerdr, ShimmedTracker};
use terminal::{a_home_naming_one_project, a_socket_of_its_own, row_of, ENTER_ALTERNATE_SCREEN};

const ROWS: u16 = 40;
const COLS: u16 = 160;

/// Every tree rather than only the staffed ones, back to the first row, down
/// onto the tree and open it, as the badge reload test walks it.
const OPEN_THE_TREE: &[u8] = b"agjlgj";

/// A gap this long between bytes means the collection is over and drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// One claim, written by a seat whose agent session was `sess-dish`, on the
/// pane `wT:p1`.
const THE_TRACKER: &str = r#"[
  {"id":"dun-1","title":"lift the ground station","status":"open",
   "priority":1,"issue_type":"epic"},
  {"id":"dun-1.1","title":"repoint the dish","status":"in_progress","parent":"dun-1",
   "dependencies":[{"depends_on_id":"dun-1","type":"parent-child"}],
   "priority":2,"issue_type":"task",
   "metadata":{"agent_pane":"wT:p1","agent_session":"sess-dish"}}
]"#;

/// `wT:p1`, live, with another agent in it under `sess-mast`.
fn a_session_with_another_agent_in_the_pane(cwd: &str) -> String {
    format!(
        r#"{{"id":"cli:agent:list","result":{{"type":"agent_list","agents":[
            {{"pane_id":"wT:p1","cwd":"{cwd}","agent_status":"working","agent":"claude",
             "title":"the manifest",
             "agent_session":{{"agent":"claude","kind":"id","source":"herdr:claude",
                               "value":"sess-mast"}}}}
        ]}}}}"#
    )
}

/// A word of the pane's title, drawn among the panes no bead claims once the
/// first collection is on the screen.
const THE_PANES_TITLE: &[u8] = "manifest".as_bytes();

/// Whole frames until `needle` is drawn on one, handed back either way, so a
/// frame still without it is what the assertions fail against.
fn frames_until_drawn(bdi: &mut Driven, needle: &[u8]) -> Vec<u8> {
    let giving_up = Instant::now() + GIVING_UP;
    let mut rows = ROWS;
    loop {
        rows = if rows == ROWS { ROWS - 1 } else { ROWS };
        let whole = bdi.resize(rows, COLS);
        let frame = bdi.answer_to(whole, GIVING_UP);
        if row_of(&frame, needle).is_some() || Instant::now() >= giving_up {
            return frame;
        }
    }
}

/// A word of the bead's title, which names its row.
const THE_BEADS_ROW: &[u8] = "repoint".as_bytes();

/// What the bead's row says took its claim.
const ITS_REFUSAL: &[u8] = "its pane holds another agent session".as_bytes();

/// The session the bead wrote and the one the pane holds, which only the
/// conflict's row names.
const THE_BEADS_SESSION: &[u8] = "not sess-dish".as_bytes();
const THE_PANES_SESSION: &[u8] = "holds agent session sess-mast".as_bytes();

#[test]
fn a_bead_whose_pane_holds_another_agent_session_names_both_sessions() {
    let home = a_home_naming_one_project("another-agent-session");
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_TRACKER);
    let herdr = ShimmedHerdr::beside(&home);
    herdr.lists(&a_session_with_another_agent_in_the_pane(
        &home.display().to_string(),
    ));
    let mut environment = tracker.environment();
    environment.extend(herdr.environment());
    environment.push(a_socket_of_its_own(&home));

    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &environment);
    bdi.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP);
    bdi.read_until(THE_PANES_TITLE, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);
    bdi.send(OPEN_THE_TREE);
    let frame = frames_until_drawn(&mut bdi, THE_BEADS_ROW);

    let bead = row_of(&frame, THE_BEADS_ROW);
    assert!(
        bead.is_some(),
        "the bead is on the screen\n{}",
        bdi.timeline()
    );
    assert_eq!(
        row_of(&frame, ITS_REFUSAL),
        bead,
        "the bead's own row says why its claim has no agent\n{}",
        bdi.timeline()
    );
    let conflict = row_of(&frame, THE_PANES_SESSION);
    assert!(
        conflict.is_some() && conflict != bead,
        "a row of the conflicts names the session the pane holds\n{}",
        bdi.timeline()
    );
    assert_eq!(
        row_of(&frame, THE_BEADS_SESSION),
        conflict,
        "and the same row names the session the bead wrote\n{}",
        bdi.timeline()
    );
}
