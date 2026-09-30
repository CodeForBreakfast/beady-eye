//! A bead named on the command line, waiting on a bead in a project the run
//! was not reading, draws that blocker as the bead it is.
//!
//! Run through the binary, because main decides which projects the run reads,
//! from the directory it was started in and the bead the command line names.

mod terminal;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde_json::Value;
use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::{ShimmedHerdr, ShimmedTracker};

/// Ferry's one bead, waiting on a bead of dunwich's.
const FERRY: &str = r#"[
  {"id":"fer-2","title":"moor the barge","status":"open","priority":2,
   "issue_type":"task","dependencies":[{"depends_on_id":"dun-7","type":"blocks"}]}
]"#;

/// Dunwich's beads: the one ferry's waits on, and a tree of its own that
/// nothing ferry holds needs.
const DUNWICH: &str = r#"[
  {"id":"dun-7","title":"lift the ground station","status":"in_progress",
   "priority":1,"issue_type":"epic"},
  {"id":"dun-3","title":"survey the moor","status":"in_progress",
   "priority":1,"issue_type":"task"}
]"#;

/// Ferry's bead waiting on dunwich's, and a tree of ferry's own beside it.
const FERRY_WITH_ANOTHER_TREE: &str = r#"[
  {"id":"fer-2","title":"moor the barge","status":"open","priority":2,
   "issue_type":"task","dependencies":[{"depends_on_id":"dun-7","type":"blocks"}]},
  {"id":"fer-4","title":"caulk the hull","status":"open","priority":2,
   "issue_type":"task"}
]"#;

/// The title of ferry's other tree, which is drawn where nothing is focused.
const FERRYS_OTHER_TREE: &str = "caulk the hull";

/// A `HOME` that is ferry's directory. Its config names ferry there and
/// dunwich in a directory under it, with `dunwich_states` in dunwich's entry.
fn a_home_where_dunwich(named: &str, dunwich_states: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("bdi-{named}-{}", std::process::id()));
    for directory in ["dunwich", ".config/beady-eye"] {
        std::fs::create_dir_all(home.join(directory)).expect("the directory is ours to make");
    }
    std::fs::write(
        the_config_in(&home),
        format!(
            "[[projects]]\nname = \"dunwich\"\npath = \"{}\"\n{dunwich_states}\n\n\
             [[projects]]\nname = \"ferry\"\npath = \"{}\"\n",
            home.join("dunwich").display(),
            home.display()
        ),
    )
    .expect("the config is ours to write");
    home
}

fn the_config_in(home: &Path) -> PathBuf {
    home.join(".config/beady-eye/config.toml")
}

/// The trackers of a home made above, each holding its project's beads.
fn the_trackers_in(home: &Path) -> ShimmedTracker {
    the_trackers_in_holding(home, FERRY, DUNWICH)
}

/// The trackers of a home made above, holding `ferry` and `dunwich`.
fn the_trackers_in_holding(home: &Path, ferry: &str, dunwich: &str) -> ShimmedTracker {
    let tracker = ShimmedTracker::beside(home);
    tracker.holds_for("dunwich", dunwich);
    tracker.holds_for(
        home.file_name()
            .and_then(|name| name.to_str())
            .expect("the home is named"),
        ferry,
    );
    tracker
}

/// The snapshot `bdi fer-2 --json` prints, started in ferry's directory.
fn ferry_named_from(home: &Path) -> Value {
    json_from(home, &the_trackers_in(home).environment(), &["fer-2"])
}

/// The snapshot `bdi --json` prints given `arguments`, started in ferry's
/// directory with `environment`.
fn json_from(home: &Path, environment: &[(String, String)], arguments: &[&str]) -> Value {
    let out = Command::new(env!("CARGO_BIN_EXE_bdi"))
        .arg("--config")
        .arg(the_config_in(home))
        .args(arguments)
        .arg("--json")
        .envs(environment.iter().cloned())
        .current_dir(home)
        .output()
        .expect("bdi runs");
    assert!(
        out.status.success(),
        "bdi exited {}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("bdi prints a snapshot")
}

/// The node for `id` in the tree rooted at ferry's bead.
fn in_ferrys_tree<'a>(snapshot: &'a Value, id: &str) -> Option<&'a Value> {
    snapshot["trees"]
        .as_array()
        .expect("trees is an array")
        .iter()
        .find(|tree| tree["root"] == "fer-2")
        .expect("ferry's bead draws a tree")["nodes"]
        .as_array()
        .expect("nodes is an array")
        .iter()
        .find(|node| node["id"] == id)
}

#[test]
fn a_blocker_whose_prefix_a_configured_project_states_is_drawn_from_that_project() {
    let snapshot = ferry_named_from(&a_home_where_dunwich("read-on-demand", "prefix = \"dun\""));

    let blocker = in_ferrys_tree(&snapshot, "dun-7").expect("the blocker is drawn");
    assert_eq!(
        (&blocker["project"], &blocker["title"], &blocker["status"]),
        (
            &Value::from("dunwich"),
            &Value::from("lift the ground station"),
            &Value::from("in_progress")
        )
    );
}

/// The control: nothing says the blocker is dunwich's, so dunwich is not
/// read and the blocker is reported where it would have hung.
#[test]
fn a_blocker_whose_prefix_no_configured_project_states_is_reported_rather_than_read() {
    let snapshot = ferry_named_from(&a_home_where_dunwich("not-read-on-demand", ""));

    assert!(in_ferrys_tree(&snapshot, "dun-7").is_none());
    assert_eq!(
        in_ferrys_tree(&snapshot, "fer-2").expect("ferry's bead is drawn")["orphaned_dependencies"],
        serde_json::json!([{"id": "dun-7", "reason": "not-read", "projects": ["dunwich"]}])
    );
}

/// Every tree's root, shown or hidden.
fn roots_of(snapshot: &Value) -> Vec<&str> {
    ["trees", "hidden_trees"]
        .iter()
        .flat_map(|list| snapshot[list].as_array().expect("a list of trees"))
        .map(|tree| tree["root"].as_str().expect("a tree names its root"))
        .collect()
}

/// A project read for a blocker is read for that bead, and its own trees are
/// no more drawn than they would be had nothing needed it.
#[test]
fn a_project_read_for_a_blocker_draws_none_of_its_own_trees() {
    let home = a_home_where_dunwich("no-roots-on-demand", "prefix = \"dun\"");

    let snapshot = json_from(&home, &the_trackers_in(&home).environment(), &[]);

    assert_eq!(roots_of(&snapshot), ["fer-2"]);
    assert_eq!(
        in_ferrys_tree(&snapshot, "dun-7").expect("the blocker is drawn")["project"],
        "dunwich"
    );
}

/// A seat in dunwich shows through a bead a drawn tree reaches, and one on a
/// bead nothing reaches is no more reported than it would be had dunwich not
/// been read. A seat in ferry on no bead is the control: it says the panes
/// were listed and a loose one is reported.
#[test]
fn a_pane_in_a_project_read_for_a_blocker_shows_only_through_a_bead_the_reach_drew() {
    let home = a_home_where_dunwich("panes-on-demand", "prefix = \"dun\"");
    let dunwich = home.join("dunwich").display().to_string();
    let herdr = ShimmedHerdr::beside(&home);
    herdr.lists(&format!(
        r#"{{"result":{{"agents":[
            {{"pane_id":"wT:p1","cwd":"{dunwich}","agent_status":"working",
             "display_agent":"dun-7"}},
            {{"pane_id":"wT:p2","cwd":"{dunwich}","agent_status":"working",
             "display_agent":"dun-3"}},
            {{"pane_id":"wT:p3","cwd":"{}","agent_status":"working"}}
        ]}}}}"#,
        home.display()
    ));
    let mut environment = the_trackers_in(&home).environment();
    environment.extend(herdr.environment());

    let snapshot = json_from(&home, &environment, &[]);

    assert_eq!(
        in_ferrys_tree(&snapshot, "dun-7").expect("the blocker is drawn")["agent"]["pane"]["id"],
        "wT:p1"
    );
    let unattributed: Vec<&Value> = snapshot["unattributed"]
        .as_array()
        .expect("unattributed is an array")
        .iter()
        .map(|loose| &loose["pane"]["id"])
        .collect();
    assert_eq!(unattributed, ["wT:p3"]);
}

/// Dunwich's beads, and one more of its own that carries the id of ferry's
/// other tree, as uncoordinated prefixes allow.
const DUNWICH_SHARING_AN_ID_WITH_FERRY: &str = r#"[
  {"id":"dun-7","title":"lift the ground station","status":"in_progress",
   "priority":1,"issue_type":"epic"},
  {"id":"fer-4","title":"dredge the channel","status":"open","priority":2,
   "issue_type":"task"}
]"#;

/// A seat in dunwich naming a bead nothing reaches is on dunwich's work, so
/// ferry's bead of the same id is no bead of the seat's.
#[test]
fn a_pane_in_a_project_read_for_a_blocker_is_not_joined_across_to_a_drawn_bead_of_its_id() {
    let home = a_home_where_dunwich("colliding-on-demand", "prefix = \"dun\"");
    let herdr = ShimmedHerdr::beside(&home);
    herdr.lists(&format!(
        r#"{{"result":{{"agents":[
            {{"pane_id":"wT:p2","cwd":"{}","agent_status":"working",
             "display_agent":"fer-4"}}
        ]}}}}"#,
        home.join("dunwich").display()
    ));
    let mut environment = the_trackers_in_holding(
        &home,
        FERRY_WITH_ANOTHER_TREE,
        DUNWICH_SHARING_AN_ID_WITH_FERRY,
    )
    .environment();
    environment.extend(herdr.environment());

    let snapshot = json_from(&home, &environment, &["--all"]);

    assert_eq!(roots_of(&snapshot), ["fer-2", "fer-4"]);
    assert_eq!(
        in_ferrys_tree(&snapshot, "dun-7").expect("the blocker is drawn")["project"],
        "dunwich"
    );
    assert_eq!(snapshot["conflicts"], serde_json::json!([]));
    assert_eq!(snapshot["unattributed"], serde_json::json!([]));
}

/// The dunwich bead ferry's waits on, under a parent of its own and holding a
/// child that waits on kadath's, and a tree of dunwich's own.
const DUNWICH_TO_KADATH: &str = r#"[
  {"id":"dun-1","title":"chart the fens","status":"open","priority":2,
   "issue_type":"epic"},
  {"id":"dun-7","title":"lift the ground station","status":"in_progress",
   "priority":1,"issue_type":"epic",
   "dependencies":[{"depends_on_id":"dun-1","type":"parent-child"}]},
  {"id":"dun-8","title":"raise the mast","status":"open","priority":2,
   "issue_type":"task",
   "dependencies":[{"depends_on_id":"dun-7","type":"parent-child"},
                   {"depends_on_id":"kad-1","type":"blocks"}]},
  {"id":"dun-3","title":"survey the moor","status":"in_progress",
   "priority":1,"issue_type":"task"}
]"#;

/// Kadath's bead that dunwich's waits on, its child, a bead it blocks, and a
/// tree of its own.
const KADATH: &str = r#"[
  {"id":"kad-1","title":"survey the plateau","status":"open","priority":2,
   "issue_type":"epic"},
  {"id":"kad-2","title":"stake the plateau","status":"open","priority":2,
   "issue_type":"task",
   "dependencies":[{"depends_on_id":"kad-1","type":"parent-child"}]},
  {"id":"kad-5","title":"build on the plateau","status":"open","priority":2,
   "issue_type":"task",
   "dependencies":[{"depends_on_id":"kad-1","type":"blocks"}]},
  {"id":"kad-9","title":"climb the peaks","status":"open","priority":2,
   "issue_type":"task"}
]"#;

/// Reach runs from a bead to its children and its blockers, whichever
/// project holds them, and never up to a parent or out to what a blocker
/// blocks.
#[test]
fn a_chain_of_blockers_across_projects_is_followed_through_blockers_and_children_only() {
    let home = a_home_where_dunwich("reach-across-a-chain", "prefix = \"dun\"");
    std::fs::create_dir_all(home.join("kadath")).expect("the directory is ours to make");
    let mut config = std::fs::read_to_string(the_config_in(&home)).expect("the config is ours");
    config.push_str(&format!(
        "\n[[projects]]\nname = \"kadath\"\npath = \"{}\"\nprefix = \"kad\"\n",
        home.join("kadath").display()
    ));
    std::fs::write(the_config_in(&home), config).expect("the config is ours to write");
    let trackers = the_trackers_in_holding(&home, FERRY, DUNWICH_TO_KADATH);
    trackers.holds_for("kadath", KADATH);

    let snapshot = json_from(&home, &trackers.environment(), &[]);

    assert_eq!(roots_of(&snapshot), ["fer-2"]);
    let mut drawn: Vec<&str> = snapshot["trees"][0]["nodes"]
        .as_array()
        .expect("nodes is an array")
        .iter()
        .map(|node| node["id"].as_str().expect("a node names its bead"))
        .collect();
    drawn.sort_unstable();
    drawn.dedup();
    assert_eq!(drawn, ["dun-7", "dun-8", "fer-2", "kad-1", "kad-2"]);
}

const ROWS: u16 = 40;

/// Wide enough for the foot to keep its notice and its keys together.
const COLS: u16 = 160;

/// A gap this long between bytes means the frame is drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// `/`, ferry's bead and Enter, which leave the selection on that bead.
const FIND_FERRYS_BEAD: &[u8] = b"/fer-2\r";

/// Shift+F, which focuses the forest on the selected bead.
const FOCUS: &[u8] = b"F";

/// Naming a bead starts the forest as Shift+F on it would.
#[test]
fn a_named_bead_waiting_on_a_project_read_on_demand_is_focused_as_shift_f_focuses_it() {
    let mut by_hand = launched("focused-by-hand", &[]);
    let unfocused = forest(&repaint(&mut by_hand, ROWS + 1));
    assert!(
        unfocused.iter().any(|row| row.contains(FERRYS_OTHER_TREE)),
        "ferry's other tree is not drawn, so there is nothing for focus to \
         fold: {unfocused:#?}"
    );

    by_hand.send(FIND_FERRYS_BEAD);
    by_hand.settle(A_SILENCE, GIVING_UP);
    by_hand.send(FOCUS);
    by_hand.settle(A_SILENCE, GIVING_UP);
    let focused = forest(&repaint(&mut by_hand, ROWS));
    assert!(
        !focused.iter().any(|row| row.contains(FERRYS_OTHER_TREE)),
        "focusing ferry's bead left ferry's other tree drawn: {focused:#?}"
    );

    let mut named = launched("focused-by-name", &["fer-2"]);
    assert_eq!(forest(&repaint(&mut named, ROWS + 1)), focused);
}

/// A `bdi` started in ferry's directory, given `arguments`, with dunwich
/// stating its prefix, and its first collection drawn.
fn launched(named: &str, arguments: &[&str]) -> Driven {
    let home = a_home_where_dunwich(named, "prefix = \"dun\"");
    let mut environment =
        the_trackers_in_holding(&home, FERRY_WITH_ANOTHER_TREE, DUNWICH).environment();
    environment.push(terminal::a_socket_of_its_own(&home));
    let mut bdi = Driven::bdi_with_arguments(ROWS, COLS, home, arguments, &environment);
    bdi.read_until(terminal::ENTER_ALTERNATE_SCREEN, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);
    bdi
}

/// The screen as it stands, rather than as it differs from the frame before:
/// a resize is answered by drawing every cell again.
#[track_caller]
fn repaint(bdi: &mut Driven, rows: u16) -> Vec<u8> {
    let repainted = bdi.resize(rows, COLS);
    bdi.answer_to(repainted, GIVING_UP)
}

/// The forest's rows, down to the line saying what the run reads. A project
/// line is cut at the age of its read, which differs between two runs.
fn forest(screen: &[u8]) -> Vec<String> {
    terminal::rows_drawn(screen)
        .into_iter()
        .take_while(|row| !row.trim_start().starts_with("reading "))
        .map(|row| match row.split_once("  ✓") {
            Some((project, _)) => project.to_string(),
            None => row.trim_end().to_string(),
        })
        .collect()
}
