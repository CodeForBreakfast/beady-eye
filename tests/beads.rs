//! `bdi --beads` writes each unfinished bead once. This runs the binary
//! because the wiring is main's: the listing itself is tested beside the
//! JSON contract, and a flag that never reached it would leave those green.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};

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

/// What `bdi --beads` wrote, read as JSON.
fn listing(config: &Path, args: &[&str]) -> Value {
    let out = Command::new(env!("CARGO_BIN_EXE_bdi"))
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

/// The tree's document already gives its ready beads, so the ready list's
/// is a second answer to one question, and bdi refuses to pick between them.
#[test]
fn a_bead_named_beside_beads_and_ready_is_refused() {
    let config = a_config_naming_two_projects("beads-tree-ready");

    let out = Command::new(env!("CARGO_BIN_EXE_bdi"))
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
