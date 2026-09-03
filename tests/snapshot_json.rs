//! `bdi --json` emits the contract in `docs/design.md`. These drive the whole
//! binary's model through the one public entry point a consumer sees.

use beady_eye::collect::bd::parse_beads;
use beady_eye::collect::herdr::Herdr;
use beady_eye::collect::run::FailureKind;
use beady_eye::collect::tracker::testing::{Asked, Fake, Fakes};
use beady_eye::config::Config;
use beady_eye::model::snapshot::Filter;
use beady_eye::model::types::Bead;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

mod canned;

use canned::{refused, Canned};

/// One project's tracker: an epic with a pane on it, a claimed task with
/// none, a claim nothing has touched in weeks, a task bd calls ready, and a
/// closed task a pane is still sitting on. Every row carries its own
/// `parent` as well as the edge, because bd writes both.
const TREE: &str = r#"[
  {"id":"orb-7","title":"lift the ground station","status":"in_progress",
   "priority":1,"issue_type":"epic","updated_at":"2026-08-29T09:00:00Z",
   "started_at":"2026-08-20T09:00:00Z","metadata":{"agent_pane":"w:p1"}},
  {"id":"orb-7.1","title":"re-point the dish","status":"in_progress","parent":"orb-7",
   "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
   "priority":2,"issue_type":"task",
   "updated_at":"2026-08-29T10:00:00Z","started_at":"2026-08-29T10:00:00Z",
   "metadata":{"blocked_on":"human"}},
  {"id":"orb-7.3","title":"lay the feeder cable","status":"in_progress","parent":"orb-7",
   "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
   "priority":1,"issue_type":"task",
   "updated_at":"2026-07-01T09:00:00Z","started_at":"2026-07-01T09:00:00Z"},
  {"id":"orb-7.4","title":"file the licence","status":"open","parent":"orb-7",
   "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
   "priority":3,"issue_type":"chore"},
  {"id":"orb-7.2","title":"survey the mast","status":"closed","parent":"orb-7",
   "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
   "priority":2,"issue_type":"task",
   "closed_at":"2026-08-28T09:00:00Z"}
]"#;

/// The same tracker with nothing claimed in it: a root waiting on work
/// elsewhere, over open tasks and a closed one. No pane can be on it and no
/// anomaly rule can fire on it, which is the one state the default filter
/// folds away.
const UNSTAFFED_TREE: &str = r#"[
  {"id":"orb-7","title":"lift the ground station","status":"blocked",
   "priority":1,"issue_type":"epic"},
  {"id":"orb-7.4","title":"file the licence","status":"open","parent":"orb-7",
   "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
   "priority":3,"issue_type":"chore"},
  {"id":"orb-7.2","title":"survey the mast","status":"closed","parent":"orb-7",
   "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
   "priority":2,"issue_type":"task",
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

[[badges]]
key = "blocked_on"
match = "human"
render = "⏸ waiting"

[anomalies]
stale_claim_days = 7
"#;

/// Rows as a test writes them, read into the beads a tracker answers with.
fn beads(rows: &str) -> Vec<Bead> {
    parse_beads(rows).expect("the rows parse")
}

/// The herdr session as these tests find it.
fn panes() -> Canned {
    Canned::default().herdr_holding(PANES)
}

/// Orbital's tracker holding `rows` in place of its usual tree, with the same
/// task ready and the same task blocked from outside it.
fn orbital_holding(rows: &str) -> Fake {
    Fake::holding(beads(rows))
        .ready(["orb-7.4"])
        .blocked("orb-7.1", &["orb-9"])
}

/// Orbital's tracker as these tests find it.
fn orbital_tracker() -> Fake {
    orbital_holding(TREE)
}

/// The one project's trackers, with orbital's staged as `tracker`.
fn orbital_with(tracker: Fake) -> Fakes {
    Fakes::default().with("orbital", tracker)
}

/// The one project's trackers as these tests find them.
fn orbital() -> Fakes {
    orbital_with(orbital_tracker())
}

/// A run recorded as wisps, in the shape bd writes one: a `molecule` that is
/// nobody's child, and its steps hanging under it by parent-child. The
/// molecule is what costs the whole run — root discovery is the only thing
/// that can place it, and without it every step hangs off a node the answer
/// does not hold.
const WISP_RUN: &str = r#"[
  {"id":"orb-wisp-gvi","title":"re-point the dish","status":"in_progress","parent":null,
   "priority":2,"issue_type":"molecule","ephemeral":true},
  {"id":"orb-wisp-7dg","title":"slew the mount","status":"closed","parent":"orb-wisp-gvi",
   "priority":2,"issue_type":"task","ephemeral":true,"dependencies":[
     {"issue_id":"orb-wisp-7dg","depends_on_id":"orb-wisp-gvi","type":"parent-child"}]},
  {"id":"orb-wisp-v4p","title":"sign off the alignment","status":"open","parent":"orb-wisp-gvi",
   "priority":2,"issue_type":"gate","ephemeral":true,"dependencies":[
     {"issue_id":"orb-wisp-v4p","depends_on_id":"orb-wisp-gvi","type":"parent-child"},
     {"issue_id":"orb-wisp-v4p","depends_on_id":"orb-wisp-7dg","type":"blocks"}]}
]"#;

/// The run is a tree of its own beside the permanent work, and every step of
/// it is drawn. A wisp bd discards is a step of the run that is invisible
/// while it happens, which is the whole reason to draw them.
#[test]
fn a_run_recorded_as_wisps_is_drawn_beside_the_permanent_work() {
    let trackers = orbital_with(orbital_tracker().also(beads(WISP_RUN)));

    let emitted = emit(&panes(), &trackers, Filter::All);

    let trees = emitted["trees"].as_array().expect("trees is an array");
    let run = trees
        .iter()
        .find(|tree| tree["root"] == "orb-wisp-gvi")
        .expect("the molecule is a root of its own");
    assert_eq!(run["title"], "re-point the dish");

    let drawn: Vec<(&str, u64)> = run["nodes"]
        .as_array()
        .expect("nodes is an array")
        .iter()
        .map(|node| {
            (
                node["id"].as_str().expect("an id"),
                node["depth"].as_u64().expect("a depth"),
            )
        })
        .collect();
    // A step reached by two edges is drawn under both, as any bead is: the
    // closed step hangs under the molecule, and again under the gate it
    // blocks.
    assert_eq!(
        drawn,
        vec![
            ("orb-wisp-gvi", 0),
            ("orb-wisp-v4p", 1),
            ("orb-wisp-7dg", 2),
            ("orb-wisp-7dg", 1),
        ]
    );
    assert_eq!(node(run, "orb-wisp-v4p")["edge"], "parent-child");
    assert_eq!(node(run, "orb-wisp-v4p")["issue_type"], "gate");

    assert!(
        trees.iter().any(|tree| tree["root"] == "orb-7"),
        "the permanent work is still drawn"
    );
}

fn cfg() -> Config {
    Config::from_toml(CONFIG).expect("the config parses")
}

fn now() -> DateTime<Utc> {
    "2026-08-30T12:00:00Z".parse().expect("the instant parses")
}

fn emit(runner: &Canned, trackers: &Fakes, filter: Filter) -> Value {
    emit_over(&cfg(), runner, trackers, filter)
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
    let emitted = emit(&panes(), &orbital(), Filter::LiveAgents);

    assert_eq!(emitted["generated_at"], "2026-08-30T12:00:00Z");
    assert_eq!(
        emitted["agents"],
        json!({"provider": "herdr", "state": "answering",
               "sessions": [{"name": "default", "state": "answering"}]})
    );
    assert_eq!(emitted["filter"], "live-agents");
    assert_eq!(emitted["hidden_trees"], json!([]));
    assert_eq!(emitted["failed_projects"], json!([]));

    // `w:p3` sits on a closed bead the one tree already draws, so a second
    // tree here is that pane rooting the bead beside its own tree.
    assert_eq!(emitted["trees"].as_array().map(Vec::len), Some(1));
    let tree = &emitted["trees"][0];
    assert_eq!(tree["project"], "orbital");
    assert_eq!(tree["root"], "orb-7");
    assert_eq!(tree["title"], "lift the ground station");
    assert_eq!(tree["tracker"], "ok");
    assert_eq!(tree["dangling"], json!([]));
    assert_eq!(tree["cycles"], json!([]));
    assert_eq!(
        tree["counts"],
        json!({"total": 5, "closed": 1, "live_agents": 2, "anomalies": 3})
    );
}

/// `nodes` is pre-flattened in render order with an explicit depth, so a
/// consumer draws it without rebuilding the tree.
#[test]
fn the_nodes_arrive_flattened_in_render_order() {
    let emitted = emit(&panes(), &orbital(), Filter::LiveAgents);
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
    let emitted = emit(&panes(), &orbital(), Filter::LiveAgents);
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
        })
    );
}

/// `agent.source` records which direction of the join resolved it, so a
/// consumer can tell a confirmed agent from an inferred one.
#[test]
fn the_agent_records_which_direction_of_the_join_resolved_it() {
    let emitted = emit(&panes(), &orbital(), Filter::LiveAgents);
    let tree = &emitted["trees"][0];

    assert_eq!(
        node(tree, "orb-7")["agent"],
        json!({
            "pane": {"session": "default", "id": "w:p1"},
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
    let emitted = emit(&panes(), &orbital(), Filter::LiveAgents);
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
    let emitted = emit(&panes(), &orbital(), Filter::LiveAgents);
    let tree = &emitted["trees"][0];

    assert_eq!(node(tree, "orb-7.4")["ready"], true);
    assert_eq!(node(tree, "orb-7.1")["ready"], false);
    assert_eq!(node(tree, "orb-7.1")["blocked_by"], json!(["orb-9"]));
    assert_eq!(node(tree, "orb-7.4")["blocked_by"], json!([]));
}

/// A pane belonging to no bead is reported with the project its directory
/// sits in, so a consumer groups it without resolving the path again, and
/// with what it reported about itself under the names a node's `agent`
/// carries the same things — null where it reported nothing, never absent.
#[test]
fn a_pane_on_no_bead_is_reported_with_its_project() {
    let emitted = emit(&panes(), &orbital(), Filter::LiveAgents);

    assert_eq!(
        emitted["unattributed"],
        json!([
            {"pane": {"session": "default", "id": "w:p2"}, "project": "orbital", "cwd": "/srv/work/orbital",
             "pane_status": "idle", "display_agent": "orb-7", "title": null},
            {"pane": {"session": "default", "id": "w:p9"}, "project": "orbital", "cwd": "/srv/work/orbital",
             "pane_status": "blocked", "display_agent": null, "title": null},
        ])
    );
}

/// A pane under no configured project is a finding about the configuration,
/// so it is its own array rather than an `unattributed` entry with the
/// project left out. The two carry different keys, which a consumer can test
/// for; a missing value would be a judgement they have to make.
#[test]
fn a_pane_under_no_configured_project_is_its_own_array() {
    let emitted = emit(&panes(), &orbital(), Filter::LiveAgents);

    assert_eq!(
        emitted["unconfigured"],
        json!([{"pane": {"session": "default", "id": "w:pF"}, "cwd": "/srv/spike", "pane_status": "idle"}])
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
            .any(|pane| pane["pane"]["id"] == "w:pF"),
        "a pane is in one array or the other, never both"
    );
}

/// bdi-2jh. A seat that claims a bead and moves on without clearing its key
/// leaves a claim standing on the pane it is still sitting in, so two beads
/// name one pane and `bdi` awards it to neither. Nothing else in the emitted
/// snapshot then says what that pane is working on — the caption is carried
/// on the agent a pane was awarded, and this one was awarded to nobody — so
/// the disagreement carries it, and a consumer can tell the live claim from
/// the stale one without going back to the tracker.
#[test]
fn a_contested_pane_is_reported_with_its_own_account_of_itself() {
    let contested = TREE.replace(
        r#""started_at":"2026-07-01T09:00:00Z"}"#,
        r#""started_at":"2026-07-01T09:00:00Z","metadata":{"agent_pane":"w:p1"}}"#,
    );
    let emitted = emit(
        &panes(),
        &orbital_with(orbital_holding(&contested)),
        Filter::LiveAgents,
    );

    assert!(
        emitted["conflicts"]
            .as_array()
            .expect("conflicts is an array")
            .contains(&json!({
                "conflict": "several-beads-name-one-pane",
                "pane": {"session": "default", "id": "w:p1"},
                "caption": "the dish",
                "beads": [
                    {"project": "orbital", "id": "orb-7"},
                    {"project": "orbital", "id": "orb-7.3"},
                ],
            })),
        "{}",
        emitted["conflicts"]
    );
}

/// The two directions of the join disagreeing is a finding, not something to
/// resolve by picking a winner.
#[test]
fn a_join_disagreement_is_reported_at_the_top_level() {
    let emitted = emit(&panes(), &orbital(), Filter::LiveAgents);

    assert_eq!(
        emitted["conflicts"],
        json!([{
            "conflict": "bead-and-pane-disagree",
            "bead": {"project": "orbital", "id": "orb-7"},
            "named_by_bead": {"session": "default", "id": "w:p1"},
            "named_by_pane": {"session": "default", "id": "w:p2"},
        }])
    );
}

/// Every root but a configured one is read off the answer, so an answer
/// holding no bead is an empty forest and not a tree the tracker failed to
/// draw.
#[test]
fn a_tracker_holding_no_bead_emits_no_tree_and_no_failure() {
    let trackers = orbital_with(orbital_holding("[]"));

    let emitted = emit(&panes(), &trackers, Filter::LiveAgents);

    assert_eq!(emitted["trees"], json!([]));
    assert_eq!(emitted["failed_projects"], json!([]));
    assert_eq!(emitted["hidden_trees"], json!([]));
}

/// One read draws a project's whole forest, so its failure is the project's
/// and not any one root's — named, with its reason, rather than a screen of
/// empty trees.
#[test]
fn a_tracker_that_stops_answering_is_named_in_the_json_as_the_project_it_is() {
    let trackers =
        orbital_with(orbital_tracker().failing(Asked::All, refused(FailureKind::Unavailable)));

    let emitted = emit(&panes(), &trackers, Filter::LiveAgents);

    assert_eq!(emitted["trees"], json!([]));
    assert_eq!(
        emitted["failed_projects"],
        json!([{"project": "orbital", "tracker": "unavailable"}])
    );
    assert_eq!(emitted["hidden_trees"], json!([]), "never filtered away");
}

#[test]
fn a_project_whose_tracker_refuses_the_credential_is_named_in_the_json() {
    let emitted = emit(&panes(), &orbital_refusing(), Filter::LiveAgents);

    assert_eq!(emitted["trees"], json!([]));
    assert_eq!(
        emitted["failed_projects"],
        json!([{"project": "orbital", "tracker": "auth"}])
    );
}

/// A bd that does not know a flag `bdi` uses is a bd to replace, which a
/// consumer cannot tell from an outage unless the reason says so.
#[test]
fn a_project_whose_bd_does_not_know_a_flag_is_named_in_the_json_as_such() {
    let trackers =
        orbital_with(orbital_tracker().failing(Asked::All, refused(FailureKind::UnknownFlag)));

    let emitted = emit(&panes(), &trackers, Filter::LiveAgents);

    assert_eq!(
        emitted["failed_projects"],
        json!([{"project": "orbital", "tracker": "unknown-flag"}])
    );
}

/// The two ways bd never ran carry their own reasons, because they want
/// different things of the reader: one is a bd to install, the other a bd or
/// a project directory to repair.
#[test]
fn a_bd_that_is_not_installed_and_one_that_will_not_start_carry_different_reasons() {
    let missing =
        orbital_with(orbital_tracker().failing(Asked::All, refused(FailureKind::NotInstalled)));
    let broken =
        orbital_with(orbital_tracker().failing(Asked::All, refused(FailureKind::Unstartable)));

    assert_eq!(
        emit(&panes(), &missing, Filter::LiveAgents)["failed_projects"],
        json!([{"project": "orbital", "tracker": "not-installed"}])
    );
    assert_eq!(
        emit(&panes(), &broken, Filter::LiveAgents)["failed_projects"],
        json!([{"project": "orbital", "tracker": "unstartable"}])
    );
}

/// A configured project whose tracker refused is still a configured project.
/// Its panes have nowhere to be attributed, which is not the same as `bdi`
/// never having been told the project exists — and telling those two apart is
/// the whole of the split.
#[test]
fn a_pane_in_a_refused_project_is_unattributed_rather_than_unconfigured() {
    let emitted = emit(&panes(), &orbital_refusing(), Filter::LiveAgents);

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
        json!([{"pane": {"session": "default", "id": "w:pF"}, "cwd": "/srv/spike", "pane_status": "idle"}]),
        "only the pane outside every configured project"
    );
}

/// The one project's trackers, with orbital's refusing its credential the
/// way bd does: naming the database and the SQL user it turned away.
fn orbital_refusing() -> Fakes {
    orbital_with(orbital_tracker().failing(Asked::All, refused(FailureKind::Auth)))
}

/// A tracker names the database and the SQL user when it refuses a
/// credential.
#[test]
fn a_trackers_own_words_never_reach_the_json() {
    let emitted = emit(&panes(), &orbital_refusing(), Filter::All).to_string();

    for leak in ["Access denied", "db.example.invalid", "3306", "'orbital'"] {
        assert!(!emitted.contains(leak), "{leak:?} survived into {emitted}");
    }
}

/// A machine with no herdr installed. `NotInstalled` is the one failure that
/// means nothing was there to run, so the contract says the provider is
/// absent rather than that something the reader had has broken.
#[test]
fn with_no_provider_installed_the_json_says_absent_and_still_carries_every_tree() {
    let nothing = Canned::default().herdr_failing(FailureKind::NotInstalled);

    let emitted = emit(&nothing, &orbital(), Filter::LiveAgents);

    assert_eq!(
        emitted["agents"],
        json!({"provider": "herdr", "state": "absent", "sessions": []})
    );
    assert_eq!(emitted["trees"][0]["root"], "orb-7");
    assert_eq!(emitted["trees"][0]["counts"]["live_agents"], 0);
    assert_eq!(emitted["unattributed"], json!([]));
    assert_eq!(emitted["conflicts"], json!([]));
}

/// The same forest, and the other reason for it: herdr is installed and would
/// not answer. Every tree still draws, and a consumer can tell the two apart
/// because only this one is a finding.
#[test]
fn a_provider_that_will_not_answer_is_told_apart_from_one_that_is_not_there() {
    let no_session = Canned::default().herdr_failing(FailureKind::Unavailable);

    let emitted = emit(&no_session, &orbital(), Filter::LiveAgents);

    assert_eq!(
        emitted["agents"],
        json!({"provider": "herdr", "state": "not-answering", "sessions": []})
    );
    assert_eq!(emitted["trees"][0]["root"], "orb-7");
    assert_eq!(emitted["trees"][0]["counts"]["live_agents"], 0);
}

/// The third way, and the one that used to be read as the first: herdr is
/// installed and the run could not be started. A consumer sees the finding,
/// not the absence a machine with no herdr at all reports.
#[test]
fn a_provider_that_is_there_and_will_not_start_is_not_reported_as_absent() {
    let broken = Canned::default().herdr_failing(FailureKind::InstalledUnstartable);

    let emitted = emit(&broken, &orbital(), Filter::LiveAgents);

    assert_eq!(
        emitted["agents"],
        json!({"provider": "herdr", "state": "not-answering", "sessions": []})
    );
    assert_eq!(emitted["trees"][0]["root"], "orb-7");
}

/// A filtered tree is reported, never dropped.
#[test]
fn a_tree_with_no_live_agent_is_reported_and_the_flag_shows_it() {
    let nobody = Canned::default().herdr_holding(r#"{"result":{"agents":[]}}"#);
    let trackers = orbital_with(Fake::holding(beads(UNSTAFFED_TREE)).ready(["orb-7.4"]));

    let filtered = emit(&nobody, &trackers, Filter::LiveAgents);
    assert_eq!(filtered["trees"], json!([]));
    assert_eq!(
        filtered["hidden_trees"],
        json!([{"project": "orbital", "root": "orb-7",
                "title": "lift the ground station", "reason": "no-live-agent"}])
    );

    let unfiltered = emit(&nobody, &trackers, Filter::All);
    assert_eq!(unfiltered["filter"], "all");
    assert_eq!(unfiltered["trees"][0]["root"], "orb-7");
    assert_eq!(unfiltered["hidden_trees"], json!([]));
}

/// A second tracker's own `orb-7`: the same bare id, a different bead, a
/// different project. Prefixes are per-tracker and uncoordinated, so this is
/// the case `(project, id)` exists for. Invented rather than captured — no
/// other project's tracker was read to write it.
const HARBOUR_TREE: &str = r#"[
  {"id":"orb-7","title":"re-dredge the north channel","status":"in_progress",
   "priority":1,"issue_type":"epic","updated_at":"2026-08-29T09:00:00Z",
   "started_at":"2026-08-25T09:00:00Z","metadata":{"agent_pane":"w:p5"}},
  {"id":"orb-7.1","title":"hire the dredger","status":"in_progress","parent":"orb-7",
   "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
   "priority":2,"issue_type":"task",
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

[[projects]]
name = "harbour"
path = "/srv/work/harbour"

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

/// Orbital's tracker beside harbour's. Every answer harbour gives contradicts
/// orbital's, so a tree drawn from the wrong tracker fails the case rather
/// than passing on a coincidence.
fn across_two_projects() -> Fakes {
    across_two_projects_with(Fake::holding(beads(HARBOUR_TREE)))
}

/// The same, with harbour's tracker staged as `harbour`.
fn across_two_projects_with(harbour: Fake) -> Fakes {
    Fakes::default()
        .with("orbital", orbital_tracker())
        .with("harbour", harbour)
}

/// The herdr session with one live pane in each project's directory.
fn panes_across() -> Canned {
    Canned::default().herdr_holding(PANES_ACROSS)
}

fn emit_over(cfg: &Config, runner: &Canned, trackers: &Fakes, filter: Filter) -> Value {
    let snapshot = beady_eye::app::run(cfg, &Herdr::new(runner), trackers, filter, now());
    serde_json::to_value(&snapshot).expect("the snapshot serialises")
}

/// Both roots are called `orb-7`, so the project each tree was read from is
/// the only thing that tells the two apart.
#[test]
fn each_tree_carries_the_project_it_was_read_from() {
    let emitted = emit_over(
        &two_projects(),
        &panes_across(),
        &across_two_projects(),
        Filter::LiveAgents,
    );

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
    let emitted = emit_over(
        &two_projects(),
        &panes_across(),
        &across_two_projects(),
        Filter::LiveAgents,
    );
    let orbital = &emitted["trees"][0];
    let harbour = &emitted["trees"][1];

    assert_eq!(node(orbital, "orb-7.1")["title"], "re-point the dish");
    assert_eq!(node(harbour, "orb-7.1")["title"], "hire the dredger");

    assert_eq!(node(orbital, "orb-7.1")["blocked_by"], json!(["orb-9"]));
    assert_eq!(node(harbour, "orb-7.1")["blocked_by"], json!([]));

    assert_eq!(node(orbital, "orb-7")["agent"]["pane"]["id"], "w:p1");
    assert_eq!(node(harbour, "orb-7")["agent"]["pane"]["id"], "w:p5");
    assert_eq!(emitted["conflicts"], json!([]));
}

/// Degrade, never disappear, at project granularity: harbour's tracker
/// refusing the credential costs harbour's trees and nothing else.
#[test]
fn one_projects_tracker_failing_leaves_the_others_trees_standing() {
    let trackers = across_two_projects_with(
        Fake::holding(beads(HARBOUR_TREE)).failing(Asked::All, refused(FailureKind::Auth)),
    );

    let emitted = emit_over(
        &two_projects(),
        &panes_across(),
        &trackers,
        Filter::LiveAgents,
    );

    assert_eq!(
        emitted["failed_projects"],
        json!([{"project": "harbour", "tracker": "auth"}])
    );

    let trees = emitted["trees"].as_array().expect("trees is an array");
    assert_eq!(trees.len(), 1);
    assert_eq!(trees[0]["project"], "orbital");
    assert_eq!(node(&trees[0], "orb-7.1")["title"], "re-point the dish");
    assert_eq!(node(&trees[0], "orb-7")["agent"]["pane"]["id"], "w:p1");

    assert_eq!(
        emitted["unattributed"],
        json!([{"pane": {"session": "default", "id": "w:p5"}, "project": "harbour", "cwd": "/srv/work/harbour",
                "pane_status": "working", "display_agent": null,
                "title": "the channel"}]),
        "the pane in the failed project is still reported"
    );
}

/// bdi-9vm. Run from a git worktree, `bdi` has no config file to read and
/// synthesises one whose project path is the worktree, while every live pane
/// is in the checkout the worktree was cut from. Not one pane is under the
/// configured path, so nothing joins — and a claimed bead saying only that it
/// has no pane sends the reader looking for a dead agent that is sitting
/// right there.
#[test]
fn a_claim_whose_pane_is_under_no_configured_path_says_that_on_the_bead() {
    let elsewhere = Config::from_toml(&CONFIG.replace("/srv/work/orbital", "/srv/wt/orbital"))
        .expect("the config parses");

    let emitted = emit_over(&elsewhere, &panes(), &orbital(), Filter::All);

    let tree = &emitted["trees"][0];
    assert_eq!(node(tree, "orb-7")["agent"], json!(null));
    assert_eq!(
        node(tree, "orb-7")["anomalies"],
        json!([{
            "rule": "orphan-claim",
            "refused": {
                "conflict": "pane-in-another-project",
                "bead": {"project": "orbital", "id": "orb-7"},
                "pane": {"session": "default", "id": "w:p1"},
                "pane_project": null,
            },
        }]),
        "the bead named a live pane, so the reason it has none is the refusal"
    );
    assert_eq!(
        node(tree, "orb-7.3")["anomalies"][0],
        json!({"rule": "orphan-claim"}),
        "a claim that named no pane has no refusal to carry"
    );
}

/// A pane in `beacon`, the same session's name on a box running the default
/// beside it, and `persistent-agents` running and not answering.
const PANES_IN_BEACON: &str = r#"{"id":"cli:agent:list","result":{"agents":[
  {"pane_id":"w:p1","cwd":"/srv/work/orbital","agent_status":"working","title":"the dish"}
]}}"#;

/// A box running three sessions: the default holds nothing, `beacon` holds
/// the seat the bead names, and `persistent-agents` will not answer.
fn three_sessions() -> Canned {
    Canned::default()
        .answering("herdr --session default agent list", NO_PANES)
        .herdr_running(&[("beacon", Some(PANES_IN_BEACON)), ("persistent-agents", None)])
}

const NO_PANES: &str = r#"{"result":{"agents":[]}}"#;

/// Every session the provider runs is in the JSON with how it answered, so a
/// consumer can tell a seat that is not there from a session it could not
/// see into — and the seats of the sessions that answered are drawn, keyed
/// on their session.
#[test]
fn a_session_that_will_not_answer_is_named_and_the_others_seats_are_still_drawn() {
    let emitted = emit(&three_sessions(), &orbital(), Filter::LiveAgents);

    assert_eq!(
        emitted["agents"],
        json!({"provider": "herdr", "state": "answering",
               "sessions": [{"name": "default", "state": "answering"},
                            {"name": "beacon", "state": "answering"},
                            {"name": "persistent-agents", "state": "not-answering"}]})
    );
    assert_eq!(
        node(&emitted["trees"][0], "orb-7")["agent"]["pane"],
        json!({"session": "beacon", "id": "w:p1"}),
        "the bead's key names the pane by id, and the one session holding it is its"
    );
}

/// A bead's key names a pane id alone, and two sessions each hold one. The
/// bead gets neither, the disagreement names the sessions, and both panes are
/// still in the JSON, each under its own session.
#[test]
fn a_pane_id_two_sessions_hold_is_a_conflict_naming_the_sessions() {
    let held_twice = Canned::default()
        .answering("herdr --session default agent list", PANES_IN_BEACON)
        .herdr_running(&[("beacon", Some(PANES_IN_BEACON))]);

    let emitted = emit(&held_twice, &orbital(), Filter::LiveAgents);

    let tree = &emitted["trees"][0];
    assert_eq!(node(tree, "orb-7")["agent"], json!(null));
    assert_eq!(
        emitted["conflicts"],
        json!([{
            "conflict": "pane-id-in-several-sessions",
            "bead": {"project": "orbital", "id": "orb-7"},
            "pane_id": "w:p1",
            "sessions": ["default", "beacon"],
        }])
    );
    assert_eq!(
        node(tree, "orb-7")["anomalies"][0]["refused"]["conflict"],
        "pane-id-in-several-sessions"
    );
    let loose: Vec<&Value> = emitted["unattributed"]
        .as_array()
        .expect("unattributed is an array")
        .iter()
        .map(|pane| &pane["pane"])
        .collect();
    assert_eq!(
        loose,
        [
            &json!({"session": "default", "id": "w:p1"}),
            &json!({"session": "beacon", "id": "w:p1"}),
        ]
    );
}
