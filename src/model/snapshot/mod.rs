//! What one collection saw, as the view is handed it: the trees it drew,
//! and everything it could not.
//!
//! One module for how a snapshot is built from what the collectors returned,
//! and one for which of its trees the filter shows and in what order. What is
//! left here is the shape itself — what a snapshot holds, and what it can be
//! asked about what it holds.

mod build;
mod filter;

pub use build::{build, build_tree};

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

use crate::model::anomaly::Anomaly;
use crate::model::badges::Badged;
use crate::model::join::{AgentRef, BeadKey, Conflict};
use crate::model::tree::{self, Link};
use crate::model::types::{Edge, PaneStatus, Status};

/// Which tier `bdi` is reading: with no herdr there are no panes, so there is
/// no agent to join and no filter to apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HerdrState {
    Ok,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Filter {
    LiveAgents,
    All,
}

/// Why a tracker could not be read, in `bdi`'s own words rather than bd's:
/// bd names the database and the SQL user when it refuses a credential, so
/// nothing it wrote is carried this far.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrackerFailure {
    /// The tracker refused the credential it was given.
    Auth,
    /// The tracker did not answer.
    Unavailable,
    /// bd never ran.
    Exec,
    /// bd answered with something `bdi` cannot read.
    Parse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrackerState {
    Ok,
    Unreachable(TrackerFailure),
    /// The tracker answered, and its answer holds no bead of this id. Only a
    /// root named outright — in config or on the command line — can be here:
    /// a discovered root came out of the same tracker's answers.
    RootNotFound,
}

/// What bd knows about dependencies that a dep-tree row does not carry: a row
/// holds its tree parent, not its blocker set. Both are asked for and never
/// inferred.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Readiness {
    pub ready: BTreeSet<String>,
    pub blocked_by: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Counts {
    pub total: usize,
    pub closed: usize,
    pub live_agents: usize,
    /// Beads carrying at least one anomaly, not rules fired. Every other count
    /// here counts beads, and a header's warning count sends a reader looking
    /// for that many rows.
    pub anomalies: usize,
}

impl Counts {
    /// The beads a tree still holds a call on. A closed one is a row on the
    /// screen and nothing anybody has left to do.
    pub(crate) fn unfinished(&self) -> usize {
        self.total - self.closed
    }

    /// What these beads add up to, each counted once.
    ///
    /// A bead drawn once for every way down to it is still one bead, and every
    /// count here is a count of beads: a line over the rows would name more
    /// work than the tracker holds and send a reader hunting for copies. That
    /// holds over one tree's nodes and over a project's trees alike — a root's
    /// dangling children stand in every tree of its project, so a project line
    /// that added its trees' counts would name them once per tree.
    pub fn over<'a>(nodes: impl IntoIterator<Item = &'a Node>) -> Self {
        let mut counted = BTreeSet::new();
        let once: Vec<&Node> = nodes
            .into_iter()
            .filter(|node| counted.insert(node.id.clone()))
            .collect();
        Counts {
            total: once.len(),
            closed: once.iter().filter(|n| n.status.is_closed()).count(),
            live_agents: once.iter().filter(|n| n.agent.is_some()).count(),
            anomalies: once.iter().filter(|n| !n.anomalies.is_empty()).count(),
        }
    }
}

/// One bead as its tree draws it: what the tracker said of it, and what the
/// join and the rules added.
///
/// A bead is drawn once for every way down to it, and this is the bead and
/// not the copy: where a copy sits, and by which kind of edge, belongs to the
/// way down.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Node {
    pub id: String,
    pub title: String,
    pub status: Status,
    pub issue_type: String,
    pub priority: u8,
    /// Open, with every dependency satisfied. From `bd ready`, which is the
    /// only thing that knows.
    pub ready: bool,
    pub blocked_by: Vec<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
    pub badges: Vec<Badged>,
    pub agent: Option<AgentRef>,
    pub anomalies: Vec<Anomaly>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tree {
    pub project: String,
    pub root: String,
    pub title: String,
    pub counts: Counts,
    pub tracker: TrackerState,
    /// Every bead the tree draws, once each: the root first, then the rest in
    /// the order a walk down from it first reaches them.
    pub beads: Vec<Node>,
    /// The ways down from each of `beads` to the beads beneath it, in render
    /// order. A bead is drawn once for every way down to it, so the tree the
    /// screen shows is this unrolled from the root — and `--json` writes it
    /// that way, as `nodes`.
    pub children: Vec<Vec<Link>>,
    /// Ids in `beads` naming work the tracker's answer does not hold — most
    /// often a parent that was deleted. A bead this tree does not draw is not
    /// reported here, whatever its own dependencies are missing.
    pub dangling: Vec<String>,
    /// Ids whose own descendants lead back to them. Each is still in
    /// `beads`, drawn where the loop was cut.
    pub cycles: Vec<String>,
}

/// A tree is written the way it is drawn: `nodes` is the beads unrolled into
/// render order, one row for every way down to a bead, each carrying the
/// depth and the edge that way down gives it. The tree holds each bead once
/// and unrolls only here, because the unrolled shape can be very much larger
/// than the tree and `--json` is the one consumer that wants the whole of it.
impl Serialize for Tree {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut tree = serializer.serialize_struct("Tree", 8)?;
        tree.serialize_field("project", &self.project)?;
        tree.serialize_field("root", &self.root)?;
        tree.serialize_field("title", &self.title)?;
        tree.serialize_field("counts", &self.counts)?;
        tree.serialize_field("tracker", &self.tracker)?;
        tree.serialize_field("nodes", &self.unrolled())?;
        tree.serialize_field("dangling", &self.dangling)?;
        tree.serialize_field("cycles", &self.cycles)?;
        tree.end()
    }
}

/// One row of `nodes`: a bead at one of its places. Every field of the bead,
/// with `depth` and `edge` — the two that belong to the place rather than
/// the bead — where the contract puts them.
#[derive(Serialize)]
struct Drawn<'a> {
    id: &'a str,
    title: &'a str,
    status: &'a Status,
    issue_type: &'a str,
    priority: u8,
    depth: u16,
    edge: Option<Edge>,
    ready: bool,
    blocked_by: &'a [String],
    started_at: Option<DateTime<Utc>>,
    closed_at: Option<DateTime<Utc>>,
    badges: &'a [Badged],
    agent: Option<&'a AgentRef>,
    anomalies: &'a [Anomaly],
}

impl Tree {
    fn unrolled(&self) -> Vec<Drawn<'_>> {
        tree::unroll(&self.children)
            .into_iter()
            .map(|placed| {
                let node = &self.beads[placed.bead];
                Drawn {
                    id: &node.id,
                    title: &node.title,
                    status: &node.status,
                    issue_type: &node.issue_type,
                    priority: node.priority,
                    depth: placed.depth,
                    edge: placed.edge,
                    ready: node.ready,
                    blocked_by: &node.blocked_by,
                    started_at: node.started_at,
                    closed_at: node.closed_at,
                    badges: &node.badges,
                    agent: node.agent.as_ref(),
                    anomalies: &node.anomalies,
                }
            })
            .collect()
    }
}

/// A project whose tracker could not be read at all. It has no root and no
/// title, and several such failures must stay apart from one another, so an
/// anonymous empty tree cannot carry it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FailedProject {
    pub project: String,
    pub tracker: TrackerFailure,
}

/// One outcome per configured project: a tree that was read, or a tracker
/// that could not be.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Collected {
    pub trees: Vec<Tree>,
    pub failed_projects: Vec<FailedProject>,
    pub read_at: BTreeMap<String, DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HiddenTree {
    pub project: String,
    pub root: String,
    pub title: String,
    pub reason: &'static str,
    /// Whether the tree took findings out of the forest with it — a bead
    /// waiting on work the tracker never returned, or a loop. Hidden, the
    /// tree draws neither, so the group holding it admits to them instead.
    /// Not part of the JSON contract, whose `hidden_trees` rows say which
    /// trees were hidden and why.
    #[serde(skip)]
    pub findings: bool,
}

/// A live pane in a configured project that no bead in it claims.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LoosePane {
    pub pane: String,
    pub project: String,
    pub cwd: String,
    pub pane_status: PaneStatus,
}

/// A live pane whose directory sits under no `[[projects]]` entry. `bdi` has
/// not failed to attribute it; it has never been told the project exists, so
/// there was no tracker to look in. There is no project to name, and the
/// absence of the field is how a consumer tells this from a `LoosePane`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnconfiguredPane {
    pub pane: String,
    pub cwd: String,
    pub pane_status: PaneStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Snapshot {
    pub generated_at: DateTime<Utc>,
    pub herdr: HerdrState,
    pub filter: Filter,
    /// The trees the filter shows, each shared with `collected` rather than
    /// copied out of it: a filter is a display choice, and a shown tree is
    /// held once however many lists point at it.
    pub trees: Vec<Arc<Tree>>,
    pub hidden_trees: Vec<HiddenTree>,
    pub failed_projects: Vec<FailedProject>,
    pub unattributed: Vec<LoosePane>,
    /// Panes in directories no configured project covers. Apart from
    /// `unattributed` because the fix is a config entry, not a bead.
    pub unconfigured: Vec<UnconfiguredPane>,
    pub conflicts: Vec<Conflict>,
    /// Every tree that was read, in the order it was read, shown or hidden.
    /// `trees` and `hidden_trees` are how the current filter divides this, and
    /// keeping it is what lets `refilter` change the filter without asking the
    /// trackers again. Not part of the JSON contract.
    #[serde(skip)]
    pub collected: Vec<Arc<Tree>>,
    /// When each configured project's tracker was last read.
    ///
    /// Not `generated_at`, which is when the snapshot was drawn. A refresh
    /// naming one project draws every project, and the ones it did not read
    /// keep the rows of whatever read last touched them — so one time for the
    /// whole snapshot would date those rows to a read that never saw them.
    /// Not part of the JSON contract: `--json` is one whole collection and
    /// nothing else, so `generated_at` answers this for a consumer.
    #[serde(skip)]
    pub read_at: BTreeMap<String, DateTime<Utc>>,
    /// Every project the config names, in the order it names them —
    /// including the ones no collection has reached yet.
    ///
    /// The trees say which projects were read, and until one is read a
    /// project has no tree to be found in. So this is what the first frame of
    /// a run is drawn from: a forest of every project, each waiting on the
    /// collection that will fill it in. It is also what fixes the order the
    /// projects are drawn in, which the trees could only imply.
    ///
    /// Not part of the JSON contract. `--json` is one whole collection, and a
    /// project drawn before its collection returns is a thing only the view
    /// ever sees.
    #[serde(skip)]
    pub projects: Vec<String>,
}

impl Snapshot {
    /// The projects a run is about, before any of them has been read.
    ///
    /// What the first frame of a run is drawn from. Every other field is
    /// empty because nothing has answered yet, and `read_at` being empty is
    /// what says so: a project absent from it has not been read, as against
    /// read and found to hold nothing.
    ///
    /// The filter is carried in because `Forest::refresh` re-filters every
    /// snapshot that arrives with the filter the forest already holds, so the
    /// one this frame is built with is the one every later collection is
    /// shown through.
    ///
    /// herdr reads as `Ok` because nothing has asked it. `Unavailable` is a
    /// finding, and drawing it here would put a notice about herdr on the
    /// screen before `bdi` had spoken to herdr at all.
    pub fn awaiting(projects: Vec<String>, filter: Filter, now: DateTime<Utc>) -> Self {
        Snapshot {
            generated_at: now,
            herdr: HerdrState::Ok,
            filter,
            trees: Vec::new(),
            hidden_trees: Vec::new(),
            failed_projects: Vec::new(),
            unattributed: Vec::new(),
            unconfigured: Vec::new(),
            conflicts: Vec::new(),
            collected: Vec::new(),
            read_at: BTreeMap::new(),
            projects,
        }
    }

    /// Whether every root of a project answered the last time it was read.
    ///
    /// One answer for a project whose roots can disagree, and it is the worse
    /// of them: a project with one root unreachable and three fine is on the
    /// screen short of that root's rows, and saying the collection went well
    /// would be a claim about work nothing drew.
    ///
    /// A project with no tree at all reads as whole. Nothing refused — its
    /// tracker never answered for a root, which is a failed project and is
    /// reported as one.
    pub fn every_root_read(&self, project: &str) -> bool {
        self.trees
            .iter()
            .filter(|tree| tree.project == project)
            .all(|tree| tree.tracker == TrackerState::Ok)
    }

    /// Where a key sits: the tree holding it and its place among that tree's
    /// beads. Bead ids are unique only within a tracker, so both halves of
    /// the key are matched together here and neither is ever matched alone
    /// anywhere else.
    pub fn locate(&self, key: &BeadKey) -> Option<(&Tree, usize)> {
        self.trees
            .iter()
            .filter(|tree| tree.project == key.project)
            .find_map(|tree| {
                let at = tree.beads.iter().position(|node| node.id == key.id)?;
                Some((tree.as_ref(), at))
            })
    }

    /// The bead a key names.
    pub fn node(&self, key: &BeadKey) -> Option<&Node> {
        self.locate(key).map(|(tree, at)| &tree.beads[at])
    }
}

impl Tree {
    /// A root whose tracker refused to answer: known by project and id, with
    /// nothing to show beneath it.
    #[cfg(test)]
    pub fn tracker_unreachable(project: &str, root: &str, failure: TrackerFailure) -> Self {
        Self::unread(project, root, TrackerState::Unreachable(failure))
    }

    /// A root that drew no rows, with why beside it.
    pub fn unread(project: &str, root: &str, tracker: TrackerState) -> Self {
        Self {
            project: project.to_string(),
            root: root.to_string(),
            title: String::new(),
            counts: Counts::default(),
            tracker,
            beads: Vec::new(),
            children: Vec::new(),
            dangling: Vec::new(),
            cycles: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::bd::parse_beads;
    use crate::collect::herdr::parse_agent_list;
    use crate::config::Config;
    use crate::model::join::{self, Joined, ProjectRows};
    use crate::model::tree::{Assembled, Nesting};
    use crate::model::types::{Bead, Pane};
    use pretty_assertions::assert_eq;

    /// One project's tree as bd writes it. `orb-7.3` is claimed and long
    /// untouched with no pane; `orb-7.2` is closed with a pane still on it.
    pub(super) const BEADS: &str = r#"[
      {"id":"orb-7","title":"lift the ground station","status":"in_progress",
       "priority":1,"issue_type":"epic","updated_at":"2026-08-29T12:00:00Z",
       "started_at":"2026-08-20T09:00:00Z","metadata":{"agent_pane":"w:p1"}},
      {"id":"orb-7.1","title":"re-point the dish","status":"open",
       "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
       "priority":2,"issue_type":"task",
       "metadata":{"blocked_on":"human"}},
      {"id":"orb-7.2","title":"survey the mast","status":"closed",
       "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
       "priority":2,"issue_type":"task","closed_at":"2026-08-28T09:00:00Z"},
      {"id":"orb-7.3","title":"lay the feeder cable","status":"in_progress",
       "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
       "priority":1,"issue_type":"task","updated_at":"2026-07-01T12:00:00Z"},
      {"id":"orb-7.4","title":"file the licence","status":"open",
       "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
       "priority":3,"issue_type":"chore"}
    ]"#;

    /// `w:p9` is a session on no bead; `w:pF` is one outside every
    /// configured project.
    pub(super) const PANES: &str = r#"{"result":{"agents":[
      {"pane_id":"w:p1","cwd":"/srv/work/orbital","agent_status":"working",
       "state_labels":{"working":"lifting the mast"}},
      {"pane_id":"w:p2","cwd":"/srv/work/orbital","agent_status":"idle",
       "display_agent":"orb-7.2","title":"survey"},
      {"pane_id":"w:p9","cwd":"/srv/work/orbital","agent_status":"blocked"},
      {"pane_id":"w:pF","cwd":"/srv/spike","agent_status":"idle"}
    ]}}"#;

    pub(super) fn cfg() -> Config {
        Config::from_toml(
            r#"
[[projects]]
name = "orbital"
path = "/srv/work/orbital"
credential_command = "secret orbital"

[[projects]]
name = "ferry"
path = "/srv/work/ferry"
credential_command = "secret ferry"

[[badges]]
key = "blocked_on"
match = "human"
render = "⏸ waiting"
"#,
        )
        .expect("the config parses")
    }

    pub(super) fn now() -> DateTime<Utc> {
        "2026-08-30T12:00:00Z".parse().expect("the instant parses")
    }

    /// The root of a hand-written tree: the one row that depends on nothing.
    fn root_row(beads: &[Bead]) -> String {
        beads
            .iter()
            .find(|b| b.dependencies.is_empty())
            .expect("a root row")
            .id
            .clone()
    }

    pub(super) fn assembled(json: &str) -> Assembled {
        let beads = parse_beads(json).expect("the rows parse");
        let root = root_row(&beads);
        Nesting::of(&beads)
            .assemble(&root)
            .expect("the rows assemble")
    }

    pub(super) fn panes(json: &str) -> Vec<Pane> {
        parse_agent_list(json).expect("the panes parse")
    }

    pub(super) fn joined(rows: &[Bead], panes: &[Pane]) -> Joined {
        let cfg = cfg();
        join::resolve(
            &[ProjectRows {
                project: "orbital",
                rows,
            }],
            panes,
            &cfg.projects,
            &cfg.join,
        )
    }

    /// bd's answers about dependencies. `orb-7.1` is blocked by a bead that is
    /// not in this tree at all, and `orb-7.4` is ready while `orb-7.1` — the
    /// other open bead — is not.
    pub(super) fn readiness() -> Readiness {
        Readiness {
            ready: BTreeSet::from(["orb-7.4".to_string()]),
            blocked_by: BTreeMap::from([(
                "orb-7.1".to_string(),
                vec!["orb-9".to_string(), "orb-7.3".to_string()],
            )]),
        }
    }

    pub(super) fn tree() -> Tree {
        let assembled = assembled(BEADS);
        let panes = panes(PANES);
        let joined = joined(&assembled.beads, &panes);
        build_tree("orbital", &assembled, &joined, &readiness(), &cfg(), now())
    }

    pub(super) fn built(trees: Vec<Tree>, filter: Filter) -> Snapshot {
        let panes = panes(PANES);
        let joined = joined(&assembled(BEADS).beads, &panes);
        build(
            Collected {
                trees,
                ..Collected::default()
            },
            &panes,
            &joined,
            &cfg(),
            HerdrState::Ok,
            filter,
            now(),
        )
    }

    pub(super) fn snapshot(trees: Vec<Tree>) -> Snapshot {
        built(trees, Filter::LiveAgents)
    }

    #[test]
    fn the_snapshot_serialises() {
        let snap = snapshot(vec![tree()]);
        let json: serde_json::Value = serde_json::to_value(&snap).expect("the snapshot serialises");

        assert_eq!(json["generated_at"], "2026-08-30T12:00:00Z");
        assert_eq!(json["herdr"], "ok");
        assert_eq!(json["filter"], "live-agents");
        assert_eq!(json["trees"][0]["tracker"], "ok");
        assert_eq!(json["trees"][0]["counts"]["total"], 5);
        assert_eq!(json["trees"][0]["nodes"][0]["id"], "orb-7");
        assert_eq!(
            json["trees"][0]["nodes"][0]["agent"]["source"],
            "agent_pane"
        );
        assert_eq!(json["unattributed"][0]["project"], "orbital");
        assert!(
            json["trees"][0]["nodes"][1]["anomalies"].is_array(),
            "anomalies is a list"
        );
    }

    /// `nodes` is written by hand, field by field, and a field added to a
    /// bead and not to the row would leave the JSON silently. So the row is
    /// held to the bead: every field the bead writes, with the same value,
    /// and nothing else but the two that belong to the place.
    #[test]
    fn a_row_of_nodes_carries_every_field_of_its_bead_and_its_place() {
        let t = tree();
        let json = serde_json::to_value(&t).expect("the tree serialises");
        let bead = serde_json::to_value(&t.beads[1]).expect("the bead serialises");
        let bead = bead.as_object().expect("a bead is an object");

        let row = json["nodes"][1].as_object().expect("a row is an object");
        let mut expected: BTreeSet<&str> = bead.keys().map(String::as_str).collect();
        expected.extend(["depth", "edge"]);
        assert_eq!(
            row.keys().map(String::as_str).collect::<BTreeSet<_>>(),
            expected
        );
        for (key, value) in bead {
            assert_eq!(&row[key], value, "{key}");
        }
    }

    /// And in the contract's order, which a parsed value cannot show: the
    /// two fields of the place follow `priority`.
    #[test]
    fn a_row_of_nodes_puts_its_place_after_the_beads_priority() {
        let written = serde_json::to_string(&tree()).expect("the tree serialises");

        assert!(
            written.contains(r#""priority":1,"depth":1,"edge":"parent-child","ready":false"#),
            "{written}"
        );
    }

    #[test]
    fn an_unreachable_tracker_serialises_with_its_reason() {
        let snap = build(
            Collected {
                trees: vec![Tree::tracker_unreachable(
                    "ferry",
                    "fry-3",
                    TrackerFailure::Auth,
                )],
                failed_projects: vec![FailedProject {
                    project: "orbital".to_string(),
                    tracker: TrackerFailure::Parse,
                }],
                ..Default::default()
            },
            &[],
            &Joined::default(),
            &cfg(),
            HerdrState::Ok,
            Filter::LiveAgents,
            now(),
        );
        let json: serde_json::Value = serde_json::to_value(&snap).expect("the snapshot serialises");

        assert_eq!(json["trees"][0]["tracker"]["unreachable"], "auth");
        assert_eq!(json["failed_projects"][0]["tracker"], "parse");
        assert_eq!(json["trees"][0]["counts"]["total"], 0);
    }

    #[test]
    fn a_root_the_tracker_does_not_hold_serialises_as_such() {
        let snap = build(
            Collected {
                trees: vec![Tree::unread("ferry", "fry-3", TrackerState::RootNotFound)],
                ..Default::default()
            },
            &[],
            &Joined::default(),
            &cfg(),
            HerdrState::Ok,
            Filter::All,
            now(),
        );
        let json: serde_json::Value = serde_json::to_value(&snap).expect("the snapshot serialises");

        assert_eq!(json["trees"][0]["tracker"], "root-not-found");
        assert!(json["failed_projects"]
            .as_array()
            .is_some_and(Vec::is_empty));
    }

    // ---- finding a bead across trackers -------------------------------

    /// A second tracker whose ids collide with `orbital`'s, because bead
    /// prefixes are per-tracker and uncoordinated. The titles are what tells
    /// two beads of one id apart, and `frr-1` belongs to this project alone.
    fn ferry() -> Tree {
        let json = r#"[
          {"id":"orb-7","title":"berth the ferry","status":"open"},
          {"id":"orb-7.1","title":"paint the hull","status":"open",
           "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}]},
          {"id":"frr-1","title":"lift the ramp","status":"open",
           "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}]}
        ]"#;
        build_tree(
            "ferry",
            &assembled(json),
            &Joined::default(),
            &Readiness::default(),
            &cfg(),
            now(),
        )
    }

    fn key(project: &str, id: &str) -> BeadKey {
        BeadKey {
            project: project.to_string(),
            id: id.to_string(),
        }
    }

    #[test]
    fn one_id_in_two_trackers_answers_for_each_project_separately() {
        let snap = built(vec![tree(), ferry()], Filter::All);

        assert_eq!(
            snap.node(&key("orbital", "orb-7.1"))
                .map(|n| n.title.as_str()),
            Some("re-point the dish")
        );
        assert_eq!(
            snap.node(&key("ferry", "orb-7.1"))
                .map(|n| n.title.as_str()),
            Some("paint the hull")
        );
        assert_eq!(
            snap.locate(&key("ferry", "orb-7.1"))
                .map(|(t, _)| t.project.as_str()),
            Some("ferry")
        );
    }

    #[test]
    fn a_bead_in_one_project_is_not_found_through_another_projects_key() {
        let snap = built(vec![tree(), ferry()], Filter::All);

        assert_eq!(snap.node(&key("ferry", "orb-7.4")), None);
        assert_eq!(snap.node(&key("orbital", "frr-1")), None);
    }
}
