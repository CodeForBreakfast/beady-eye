//! `bdi --beads <bead>` writes the readiness of one bead's dependency tree:
//! the bead and everything it depends on, followed into other trackers and
//! never down a parent link. These drive the model through the run a
//! consumer sees.

use beady_eye::collect::bd::parse_beads;
use beady_eye::collect::herdr::Herdr;
use beady_eye::collect::run::FailureKind;
use beady_eye::collect::tracker::testing::{Asked, Fake, Fakes};
use beady_eye::config::Config;
use beady_eye::model::snapshot::{DependencyListing, Filter, Only};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use crate::canned::{refused, Canned};

const ARKHAM_AND_DUNWICH: &str = r#"
[[projects]]
name = "arkham"
path = "/srv/work/arkham"

[[projects]]
name = "dunwich"
path = "/srv/work/dunwich"

[roots.explicit]
dunwich = ["dun-404"]

[delivery]
outside_key = "borrowed"
outcome_label = "landed"
"#;

/// The archive's reopening waits on six beads and a closed one, and has a
/// child of its own that nothing waits on. One of the six is another child
/// of its own, one is open and waits on nothing, one is being worked, one waits on the gate
/// of another effort's pull request, one waits on a bead in dunwich's tracker, and one is
/// deferred.
/// The cataloguing is ready and nothing waits on it, so it is outside the
/// tree.
const ARKHAM: &str = r#"[
  {"id":"ark-1","title":"reopen the archive","status":"open","priority":1,"issue_type":"epic",
   "dependencies":[
     {"depends_on_id":"ark-2","type":"blocks"},
     {"depends_on_id":"ark-3","type":"blocks"},
     {"depends_on_id":"ark-4","type":"blocks"},
     {"depends_on_id":"ark-5","type":"blocks"},
     {"depends_on_id":"ark-7","type":"blocks"},
     {"depends_on_id":"ark-8","type":"blocks"},
     {"depends_on_id":"ark-1.2","type":"blocks"}]},
  {"id":"ark-1.1","title":"paint the reading room","status":"open","parent":"ark-1",
   "priority":2,"issue_type":"task",
   "dependencies":[{"depends_on_id":"ark-1","type":"parent-child"}]},
  {"id":"ark-1.2","title":"label the shelves","status":"open","parent":"ark-1",
   "priority":2,"issue_type":"task",
   "dependencies":[{"depends_on_id":"ark-1","type":"parent-child"}]},
  {"id":"ark-2","title":"air the stacks","status":"open","priority":2,"issue_type":"task",
   "description":"open every window in the stacks"},
  {"id":"ark-3","title":"mend the index","status":"in_progress","priority":2,"issue_type":"task",
   "metadata":{"agent_pane":"w:p4"}},
  {"id":"ark-4","title":"bind the ledgers","status":"open","priority":2,"issue_type":"task",
   "description":"sew each ledger into calf",
   "dependencies":[{"depends_on_id":"ark-6","type":"blocks"}]},
  {"id":"ark-5","title":"light the lamps","status":"open","priority":2,"issue_type":"task",
   "dependencies":[{"depends_on_id":"dun-1","type":"blocks"}]},
  {"id":"ark-6","title":"the ledgers' pull request has merged","status":"open","priority":2,
   "issue_type":"gate","await_type":"gh:pr","await_id":"12","description":"wait for the binding change",
   "metadata":{"repo":"miskatonic/ledgers","borrowed":true}},
  {"id":"ark-7","title":"order the vellum","status":"deferred","priority":3,"issue_type":"task",
   "defer_until":"2026-11-01T09:00:00Z"},
  {"id":"ark-8","title":"sweep the porch","status":"closed","priority":3,"issue_type":"task"},
  {"id":"ark-9","title":"catalogue the maps","status":"open","priority":2,"issue_type":"task"}
]"#;

/// The east wing's opening waits on nine beads, each delivered in its own
/// way. The doors' pull request is open. The windows wait on a molecule's
/// outcome step, which waits on its pull request's gate. The lamps' pull
/// request has merged, and so has the molecule the rugs waited on, whose
/// outcome step waited on a step that is no pull request. The brass
/// and the hall wait on another effort's pull requests, one open and one
/// merged, whose gates are marked outside. The ceiling's pull request has
/// merged, but it waits on the doors too. The airing waits on a timer gate.
/// The plaque's molecule ends in a gate on a workflow run, labelled as its
/// outcome step, and the run has passed.
const EAST_WING: &str = r#"[
  {"id":"ark-20","title":"open the east wing","status":"open","priority":1,"issue_type":"epic",
   "dependencies":[
     {"depends_on_id":"ark-21","type":"blocks"},
     {"depends_on_id":"ark-22","type":"blocks"},
     {"depends_on_id":"ark-23","type":"blocks"},
     {"depends_on_id":"ark-24","type":"blocks"},
     {"depends_on_id":"ark-25","type":"blocks"},
     {"depends_on_id":"ark-26","type":"blocks"},
     {"depends_on_id":"ark-27","type":"blocks"},
     {"depends_on_id":"ark-28","type":"blocks"},
     {"depends_on_id":"ark-29","type":"blocks"},
     {"depends_on_id":"ark-37","type":"blocks"},
     {"depends_on_id":"ark-38","type":"blocks"},
     {"depends_on_id":"ark-40","type":"blocks"},
     {"depends_on_id":"ark-41","type":"blocks"},
     {"depends_on_id":"ark-43","type":"blocks"}]},
  {"id":"ark-41","title":"oil the hinges","status":"open","priority":2,"issue_type":"task",
   "dependencies":[{"depends_on_id":"ark-42","type":"blocks"}]},
  {"id":"ark-42","title":"the hinges' pull request is approved","status":"closed","priority":2,
   "issue_type":"gate","await_type":"gh:pr","await_id":"35",
   "metadata":{"repo":"miskatonic/archive","awaits":"approved"}},
  {"id":"ark-43","title":"review the joinery","status":"open","priority":2,"issue_type":"task",
   "dependencies":[{"depends_on_id":"ark-44","type":"blocks"}]},
  {"id":"ark-44","title":"the joinery's pull request is ready for review","status":"open",
   "priority":2,"issue_type":"gate","await_type":"gh:pr","await_id":"36",
   "metadata":{"repo":"miskatonic/archive","awaits":"ready_for_review"}},
  {"id":"ark-40","title":"stoke the boiler","status":"open","priority":2,"issue_type":"task",
   "dependencies":[{"depends_on_id":"dun-3","type":"blocks"}]},
  {"id":"ark-37","title":"dust the shelves","status":"open","priority":2,"issue_type":"task",
   "dependencies":[{"depends_on_id":"ark-mol-1.3","type":"blocks"}]},
  {"id":"ark-38","title":"wax the floor","status":"open","priority":2,"issue_type":"task",
   "dependencies":[{"depends_on_id":"ark-39","type":"blocks"}]},
  {"id":"ark-39","title":"the floor's pull request has merged","status":"closed","priority":2,
   "issue_type":"gate","await_type":"gh:pr","await_id":"9","labels":["landed"],
   "metadata":{"repo":"miskatonic/foundry","borrowed":true}},
  {"id":"ark-27","title":"paint the ceiling","status":"open","priority":2,"issue_type":"task",
   "dependencies":[{"depends_on_id":"ark-34","type":"blocks"},
                   {"depends_on_id":"ark-21","type":"blocks"}]},
  {"id":"ark-28","title":"air the wing","status":"open","priority":2,"issue_type":"task",
   "dependencies":[{"depends_on_id":"ark-35","type":"blocks"}]},
  {"id":"ark-29","title":"mount the plaque","status":"open","priority":2,"issue_type":"task",
   "dependencies":[{"depends_on_id":"ark-36","type":"blocks"}]},
  {"id":"ark-34","title":"the ceiling's pull request has merged","status":"closed","priority":2,
   "issue_type":"gate","await_type":"gh:pr","await_id":"30",
   "metadata":{"repo":"miskatonic/archive"}},
  {"id":"ark-35","title":"a week has passed","status":"open","priority":2,
   "issue_type":"gate","await_type":"timer"},
  {"id":"ark-36","title":"the plaque's workflow run has passed","status":"closed","priority":2,
   "issue_type":"gate","await_type":"gh:run","await_id":"5150","labels":["landed"],
   "metadata":{"repo":"miskatonic/archive"}},
  {"id":"ark-21","title":"fit the doors","status":"open","priority":2,"issue_type":"task",
   "dependencies":[{"depends_on_id":"ark-30","type":"blocks"}]},
  {"id":"ark-22","title":"glaze the windows","status":"open","priority":2,"issue_type":"task",
   "dependencies":[{"depends_on_id":"ark-mol-2.2","type":"blocks"}]},
  {"id":"ark-23","title":"hang the lamps","status":"open","priority":2,"issue_type":"task",
   "dependencies":[{"depends_on_id":"ark-31","type":"blocks"}]},
  {"id":"ark-24","title":"lay the rugs","status":"open","priority":2,"issue_type":"task",
   "dependencies":[{"depends_on_id":"ark-mol-1.2","type":"blocks"}]},
  {"id":"ark-25","title":"polish the brass","status":"open","priority":2,"issue_type":"task",
   "dependencies":[{"depends_on_id":"ark-32","type":"blocks"}]},
  {"id":"ark-26","title":"sweep the hall","status":"open","priority":2,"issue_type":"task",
   "dependencies":[{"depends_on_id":"ark-33","type":"blocks"}]},
  {"id":"ark-30","title":"the doors' pull request has merged","status":"open","priority":2,
   "issue_type":"gate","await_type":"gh:pr","await_id":"31",
   "metadata":{"repo":"miskatonic/archive"}},
  {"id":"ark-31","title":"the lamps' pull request has merged","status":"closed","priority":2,
   "issue_type":"gate","await_type":"gh:pr","await_id":"29",
   "metadata":{"repo":"miskatonic/archive"}},
  {"id":"ark-32","title":"the brass's pull request has merged","status":"open","priority":2,
   "issue_type":"gate","await_type":"gh:pr","await_id":"7",
   "metadata":{"repo":"miskatonic/foundry","borrowed":true}},
  {"id":"ark-33","title":"the hall's pull request has merged","status":"closed","priority":2,
   "issue_type":"gate","await_type":"gh:pr","await_id":"8",
   "metadata":{"repo":"miskatonic/foundry","borrowed":true}},
  {"id":"ark-mol-1","title":"deliver the rugs","status":"closed","priority":2,"issue_type":"molecule"},
  {"id":"ark-mol-1.1","title":"the rugs' checks have passed","status":"closed","priority":2,
   "issue_type":"task","parent":"ark-mol-1",
   "dependencies":[{"depends_on_id":"ark-mol-1","type":"parent-child"}]},
  {"id":"ark-mol-1.2","title":"the rugs are down","status":"closed","priority":2,"issue_type":"task",
   "labels":["landed"],"parent":"ark-mol-1",
   "dependencies":[{"depends_on_id":"ark-mol-1","type":"parent-child"},
                   {"depends_on_id":"ark-mol-1.1","type":"blocks"}]},
  {"id":"ark-mol-1.3","title":"the shelves are dusted","status":"pinned","priority":2,
   "issue_type":"task","labels":["landed"],"parent":"ark-mol-1",
   "dependencies":[{"depends_on_id":"ark-mol-1","type":"parent-child"}]},
  {"id":"ark-mol-2","title":"deliver the windows","status":"open","priority":2,"issue_type":"molecule"},
  {"id":"ark-mol-2.1","title":"the windows' pull request has merged","status":"open","priority":2,
   "issue_type":"gate","await_type":"gh:pr","await_id":"33","parent":"ark-mol-2",
   "metadata":{"repo":"miskatonic/archive"},
   "dependencies":[{"depends_on_id":"ark-mol-2","type":"parent-child"}]},
  {"id":"ark-mol-2.2","title":"the windows are in","status":"open","priority":2,"issue_type":"task",
   "labels":["landed"],"parent":"ark-mol-2",
   "dependencies":[{"depends_on_id":"ark-mol-2","type":"parent-child"},
                   {"depends_on_id":"ark-mol-2.1","type":"blocks"}]}
]"#;

/// The oil the lamps wait on, which waits in turn on the barge.
const DUNWICH: &str = r#"[
  {"id":"dun-1","title":"deliver the oil","status":"open","priority":2,"issue_type":"task",
   "dependencies":[{"depends_on_id":"dun-2","type":"blocks"}]},
  {"id":"dun-2","title":"float the barge","status":"open","priority":2,"issue_type":"task"},
  {"id":"dun-3","title":"the coal's pull request has merged","status":"open","priority":2,
   "issue_type":"gate","await_type":"gh:pr","await_id":"12",
   "metadata":{"repo":"miskatonic/colliery"}}
]"#;

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-04T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

/// Arkham's tracker beside `dunwich`, each answering ready what bd would.
/// bd counts a bead being worked as ready, and cannot see that ark-5 waits
/// on dun-1, so it answers both ready.
fn trackers_beside(dunwich: Fake) -> Fakes {
    Fakes::default()
        .with(
            "arkham",
            Fake::holding(
                [ARKHAM, EAST_WING]
                    .iter()
                    .flat_map(|rows| parse_beads(rows).expect("the rows parse"))
                    .collect(),
            )
            .ready([
                "ark-1.1",
                "ark-1.2",
                "ark-2",
                "ark-3",
                "ark-5",
                "ark-6",
                "ark-9",
                "ark-23",
                "ark-24",
                "ark-26",
                "ark-29",
                "ark-30",
                "ark-32",
                "ark-35",
                "ark-37",
                "ark-38",
                "ark-40",
                "ark-41",
                "ark-44",
                "ark-mol-2.1",
            ])
            .blocked(
                "ark-1",
                &["ark-1.2", "ark-2", "ark-3", "ark-4", "ark-5", "ark-7"],
            )
            .blocked("ark-4", &["ark-6"])
            .blocked(
                "ark-20",
                &[
                    "ark-21", "ark-22", "ark-23", "ark-24", "ark-25", "ark-26", "ark-27", "ark-28",
                    "ark-29", "ark-37", "ark-38", "ark-40", "ark-41", "ark-43",
                ],
            )
            .blocked("ark-43", &["ark-44"])
            .blocked("ark-21", &["ark-30"])
            .blocked("ark-27", &["ark-21"])
            .blocked("ark-28", &["ark-35"])
            .blocked("ark-22", &["ark-mol-2.2"])
            .blocked("ark-25", &["ark-32"])
            .blocked("ark-mol-2.2", &["ark-mol-2.1"]),
        )
        .with("dunwich", dunwich)
}

fn dunwich() -> Fake {
    Fake::holding(parse_beads(DUNWICH).expect("the rows parse"))
        .ready(["dun-2", "dun-3"])
        .blocked("dun-1", &["dun-2"])
}

/// The document a run naming `named` writes.
fn tree_of(named: &[&str]) -> Value {
    tree_over(named, trackers_beside(dunwich()), &Only::default(), false)
}

/// The document a run naming `named` writes over `trackers`, keeping the
/// beads `only` keeps, without descriptions where `brief`.
fn tree_over(named: &[&str], trackers: Fakes, only: &Only, brief: bool) -> Value {
    tree_configured(ARKHAM_AND_DUNWICH, named, trackers, only, brief)
}

/// The document a run under `config` naming `named` writes over `trackers`.
fn tree_configured(
    config: &str,
    named: &[&str],
    trackers: Fakes,
    only: &Only,
    brief: bool,
) -> Value {
    let named: Vec<String> = named.iter().map(|bead| bead.to_string()).collect();
    let cfg = Config::from_toml(config)
        .expect("the config parses")
        .with_roots_named_on_the_command_line(&named)
        .expect("every bead names a project being read");
    let nobody = Canned::default().herdr_holding(r#"{"result":{"agents":[]}}"#);
    let snapshot = beady_eye::app::run(&cfg, &Herdr::new(&nobody), &trackers, Filter::All, now());
    serde_json::to_value(DependencyListing::of(
        &snapshot,
        &cfg.roots.named_beads(),
        only,
        brief,
        &cfg.delivery,
    ))
    .expect("the document serialises")
}

/// Each `project:id` in `list`, in the order the document gives them.
fn keys(document: &Value, list: &str) -> Vec<String> {
    document[list]
        .as_array()
        .unwrap_or_else(|| panic!("{list} is an array: {document}"))
        .iter()
        .map(|bead| {
            format!(
                "{}:{}",
                bead["project"].as_str().unwrap(),
                bead["id"].as_str().unwrap()
            )
        })
        .collect()
}

fn entry<'a>(document: &'a Value, list: &str, id: &str) -> &'a Value {
    document[list]
        .as_array()
        .unwrap_or_else(|| panic!("{list} is an array: {document}"))
        .iter()
        .find(|bead| bead["id"] == id)
        .unwrap_or_else(|| panic!("{id} is in {list}: {document}"))
}

/// The tree reaches into dunwich and down to its barge, and never down a
/// parent link alone, so it reaches the shelves the archive also waits on
/// and not the reading room. The cataloguing is ready but outside the tree,
/// and the swept porch is finished.
#[test]
fn the_tree_is_the_bead_and_everything_it_depends_on_in_any_tracker() {
    let document = tree_of(&["arkham:ark-1"]);

    let mut every: Vec<String> = [
        "ready",
        "ready_gates",
        "delivered",
        "in_delivery",
        "waiting",
    ]
    .iter()
    .flat_map(|list| keys(&document, list))
    .collect();
    every.sort();
    assert_eq!(
        every,
        [
            "arkham:ark-1",
            "arkham:ark-1.2",
            "arkham:ark-2",
            "arkham:ark-3",
            "arkham:ark-4",
            "arkham:ark-5",
            "arkham:ark-6",
            "arkham:ark-7",
            "dunwich:dun-1",
            "dunwich:dun-2",
        ]
    );
}

/// A ready bead is open with nothing unfinished beneath it, in its own
/// tracker or another. bd answers ark-5 ready, and the oil it waits on in
/// dunwich is not delivered, so it is not.
#[test]
fn the_ready_beads_are_open_with_every_dependency_finished_in_any_tracker() {
    let document = tree_of(&["arkham:ark-1"]);

    assert_eq!(
        keys(&document, "ready"),
        ["arkham:ark-1.2", "arkham:ark-2", "dunwich:dun-2"]
    );
}

/// A gate free to close is something to wait for rather than to start, so
/// it is listed apart, with what it waits on.
#[test]
fn a_ready_gate_is_listed_apart_from_the_ready_beads_with_what_it_awaits() {
    let document = tree_of(&["arkham:ark-1"]);

    assert_eq!(keys(&document, "ready_gates"), ["arkham:ark-6"]);
    let gate = entry(&document, "ready_gates", "ark-6");
    assert_eq!(
        (&gate["await_type"], &gate["await_id"], &gate["repo"]),
        (&json!("gh:pr"), &json!("12"), &json!("miskatonic/ledgers"))
    );
}

/// Every other bead says why it waits: the dependencies it waits on, each
/// with its project and status and, for a gate, what the gate awaits.
#[test]
fn a_waiting_bead_names_each_unmet_dependency_with_its_project_and_status() {
    let document = tree_of(&["arkham:ark-1"]);

    assert_eq!(
        entry(&document, "waiting", "ark-5")["unmet_dependencies"],
        json!([{"project": "dunwich", "id": "dun-1", "title": "deliver the oil",
                "status": "open", "issue_type": "task"}])
    );
    assert_eq!(
        entry(&document, "waiting", "ark-4")["unmet_dependencies"],
        json!([{"project": "arkham", "id": "ark-6",
                "title": "the ledgers' pull request has merged", "status": "open",
                "issue_type": "gate", "await_type": "gh:pr", "await_id": "12",
                "repo": "miskatonic/ledgers"}])
    );
    let unmet: Vec<&str> = entry(&document, "waiting", "ark-1")["unmet_dependencies"]
        .as_array()
        .expect("an array")
        .iter()
        .map(|dependency| dependency["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        unmet,
        ["ark-1.2", "ark-2", "ark-3", "ark-4", "ark-5", "ark-7"]
    );
}

/// A bead being worked or deferred waits on its status, which it carries,
/// with the date a deferral runs to.
#[test]
fn a_parked_bead_waits_on_its_status() {
    let document = tree_of(&["arkham:ark-1"]);

    let worked = entry(&document, "waiting", "ark-3");
    assert_eq!(worked["status"], "in_progress");
    assert_eq!(worked["unmet_dependencies"], json!([]));
    let deferred = entry(&document, "waiting", "ark-7");
    assert_eq!(
        (&deferred["status"], &deferred["defer_until"]),
        (&json!("deferred"), &json!("2026-11-01T09:00:00Z"))
    );
}

/// Each bead carries its metadata, so the key a setup names its seat's pane
/// under is on every node.
#[test]
fn each_bead_carries_the_pane_its_metadata_names() {
    let document = tree_of(&["arkham:ark-1"]);

    assert_eq!(
        entry(&document, "waiting", "ark-3")["metadata"]["agent_pane"],
        "w:p4"
    );
}

/// Dunwich's tracker refused its credential, so nothing says whether the oil
/// is delivered. The lamps wait on it as a blocker not read, rather than as
/// an unmet dependency.
#[test]
fn a_waiting_bead_names_a_blocker_in_a_tracker_that_could_not_be_read() {
    let document = tree_over(
        &["arkham:ark-1"],
        trackers_beside(dunwich().failing(Asked::All, refused(FailureKind::Auth))),
        &Only::default(),
        false,
    );

    let lamps = entry(&document, "waiting", "ark-5");
    assert_eq!(lamps["unmet_dependencies"], json!([]));
    assert_eq!(
        lamps["orphaned_dependencies"],
        json!([{"id": "dun-1", "reason": "not-read", "projects": ["dunwich"]}])
    );
}

/// A trim to tasks leaves the archive's epic and the ledgers' gate out of
/// every list. It chooses which beads are written and not what is written
/// of them, so the ledgers still wait on the gate.
#[test]
fn a_trim_keeps_only_the_beads_it_matches_in_each_list() {
    let tasks = Only {
        issue_type: Some("task".to_string()),
        ..Only::default()
    };
    let document = tree_over(&["arkham:ark-1"], trackers_beside(dunwich()), &tasks, false);

    assert_eq!(
        keys(&document, "ready"),
        ["arkham:ark-1.2", "arkham:ark-2", "dunwich:dun-2"]
    );
    assert_eq!(keys(&document, "ready_gates"), Vec::<String>::new());
    assert_eq!(
        keys(&document, "waiting"),
        [
            "arkham:ark-3",
            "arkham:ark-4",
            "arkham:ark-5",
            "arkham:ark-7",
            "dunwich:dun-1"
        ]
    );
    assert_eq!(
        entry(&document, "waiting", "ark-4")["unmet_dependencies"][0]["id"],
        "ark-6"
    );
}

/// As `bd ready --brief`: every entry in the three lists is written without
/// its description, and with everything else it says.
#[test]
fn brief_leaves_each_description_out_of_the_tree() {
    let full = tree_of(&["arkham:ark-1"]);
    let brief = tree_over(
        &["arkham:ark-1"],
        trackers_beside(dunwich()),
        &Only::default(),
        true,
    );

    assert_eq!(
        entry(&full, "ready", "ark-2")["description"],
        "open every window in the stacks"
    );
    assert_eq!(
        entry(&full, "ready_gates", "ark-6")["description"],
        "wait for the binding change"
    );
    assert_eq!(
        entry(&full, "waiting", "ark-4")["description"],
        "sew each ledger into calf"
    );
    assert_eq!(
        entry(&full, "ready", "ark-1.2")["parent"],
        json!({"project": "arkham", "id": "ark-1"})
    );
    for list in ["ready", "ready_gates", "waiting"] {
        let mut described = full[list].as_array().unwrap().clone();
        for bead in &mut described {
            bead.as_object_mut().unwrap().remove("description");
        }
        assert_eq!(brief[list], json!(described), "{list}");
    }
}

/// A root the config names and the tracker lacks is no part of the tree
/// asked about.
#[test]
fn a_root_outside_the_tree_that_went_unread_is_left_out() {
    let document = tree_of(&["arkham:ark-1"]);

    assert_eq!(document["unread_trees"], json!([]));
}

/// A bead the tracker does not hold has no tree, and says so rather than
/// writing an empty one.
#[test]
fn a_named_bead_the_tracker_lacks_is_reported_as_unread() {
    let document = tree_of(&["arkham:ark-404"]);

    assert_eq!(keys(&document, "waiting"), Vec::<String>::new());
    assert_eq!(
        document["unread_trees"],
        json!([{"project": "arkham", "root": "ark-404", "tracker": "root-not-found"}])
    );
}

/// Every bead of the east wing's tree under `config`, with the list the
/// document puts it in.
fn east_wing(config: &str) -> Vec<(String, String)> {
    let document = tree_configured(
        config,
        &["arkham:ark-20"],
        trackers_beside(dunwich()),
        &Only::default(),
        false,
    );
    let mut every: Vec<(String, String)> = [
        "ready",
        "ready_gates",
        "delivered",
        "in_delivery",
        "waiting",
    ]
    .iter()
    .flat_map(|list| {
        keys(&document, list)
            .into_iter()
            .map(move |key| (key, list.to_string()))
    })
    .collect();
    every.sort();
    every
}

fn listed(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(key, list)| (key.to_string(), list.to_string()))
        .collect()
}

/// A bead held by an open pull request's gate is in delivery, and says which
/// gate and which pull request. The gate itself is free to close, so it is a
/// ready gate.
#[test]
fn a_bead_held_by_an_open_pull_request_is_in_delivery() {
    let document = tree_of(&["arkham:ark-20"]);

    let doors = entry(&document, "in_delivery", "ark-21");
    assert_eq!(
        doors["pull_requests"],
        json!([{"project": "arkham", "gate": "ark-30", "repo": "miskatonic/archive", "number": "31"}])
    );
    assert_eq!(
        doors["unmet_dependencies"][0]["id"], "ark-30",
        "a bead in delivery still says what it waits on"
    );
    assert!(keys(&document, "ready_gates").contains(&"arkham:ark-30".to_string()));
}

/// A pull request names the project of its gate, since a gate in another
/// tracker can share its id with one in this.
#[test]
fn a_pull_request_names_the_tracker_its_gate_is_in() {
    let document = tree_of(&["arkham:ark-20"]);

    assert_eq!(
        entry(&document, "in_delivery", "ark-40")["pull_requests"],
        json!([{"project": "dunwich", "gate": "dun-3", "repo": "miskatonic/colliery", "number": "12"}])
    );
}

/// A bead waiting on a molecule's outcome step is in delivery through the
/// gate that step waits on. The step is the molecule's and not the work, so
/// it is not listed as in delivery itself.
#[test]
fn a_bead_waiting_on_an_outcome_step_is_in_delivery_and_the_step_is_not() {
    let document = tree_of(&["arkham:ark-20"]);

    assert_eq!(
        entry(&document, "in_delivery", "ark-22")["pull_requests"],
        json!([{"project": "arkham", "gate": "ark-mol-2.1", "repo": "miskatonic/archive", "number": "33"}])
    );
    assert!(keys(&document, "waiting").contains(&"arkham:ark-mol-2.2".to_string()));
}

/// A ready bead whose pull request has merged, or whose molecule's outcome
/// step has closed, is already delivered. It is to close rather than to
/// start, so it is not among the ready beads.
#[test]
fn a_ready_bead_whose_delivery_closed_is_delivered_and_not_ready() {
    let document = tree_of(&["arkham:ark-20"]);

    assert_eq!(
        keys(&document, "delivered"),
        ["arkham:ark-23", "arkham:ark-24", "arkham:ark-29"]
    );
    assert_eq!(
        entry(&document, "delivered", "ark-23")["pull_requests"],
        json!([{"project": "arkham", "gate": "ark-31", "repo": "miskatonic/archive", "number": "29"}])
    );
    assert_eq!(
        entry(&document, "delivered", "ark-24")["pull_requests"],
        json!([])
    );
    assert!(!keys(&document, "ready").contains(&"arkham:ark-23".to_string()));
}

/// A gate marked outside is another effort's pull request, so the brass waits
/// on it and the hall and the floor are ready once theirs close, even where
/// the gate is labelled an outcome. A merged pull request does not deliver
/// the ceiling while it waits on the doors, and a timer gate is no pull
/// request, so both wait. A pinned outcome step never closed, so the shelves
/// are ready rather than delivered. A gate awaiting a review milestone rather
/// than the merge delivers nothing, so the hinges are ready and the joinery's
/// review waits.
#[test]
fn only_a_pull_request_of_this_effort_delivers_a_bead() {
    assert_eq!(
        east_wing(ARKHAM_AND_DUNWICH),
        listed(&[
            ("arkham:ark-20", "waiting"),
            ("arkham:ark-21", "in_delivery"),
            ("arkham:ark-22", "in_delivery"),
            ("arkham:ark-23", "delivered"),
            ("arkham:ark-24", "delivered"),
            ("arkham:ark-25", "waiting"),
            ("arkham:ark-26", "ready"),
            ("arkham:ark-27", "waiting"),
            ("arkham:ark-28", "waiting"),
            ("arkham:ark-29", "delivered"),
            ("arkham:ark-30", "ready_gates"),
            ("arkham:ark-32", "ready_gates"),
            ("arkham:ark-35", "ready_gates"),
            ("arkham:ark-37", "ready"),
            ("arkham:ark-38", "ready"),
            ("arkham:ark-40", "in_delivery"),
            ("arkham:ark-41", "ready"),
            ("arkham:ark-43", "waiting"),
            ("arkham:ark-44", "ready_gates"),
            ("arkham:ark-mol-2.1", "ready_gates"),
            ("arkham:ark-mol-2.2", "waiting"),
            ("dunwich:dun-3", "ready_gates"),
        ])
    );
}

/// With no `[delivery]` section, no gate is outside and no step is an
/// outcome. Every pull request's gate delivers, and a bead behind a step
/// that is no gate waits on the step.
#[test]
fn without_a_delivery_section_every_gate_delivers_and_no_step_is_an_outcome() {
    let unconfigured = ARKHAM_AND_DUNWICH
        .split("[delivery]")
        .next()
        .expect("the config before its delivery section");

    assert_eq!(
        east_wing(unconfigured),
        listed(&[
            ("arkham:ark-20", "waiting"),
            ("arkham:ark-21", "in_delivery"),
            ("arkham:ark-22", "waiting"),
            ("arkham:ark-23", "delivered"),
            ("arkham:ark-24", "ready"),
            ("arkham:ark-25", "in_delivery"),
            ("arkham:ark-26", "delivered"),
            ("arkham:ark-27", "waiting"),
            ("arkham:ark-28", "waiting"),
            ("arkham:ark-29", "ready"),
            ("arkham:ark-30", "ready_gates"),
            ("arkham:ark-32", "ready_gates"),
            ("arkham:ark-35", "ready_gates"),
            ("arkham:ark-37", "ready"),
            ("arkham:ark-38", "delivered"),
            ("arkham:ark-40", "in_delivery"),
            ("arkham:ark-41", "ready"),
            ("arkham:ark-43", "waiting"),
            ("arkham:ark-44", "ready_gates"),
            ("arkham:ark-mol-2.1", "ready_gates"),
            ("arkham:ark-mol-2.2", "in_delivery"),
            ("dunwich:dun-3", "ready_gates"),
        ])
    );
}
