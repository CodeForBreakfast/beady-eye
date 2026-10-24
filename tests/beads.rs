//! `bdi --beads` writes each unfinished bead once. This runs the binary
//! because the wiring is main's: the listing itself is tested beside the
//! JSON contract, and a flag that never reached it would leave those green.

mod terminal;

use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use terminal::shims::ShimmedTracker;
use terminal::{a_home_naming_one_project_settled, bdi_on_pipes};

/// Both projects point at the directory holding the config, which holds no
/// tracker, so each one bdi reads fails and says so under its own name.
const TWO_PROJECTS: &str = "\
[[projects]]
name = \"arkham\"
path = \"{}\"

[[projects]]
name = \"kadath\"
path = \"{}\"
";

fn a_config_naming_two_projects(named: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("bdi-{named}-{}", std::process::id()));
    std::fs::create_dir_all(&home).expect("the directory is ours to make");
    let path = home.join("config.toml");
    std::fs::write(
        &path,
        TWO_PROJECTS.replace("{}", &home.display().to_string()),
    )
    .expect("the config is ours to write");
    path
}

/// The directory holding the config, which is the `HOME` a run reading it has.
fn the_directory_holding(config: &Path) -> &Path {
    config.parent().expect("the config sits in a directory")
}

/// What `bdi --beads` wrote, read as JSON.
fn listing(config: &Path, args: &[&str]) -> Value {
    let out = bdi_on_pipes(the_directory_holding(config))
        .arg("--config")
        .arg(config)
        .arg("--beads")
        .args(args)
        .output()
        .expect("bdi runs");
    assert!(
        out.status.success(),
        "bdi exited {}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("bdi --beads writes JSON")
}

fn projects_failed(listing: &Value) -> Vec<&str> {
    listing["failed_projects"]
        .as_array()
        .expect("failed_projects is an array")
        .iter()
        .map(|failed| failed["project"].as_str().expect("a project"))
        .collect()
}

/// Neither tracker answers, so the list is empty and says why, and a
/// `--project` narrows the run as it does any other.
#[test]
fn the_beads_listing_reads_the_projects_the_run_reads_and_names_the_ones_that_failed() {
    let config = a_config_naming_two_projects("beads");

    let whole = listing(&config, &[]);
    assert_eq!(whole["beads"], json!([]));
    assert_eq!(projects_failed(&whole), ["arkham", "kadath"]);

    let scoped = listing(&config, &["--project", "arkham"]);
    assert_eq!(projects_failed(&scoped), ["arkham"]);
}

/// A bead named beside `--beads` writes its dependency tree's document in
/// place of every bead, and a tracker that fails is named there as anywhere.
#[test]
fn a_bead_named_beside_beads_writes_its_dependency_tree() {
    let config = a_config_naming_two_projects("beads-tree");

    let tree = listing(&config, &["arkham:ark-1"]);

    assert!(tree.get("beads").is_none(), "{tree}");
    for list in ["ready", "ready_gates", "waiting"] {
        assert_eq!(tree[list], json!([]), "{list} in {tree}");
    }
    assert_eq!(projects_failed(&tree), ["arkham", "kadath"]);
}

/// A bead waiting on a closed step that the config's `[delivery]` labels an
/// outcome is delivered, which only that config can say.
#[test]
fn a_bead_named_beside_beads_is_delivered_as_the_config_says() {
    let home = a_home_naming_one_project_settled(
        "beads-tree-delivered",
        "\n[delivery]\noutcome_label = \"landed\"\n",
    );
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(
        r#"[
  {"id":"ark-1","title":"lay the rugs","status":"open","priority":2,"issue_type":"task",
   "dependencies":[{"depends_on_id":"ark-2","type":"blocks"}]},
  {"id":"ark-2","title":"the rugs are down","status":"closed","priority":2,"issue_type":"task",
   "labels":["landed"]}
]"#,
    );
    tracker.answers_with(
        "ready --brief --limit 0 --json",
        r#"[{"id":"ark-1","title":"lay the rugs","status":"open","issue_type":"task"}]"#,
    );

    let out = bdi_on_pipes(&home)
        .args(["--beads", "arkham:ark-1"])
        .current_dir(&home)
        .envs(tracker.environment())
        .output()
        .expect("bdi runs");
    assert!(
        out.status.success(),
        "bdi exited {}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let tree: Value = serde_json::from_slice(&out.stdout).expect("bdi --beads writes JSON");

    assert_eq!(tree["ready"], json!([]), "{tree}");
    assert_eq!(tree["delivered"][0]["id"], "ark-1", "{tree}");
}

/// The tree's document already gives its ready beads, so the ready list's
/// is a second answer to one question, and bdi refuses to pick between them.
#[test]
fn a_bead_named_beside_beads_and_ready_is_refused() {
    let config = a_config_naming_two_projects("beads-tree-ready");

    let out = bdi_on_pipes(the_directory_holding(&config))
        .arg("--config")
        .arg(&config)
        .args(["--beads", "--ready", "arkham:ark-1"])
        .output()
        .expect("bdi runs");

    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("leave out one or the other"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
