//! A tracker that files loose beads in bulk gives a forest of hundreds of
//! roots, most of them one bead. Measured against summit-works on 2026-08-31:
//! 562 roots, 530 of them holding exactly one unfinished bead. Nothing may be
//! dropped, so the only thing left is the order they come in.
//!
//! `tests/fixtures/bulk_loose_roots.json` is that shape in miniature — six
//! loose beads whose ids sort ahead of both efforts that hold work.

use beady_eye::config::Config;
use beady_eye::model::snapshot::Filter;
use chrono::{DateTime, Utc};
use serde_json::Value;

mod canned;

use canned::{Canned, PROBE_CALL, WORKING_ROOT};

const TRACKER: &str = include_str!("fixtures/bulk_loose_roots.json");

const PANES: &str = r#"{"result":{"agents":[
  {"pane_id":"w:p1","cwd":"/srv/work/orbital","agent_status":"working","title":"the dish"}
]}}"#;

const CONFIG: &str = r#"
[[projects]]
name = "orbital"
path = "/srv/work/orbital"
"#;

/// What the tracker answers discovery with: every bead of the fixture bar the
/// closed one, each carrying its own parent. Derived from the fixture so the
/// two answers cannot drift apart.
fn unfinished_rows() -> String {
    let beads: Vec<Value> = serde_json::from_str(TRACKER).expect("the fixture parses");
    let rows: Vec<Value> = beads
        .iter()
        .filter(|bead| bead["status"] != "closed")
        .map(|bead| {
            serde_json::json!({
                "id": bead["id"],
                "title": bead["title"],
                "status": bead["status"],
                "parent": bead["parent"],
            })
        })
        .collect();
    serde_json::to_string(&rows).expect("the rows serialise")
}

const ORBITAL_DIR: &str = "/srv/work/orbital";

/// A bd call as the runner spells it: the tracker named outright, and writes
/// refused.
fn spelled(subcommand: &str) -> String {
    format!("bd -C {ORBITAL_DIR} --readonly {subcommand}")
}

fn canned() -> Canned {
    Canned::default()
        .answering("herdr agent list", PANES)
        // Named by a path alone, so its tracker is reached by entering it.
        .answering(&format!("direnv exec {ORBITAL_DIR} env -0"), "")
        .answering(&spelled(PROBE_CALL), WORKING_ROOT)
        .answering(
            &spelled("list --status open,in_progress,blocked,deferred --limit 0 --json"),
            &unfinished_rows(),
        )
        // This tracker keeps no wisps. What a wisp root does to the order is
        // the same as any other root's: it has counts like the rest.
        .answering(&spelled("query ephemeral=true --limit 0 --json"), "[]")
        .answering(
            &spelled("query ephemeral=true --all --limit 0 --json"),
            "[]",
        )
        .answering(&spelled("ready --limit 0 --json"), "[]")
        .answering(&spelled("blocked --json"), "[]")
        .answering(&spelled("list --all --limit 0 --json"), TRACKER)
}

fn now() -> DateTime<Utc> {
    "2026-08-31T09:00:00Z".parse().expect("the instant parses")
}

fn roots(filter: Filter) -> Vec<String> {
    let cfg = Config::from_toml(CONFIG).expect("the config parses");
    let snapshot = beady_eye::app::run(&cfg, &canned(), filter, now());
    snapshot
        .trees
        .iter()
        .map(|tree| tree.root.clone())
        .collect()
}

/// Every root is still drawn — the bulk is not capped, collapsed or filtered
/// away — but the efforts that hold work come first and the loose beads make
/// up the tail.
#[test]
fn the_efforts_that_hold_work_come_before_the_beads_filed_in_bulk() {
    assert_eq!(
        roots(Filter::All),
        [
            "orb-d1", // one agent on it
            "orb-c3", // five beads left, nobody on it
            "orb-b1", "orb-b2", "orb-b3", "orb-b4", "orb-b5", "orb-b6",
        ]
    );
}

/// The reason agents sort first: `a` adds the rest of the forest below what
/// the reader was already looking at rather than shuffling it.
#[test]
fn showing_every_tree_leaves_the_staffed_ones_where_they_were() {
    let filtered = roots(Filter::LiveAgents);
    let all = roots(Filter::All);

    assert_eq!(filtered, ["orb-d1"]);
    assert_eq!(all[..filtered.len()], filtered[..]);
}
