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
use terminal::shims::ShimmedTracker;

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

/// The title of dunwich's own tree, which is drawn where dunwich is read and
/// nothing is focused.
const DUNWICHS_OWN_TREE: &str = "survey the moor";

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
    let tracker = ShimmedTracker::beside(home);
    tracker.holds_for("dunwich", DUNWICH);
    tracker.holds_for(
        home.file_name()
            .and_then(|name| name.to_str())
            .expect("the home is named"),
        FERRY,
    );
    tracker
}

/// The snapshot `bdi fer-2 --json` prints, started in ferry's directory.
fn ferry_named_from(home: &Path) -> Value {
    let out = Command::new(env!("CARGO_BIN_EXE_bdi"))
        .arg("--config")
        .arg(the_config_in(home))
        .args(["fer-2", "--json"])
        .envs(the_trackers_in(home).environment())
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

const ROWS: u16 = 40;

/// Wide enough for the foot to keep its notice and its keys together.
const COLS: u16 = 160;

/// A gap this long between bytes means the frame is drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// `/`, ferry's bead and Enter, which leave the selection on that bead.
const FIND_FERRYS_BEAD: &[u8] = b"/fer-2\r";

/// Shift+F, which focuses the forest on the selected bead.
const FOCUS: &[u8] = b"F";

/// Naming a bead starts the forest as Shift+F on it would, and a project read
/// on demand for that bead is folded with the rest.
#[test]
fn a_project_read_on_demand_for_a_named_bead_is_folded_as_focusing_the_bead_folds_it() {
    let mut by_hand = launched("focused-by-hand", &[]);
    let unfocused = forest(&repaint(&mut by_hand, ROWS + 1));
    assert!(
        unfocused.iter().any(|row| row.contains(DUNWICHS_OWN_TREE)),
        "dunwich's own tree is not drawn, so nothing read it on demand and \
         there is nothing for focus to fold: {unfocused:#?}"
    );

    by_hand.send(FIND_FERRYS_BEAD);
    by_hand.settle(A_SILENCE, GIVING_UP);
    by_hand.send(FOCUS);
    by_hand.settle(A_SILENCE, GIVING_UP);
    let focused = forest(&repaint(&mut by_hand, ROWS));
    assert!(
        !focused.iter().any(|row| row.contains(DUNWICHS_OWN_TREE)),
        "focusing ferry's bead left dunwich's own tree drawn: {focused:#?}"
    );

    let mut named = launched("focused-by-name", &["fer-2"]);
    assert_eq!(forest(&repaint(&mut named, ROWS + 1)), focused);
}

/// A `bdi` started in ferry's directory, given `arguments`, with dunwich
/// stating its prefix, and its first collection drawn.
fn launched(named: &str, arguments: &[&str]) -> Driven {
    let home = a_home_where_dunwich(named, "prefix = \"dun\"");
    let mut environment = the_trackers_in(&home).environment();
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
