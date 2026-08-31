use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::config::Config;
use crate::model::anomaly::{self, Anomaly};
use crate::model::join::{self, AgentRef, Badged, BeadKey, Conflict, Joined};
use crate::model::tree::Assembled;
use crate::model::types::{Edge, Pane, PaneStatus, Status};

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
    fn unfinished(&self) -> usize {
        self.total - self.closed
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
    /// Ids whose declared parent was absent from the tracker's answer. Each is
    /// still in `nodes`, re-parented onto the root.
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
}

impl Snapshot {
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

    /// A tree we could not read has no agents to count, so the filter would
    /// hide it for the one reason it must not: an unreadable tracker and a
    /// tracker with no work would look the same.
    fn survives(&self, filter: Filter) -> bool {
        match filter {
            Filter::All => true,
            Filter::LiveAgents => self.counts.live_agents > 0 || self.tracker != TrackerState::Ok,
        }
    }
}

/// Draw one project's assembled rows as a tree, with the agents already
/// resolved across every project.
pub fn build_tree(
    project: &str,
    assembled: &Assembled,
    joined: &Joined,
    readiness: &Readiness,
    cfg: &Config,
    now: DateTime<Utc>,
) -> Tree {
    let nodes: Vec<Node> = assembled
        .rows
        .iter()
        .map(|placed| {
            let bead = &placed.bead;
            let key = BeadKey {
                project: project.to_string(),
                id: bead.id.clone(),
            };
            let agent = joined.agents.get(&key).cloned();
            let refused = joined.refused.get(&key);
            Node {
                id: bead.id.clone(),
                title: bead.title.clone(),
                status: bead.status.clone(),
                issue_type: bead.issue_type.clone(),
                priority: bead.priority,
                depth: placed.depth,
                edge: placed.edge.clone(),
                ready: readiness.ready.contains(&bead.id),
                blocked_by: readiness
                    .blocked_by
                    .get(&bead.id)
                    .cloned()
                    .unwrap_or_default(),
                started_at: bead.started_at,
                closed_at: bead.closed_at,
                badges: join::badges_for(bead, &cfg.badges),
                anomalies: anomaly::detect(bead, agent.as_ref(), refused, &cfg.anomalies, now),
                agent,
                truncated: bead.truncated,
            }
        })
        .collect();

    // A bead drawn once for every way down to it is still one bead, and every
    // count here is a count of beads: a header over the rows would name more
    // work than the tracker holds and send a reader hunting for copies.
    let mut counted = BTreeSet::new();
    let once: Vec<&Node> = nodes
        .iter()
        .filter(|node| counted.insert(node.id.clone()))
        .collect();
    let counts = Counts {
        total: once.len(),
        closed: once.iter().filter(|n| n.status.is_closed()).count(),
        live_agents: once.iter().filter(|n| n.agent.is_some()).count(),
        anomalies: once.iter().filter(|n| !n.anomalies.is_empty()).count(),
    };

    let root = nodes.first();
    Tree {
        project: project.to_string(),
        root: root.map(|n| n.id.clone()).unwrap_or_default(),
        title: root.map(|n| n.title.clone()).unwrap_or_default(),
        counts,
        tracker: TrackerState::Ok,
        nodes,
        dangling: assembled.dangling.clone(),
        cycles: assembled.cycles.clone(),
    }
}

/// A project's trees in the order a reader meets them.
///
/// A tracker that files loose beads in bulk gives hundreds of roots holding
/// one bead each — 530 of 562, measured on one such tracker — and none of
/// them may be dropped, so the order they come in is the whole of what the
/// reader has:
///
/// - a tree `bdi` could not read leads, for the reason `Tree::survives` shows
///   it whatever the filter says — a root buried under hundreds of others has
///   disappeared as surely as one that was dropped;
/// - then the staffed trees, so that showing every tree adds the rest of the
///   forest below what the reader was already looking at rather than
///   shuffling it;
/// - then the trees with the most unfinished work in them;
/// - then the root's id, so a redraw moves nothing.
///
/// Projects keep the order the config named them in and a project's trees
/// stay together, so this orders within each project's run rather than across
/// the forest.
fn in_flight_first(trees: &mut [Tree]) {
    for project in trees.chunk_by_mut(|a, b| a.project == b.project) {
        project.sort_by(|a, b| {
            (b.tracker != TrackerState::Ok)
                .cmp(&(a.tracker != TrackerState::Ok))
                .then(b.counts.live_agents.cmp(&a.counts.live_agents))
                .then(b.counts.unfinished().cmp(&a.counts.unfinished()))
                .then_with(|| a.root.cmp(&b.root))
        });
    }
}

/// Divide the collected trees into the ones the filter shows and the ones it
/// hides. With no herdr there are no panes, so there is no agent to filter on
/// and every tree renders.
fn partition(trees: &[Tree], herdr: HerdrState, filter: Filter) -> (Vec<Tree>, Vec<HiddenTree>) {
    let filter = match herdr {
        HerdrState::Ok => filter,
        HerdrState::Unavailable => Filter::All,
    };
    let (shown, hidden): (Vec<&Tree>, Vec<&Tree>) = trees.iter().partition(|t| t.survives(filter));

    (
        shown.into_iter().cloned().collect(),
        hidden
            .into_iter()
            .map(|t| HiddenTree {
                project: t.project.clone(),
                root: t.root.clone(),
                title: t.title.clone(),
                reason: "no-live-agent",
            })
            .collect(),
    )
}

/// Re-apply the filter to a snapshot already in hand. Which trees show is a
/// display choice over what was collected, so nothing is read again and the
/// answer is the one `build` would have given for that filter.
pub fn refilter(snapshot: &Snapshot, filter: Filter) -> Snapshot {
    let (trees, hidden_trees) = partition(&snapshot.collected, snapshot.herdr, filter);

    Snapshot {
        filter,
        trees,
        hidden_trees,
        ..snapshot.clone()
    }
}

/// Gather the trees into one snapshot, hiding what the filter hides and
/// reporting everything that belongs to no tree.
pub fn build(
    collected: Collected,
    panes: &[Pane],
    joined: &Joined,
    cfg: &Config,
    herdr: HerdrState,
    filter: Filter,
    now: DateTime<Utc>,
) -> Snapshot {
    let Collected {
        mut trees,
        failed_projects,
    } = collected;
    in_flight_first(&mut trees);
    let (shown, hidden) = partition(&trees, herdr, filter);

    let (mut unattributed, mut unconfigured) = (Vec::new(), Vec::new());
    for pane in join::unattributed(panes, joined) {
        let cwd = pane.cwd.display().to_string();
        match join::project_of(&pane.cwd, &cfg.projects) {
            Some(project) => unattributed.push(LoosePane {
                pane: pane.pane_id.clone(),
                project: project.name.clone(),
                cwd,
                pane_status: pane.agent_status.clone(),
            }),
            None => unconfigured.push(UnconfiguredPane {
                pane: pane.pane_id.clone(),
                cwd,
                pane_status: pane.agent_status.clone(),
            }),
        }
    }

    Snapshot {
        generated_at: now,
        herdr,
        filter,
        trees: shown,
        hidden_trees: hidden,
        failed_projects,
        unattributed,
        unconfigured,
        conflicts: joined.conflicts.clone(),
        collected: trees,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::bd::parse_beads;
    use crate::collect::herdr::parse_agent_list;
    use crate::model::join::{JoinSource, ProjectRows};
    use crate::model::tree::{assemble, Placed};
    use pretty_assertions::assert_eq;

    /// One project's tree as bd writes it. `orb-7.3` is claimed and long
    /// untouched with no pane; `orb-7.2` is closed with a pane still on it.
    const BEADS: &str = r#"[
      {"id":"orb-7","title":"lift the ground station","status":"in_progress","parent_id":"",
       "priority":1,"issue_type":"epic","updated_at":"2026-08-29T12:00:00Z",
       "started_at":"2026-08-20T09:00:00Z","metadata":{"agent_pane":"w:p1"}},
      {"id":"orb-7.1","title":"re-point the dish","status":"open","parent_id":"orb-7",
       "priority":2,"issue_type":"task","edge_from_parent":"parent-child",
       "metadata":{"blocked_on":"human"}},
      {"id":"orb-7.2","title":"survey the mast","status":"closed","parent_id":"orb-7",
       "priority":2,"issue_type":"task","closed_at":"2026-08-28T09:00:00Z"},
      {"id":"orb-7.3","title":"lay the feeder cable","status":"in_progress","parent_id":"orb-7",
       "priority":1,"issue_type":"task","updated_at":"2026-07-01T12:00:00Z"},
      {"id":"orb-7.4","title":"file the licence","status":"open","parent_id":"orb-7",
       "priority":3,"issue_type":"chore"}
    ]"#;

    /// `w:p9` is a session on no bead; `w:pF` is one outside every
    /// configured project.
    const PANES: &str = r#"{"result":{"agents":[
      {"pane_id":"w:p1","cwd":"/srv/work/orbital","agent_status":"working",
       "state_labels":{"working":"lifting the mast"}},
      {"pane_id":"w:p2","cwd":"/srv/work/orbital","agent_status":"idle",
       "display_agent":"orb-7.2","title":"survey"},
      {"pane_id":"w:p9","cwd":"/srv/work/orbital","agent_status":"blocked"},
      {"pane_id":"w:pF","cwd":"/srv/spike","agent_status":"idle"}
    ]}}"#;

    fn cfg() -> Config {
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

    fn now() -> DateTime<Utc> {
        "2026-08-30T12:00:00Z".parse().expect("the instant parses")
    }

    /// The root of a hand-written tree: the one row naming no parent, which
    /// is how `bd dep tree` marks it.
    fn root_row(beads: &[crate::model::types::Bead]) -> String {
        beads
            .iter()
            .find(|b| b.parent_id.is_none())
            .expect("a root row")
            .id
            .clone()
    }

    fn assembled(json: &str) -> Assembled {
        let beads = parse_beads(json).expect("the rows parse");
        let root = root_row(&beads);
        assemble(beads, &root).expect("the rows assemble")
    }

    fn panes(json: &str) -> Vec<Pane> {
        parse_agent_list(json).expect("the panes parse")
    }

    fn joined(rows: &[Placed], panes: &[Pane]) -> Joined {
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
    fn readiness() -> Readiness {
        Readiness {
            ready: BTreeSet::from(["orb-7.4".to_string()]),
            blocked_by: BTreeMap::from([(
                "orb-7.1".to_string(),
                vec!["orb-9".to_string(), "orb-7.3".to_string()],
            )]),
        }
    }

    fn tree() -> Tree {
        let assembled = assembled(BEADS);
        let panes = panes(PANES);
        let joined = joined(&assembled.rows, &panes);
        build_tree("orbital", &assembled, &joined, &readiness(), &cfg(), now())
    }

    fn node<'a>(tree: &'a Tree, id: &str) -> &'a Node {
        tree.nodes
            .iter()
            .find(|n| n.id == id)
            .unwrap_or_else(|| panic!("{id} is among the nodes"))
    }

    fn built(trees: Vec<Tree>, filter: Filter) -> Snapshot {
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

    fn snapshot(trees: Vec<Tree>) -> Snapshot {
        built(trees, Filter::LiveAgents)
    }

    /// A tree nobody is working in, told apart from its neighbours by its root.
    fn quiet(root: &str, title: &str) -> Tree {
        let mut t = tree();
        t.root = root.to_string();
        t.title = title.to_string();
        t.counts.live_agents = 0;
        t.nodes.iter_mut().for_each(|n| n.agent = None);
        t
    }

    /// A quiet tree that has something to report: a bead whose parent bd never
    /// returned, a bead blocked by its own forebear, and a subtree bd cut
    /// short.
    fn quiet_with_reports() -> Tree {
        let json = r#"[
          {"id":"orb-6","title":"the far side","status":"open","parent_id":""},
          {"id":"orb-6.2","title":"child of a bead bd did not return","status":"open",
           "parent_id":"orb-6.1"},
          {"id":"orb-6.3","title":"one","status":"open",
           "dependencies":[{"depends_on_id":"orb-6","type":"parent-child"},
                           {"depends_on_id":"orb-6","type":"blocks"}]},
          {"id":"orb-6.4","title":"two","status":"open","parent_id":"orb-6.3"},
          {"id":"orb-6.5","title":"cut short","status":"open","parent_id":"orb-6",
           "truncated":true}
        ]"#;
        build_tree(
            "orbital",
            &assembled(json),
            &Joined::default(),
            &Readiness::default(),
            &cfg(),
            now(),
        )
    }

    /// A quiet tree, a live one, and another quiet one: the interleaving a
    /// lifted filter has to put back.
    fn interleaved() -> Vec<Tree> {
        vec![quiet("orb-2", "quiet work"), tree(), quiet_with_reports()]
    }

    /// A tree with nothing in it but the numbers the order is made from.
    fn counted(project: &str, root: &str, live_agents: usize, total: usize, closed: usize) -> Tree {
        Tree {
            project: project.to_string(),
            root: root.to_string(),
            title: String::new(),
            counts: Counts {
                total,
                closed,
                live_agents,
                anomalies: 0,
            },
            tracker: TrackerState::Ok,
            nodes: Vec::new(),
            dangling: Vec::new(),
            cycles: Vec::new(),
        }
    }

    fn ordered(mut trees: Vec<Tree>) -> Vec<String> {
        in_flight_first(&mut trees);
        trees
            .into_iter()
            .map(|t| format!("{}:{}", t.project, t.root))
            .collect()
    }

    /// Which project comes first is the config's to say, so the order runs
    /// within a project rather than over the whole forest.
    #[test]
    fn a_projects_trees_stay_together_where_the_config_put_them() {
        assert_eq!(
            ordered(vec![
                counted("orbital", "orb-1", 0, 2, 0),
                counted("orbital", "orb-2", 0, 9, 0),
                counted("ferry", "fer-1", 1, 1, 0),
                counted("ferry", "fer-2", 0, 40, 0),
            ]),
            [
                "orbital:orb-2",
                "orbital:orb-1",
                "ferry:fer-1",
                "ferry:fer-2"
            ]
        );
    }

    /// A closed bead is a row, not work, so a long-finished effort does not
    /// outrank a small one still going.
    #[test]
    fn a_tree_of_finished_beads_does_not_outrank_a_smaller_one_still_going() {
        assert_eq!(
            ordered(vec![
                counted("orbital", "orb-done", 0, 90, 88),
                counted("orbital", "orb-going", 0, 5, 0),
            ]),
            ["orbital:orb-going", "orbital:orb-done"]
        );
    }

    /// A tracker that could not be read has no counts to sort on, and a root
    /// buried under hundreds of others has disappeared as surely as a dropped
    /// one.
    #[test]
    fn a_tree_bdi_could_not_read_leads_the_forest() {
        let mut trees = vec![
            counted("orbital", "orb-1", 1, 40, 0),
            Tree::tracker_unreachable("orbital", "orb-9", TrackerFailure::Auth),
        ];
        in_flight_first(&mut trees);

        assert_eq!(trees[0].root, "orb-9");
    }

    /// Nothing tells the beads filed in bulk apart, so the tail is at least
    /// the same tail on every redraw.
    #[test]
    fn trees_holding_the_same_work_come_in_id_order() {
        assert_eq!(
            ordered(vec![
                counted("orbital", "orb-c", 0, 1, 0),
                counted("orbital", "orb-a", 0, 1, 0),
                counted("orbital", "orb-b", 0, 1, 0),
            ]),
            ["orbital:orb-a", "orbital:orb-b", "orbital:orb-c"]
        );
    }

    #[test]
    fn the_nodes_are_flattened_in_render_order_with_their_depth() {
        let t = tree();
        let order: Vec<&str> = t.nodes.iter().map(|n| n.id.as_str()).collect();

        assert_eq!(
            order,
            ["orb-7", "orb-7.3", "orb-7.1", "orb-7.4", "orb-7.2"],
            "work in flight leads, finished work trails"
        );
        assert_eq!(t.nodes[0].depth, 0);
        assert_eq!(node(&t, "orb-7.3").depth, 1);
    }

    #[test]
    fn the_root_names_the_tree() {
        let t = tree();
        assert_eq!(t.project, "orbital");
        assert_eq!(t.root, "orb-7");
        assert_eq!(t.title, "lift the ground station");
        assert_eq!(t.tracker, TrackerState::Ok);
    }

    #[test]
    fn the_counts_come_from_the_nodes() {
        let t = tree();
        assert_eq!(
            t.counts,
            Counts {
                total: 5,
                closed: 1,
                live_agents: 2,
                anomalies: 2,
            }
        );
    }

    #[test]
    fn a_bead_firing_two_rules_is_counted_once() {
        let t = tree();
        assert_eq!(
            node(&t, "orb-7.3").anomalies,
            vec![
                Anomaly::OrphanClaim { refused: None },
                Anomaly::StaleClaim { days: 60 }
            ],
            "an old claim whose agent died is both"
        );
        assert_eq!(node(&t, "orb-7.2").anomalies, vec![Anomaly::StalePane]);
        assert_eq!(
            t.counts.anomalies, 2,
            "the count is beads to look at, not rules that fired"
        );
    }

    #[test]
    fn ready_comes_from_bd_rather_than_the_bead_s_status() {
        let t = tree();
        assert!(node(&t, "orb-7.4").ready);
        assert!(
            !node(&t, "orb-7.1").ready,
            "open is not ready; only bd knows which"
        );
        assert!(!node(&t, "orb-7").ready);
    }

    #[test]
    fn blocked_by_comes_from_bd_rather_than_the_tree() {
        let t = tree();
        assert_eq!(
            node(&t, "orb-7.1").blocked_by,
            ["orb-9", "orb-7.3"],
            "a blocker outside the tree is still a blocker"
        );
        assert!(node(&t, "orb-7.4").blocked_by.is_empty());
    }

    #[test]
    fn the_contract_fields_reach_the_node() {
        let t = tree();
        let root = node(&t, "orb-7");
        assert_eq!(root.status, Status::InProgress);
        assert_eq!(root.issue_type, "epic");
        assert_eq!(root.priority, 1);
        assert_eq!(
            root.started_at,
            Some("2026-08-20T09:00:00Z".parse().unwrap())
        );
        assert_eq!(root.closed_at, None);
        assert_eq!(root.edge, None);
        assert!(!root.truncated);

        let child = node(&t, "orb-7.1");
        assert_eq!(child.edge, Some(Edge::ParentChild));
        assert_eq!(
            node(&t, "orb-7.2").closed_at,
            Some("2026-08-28T09:00:00Z".parse().unwrap())
        );
    }

    #[test]
    fn the_badges_reach_the_node() {
        let t = tree();
        assert_eq!(
            node(&t, "orb-7.1").badges,
            vec![Badged {
                key: "blocked_on".to_string(),
                text: "⏸ waiting".to_string(),
            }]
        );
        assert!(node(&t, "orb-7").badges.is_empty());
    }

    #[test]
    fn the_agent_reaches_the_node_with_the_direction_that_resolved_it() {
        let t = tree();
        let claimed = node(&t, "orb-7")
            .agent
            .as_ref()
            .expect("the bead named a pane");
        assert_eq!(claimed.pane, "w:p1");
        assert_eq!(claimed.source, JoinSource::AgentPane);
        assert_eq!(claimed.title.as_deref(), Some("lifting the mast"));

        let inferred = node(&t, "orb-7.2")
            .agent
            .as_ref()
            .expect("the pane named the bead");
        assert_eq!(inferred.source, JoinSource::DisplayAgent);
        assert!(node(&t, "orb-7.1").agent.is_none());
    }

    #[test]
    fn an_agent_on_a_colliding_id_in_another_project_is_not_this_tree_s() {
        let a = assembled(BEADS);
        let p = panes(PANES);
        let mut j = joined(&a.rows, &p);
        j.agents.insert(
            BeadKey {
                project: "ferry".to_string(),
                id: "orb-7.4".to_string(),
            },
            AgentRef {
                pane: "w:pB".to_string(),
                pane_status: PaneStatus::Working,
                title: None,
                source: JoinSource::AgentPane,
            },
        );

        let t = build_tree("orbital", &a, &j, &readiness(), &cfg(), now());

        assert!(
            node(&t, "orb-7.4").agent.is_none(),
            "prefixes are per-tracker; an id alone does not name a bead"
        );
        assert_eq!(t.counts.live_agents, 2);
    }

    #[test]
    fn a_bead_whose_parent_is_absent_is_reported_and_kept() {
        let json = r#"[
          {"id":"orb-4","title":"root","status":"open","parent_id":""},
          {"id":"orb-4.2","title":"child of a bead bd did not return",
           "status":"open","parent_id":"orb-4.1"}
        ]"#;
        let a = assembled(json);
        let t = build_tree(
            "orbital",
            &a,
            &Joined::default(),
            &Readiness::default(),
            &cfg(),
            now(),
        );

        assert_eq!(t.dangling, ["orb-4.2"]);
        assert_eq!(t.counts.total, 2, "a reported bead is still drawn");
        assert_eq!(node(&t, "orb-4.2").depth, 1);
    }

    /// The nesting is the whole of what this tree says, so each row has to
    /// carry which kind of edge put it where it is. A copy reached because a
    /// bead blocks its forebear and a copy reached because it is that
    /// forebear's child are drawn the same way and mean different things.
    #[test]
    fn a_node_carries_the_kind_of_edge_its_copy_was_reached_by() {
        let json = r#"[
          {"id":"orb-9","title":"root","status":"open","parent_id":""},
          {"id":"orb-9.1","title":"waiting","status":"open",
           "dependencies":[{"depends_on_id":"orb-9","type":"parent-child"},
                           {"depends_on_id":"orb-9.2","type":"blocks"}]},
          {"id":"orb-9.2","title":"what it waits on","status":"closed",
           "parent_id":"orb-9"}
        ]"#;
        let t = build_tree(
            "orbital",
            &assembled(json),
            &Joined::default(),
            &Readiness::default(),
            &cfg(),
            now(),
        );

        let edges: Vec<(&str, Option<&Edge>)> = t
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), node.edge.as_ref()))
            .collect();

        assert_eq!(
            edges,
            vec![
                ("orb-9", None),
                ("orb-9.1", Some(&Edge::ParentChild)),
                ("orb-9.2", Some(&Edge::Blocks)),
                ("orb-9.2", Some(&Edge::ParentChild)),
            ]
        );
    }

    #[test]
    fn a_header_counts_beads_rather_than_the_rows_they_are_drawn_on() {
        // `orb-8.9` blocks both of the root's children, so it is drawn three
        // times. A header saying five would send a reader looking for a bead
        // that is not there.
        let json = r#"[
          {"id":"orb-8","title":"root","status":"open","parent_id":""},
          {"id":"orb-8.1","title":"one","status":"open",
           "dependencies":[{"depends_on_id":"orb-8","type":"parent-child"},
                           {"depends_on_id":"orb-8.9","type":"blocks"}]},
          {"id":"orb-8.2","title":"two","status":"open",
           "dependencies":[{"depends_on_id":"orb-8","type":"parent-child"},
                           {"depends_on_id":"orb-8.9","type":"blocks"}]},
          {"id":"orb-8.9","title":"what both wait on","status":"closed",
           "parent_id":"orb-8"}
        ]"#;
        let t = build_tree(
            "orbital",
            &assembled(json),
            &Joined::default(),
            &Readiness::default(),
            &cfg(),
            now(),
        );

        assert_eq!(t.nodes.len(), 6, "a copy per way down");
        assert_eq!(t.counts.total, 4);
        assert_eq!(t.counts.closed, 1);
    }

    #[test]
    fn a_cycle_is_reported_and_its_beads_kept() {
        // `orb-5.1` hangs under `orb-5` and is blocked by it, so each must
        // finish before the other.
        let json = r#"[
          {"id":"orb-5","title":"root","status":"open","parent_id":""},
          {"id":"orb-5.1","title":"one","status":"open",
           "dependencies":[{"depends_on_id":"orb-5","type":"parent-child"},
                           {"depends_on_id":"orb-5","type":"blocks"}]},
          {"id":"orb-5.2","title":"two","status":"open","parent_id":"orb-5.1"}
        ]"#;
        let a = assembled(json);
        let t = build_tree(
            "orbital",
            &a,
            &Joined::default(),
            &Readiness::default(),
            &cfg(),
            now(),
        );

        assert_eq!(t.cycles, ["orb-5"]);
        assert_eq!(t.counts.total, 3);
    }

    #[test]
    fn a_tree_with_no_live_agent_is_hidden_and_reported() {
        let mut quiet = tree();
        quiet.root = "orb-2".to_string();
        quiet.title = "quiet work".to_string();
        quiet.counts.live_agents = 0;
        quiet.nodes.iter_mut().for_each(|n| n.agent = None);

        let snap = snapshot(vec![tree(), quiet]);

        assert_eq!(snap.trees.len(), 1);
        assert_eq!(snap.trees[0].root, "orb-7");
        assert_eq!(
            snap.hidden_trees,
            vec![HiddenTree {
                project: "orbital".to_string(),
                root: "orb-2".to_string(),
                title: "quiet work".to_string(),
                reason: "no-live-agent",
            }],
            "a filtered tree is reported, never dropped"
        );
    }

    #[test]
    fn a_tree_whose_tracker_failed_is_never_hidden() {
        let broken = Tree::tracker_unreachable("ferry", "fry-3", TrackerFailure::Auth);
        let snap = snapshot(vec![broken]);

        assert_eq!(
            snap.trees.len(),
            1,
            "a tracker we could not read has no agents to count, so hiding it \
             would be indistinguishable from having no work"
        );
        assert!(snap.hidden_trees.is_empty());
    }

    #[test]
    fn without_herdr_nothing_is_filtered() {
        let mut quiet = tree();
        quiet.counts.live_agents = 0;
        quiet.nodes.iter_mut().for_each(|n| n.agent = None);

        let snap = build(
            Collected {
                trees: vec![quiet],
                ..Collected::default()
            },
            &[],
            &Joined::default(),
            &cfg(),
            HerdrState::Unavailable,
            Filter::LiveAgents,
            now(),
        );

        assert_eq!(
            snap.trees.len(),
            1,
            "with no panes there is no filter to apply"
        );
        assert!(snap.hidden_trees.is_empty());
        assert!(snap.unattributed.is_empty());
    }

    #[test]
    fn asking_for_every_tree_hides_none() {
        let mut quiet = tree();
        quiet.counts.live_agents = 0;
        quiet.nodes.iter_mut().for_each(|n| n.agent = None);

        let snap = build(
            Collected {
                trees: vec![quiet],
                ..Collected::default()
            },
            &panes(PANES),
            &Joined::default(),
            &cfg(),
            HerdrState::Ok,
            Filter::All,
            now(),
        );

        assert_eq!(snap.trees.len(), 1);
        assert!(snap.hidden_trees.is_empty());
    }

    #[test]
    fn a_pane_on_no_bead_lands_in_unattributed_with_its_project() {
        let snap = snapshot(vec![tree()]);

        assert_eq!(
            snap.unattributed,
            vec![LoosePane {
                pane: "w:p9".to_string(),
                project: "orbital".to_string(),
                cwd: "/srv/work/orbital".to_string(),
                pane_status: PaneStatus::Blocked,
            }],
            "a pane in a known project that no bead claims is unattributed"
        );
    }

    /// The two loose cases ask different things of the reader: one is an agent
    /// off the tracked work, the other is a project `bdi` was never told about.
    #[test]
    fn a_pane_under_no_configured_project_is_reported_apart_from_the_unattributed() {
        let snap = snapshot(vec![tree()]);

        assert_eq!(
            snap.unconfigured,
            vec![UnconfiguredPane {
                pane: "w:pF".to_string(),
                cwd: "/srv/spike".to_string(),
                pane_status: PaneStatus::Idle,
            }]
        );
        assert!(
            !snap.unattributed.iter().any(|p| p.pane == "w:pF"),
            "a pane is in one list or the other, never both"
        );
    }

    #[test]
    fn the_join_s_conflicts_reach_the_snapshot() {
        let disagreeing = r#"{"result":{"agents":[
          {"pane_id":"w:p1","cwd":"/srv/work/orbital","agent_status":"working"},
          {"pane_id":"w:p2","cwd":"/srv/work/orbital","agent_status":"idle",
           "display_agent":"orb-7"}
        ]}}"#;
        let a = assembled(BEADS);
        let p = panes(disagreeing);
        let j = joined(&a.rows, &p);

        let snap = build(
            Collected::default(),
            &p,
            &j,
            &cfg(),
            HerdrState::Ok,
            Filter::LiveAgents,
            now(),
        );

        assert_eq!(
            snap.conflicts,
            vec![Conflict::BeadAndPaneDisagree {
                bead: BeadKey {
                    project: "orbital".to_string(),
                    id: "orb-7".to_string(),
                },
                named_by_bead: "w:p1".to_string(),
                named_by_pane: "w:p2".to_string(),
            }],
            "a disagreement computed by the join must not stop at the model boundary"
        );
    }

    #[test]
    fn a_project_whose_tracker_failed_is_reported_apart_from_the_trees() {
        let failed = vec![
            FailedProject {
                project: "ferry".to_string(),
                tracker: TrackerFailure::Auth,
            },
            FailedProject {
                project: "orbital".to_string(),
                tracker: TrackerFailure::Unavailable,
            },
        ];

        let snap = build(
            Collected {
                trees: Vec::new(),
                failed_projects: failed.clone(),
            },
            &[],
            &Joined::default(),
            &cfg(),
            HerdrState::Ok,
            Filter::LiveAgents,
            now(),
        );

        assert_eq!(
            snap.failed_projects, failed,
            "two failures with no root must stay apart from one another"
        );
        assert!(snap.trees.is_empty());
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
    fn lifting_the_filter_gives_what_a_fresh_collection_would_have() {
        let filtered = built(interleaved(), Filter::LiveAgents);

        assert_eq!(
            refilter(&filtered, Filter::All),
            built(interleaved(), Filter::All),
            "a display change must not read differently from a collection"
        );
    }

    #[test]
    fn re_applying_the_filter_gives_what_a_fresh_collection_would_have() {
        let all = built(interleaved(), Filter::All);

        assert_eq!(
            refilter(&all, Filter::LiveAgents),
            built(interleaved(), Filter::LiveAgents)
        );
    }

    #[test]
    fn a_lifted_filter_puts_the_hidden_trees_back_among_the_shown_ones() {
        let lifted = refilter(&built(interleaved(), Filter::LiveAgents), Filter::All);

        let roots: Vec<&str> = lifted.trees.iter().map(|t| t.root.as_str()).collect();
        assert_eq!(
            roots,
            ["orb-7", "orb-6", "orb-2"],
            "the forest's own order, not the shown ones followed by the hidden ones"
        );
        assert!(lifted.hidden_trees.is_empty());
        assert_eq!(lifted.filter, Filter::All);
    }

    #[test]
    fn the_filter_goes_off_and_on_again_without_drift() {
        let filtered = built(interleaved(), Filter::LiveAgents);

        assert_eq!(
            refilter(&refilter(&filtered, Filter::All), Filter::LiveAgents),
            filtered
        );
    }

    #[test]
    fn a_hidden_tree_comes_back_whole() {
        let lifted = refilter(&built(interleaved(), Filter::LiveAgents), Filter::All);
        let back = lifted
            .trees
            .iter()
            .find(|t| t.root == "orb-6")
            .expect("the hidden tree is back");

        assert_eq!(
            back,
            &quiet_with_reports(),
            "what a filter hid it must be able to show again"
        );
        assert_eq!(back.dangling, ["orb-6.2"]);
        assert_eq!(back.cycles, ["orb-6"]);
        assert!(back.nodes.iter().any(|n| n.truncated));
    }

    #[test]
    fn what_belongs_to_no_tree_survives_a_refilter() {
        let mut before = built(interleaved(), Filter::LiveAgents);
        before.failed_projects = vec![FailedProject {
            project: "ferry".to_string(),
            tracker: TrackerFailure::Auth,
        }];
        before.conflicts = vec![Conflict::BeadAndPaneDisagree {
            bead: BeadKey {
                project: "orbital".to_string(),
                id: "orb-7".to_string(),
            },
            named_by_bead: "w:p1".to_string(),
            named_by_pane: "w:p2".to_string(),
        }];
        assert!(!before.unattributed.is_empty(), "there are panes to lose");

        let after = refilter(&refilter(&before, Filter::All), Filter::LiveAgents);

        assert_eq!(after.failed_projects, before.failed_projects);
        assert_eq!(after.unattributed, before.unattributed);
        assert_eq!(after.conflicts, before.conflicts);
        assert_eq!(
            after.generated_at, before.generated_at,
            "a refilter is not a new reading"
        );
    }

    #[test]
    fn a_tracker_that_could_not_be_read_is_never_hidden_by_a_refilter() {
        let broken = Tree::tracker_unreachable("ferry", "fry-3", TrackerFailure::Auth);

        let filtered = refilter(
            &built(vec![broken.clone()], Filter::All),
            Filter::LiveAgents,
        );

        assert_eq!(filtered.trees, vec![broken]);
        assert!(filtered.hidden_trees.is_empty());
    }

    #[test]
    fn without_herdr_a_refilter_hides_nothing() {
        let blind = build(
            Collected {
                trees: vec![quiet("orb-2", "quiet work")],
                ..Collected::default()
            },
            &[],
            &Joined::default(),
            &cfg(),
            HerdrState::Unavailable,
            Filter::All,
            now(),
        );

        let filtered = refilter(&blind, Filter::LiveAgents);

        assert_eq!(
            filtered.trees.len(),
            1,
            "with no panes there is no filter to apply"
        );
        assert!(filtered.hidden_trees.is_empty());
    }

    #[test]
    fn what_a_refilter_needs_is_not_part_of_the_contract() {
        let json: serde_json::Value =
            serde_json::to_value(snapshot(interleaved())).expect("the snapshot serialises");

        assert!(
            json.get("collected").is_none(),
            "the trees kept for a refilter are not emitted"
        );
        assert_eq!(json["trees"][0]["root"], "orb-7");
        assert_eq!(json["hidden_trees"][0]["root"], "orb-6");
    }
    // ---- finding a bead across trackers -------------------------------

    /// A second tracker whose ids collide with `orbital`'s, because bead
    /// prefixes are per-tracker and uncoordinated. The titles are what tells
    /// two beads of one id apart, and `frr-1` belongs to this project alone.
    fn ferry() -> Tree {
        let json = r#"[
          {"id":"orb-7","title":"berth the ferry","status":"open","parent_id":""},
          {"id":"orb-7.1","title":"paint the hull","status":"open","parent_id":"orb-7"},
          {"id":"frr-1","title":"lift the ramp","status":"open","parent_id":"orb-7"}
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
