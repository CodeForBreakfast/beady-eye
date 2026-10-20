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

mod canned;

use canned::{refused, Canned};

const ARKHAM_AND_DUNWICH: &str = r#"
[[projects]]
name = "arkham"
path = "/srv/work/arkham"

[[projects]]
name = "dunwich"
path = "/srv/work/dunwich"

[roots.explicit]
dunwich = ["dun-404"]
"#;

/// The archive's reopening waits on six beads and a closed one, and has a
/// child of its own that nothing waits on. One of the six is another child
/// of its own, one is open and waits on nothing, one is being worked, one waits on a pull request's
/// gate, one waits on a bead in dunwich's tracker, and one is deferred.
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
   "metadata":{"repo":"miskatonic/ledgers"}},
  {"id":"ark-7","title":"order the vellum","status":"deferred","priority":3,"issue_type":"task",
   "defer_until":"2026-11-01T09:00:00Z"},
  {"id":"ark-8","title":"sweep the porch","status":"closed","priority":3,"issue_type":"task"},
  {"id":"ark-9","title":"catalogue the maps","status":"open","priority":2,"issue_type":"task"}
]"#;

/// The oil the lamps wait on, which waits in turn on the barge.
const DUNWICH: &str = r#"[
  {"id":"dun-1","title":"deliver the oil","status":"open","priority":2,"issue_type":"task",
   "dependencies":[{"depends_on_id":"dun-2","type":"blocks"}]},
  {"id":"dun-2","title":"float the barge","status":"open","priority":2,"issue_type":"task"}
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
            Fake::holding(parse_beads(ARKHAM).expect("the rows parse"))
                .ready([
                    "ark-1.1", "ark-1.2", "ark-2", "ark-3", "ark-5", "ark-6", "ark-9",
                ])
                .blocked(
                    "ark-1",
                    &["ark-1.2", "ark-2", "ark-3", "ark-4", "ark-5", "ark-7"],
                )
                .blocked("ark-4", &["ark-6"]),
        )
        .with("dunwich", dunwich)
}

fn dunwich() -> Fake {
    Fake::holding(parse_beads(DUNWICH).expect("the rows parse"))
        .ready(["dun-2"])
        .blocked("dun-1", &["dun-2"])
}

/// The document a run naming `named` writes.
fn tree_of(named: &[&str]) -> Value {
    tree_over(named, trackers_beside(dunwich()), &Only::default(), false)
}

/// The document a run naming `named` writes over `trackers`, keeping the
/// beads `only` keeps, without descriptions where `brief`.
fn tree_over(named: &[&str], trackers: Fakes, only: &Only, brief: bool) -> Value {
    let named: Vec<String> = named.iter().map(|bead| bead.to_string()).collect();
    let cfg = Config::from_toml(ARKHAM_AND_DUNWICH)
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

    let mut every: Vec<String> = ["ready", "ready_gates", "waiting"]
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
