//! The README's command line for listing the beads that depend on a bead
//! lists them, whichever project's tracker holds the edge, and after the bead
//! they wait on has closed.
//!
//! Run through the binary and the `jq` filter README gives, because a release
//! process cites that command line rather than anything inside `bdi`.

mod terminal;

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::{json, Value};
use terminal::shims::ShimmedTracker;

/// The filter README's "Several trackers" gives, which has to change with it.
/// README is not in the source this test is built from, so the test cannot
/// read it there.
const FILTER: &str = r#"
    [.trees[].nodes
     | foreach .[] as $node ([]; .[:$node.depth] + [$node])
     | select(.[-1] | .project == $project and .id == $id and .edge == "blocks")
     | .[-2] | {project, id, status}]
    | unique"#;

/// Dunwich's bead that the others wait on, closed; a bead of dunwich's own
/// waiting on it; and a child of it, which hangs under it without waiting.
const DUNWICH: &str = r#"[
  {"id":"dun-7","title":"lift the ground station","status":"closed",
   "priority":1,"issue_type":"epic","closed_at":"2026-08-28T09:00:00Z"},
  {"id":"dun-3","title":"survey the moor","status":"open","priority":2,
   "issue_type":"task","dependencies":[{"depends_on_id":"dun-7","type":"blocks"}]},
  {"id":"dun-7.1","title":"re-point the dish","status":"open","priority":2,
   "issue_type":"task","parent":"dun-7",
   "dependencies":[{"depends_on_id":"dun-7","type":"parent-child"}]}
]"#;

/// Arkham's beads waiting on dunwich's: one at the top of its tree, one under
/// a parent, and one that waits on the first and so has dunwich's bead two
/// levels beneath it.
const ARKHAM: &str = r#"[
  {"id":"ark-5","title":"copy the manuscript","status":"open","priority":2,
   "issue_type":"task","dependencies":[{"depends_on_id":"dun-7","type":"blocks"}]},
  {"id":"ark-1","title":"catalogue the library","status":"in_progress",
   "priority":1,"issue_type":"epic"},
  {"id":"ark-1.2","title":"index the folios","status":"in_progress","priority":2,
   "issue_type":"task","parent":"ark-1",
   "dependencies":[{"depends_on_id":"ark-1","type":"parent-child"},
                   {"depends_on_id":"dun-7","type":"blocks"}]},
  {"id":"ark-8","title":"bind the copy","status":"open","priority":3,
   "issue_type":"task","dependencies":[{"depends_on_id":"ark-5","type":"blocks"}]}
]"#;

/// A home holding both projects, each in a directory of its own, and the
/// config naming them.
fn a_home(named: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("bdi-{named}-{}", std::process::id()));
    for directory in ["arkham", "dunwich", ".config/beady-eye"] {
        std::fs::create_dir_all(home.join(directory)).expect("the directory is ours to make");
    }
    std::fs::write(
        home.join(".config/beady-eye/config.toml"),
        format!(
            "[[projects]]\nname = \"arkham\"\npath = \"{}\"\n\n\
             [[projects]]\nname = \"dunwich\"\npath = \"{}\"\n",
            home.join("arkham").display(),
            home.join("dunwich").display()
        ),
    )
    .expect("the config is ours to write");
    home
}

/// What the README's command line prints for `project`'s bead `id`.
fn dependents_of(project: &str, id: &str) -> Value {
    let home = a_home(&format!("dependents-of-{id}"));
    let trackers = ShimmedTracker::beside(&home);
    trackers.holds_for("arkham", ARKHAM);
    trackers.holds_for("dunwich", DUNWICH);

    let snapshot = Command::new(env!("CARGO_BIN_EXE_bdi"))
        .arg("--config")
        .arg(home.join(".config/beady-eye/config.toml"))
        .args(["--json", "--all", "--all-projects"])
        .envs(trackers.environment())
        .current_dir(&home)
        .output()
        .expect("bdi runs");
    assert!(
        snapshot.status.success(),
        "bdi exited {}: {}",
        snapshot.status,
        String::from_utf8_lossy(&snapshot.stderr)
    );

    let mut jq = Command::new("jq")
        .args(["--arg", "project", project, "--arg", "id", id, FILTER])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("jq runs");
    jq.stdin
        .take()
        .expect("jq's stdin is piped")
        .write_all(&snapshot.stdout)
        .expect("jq reads the snapshot");
    let listed = jq.wait_with_output().expect("jq finishes");
    assert!(
        listed.status.success(),
        "jq exited {}: {}",
        listed.status,
        String::from_utf8_lossy(&listed.stderr)
    );
    serde_json::from_slice(&listed.stdout).expect("jq prints JSON")
}

#[test]
fn every_bead_that_depends_on_a_closed_bead_is_listed_with_its_status_from_either_tracker() {
    assert_eq!(
        dependents_of("dunwich", "dun-7"),
        json!([
            {"project": "arkham", "id": "ark-1.2", "status": "in_progress"},
            {"project": "arkham", "id": "ark-5", "status": "open"},
            {"project": "dunwich", "id": "dun-3", "status": "open"}
        ])
    );
}

/// The control: a bead the others reach only through another bead has that
/// bead alone for a dependent.
#[test]
fn a_bead_reached_through_another_lists_only_the_one_above_it() {
    assert_eq!(
        dependents_of("arkham", "ark-5"),
        json!([{"project": "arkham", "id": "ark-8", "status": "open"}])
    );
}
