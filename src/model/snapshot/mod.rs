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
pub use filter::refilter;

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::model::anomaly::Anomaly;
use crate::model::badges::Badged;
use crate::model::join::{AgentRef, BeadKey, Conflict};
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Node {
    pub id: String,
    pub title: String,
    pub status: Status,
    pub issue_type: String,
    pub priority: u8,
    pub depth: u16,
    pub edge: Option<Edge>,
    /// Open, with every dependency satisfied. From `bd ready`, which is the
    /// only thing that knows.
    pub ready: bool,
    pub blocked_by: Vec<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
    pub badges: Vec<Badged>,
    pub agent: Option<AgentRef>,
    pub anomalies: Vec<Anomaly>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Tree {
    pub project: String,
    pub root: String,
    pub title: String,
    pub counts: Counts,
    pub tracker: TrackerState,
    pub nodes: Vec<Node>,
    /// Ids in `nodes` naming work the tracker's answer does not hold — most
    /// often a parent that was deleted. A bead this tree does not draw is not
    /// reported here, whatever its own dependencies are missing.
    pub dangling: Vec<String>,
    /// Ids whose own descendants lead back to them. Each is still in
    /// `nodes`, drawn where the loop was cut.
    pub cycles: Vec<String>,
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
    pub trees: Vec<Tree>,
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
    pub collected: Vec<Tree>,
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
}

impl Snapshot {
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
    /// nodes. Bead ids are unique only within a tracker, so both halves of
    /// the key are matched together here and neither is ever matched alone
    /// anywhere else.
    pub fn locate(&self, key: &BeadKey) -> Option<(&Tree, usize)> {
        self.trees
            .iter()
            .filter(|tree| tree.project == key.project)
            .find_map(|tree| {
                let at = tree.nodes.iter().position(|node| node.id == key.id)?;
                Some((tree, at))
            })
    }

    /// The bead a key names.
    pub fn node(&self, key: &BeadKey) -> Option<&Node> {
        self.locate(key).map(|(tree, at)| &tree.nodes[at])
    }
}

impl Tree {
    /// A root whose tracker refused to answer: known by project and id, with
    /// nothing to show beneath it.
    pub fn tracker_unreachable(project: &str, root: &str, failure: TrackerFailure) -> Self {
        Self {
            project: project.to_string(),
            root: root.to_string(),
            title: String::new(),
            counts: Counts::default(),
            tracker: TrackerState::Unreachable(failure),
            nodes: Vec::new(),
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
    use crate::model::tree::{assemble, Assembled, Placed};
    use crate::model::types::Pane;
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
    fn root_row(beads: &[crate::model::types::Bead]) -> String {
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
        assemble(beads, &root).expect("the rows assemble")
    }

    pub(super) fn panes(json: &str) -> Vec<Pane> {
        parse_agent_list(json).expect("the panes parse")
    }

    pub(super) fn joined(rows: &[Placed], panes: &[Pane]) -> Joined {
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
        let joined = joined(&assembled.rows, &panes);
        build_tree("orbital", &assembled, &joined, &readiness(), &cfg(), now())
    }

    pub(super) fn built(trees: Vec<Tree>, filter: Filter) -> Snapshot {
        let panes = panes(PANES);
        let joined = joined(&assembled(BEADS).rows, &panes);
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
