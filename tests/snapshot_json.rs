//! `bdi --json` emits the contract in `docs/design.md`. These drive the whole
//! binary's model through the one public entry point a consumer sees.

use beady_eye::collect::run::FailureKind;
use beady_eye::config::Config;
use beady_eye::model::snapshot::Filter;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

mod canned;

use canned::Canned;

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
  {"pane_id":"w:p9","cwd":"/srv/work/orbital","agent_status":"blocked"},
  {"pane_id":"w:pF","cwd":"/srv/spike","agent_status":"idle"}
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

fn canned() -> Canned {
    Canned::default()
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
        .answering(
            "bd show orb-7.2 --json",
            r#"[{"id":"orb-7.2","parent":"orb-7"}]"#,
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

/// A pane under no configured project is a finding about the configuration,
/// so it is its own array rather than an `unattributed` entry with the
/// project left out. The two carry different keys, which a consumer can test
/// for; a missing value would be a judgement they have to make.
#[test]
fn a_pane_under_no_configured_project_is_its_own_array() {
    let emitted = emit(&canned(), Filter::LiveAgents);

    assert_eq!(
        emitted["unconfigured"],
        json!([{"pane": "w:pF", "cwd": "/srv/spike", "pane_status": "idle"}])
    );
    assert!(
        emitted["unconfigured"][0].get("project").is_none(),
        "there is no project to name, so there is no key"
    );
    assert!(
        !emitted["unattributed"]
            .as_array()
            .expect("unattributed is an array")
            .iter()
            .any(|pane| pane["pane"] == "w:pF"),
        "a pane is in one array or the other, never both"
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

/// A configured project whose tracker refused is still a configured project.
/// Its panes have nowhere to be attributed, which is not the same as `bdi`
/// never having been told the project exists — and telling those two apart is
/// the whole of the split.
#[test]
fn a_pane_in_a_refused_project_is_unattributed_rather_than_unconfigured() {
    let runner = canned().failing(
        "bd list --status in_progress --limit 0 --json",
        FailureKind::Auth,
    );

    let emitted = emit(&runner, Filter::LiveAgents);

    let unattributed = emitted["unattributed"]
        .as_array()
        .expect("unattributed is an array");
    assert!(
        unattributed
            .iter()
            .all(|pane| pane["project"] == "orbital" && pane["cwd"] == "/srv/work/orbital"),
        "{unattributed:#?}"
    );
    assert_eq!(
        emitted["unconfigured"],
        json!([{"pane": "w:pF", "cwd": "/srv/spike", "pane_status": "idle"}]),
        "only the pane outside every configured project"
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

const HARBOUR_DIR: &str = "/srv/work/harbour";

/// A second tracker's own `orb-7`: the same bare id, a different bead, a
/// different project. Prefixes are per-tracker and uncoordinated, so this is
/// the case `(project, id)` exists for. Invented rather than captured — no
/// other project's tracker was read to write it.
const HARBOUR_TREE: &str = r#"[
  {"id":"orb-7","title":"re-dredge the north channel","status":"in_progress","parent_id":"",
   "priority":1,"issue_type":"epic","updated_at":"2026-08-29T09:00:00Z",
   "started_at":"2026-08-25T09:00:00Z","metadata":{"agent_pane":"w:p5"}},
  {"id":"orb-7.1","title":"hire the dredger","status":"in_progress","parent_id":"orb-7",
   "priority":2,"issue_type":"task","edge_from_parent":"parent-child",
   "updated_at":"2026-08-29T11:00:00Z","started_at":"2026-08-29T11:00:00Z"}
]"#;

/// One live pane in each project's directory.
const PANES_ACROSS: &str = r#"{"id":"cli:agent:list","result":{"agents":[
  {"pane_id":"w:p1","cwd":"/srv/work/orbital","agent_status":"working","title":"the dish"},
  {"pane_id":"w:p5","cwd":"/srv/work/harbour","agent_status":"working","title":"the channel"}
]}}"#;

const TWO_PROJECTS: &str = r#"
[[projects]]
name = "orbital"
path = "/srv/work/orbital"
credential_command = "pass show orbital/tracker"

[[projects]]
name = "harbour"
path = "/srv/work/harbour"
credential_command = "pass show harbour/tracker"

[roots]
metadata_keys = ["working_topic"]

[[badges]]
key = "blocked_on"
match = "human"
render = "⏸ waiting"

[anomalies]
stale_claim_days = 7
"#;

fn two_projects() -> Config {
    Config::from_toml(TWO_PROJECTS).expect("the config parses")
}

/// Orbital answering wherever it is asked, plus a harbour tracker that answers
/// only in harbour's own directory. Every answer harbour gives contradicts
/// orbital's, so a call that reached the wrong directory replays the wrong
/// tracker and the case fails rather than passing on a coincidence.
fn across_two_projects() -> Canned {
    canned()
        .answering("herdr agent list", PANES_ACROSS)
        .answering("sh -c pass show orbital/tracker", "orbital-secret\n")
        .answering("sh -c pass show harbour/tracker", "harbour-secret\n")
        .answering_in(
            HARBOUR_DIR,
            "bd list --status in_progress --limit 0 --json",
            r#"[{"id":"orb-7.1","title":"hire the dredger","status":"in_progress"}]"#,
        )
        .answering_in(
            HARBOUR_DIR,
            "bd list --status blocked --limit 0 --json",
            "[]",
        )
        .answering_in(
            HARBOUR_DIR,
            "bd list --has-metadata-key working_topic --limit 0 --json",
            "[]",
        )
        .answering_in(HARBOUR_DIR, "bd ready --limit 0 --json", "[]")
        .answering_in(HARBOUR_DIR, "bd blocked --json", "[]")
        .answering_in(
            HARBOUR_DIR,
            "bd show orb-7.1 --json",
            r#"[{"id":"orb-7.1","parent":"orb-7"}]"#,
        )
        .answering_in(
            HARBOUR_DIR,
            "bd show orb-7 --json",
            r#"[{"id":"orb-7","parent":null}]"#,
        )
        .answering_in(
            HARBOUR_DIR,
            "bd dep tree orb-7 --direction=up --json",
            HARBOUR_TREE,
        )
}

fn emit_over(cfg: &Config, runner: &Canned, filter: Filter) -> Value {
    let snapshot = beady_eye::app::run(cfg, runner, filter, now());
    serde_json::to_value(&snapshot).expect("the snapshot serialises")
}

/// Both roots are called `orb-7`, so the project each tree was read from is
/// the only thing that tells the two apart.
#[test]
fn each_tree_carries_the_project_it_was_read_from() {
    let emitted = emit_over(&two_projects(), &across_two_projects(), Filter::LiveAgents);

    let trees = emitted["trees"].as_array().expect("trees is an array");
    assert_eq!(trees.len(), 2);

    assert_eq!(trees[0]["project"], "orbital");
    assert_eq!(trees[0]["root"], "orb-7");
    assert_eq!(trees[0]["title"], "lift the ground station");

    assert_eq!(trees[1]["project"], "harbour");
    assert_eq!(trees[1]["root"], "orb-7");
    assert_eq!(trees[1]["title"], "re-dredge the north channel");
}

/// The two `orb-7.1`s are different beads: each carries its own tracker's
/// title, its own tracker's readiness, and the agent in its own project.
#[test]
fn a_bare_id_in_two_trackers_names_two_beads() {
    let emitted = emit_over(&two_projects(), &across_two_projects(), Filter::LiveAgents);
    let orbital = &emitted["trees"][0];
    let harbour = &emitted["trees"][1];

    assert_eq!(node(orbital, "orb-7.1")["title"], "re-point the dish");
    assert_eq!(node(harbour, "orb-7.1")["title"], "hire the dredger");

    assert_eq!(node(orbital, "orb-7.1")["blocked_by"], json!(["orb-9"]));
    assert_eq!(node(harbour, "orb-7.1")["blocked_by"], json!([]));

    assert_eq!(node(orbital, "orb-7")["agent"]["pane"], "w:p1");
    assert_eq!(node(harbour, "orb-7")["agent"]["pane"], "w:p5");
    assert_eq!(emitted["conflicts"], json!([]));
}

/// Degrade, never disappear, at project granularity: harbour's tracker
/// refusing the credential costs harbour's trees and nothing else.
#[test]
fn one_projects_tracker_failing_leaves_the_others_trees_standing() {
    let runner = across_two_projects().failing_in(
        HARBOUR_DIR,
        "bd list --status in_progress --limit 0 --json",
        FailureKind::Auth,
    );

    let emitted = emit_over(&two_projects(), &runner, Filter::LiveAgents);

    assert_eq!(
        emitted["failed_projects"],
        json!([{"project": "harbour", "tracker": "auth"}])
    );

    let trees = emitted["trees"].as_array().expect("trees is an array");
    assert_eq!(trees.len(), 1);
    assert_eq!(trees[0]["project"], "orbital");
    assert_eq!(node(&trees[0], "orb-7.1")["title"], "re-point the dish");
    assert_eq!(node(&trees[0], "orb-7")["agent"]["pane"], "w:p1");

    assert_eq!(
        emitted["unattributed"],
        json!([{"pane": "w:p5", "project": "harbour", "cwd": HARBOUR_DIR,
                "pane_status": "working"}]),
        "the pane in the failed project is still reported"
    );
}
