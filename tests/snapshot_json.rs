//! `bdi --json` emits the contract in `docs/design.md`. These drive the whole
//! binary's model through the one public entry point a consumer sees.

use std::collections::HashMap;
use std::path::Path;

use beady_eye::collect::run::{Env, FailureKind, RunFailure, Runner};
use beady_eye::config::Config;
use beady_eye::model::snapshot::Filter;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

/// One project's tracker: an epic with a pane on it, a claimed task with
/// none, a claim nothing has touched in weeks, a task bd calls ready, and a
/// closed task a pane is still sitting on.
const TREE: &str = r#"[
  {"id":"orb-7","title":"lift the ground station","status":"in_progress","parent_id":"",
   "priority":1,"issue_type":"epic","updated_at":"2026-08-29T09:00:00Z",
   "started_at":"2026-08-20T09:00:00Z","metadata":{"agent_pane":"w:p1"}},
  {"id":"orb-7.1","title":"re-point the dish","status":"in_progress","parent_id":"orb-7",
   "priority":2,"issue_type":"task","edge_from_parent":"parent-child",
   "updated_at":"2026-08-29T10:00:00Z","started_at":"2026-08-29T10:00:00Z",
   "metadata":{"blocked_on":"human"}},
  {"id":"orb-7.3","title":"lay the feeder cable","status":"in_progress","parent_id":"orb-7",
   "priority":1,"issue_type":"task","edge_from_parent":"parent-child",
   "updated_at":"2026-07-01T09:00:00Z","started_at":"2026-07-01T09:00:00Z"},
  {"id":"orb-7.4","title":"file the licence","status":"open","parent_id":"orb-7",
   "priority":3,"issue_type":"chore","edge_from_parent":"parent-child"},
  {"id":"orb-7.2","title":"survey the mast","status":"closed","parent_id":"orb-7",
   "priority":2,"issue_type":"task","edge_from_parent":"parent-child",
   "closed_at":"2026-08-28T09:00:00Z"}
]"#;

/// `w:p2` names a bead that named a different pane; `w:p9` names nothing.
const PANES: &str = r#"{"id":"cli:agent:list","result":{"agents":[
  {"pane_id":"w:p1","cwd":"/srv/work/orbital","agent_status":"working","title":"the dish"},
  {"pane_id":"w:p2","cwd":"/srv/work/orbital","agent_status":"idle","display_agent":"orb-7"},
  {"pane_id":"w:p3","cwd":"/srv/work/orbital","agent_status":"idle","display_agent":"orb-7.2"},
  {"pane_id":"w:p9","cwd":"/srv/work/orbital","agent_status":"blocked"}
]}}"#;

const CONFIG: &str = r#"
[[projects]]
name = "orbital"
path = "/srv/work/orbital"

[roots]
metadata_keys = ["working_topic"]

[[badges]]
key = "blocked_on"
match = "human"
render = "⏸ waiting"

[anomalies]
stale_claim_days = 7
"#;

/// A runner that replays one canned answer per command line. The directory
/// and the environment each call carries are the unit tests' business.
struct Canned(HashMap<String, Result<String, RunFailure>>);

impl Runner for Canned {
    fn run(
        &self,
        program: &str,
        args: &[&str],
        _cwd: Option<&Path>,
        _env: &Env,
    ) -> Result<String, RunFailure> {
        let argv = format!("{program} {}", args.join(" "));
        self.0
            .get(&argv)
            .cloned()
            .unwrap_or_else(|| panic!("no canned response for `{argv}`"))
    }
}

impl Canned {
    fn answering(mut self, argv: &str, out: &str) -> Self {
        self.0.insert(argv.to_string(), Ok(out.to_string()));
        self
    }

    fn failing(mut self, argv: &str, kind: FailureKind) -> Self {
        self.0.insert(
            argv.to_string(),
            Err(RunFailure {
                kind,
                program: "bd".to_string(),
                detail: "Access denied for user 'orbital' at db.example.invalid:3306".to_string(),
            }),
        );
        self
    }
}

fn canned() -> Canned {
    Canned(HashMap::new())
        .answering("herdr agent list", PANES)
        .answering(
            "bd list --status in_progress --limit 0 --json",
            r#"[{"id":"orb-7.1","title":"re-point the dish","status":"in_progress"}]"#,
        )
        .answering("bd list --status blocked --limit 0 --json", "[]")
        .answering(
            "bd list --has-metadata-key working_topic --limit 0 --json",
            "[]",
        )
        .answering(
            "bd ready --limit 0 --json",
            r#"[{"id":"orb-7.4","title":"file the licence","status":"open"}]"#,
        )
        .answering(
            "bd blocked --json",
            r#"[{"id":"orb-7.1","blocked_by":["orb-9"],"blocked_by_count":1}]"#,
        )
        .answering(
            "bd show orb-7.1 --json",
            r#"[{"id":"orb-7.1","parent":"orb-7"}]"#,
        )
        .answering("bd show orb-7 --json", r#"[{"id":"orb-7","parent":null}]"#)
        .answering("bd dep tree orb-7 --direction=up --json", TREE)
}

fn cfg() -> Config {
    Config::from_toml(CONFIG).expect("the config parses")
}

fn now() -> DateTime<Utc> {
    "2026-08-30T12:00:00Z".parse().expect("the instant parses")
}

fn emit(runner: &Canned, filter: Filter) -> Value {
    let snapshot = beady_eye::app::run(&cfg(), runner, filter, now());
    serde_json::to_value(&snapshot).expect("the snapshot serialises")
}

fn node<'a>(tree: &'a Value, id: &str) -> &'a Value {
    tree["nodes"]
        .as_array()
        .expect("nodes is an array")
        .iter()
        .find(|node| node["id"] == id)
        .unwrap_or_else(|| panic!("{id} is among the nodes"))
}

#[test]
fn the_json_carries_the_contract_fields() {
    let emitted = emit(&canned(), Filter::LiveAgents);

    assert_eq!(emitted["generated_at"], "2026-08-30T12:00:00Z");
    assert_eq!(emitted["herdr"], "ok");
    assert_eq!(emitted["filter"], "live-agents");
    assert_eq!(emitted["hidden_trees"], json!([]));
    assert_eq!(emitted["failed_projects"], json!([]));

    let tree = &emitted["trees"][0];
    assert_eq!(tree["project"], "orbital");
    assert_eq!(tree["root"], "orb-7");
    assert_eq!(tree["title"], "lift the ground station");
    assert_eq!(tree["tracker"], "ok");
    assert_eq!(tree["dangling"], json!([]));
    assert_eq!(tree["unreachable"], json!([]));
    assert_eq!(
        tree["counts"],
        json!({"total": 5, "closed": 1, "live_agents": 2, "anomalies": 3})
    );
}

/// `nodes` is pre-flattened in render order with an explicit depth, so a
/// consumer draws it without rebuilding the tree.
#[test]
fn the_nodes_arrive_flattened_in_render_order() {
    let emitted = emit(&canned(), Filter::LiveAgents);
    let nodes = emitted["trees"][0]["nodes"]
        .as_array()
        .expect("nodes is an array");

    let order: Vec<(&str, u64)> = nodes
        .iter()
        .map(|node| {
            (
                node["id"].as_str().expect("an id"),
                node["depth"].as_u64().expect("a depth"),
            )
        })
        .collect();

    assert_eq!(
        order,
        vec![
            ("orb-7", 0),
            ("orb-7.3", 1),
            ("orb-7.1", 1),
            ("orb-7.4", 1),
            ("orb-7.2", 1),
        ]
    );
}

#[test]
fn a_node_carries_every_field_the_contract_names() {
    let emitted = emit(&canned(), Filter::LiveAgents);
    let claimed = node(&emitted["trees"][0], "orb-7.1");

    assert_eq!(
        claimed,
        &json!({
            "id": "orb-7.1",
            "title": "re-point the dish",
            "status": "in_progress",
            "issue_type": "task",
            "priority": 2,
            "depth": 1,
            "edge": "parent-child",
            "ready": false,
            "blocked_by": ["orb-9"],
            "started_at": "2026-08-29T10:00:00Z",
            "closed_at": null,
            "badges": [{"key": "blocked_on", "text": "⏸ waiting"}],
            "agent": null,
            "anomalies": [{"rule": "orphan-claim"}],
            "truncated": false,
        })
    );
}

/// `agent.source` records which direction of the join resolved it, so a
/// consumer can tell a confirmed agent from an inferred one.
#[test]
fn the_agent_records_which_direction_of_the_join_resolved_it() {
    let emitted = emit(&canned(), Filter::LiveAgents);
    let tree = &emitted["trees"][0];

    assert_eq!(
        node(tree, "orb-7")["agent"],
        json!({
            "pane": "w:p1",
            "pane_status": "working",
            "title": "the dish",
            "source": "agent_pane",
        }),
        "the bead named its pane"
    );
    assert_eq!(
        node(tree, "orb-7.2")["agent"]["source"],
        "display_agent",
        "the pane named its bead"
    );
}

/// The field is `anomalies`, a list, and a bead carries every rule that
/// fires. `docs/design.md`'s example still shows a single `anomaly`; its
/// prose governs and the model follows the prose.
#[test]
fn a_node_carries_every_anomaly_that_fires_on_it() {
    let emitted = emit(&canned(), Filter::LiveAgents);
    let tree = &emitted["trees"][0];

    assert_eq!(
        node(tree, "orb-7.3")["anomalies"],
        json!([{"rule": "orphan-claim"}, {"rule": "stale-claim", "days": 60}])
    );
    assert_eq!(
        node(tree, "orb-7.2")["anomalies"],
        json!([{"rule": "stale-pane"}])
    );
    assert_eq!(node(tree, "orb-7")["anomalies"], json!([]));
}

#[test]
fn readiness_reaches_the_json_as_bd_reported_it() {
    let emitted = emit(&canned(), Filter::LiveAgents);
    let tree = &emitted["trees"][0];

    assert_eq!(node(tree, "orb-7.4")["ready"], true);
    assert_eq!(node(tree, "orb-7.1")["ready"], false);
    assert_eq!(node(tree, "orb-7.1")["blocked_by"], json!(["orb-9"]));
    assert_eq!(node(tree, "orb-7.4")["blocked_by"], json!([]));
}

/// A pane belonging to no bead is reported with the project its directory
/// sits in, so a consumer groups it without resolving the path again.
#[test]
fn a_pane_on_no_bead_is_reported_with_its_project() {
    let emitted = emit(&canned(), Filter::LiveAgents);

    assert_eq!(
        emitted["unattributed"],
        json!([
            {"pane": "w:p2", "project": "orbital", "cwd": "/srv/work/orbital",
             "pane_status": "idle"},
            {"pane": "w:p9", "project": "orbital", "cwd": "/srv/work/orbital",
             "pane_status": "blocked"},
        ])
    );
}

/// The two directions of the join disagreeing is a finding, not something to
/// resolve by picking a winner.
#[test]
fn a_join_disagreement_is_reported_at_the_top_level() {
    let emitted = emit(&canned(), Filter::LiveAgents);

    assert_eq!(
        emitted["conflicts"],
        json!([{
            "conflict": "bead-and-pane-disagree",
            "bead": {"project": "orbital", "id": "orb-7"},
            "named_by_bead": "w:p1",
            "named_by_pane": "w:p2",
        }])
    );
}

#[test]
fn a_root_whose_tracker_cannot_be_read_is_named_in_the_json() {
    let runner = canned().failing(
        "bd dep tree orb-7 --direction=up --json",
        FailureKind::Unavailable,
    );

    let emitted = emit(&runner, Filter::LiveAgents);

    assert_eq!(emitted["trees"][0]["root"], "orb-7");
    assert_eq!(emitted["trees"][0]["project"], "orbital");
    assert_eq!(
        emitted["trees"][0]["tracker"],
        json!({"unreachable": "unavailable"})
    );
    assert_eq!(emitted["trees"][0]["nodes"], json!([]));
    assert_eq!(emitted["hidden_trees"], json!([]), "never filtered away");
}

#[test]
fn a_project_whose_tracker_refuses_the_credential_is_named_in_the_json() {
    let runner = canned().failing(
        "bd list --status in_progress --limit 0 --json",
        FailureKind::Auth,
    );

    let emitted = emit(&runner, Filter::LiveAgents);

    assert_eq!(emitted["trees"], json!([]));
    assert_eq!(
        emitted["failed_projects"],
        json!([{"project": "orbital", "tracker": "auth"}])
    );
}

/// bd names the database and the SQL user when it refuses a credential.
#[test]
fn bds_own_words_never_reach_the_json() {
    let runner = canned().failing(
        "bd list --status in_progress --limit 0 --json",
        FailureKind::Auth,
    );

    let emitted = emit(&runner, Filter::All).to_string();

    for leak in ["Access denied", "db.example.invalid", "3306", "'orbital'"] {
        assert!(!emitted.contains(leak), "{leak:?} survived into {emitted}");
    }
}

#[test]
fn without_herdr_the_json_says_so_and_still_carries_every_tree() {
    let runner = canned().failing("herdr agent list", FailureKind::Exec);

    let emitted = emit(&runner, Filter::LiveAgents);

    assert_eq!(emitted["herdr"], "unavailable");
    assert_eq!(emitted["trees"][0]["root"], "orb-7");
    assert_eq!(emitted["trees"][0]["counts"]["live_agents"], 0);
    assert_eq!(emitted["unattributed"], json!([]));
    assert_eq!(emitted["conflicts"], json!([]));
}

/// A filtered tree is reported, never dropped.
#[test]
fn a_tree_with_no_live_agent_is_reported_and_the_flag_shows_it() {
    let runner = canned().answering("herdr agent list", r#"{"result":{"agents":[]}}"#);

    let filtered = emit(&runner, Filter::LiveAgents);
    assert_eq!(filtered["trees"], json!([]));
    assert_eq!(
        filtered["hidden_trees"],
        json!([{"project": "orbital", "root": "orb-7",
                "title": "lift the ground station", "reason": "no-live-agent"}])
    );

    let unfiltered = emit(&runner, Filter::All);
    assert_eq!(unfiltered["filter"], "all");
    assert_eq!(unfiltered["trees"][0]["root"], "orb-7");
    assert_eq!(unfiltered["hidden_trees"], json!([]));
}
