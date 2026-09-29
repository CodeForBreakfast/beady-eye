//! A bead named on the command line, waiting on a bead in a project the run
//! was not reading, draws that blocker as the bead it is.
//!
//! Run through the binary, because main decides which projects the run reads,
//! from the directory it was started in and the bead the command line names.

mod terminal;

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use terminal::shims::ShimmedTracker;

/// Ferry's one bead, waiting on a bead of dunwich's.
const FERRY: &str = r#"[
  {"id":"fer-2","title":"moor the barge","status":"open","priority":2,
   "issue_type":"task","dependencies":[{"depends_on_id":"dun-7","type":"blocks"}]}
]"#;

/// Dunwich's bead, which ferry's waits on.
const DUNWICH: &str = r#"[
  {"id":"dun-7","title":"lift the ground station","status":"in_progress",
   "priority":1,"issue_type":"epic"}
]"#;

/// A `HOME` whose config names dunwich, with `dunwich_states` in its entry,
/// and ferry, each in a directory of its own under it.
fn a_home_where_dunwich(named: &str, dunwich_states: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("bdi-{named}-{}", std::process::id()));
    for project in ["dunwich", "ferry"] {
        std::fs::create_dir_all(home.join(project)).expect("the directory is ours to make");
    }
    std::fs::write(
        home.join("config.toml"),
        format!(
            "[[projects]]\nname = \"dunwich\"\npath = \"{}\"\n{dunwich_states}\n\n\
             [[projects]]\nname = \"ferry\"\npath = \"{}\"\n",
            home.join("dunwich").display(),
            home.join("ferry").display()
        ),
    )
    .expect("the config is ours to write");
    home
}

/// The snapshot `bdi fer-2 --json` prints, started in ferry's directory.
fn ferry_named_from(home: &Path) -> Value {
    let tracker = ShimmedTracker::beside(home);
    tracker.holds_for("dunwich", DUNWICH);
    tracker.holds_for("ferry", FERRY);
    let out = Command::new(env!("CARGO_BIN_EXE_bdi"))
        .arg("--config")
        .arg(home.join("config.toml"))
        .args(["fer-2", "--json"])
        .envs(tracker.environment())
        .current_dir(home.join("ferry"))
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
