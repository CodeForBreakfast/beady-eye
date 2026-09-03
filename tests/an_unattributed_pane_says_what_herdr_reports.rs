//! A pane no bead claims is drawn with what herdr reports about it, on the
//! screen of a real `bdi`.
//!
//! herdr reports more than a pane's id: the `display_agent` the agent in it
//! stamped, its title, and a label for the state it is in. A reader looking at
//! an unattributed pane's row should not have to go to herdr's own sidebar to
//! learn what is sitting in the pane, so the row says it. This drives the
//! binary a reader runs against a session that says all of it, and looks for
//! the words on screen.

mod terminal;

use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::{ShimmedHerdr, ShimmedTracker};
use terminal::{a_home_naming_one_project, contains, ENTER_ALTERNATE_SCREEN};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// A gap this long between bytes means the collection is over and drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// How long a repaint gets before waiting for it is called stalling.
const LONG_ENOUGH_TO_ANSWER: Duration = Duration::from_secs(10);

/// What `bd list --all --json` said about this project's own tracker. The
/// tracker has to answer: a project whose root would not read takes its
/// loose panes onto its own line instead of drawing them a row each.
const THE_TRACKER: &str = include_str!("fixtures/bd_list.json");

/// The `display_agent` the pane stamped, and the label it gave the state it
/// is in. Each is one word, because a repaint reaches the wire a word at a
/// time with a cursor move where each space would be; and neither is a bead
/// id in the capture above, so the pane names no bead and stays loose.
const ITS_DISPLAY_AGENT: &str = "quartermaster";
const ITS_WORKING_LABEL: &str = "tallying-lamp-oil";

/// A herdr session holding one working pane in the configured project's own
/// directory, with every field herdr reports filled in.
fn a_session_with_one_talkative_pane(cwd: &str) -> String {
    format!(
        r#"{{"id":"cli:agent:list","result":{{"type":"agent_list","agents":[
            {{"pane_id":"wT:p1","cwd":"{cwd}","agent_status":"working",
             "display_agent":"{ITS_DISPLAY_AGENT}",
             "title":"the manifest",
             "state_labels":{{"idle":"asleep","working":"{ITS_WORKING_LABEL}"}}}}
        ]}}}}"#
    )
}

#[test]
fn an_unattributed_panes_row_says_what_herdr_reports_about_it() {
    let home = a_home_naming_one_project("reports");
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_TRACKER);
    let herdr = ShimmedHerdr::beside(&home);
    herdr.lists(&a_session_with_one_talkative_pane(
        &home.display().to_string(),
    ));
    let mut environment = tracker.environment();
    environment.extend(herdr.environment());

    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &environment);
    bdi.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);

    assert_eq!(
        tracker.unanswered(),
        Vec::<String>::new(),
        "bd was asked something the shim had no answer for, so the real bd \
         answered instead and the project was read as having no tracker"
    );

    let repainted = bdi.resize(ROWS + 1, COLS);
    let screen = bdi.answer_to(repainted, LONG_ENOUGH_TO_ANSWER);
    assert!(
        contains(&screen, b"wT:p1"),
        "the pane is not on the screen at all, so nothing below is about its \
         row. The screen bdi drew: {:?}\n{}",
        String::from_utf8_lossy(&screen),
        bdi.timeline()
    );
    for word in [ITS_DISPLAY_AGENT, ITS_WORKING_LABEL] {
        assert!(
            contains(&screen, word.as_bytes()),
            "{word:?} is not on the screen, so the pane's row does not say \
             what herdr reported. The screen bdi drew: {:?}\n{}",
            String::from_utf8_lossy(&screen),
            bdi.timeline()
        );
    }
}
