//! One project's rows drawn as a tree, and every project's trees gathered
//! into the snapshot, with the live panes that belong to none of them.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::config::Config;
use crate::model::anomaly;
use crate::model::badges;
use crate::model::edges::Relations;
use crate::model::join::{self, BeadKey, Conflict, Joined};
use crate::model::tree::Assembled;
use crate::model::types::{Pane, PaneKey};

use super::filter::{in_flight_first, partition};
use super::{
    AgentProvider, Collected, Counts, Filter, LoosePane, Node, ProviderState, Readiness, Snapshot,
    TrackerState, Tree, UnconfiguredPane,
};

/// Draw one project's assembled rows as a tree, with the agents already
/// resolved across every project.
///
/// `relations` is read for the whole answer rather than for this tree: what
/// a bead blocks is found on the beads that wait on it, and those can sit in
/// another tree. `agents` is how the run went for panes, which the anomaly
/// rules need: a pane the join did not award and a pane nothing was asked
/// about are different facts, and `joined` alone reads the same for both.
#[allow(
    clippy::too_many_arguments,
    reason = "each argument is a distinct thing one tree is drawn from. Two \
              that start travelling together — always passed as a pair, or \
              each derived from the other — are a concept, and naming it \
              drops the count below the threshold again."
)]
pub fn build_tree(
    project: &str,
    assembled: &Assembled,
    joined: &Joined,
    readiness: &Readiness,
    relations: &BTreeMap<String, Relations>,
    agents: ProviderState,
    cfg: &Config,
    now: DateTime<Utc>,
) -> Tree {
    let badges = cfg.badges_for_project(project);
    let beads: Vec<Node> = assembled
        .beads
        .iter()
        .map(|bead| {
            let key = BeadKey {
                project: project.to_string(),
                id: bead.id.clone(),
            };
            let agent = joined.agents.get(&key).cloned();
            let refused = joined.refused.get(&key);
            let out_of_reach = joined.out_of_reach.contains(&key);
            let tied = relations.get(&bead.id).cloned().unwrap_or_default();
            Node {
                id: bead.id.clone(),
                title: bead.title.clone(),
                status: bead.status.clone(),
                issue_type: bead.issue_type.clone(),
                priority: bead.priority,
                ready: readiness.ready.contains(&bead.id),
                blocked_by: readiness
                    .blocked_by
                    .get(&bead.id)
                    .cloned()
                    .unwrap_or_default(),
                started_at: bead.started_at,
                closed_at: bead.closed_at,
                badges: badges::badges_for(bead, &badges),
                anomalies: anomaly::detect(
                    bead,
                    agent.as_ref(),
                    refused,
                    agents,
                    out_of_reach,
                    &cfg.anomalies,
                    now,
                ),
                agent,
                description: bead.description.clone().unwrap_or_default(),
                notes: bead.notes.clone().unwrap_or_default(),
                owner: bead.owner.clone(),
                parent: tied.parent,
                depends_on: tied.depends_on,
                blocks: tied.blocks,
            }
        })
        .collect();

    let counts = Counts::over(&beads);

    let root = beads.first();
    Tree {
        project: project.to_string(),
        root: root.map(|n| n.id.clone()).unwrap_or_default(),
        title: root.map(|n| n.title.clone()).unwrap_or_default(),
        counts,
        tracker: TrackerState::Ok,
        beads,
        children: assembled.children.clone(),
        dangling: assembled.dangling.clone(),
        cycles: assembled.cycles.clone(),
    }
}

/// Gather the trees into one snapshot, hiding what the filter hides and
/// reporting everything that belongs to no tree.
pub fn build(
    collected: Collected,
    panes: &[Pane],
    joined: &Joined,
    cfg: &Config,
    agents: AgentProvider,
    filter: Filter,
    now: DateTime<Utc>,
) -> Snapshot {
    let Collected {
        mut trees,
        failed_projects,
        read_at,
    } = collected;
    in_flight_first(&mut trees);
    let trees: Vec<Arc<Tree>> = trees.into_iter().map(Arc::new).collect();
    let (shown, hidden) = partition(&trees, agents.state, filter);

    // Taken against the panes that came away with nothing rather than read
    // off the disagreements alone: several panes naming one bead leaves each
    // of them free to hold some other bead by the exact direction, and the
    // pane a bead's own key named wins outright over one that named it back.
    // Either would otherwise have a working pane's row saying its claim was
    // refused.
    let refused: HashSet<PaneKey> = joined
        .conflicts
        .iter()
        .flat_map(Conflict::refused_panes)
        .collect();

    let (mut unattributed, mut unconfigured) = (Vec::new(), Vec::new());
    for pane in join::unattributed(panes, joined) {
        let cwd = pane.cwd.display().to_string();
        match join::project_of(pane, &cfg.projects) {
            // A pane in a project this run left out is on another desktop's
            // work: neither drawn nor reported.
            Some(project) if !cfg.reads(&project.name) => {}
            Some(project) => unattributed.push(LoosePane {
                claim_refused: refused.contains(&pane.key()),
                pane: pane.key(),
                project: project.name.clone(),
                cwd,
                pane_status: pane.agent_status.clone(),
                display_agent: pane.display_agent.clone(),
                title: pane.caption().map(str::to_string),
            }),
            None => unconfigured.push(UnconfiguredPane {
                pane: pane.key(),
                cwd,
                pane_status: pane.agent_status.clone(),
            }),
        }
    }

    Snapshot {
        generated_at: now,
        agents,
        filter,
        trees: shown,
        hidden_trees: hidden,
        failed_projects,
        unattributed,
        unconfigured,
        conflicts: joined.conflicts.clone(),
        projects_named_without_git: cfg.projects_named_without_git(),
        collected: trees,
        read_at,
        projects: cfg.read().map(|p| p.name.clone()).collect(),
        scope: cfg.scope.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::bd::parse_beads;
    use crate::config::Scope;
    use crate::model::anomaly::Anomaly;
    use crate::model::badges::Badged;
    use crate::model::edges::relations;
    use crate::model::join::{AgentRef, BeadKey, Conflict, JoinSource, Listed};
    use crate::model::snapshot::tests::*;
    use crate::model::snapshot::{a_provider, ProviderState};
    use crate::model::snapshot::{FailedProject, TrackerFailure};
    use crate::model::tree::unroll;
    use crate::model::types::testing::key;
    use crate::model::types::{Edge, PaneStatus, Status};
    use pretty_assertions::assert_eq;
    use std::path::{Path, PathBuf};

    fn node<'a>(tree: &'a Tree, id: &str) -> &'a Node {
        tree.beads
            .iter()
            .find(|n| n.id == id)
            .unwrap_or_else(|| panic!("{id} is among the beads"))
    }

    /// The tree as it is drawn: one row per way down to a bead, with the
    /// depth and the edge that way down gives it.
    fn rows(tree: &Tree) -> Vec<(&str, u16, Option<Edge>)> {
        unroll(&tree.children)
            .into_iter()
            .map(|placed| {
                (
                    tree.beads[placed.bead].id.as_str(),
                    placed.depth,
                    placed.edge,
                )
            })
            .collect()
    }

    #[test]
    fn the_beads_are_held_in_render_order_and_unroll_with_their_depth() {
        let t = tree();
        let order: Vec<&str> = t.beads.iter().map(|n| n.id.as_str()).collect();

        assert_eq!(
            order,
            ["orb-7", "orb-7.3", "orb-7.1", "orb-7.4", "orb-7.2"],
            "work in flight leads, finished work trails"
        );
        assert_eq!(
            rows(&t),
            vec![
                ("orb-7", 0, None),
                ("orb-7.3", 1, Some(Edge::ParentChild)),
                ("orb-7.1", 1, Some(Edge::ParentChild)),
                ("orb-7.4", 1, Some(Edge::ParentChild)),
                ("orb-7.2", 1, Some(Edge::ParentChild)),
            ]
        );
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

    /// The same forest read on a machine with no agent provider: nothing
    /// answers for panes, so the join is handed none.
    fn read_without_a_provider(state: ProviderState) -> Snapshot {
        let assembled = assembled(BEADS);
        let joined = joined(&assembled.beads, &[]);
        let relations = relations(&assembled.beads);
        let tree = build_tree(
            "orbital",
            &assembled,
            &joined,
            &readiness(),
            &relations,
            state,
            &cfg(),
            now(),
        );
        build(
            Collected {
                trees: vec![tree],
                ..Collected::default()
            },
            &[],
            &joined,
            &cfg(),
            a_provider(state),
            Filter::All,
            now(),
        )
    }

    /// Every anomaly a run reported, against the bead it fired on.
    fn anomalies(snap: &Snapshot) -> Vec<(&str, &[Anomaly])> {
        snap.trees
            .iter()
            .flat_map(|t| t.beads.iter())
            .map(|n| (n.id.as_str(), n.anomalies.as_slice()))
            .filter(|(_, fired)| !fired.is_empty())
            .collect()
    }

    /// `orphan-claim` is the one rule that keys on a pane being *absent*, so
    /// it is the one a run with nothing to answer for panes turns into a lie:
    /// every claim in flight reads as an agent that died. `orb-7` is such a
    /// claim, and `bd` alone holds no fact against it — it is fresh, so even
    /// its age says nothing. The four rules that key on a pane being *there*
    /// fall silent on their own, and asserting the whole set is what keeps
    /// that true of the rule somebody adds next.
    #[test]
    fn with_no_provider_only_the_rule_bd_answers_alone_still_fires() {
        let snap = read_without_a_provider(ProviderState::Absent);

        assert_eq!(
            anomalies(&snap),
            vec![("orb-7.3", [Anomaly::StaleClaim { days: 60 }].as_slice())],
            "how long a claim has sat there is bd's own answer"
        );
        assert_eq!(snap.trees[0].counts.anomalies, 1);
        assert!(snap.unattributed.is_empty());
        assert!(snap.unconfigured.is_empty());
    }

    /// A provider that is installed and would not answer holds no more about
    /// panes than one that was never installed. Which of the two the reader
    /// is looking at is said in the foot and the tail band; the rules stay
    /// out of it.
    #[test]
    fn a_provider_that_would_not_answer_silences_the_rule_the_same_way() {
        let snap = read_without_a_provider(ProviderState::NotAnswering);

        assert_eq!(
            anomalies(&snap),
            vec![("orb-7.3", [Anomaly::StaleClaim { days: 60 }].as_slice())]
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
        assert_eq!(
            node(&t, "orb-7.2").closed_at,
            Some("2026-08-28T09:00:00Z".parse().unwrap())
        );
    }

    /// What `bd show` says of a bead, carried on the node so the screen can
    /// say it without asking the tracker again: the words the row holds, and
    /// what the answer ties the bead to — each with the status and title the
    /// answer gave it, including a bead in no tree of its own.
    #[test]
    fn the_bead_as_bd_show_gives_it_reaches_the_node() {
        let json = r#"[
          {"id":"orb-6","title":"root","status":"open","owner":"kim",
           "description":"lift the whole station","notes":"the crane is booked"},
          {"id":"orb-6.1","title":"waiting","status":"open",
           "dependencies":[{"depends_on_id":"orb-6","type":"parent-child"},
                           {"depends_on_id":"orb-6.2","type":"blocks"}]},
          {"id":"orb-6.2","title":"what it waits on","status":"closed",
           "dependencies":[{"depends_on_id":"orb-6","type":"parent-child"}]}
        ]"#;
        let beads = parse_beads(json).expect("the rows parse");
        let relations = relations(&beads);
        let t = build_tree(
            "orbital",
            &assembled(json),
            &Joined::default(),
            &Readiness::default(),
            &relations,
            ProviderState::Answering,
            &cfg(),
            now(),
        );

        let root = node(&t, "orb-6");
        assert_eq!(root.description, "lift the whole station");
        assert_eq!(root.notes, "the crane is booked");
        assert_eq!(root.owner.as_deref(), Some("kim"));
        assert_eq!(root.parent, None);
        assert_eq!(root.depends_on, vec![]);
        assert_eq!(root.blocks, vec![]);

        let waiting = node(&t, "orb-6.1");
        assert_eq!(waiting.description, "", "a row with nothing to say");
        assert_eq!(waiting.notes, "");
        assert_eq!(waiting.owner, None);
        assert_eq!(
            waiting.parent.as_ref().map(|p| p.id.as_str()),
            Some("orb-6")
        );
        assert_eq!(
            waiting
                .depends_on
                .iter()
                .map(|r| (r.id.as_str(), r.status.clone(), r.title.as_deref()))
                .collect::<Vec<_>>(),
            [("orb-6.2", Some(Status::Closed), Some("what it waits on"))]
        );

        let waited_on = node(&t, "orb-6.2");
        assert_eq!(
            waited_on
                .blocks
                .iter()
                .map(|r| (r.id.as_str(), r.status.clone(), r.title.as_deref()))
                .collect::<Vec<_>>(),
            [("orb-6.1", Some(Status::Open), Some("waiting"))]
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

    /// A project's own entry for a key is what its beads draw, and the global
    /// entry for that key never reaches the tree.
    #[test]
    fn a_projects_own_badge_reaches_its_nodes_in_place_of_the_global_one() {
        let assembled = assembled(BEADS);
        let panes = panes(PANES);
        let joined = joined(&assembled.beads, &panes);
        let relations = relations(&assembled.beads);
        let cfg = Config::from_toml(
            r#"
[[projects]]
name = "orbital"
path = "/srv/work/orbital"

[[projects.badges]]
key    = "blocked_on"
match  = "human"
render = "⏸ ask the ground station"

[[badges]]
key    = "blocked_on"
match  = "human"
render = "⏸ waiting"
"#,
        )
        .expect("the config parses");

        let t = build_tree(
            "orbital",
            &assembled,
            &joined,
            &readiness(),
            &relations,
            ProviderState::Answering,
            &cfg,
            now(),
        );

        assert_eq!(
            node(&t, "orb-7.1").badges,
            vec![Badged {
                key: "blocked_on".to_string(),
                text: "⏸ ask the ground station".to_string(),
            }]
        );
    }

    #[test]
    fn the_agent_reaches_the_node_with_the_direction_that_resolved_it() {
        let t = tree();
        let claimed = node(&t, "orb-7")
            .agent
            .as_ref()
            .expect("the bead named a pane");
        assert_eq!(claimed.pane, key("w:p1"));
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
        let mut j = joined(&a.beads, &p);
        j.agents.insert(
            BeadKey {
                project: "ferry".to_string(),
                id: "orb-7.4".to_string(),
            },
            AgentRef {
                pane: key("w:pB"),
                pane_status: PaneStatus::Working,
                title: None,
                source: JoinSource::AgentPane,
            },
        );

        let t = build_tree(
            "orbital",
            &a,
            &j,
            &readiness(),
            &BTreeMap::new(),
            ProviderState::Answering,
            &cfg(),
            now(),
        );

        assert!(
            node(&t, "orb-7.4").agent.is_none(),
            "prefixes are per-tracker; an id alone does not name a bead"
        );
        assert_eq!(t.counts.live_agents, 2);
    }

    #[test]
    fn a_bead_whose_parent_is_absent_is_reported_and_kept() {
        let json = r#"[
          {"id":"orb-4","title":"root","status":"open"},
          {"id":"orb-4.2","title":"waiting on a bead bd did not return","status":"open",
           "dependencies":[{"depends_on_id":"orb-4","type":"parent-child"},
                           {"depends_on_id":"orb-4.1","type":"blocks"}]}
        ]"#;
        let a = assembled(json);
        let t = build_tree(
            "orbital",
            &a,
            &Joined::default(),
            &Readiness::default(),
            &BTreeMap::new(),
            ProviderState::Answering,
            &cfg(),
            now(),
        );

        assert_eq!(t.dangling, ["orb-4.2"]);
        assert_eq!(t.counts.total, 2, "a reported bead is still drawn");
        assert_eq!(rows(&t)[1], ("orb-4.2", 1, Some(Edge::ParentChild)));
    }

    /// The nesting is the whole of what this tree says, so each row has to
    /// carry which kind of edge put it where it is. A copy reached because a
    /// bead blocks its forebear and a copy reached because it is that
    /// forebear's child are drawn the same way and mean different things.
    #[test]
    fn a_node_carries_the_kind_of_edge_its_copy_was_reached_by() {
        let json = r#"[
          {"id":"orb-9","title":"root","status":"open"},
          {"id":"orb-9.1","title":"waiting","status":"open",
           "dependencies":[{"depends_on_id":"orb-9","type":"parent-child"},
                           {"depends_on_id":"orb-9.2","type":"blocks"}]},
          {"id":"orb-9.2","title":"what it waits on","status":"closed",
           "dependencies":[{"depends_on_id":"orb-9","type":"parent-child"}]}
        ]"#;
        let t = build_tree(
            "orbital",
            &assembled(json),
            &Joined::default(),
            &Readiness::default(),
            &BTreeMap::new(),
            ProviderState::Answering,
            &cfg(),
            now(),
        );

        assert_eq!(
            rows(&t),
            vec![
                ("orb-9", 0, None),
                ("orb-9.1", 1, Some(Edge::ParentChild)),
                ("orb-9.2", 2, Some(Edge::Blocks)),
                ("orb-9.2", 1, Some(Edge::ParentChild)),
            ]
        );
    }

    #[test]
    fn a_header_counts_beads_rather_than_the_rows_they_are_drawn_on() {
        // `orb-8.9` blocks both of the root's children, so it is drawn three
        // times. A header saying five would send a reader looking for a bead
        // that is not there.
        let json = r#"[
          {"id":"orb-8","title":"root","status":"open"},
          {"id":"orb-8.1","title":"one","status":"open",
           "dependencies":[{"depends_on_id":"orb-8","type":"parent-child"},
                           {"depends_on_id":"orb-8.9","type":"blocks"}]},
          {"id":"orb-8.2","title":"two","status":"open",
           "dependencies":[{"depends_on_id":"orb-8","type":"parent-child"},
                           {"depends_on_id":"orb-8.9","type":"blocks"}]},
          {"id":"orb-8.9","title":"what both wait on","status":"closed",
           "dependencies":[{"depends_on_id":"orb-8","type":"parent-child"}]}
        ]"#;
        let t = build_tree(
            "orbital",
            &assembled(json),
            &Joined::default(),
            &Readiness::default(),
            &BTreeMap::new(),
            ProviderState::Answering,
            &cfg(),
            now(),
        );

        assert_eq!(rows(&t).len(), 6, "a copy per way down");
        assert_eq!(t.beads.len(), 4, "held once each");
        assert_eq!(t.counts.total, 4);
        assert_eq!(t.counts.closed, 1);
    }

    #[test]
    fn a_cycle_is_reported_and_its_beads_kept() {
        // `orb-5.1` hangs under `orb-5` and is blocked by it, so each must
        // finish before the other.
        let json = r#"[
          {"id":"orb-5","title":"root","status":"open"},
          {"id":"orb-5.1","title":"one","status":"open",
           "dependencies":[{"depends_on_id":"orb-5","type":"parent-child"},
                           {"depends_on_id":"orb-5","type":"blocks"}]},
          {"id":"orb-5.2","title":"two","status":"open",
           "dependencies":[{"depends_on_id":"orb-5.1","type":"parent-child"}]}
        ]"#;
        let a = assembled(json);
        let t = build_tree(
            "orbital",
            &a,
            &Joined::default(),
            &Readiness::default(),
            &BTreeMap::new(),
            ProviderState::Answering,
            &cfg(),
            now(),
        );

        assert_eq!(t.cycles, ["orb-5"]);
        assert_eq!(t.counts.total, 3);
    }

    #[test]
    fn a_pane_on_no_bead_lands_in_unattributed_with_its_project() {
        let snap = snapshot(vec![tree()]);

        assert_eq!(
            snap.unattributed,
            vec![LoosePane {
                pane: key("w:p9"),
                project: "orbital".to_string(),
                cwd: "/srv/work/orbital".to_string(),
                pane_status: PaneStatus::Blocked,
                display_agent: None,
                title: None,
                claim_refused: false,
            }],
            "a pane in a known project that no bead claims is unattributed"
        );
    }

    /// Two beads naming one pane, and beside it a pane nothing names at all.
    /// The join refuses the contested claim both ways, so both panes come
    /// away unattributed — and only one of them was refused anything.
    #[test]
    fn a_pane_whose_claim_the_join_refused_is_apart_from_one_nothing_claims() {
        let contested = r#"[
          {"id":"orb-1","title":"root","status":"open"},
          {"id":"orb-1.1","title":"one","status":"in_progress",
           "metadata":{"agent_pane":"w:p5"},
           "dependencies":[{"depends_on_id":"orb-1","type":"parent-child"}]},
          {"id":"orb-1.2","title":"two","status":"in_progress",
           "metadata":{"agent_pane":"w:p5"},
           "dependencies":[{"depends_on_id":"orb-1","type":"parent-child"}]}
        ]"#;
        let panes = panes(
            r#"{"result":{"agents":[
              {"pane_id":"w:p5","cwd":"/srv/work/orbital","agent_status":"working"},
              {"pane_id":"w:p9","cwd":"/srv/work/orbital","agent_status":"idle"}
            ]}}"#,
        );
        let assembled = assembled(contested);
        let joined = joined(&assembled.beads, &panes);
        let snap = build(
            Collected::default(),
            &panes,
            &joined,
            &cfg(),
            a_provider(ProviderState::Answering),
            Filter::All,
            now(),
        );

        assert_eq!(
            snap.unattributed
                .iter()
                .map(|loose| (loose.pane.id.as_str(), loose.claim_refused))
                .collect::<Vec<_>>(),
            vec![("w:p5", true), ("w:p9", false)]
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
                pane: key("w:pF"),
                cwd: "/srv/spike".to_string(),
                pane_status: PaneStatus::Idle,
            }]
        );
        assert!(
            !snap.unattributed.iter().any(|p| p.pane == key("w:pF")),
            "a pane is in one list or the other, never both"
        );
    }

    /// A pane under a project the scope left out is neither drawn nor
    /// reported: not loose, because it is on another desktop's work, and not
    /// unconfigured, because the config still names its project.
    #[test]
    fn a_pane_under_a_project_the_scope_left_out_is_neither_loose_nor_unconfigured() {
        let cfg = cfg()
            .scoped_to(&["orbital".to_string()])
            .expect("orbital is configured");
        let panes = panes(
            r#"{"result":{"agents":[
              {"pane_id":"w:p2","cwd":"/srv/work/ferry/src","agent_status":"idle"}
            ]}}"#,
        );
        let joined = join::resolve(&[], Listed::all(&panes), &cfg);

        let snap = build(
            Collected {
                trees: Vec::new(),
                failed_projects: Vec::new(),
                read_at: std::collections::BTreeMap::new(),
            },
            &panes,
            &joined,
            &cfg,
            a_provider(ProviderState::Answering),
            Filter::All,
            now(),
        );

        assert_eq!(snap.unattributed, vec![]);
        assert_eq!(snap.unconfigured, vec![]);
    }

    /// A pane in a linked worktree of the excluded project, placed outside
    /// that project's path: as it stands it is held by nothing, because an
    /// excluded project is never asked where it is worked. Where its
    /// directory sits in the main working tree is what places it, and a
    /// pane the main tree places in the excluded project is on another
    /// desktop's work like any other pane there.
    fn a_pane_in_a_linked_worktree_of_ferry() -> Vec<Pane> {
        let mut live = panes(
            r#"{"result":{"agents":[
              {"pane_id":"w:p2","cwd":"/tmp/seat-a/wt/src","agent_status":"idle"}
            ]}}"#,
        );
        let pane = live
            .remove(0)
            .with_cwd_in_the_main_working_tree(Some(PathBuf::from("/srv/work/ferry/src")));
        vec![pane]
    }

    fn built_over(panes: &[Pane], cfg: &Config) -> Snapshot {
        let joined = join::resolve(&[], Listed::all(panes), cfg);
        build(
            Collected::default(),
            panes,
            &joined,
            cfg,
            a_provider(ProviderState::Answering),
            Filter::All,
            now(),
        )
    }

    #[test]
    fn a_pane_in_a_linked_worktree_of_an_excluded_project_is_neither_loose_nor_unconfigured() {
        let cfg = cfg()
            .scoped_to(&["orbital".to_string()])
            .expect("orbital is configured");

        let snap = built_over(&a_pane_in_a_linked_worktree_of_ferry(), &cfg);

        assert_eq!(snap.unattributed, vec![]);
        assert_eq!(snap.unconfigured, vec![]);
    }

    /// The same pane under a run that reads its project is loose in that
    /// project, which is what tells placing it from dropping it.
    #[test]
    fn a_pane_in_a_linked_worktree_of_a_read_project_is_loose_in_that_project() {
        let snap = built_over(&a_pane_in_a_linked_worktree_of_ferry(), &cfg());

        assert_eq!(
            snap.unattributed,
            vec![LoosePane {
                pane: key("w:p2"),
                project: "ferry".to_string(),
                cwd: "/tmp/seat-a/wt/src".to_string(),
                pane_status: PaneStatus::Idle,
                display_agent: None,
                title: None,
                claim_refused: false,
            }],
            "reported where it is, placed where its main working tree is"
        );
        assert_eq!(snap.unconfigured, vec![]);
    }

    /// The view says on the screen when the directory chose the scope, and
    /// the snapshot is the only thing the view reads.
    #[test]
    fn the_scope_reaches_the_snapshot() {
        let cfg = cfg().scoped_to_the_project_holding(Path::new("/srv/work/ferry/src"));
        let joined = join::resolve(&[], Listed::all(&[]), &cfg);

        let snap = build(
            Collected::default(),
            &[],
            &joined,
            &cfg,
            a_provider(ProviderState::Answering),
            Filter::All,
            now(),
        );

        assert_eq!(
            snap.scope,
            Scope::Directory {
                project: "ferry".to_string(),
                widened: Vec::new(),
            }
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
        let j = joined(&a.beads, &p);

        let snap = build(
            Collected::default(),
            &p,
            &j,
            &cfg(),
            a_provider(ProviderState::Answering),
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
                named_by_bead: key("w:p1"),
                named_by_pane: key("w:p2"),
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
                ..Default::default()
            },
            &[],
            &Joined::default(),
            &cfg(),
            a_provider(ProviderState::Answering),
            Filter::LiveAgents,
            now(),
        );

        assert_eq!(
            snap.failed_projects, failed,
            "two failures with no root must stay apart from one another"
        );
        assert!(snap.trees.is_empty());
    }

    /// What herdr reported about a loose pane reaches the snapshot with it.
    /// `wG:p6` in the capture stamped a `display_agent`, a title and a label
    /// per state; the caption is the label for the state it is in, by the
    /// rule a bead's agent already gets.
    #[test]
    fn a_loose_pane_carries_what_herdr_reported_about_it() {
        let cfg = Config::from_toml(
            r#"
[[projects]]
name = "orbital"
path = "/tmp/bdi-ground/orbital"
"#,
        )
        .expect("the config parses");
        let p = panes(include_str!(
            "../../../tests/fixtures/herdr_agent_list.json"
        ));
        let j = join::resolve(&[], Listed::all(&p), &cfg);

        let snap = build(
            Collected::default(),
            &p,
            &j,
            &cfg,
            a_provider(ProviderState::Answering),
            Filter::All,
            now(),
        );

        let p6 = snap
            .unattributed
            .iter()
            .find(|p| p.pane.id == "wG:p6")
            .expect("in orbital's directory and on no bead");
        assert_eq!(p6.display_agent.as_deref(), Some("orb-2kd.5"));
        assert_eq!(
            p6.title.as_deref(),
            Some("writing the parser and its tests")
        );
    }
}
