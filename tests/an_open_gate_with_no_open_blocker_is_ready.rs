//! An open gate with no open blocker is as ready as any other bead, in the
//! listing `bdi --beads` writes and on its row of the tree.
//!
//! A bare `bd ready` leaves gates out, as work nobody claims, so the shim
//! answers the two questions apart the way bd does: the bare one with no
//! gate, and the one asked for gates with the gates that have no open
//! blocker.

mod terminal;

use std::path::Path;
use std::process::Command;

use serde_json::Value;
use terminal::a_home_naming_one_project;
use terminal::shims::ShimmedTracker;

/// Three beads each blocked by a `gh:pr` gate, as bd 1.3.0 wrote them.
const GH_PR_GATES: &str = include_str!("fixtures/bd_1.3.0_gh_pr_gates.json");

/// What bd says is ready when asked for gates: all three, since none of
/// them waits on anything.
const READY_GATES: &str = r#"[
  {"id":"dun-w8h","title":"Gate: gh:pr","status":"open","issue_type":"gate"},
  {"id":"dun-5s6","title":"Gate: gh:pr","status":"open","issue_type":"gate"},
  {"id":"dun-auk","title":"Gate: gh:pr","status":"open","issue_type":"gate"}
]"#;

/// What a one-shot `bdi` run in `home` with `args` wrote, read as JSON.
fn one_shot(home: &Path, tracker: &ShimmedTracker, args: &[&str]) -> Value {
    let out = Command::new(env!("CARGO_BIN_EXE_bdi"))
        .args(args)
        .current_dir(home)
        .env("HOME", home)
        .env_remove("BEADS_DIR")
        .env_remove("BEADS_DOLT_PASSWORD")
        .env_remove("BDI_PROJECT")
        .envs(tracker.environment())
        .output()
        .expect("bdi runs");
    assert!(
        out.status.success(),
        "bdi exited {}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("bdi writes JSON")
}

fn ready<'a>(beads: impl IntoIterator<Item = &'a Value>, id: &str) -> &'a Value {
    &beads
        .into_iter()
        .find(|bead| bead["id"] == id)
        .unwrap_or_else(|| panic!("{id} is listed"))["ready"]
}

#[test]
fn an_open_gate_with_no_open_blocker_is_ready_in_the_listing_and_on_its_row() {
    let home = a_home_naming_one_project("ready-gate");
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(GH_PR_GATES);
    tracker.answers_with("ready --type gate --limit 0 --json", READY_GATES);

    let listing = one_shot(&home, &tracker, &["--beads"]);
    let forest = one_shot(&home, &tracker, &["--json", "--all"]);

    assert_eq!(tracker.unanswered(), Vec::<String>::new());
    let listed = listing["beads"].as_array().expect("beads");
    let rows: Vec<&Value> = forest["trees"]
        .as_array()
        .expect("trees")
        .iter()
        .flat_map(|tree| tree["nodes"].as_array().expect("nodes"))
        .collect();
    for (id, is_ready) in [("dun-w8h", true), ("dun-5s6", true), ("dun-4ga", false)] {
        assert_eq!(ready(listed, id), is_ready, "{id} in the listing");
        assert_eq!(ready(rows.iter().copied(), id), is_ready, "{id} on its row");
    }
}
