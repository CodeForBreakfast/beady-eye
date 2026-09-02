//! One project's rows drawn as a tree, and every project's trees gathered
//! into the snapshot, with the live panes that belong to none of them.

use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::config::Config;
use crate::model::anomaly;
use crate::model::badges;
use crate::model::join::{self, BeadKey, Joined};
use crate::model::tree::Assembled;
use crate::model::types::Pane;

use super::filter::{in_flight_first, partition};
use super::{
    Collected, Counts, Filter, HerdrState, LoosePane, Node, Readiness, Snapshot, TrackerState,
    Tree, UnconfiguredPane,
};

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
                badges: badges::badges_for(bead, &cfg.badges),
                anomalies: anomaly::detect(bead, agent.as_ref(), refused, &cfg.anomalies, now),
                agent,
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
    herdr: HerdrState,
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
    let (shown, hidden) = partition(&trees, herdr, filter);

    let (mut unattributed, mut unconfigured) = (Vec::new(), Vec::new());
    for pane in join::unattributed(panes, joined) {
        let cwd = pane.cwd.display().to_string();
        match join::project_of(&pane.cwd, &cfg.projects) {
            // A pane in a project this run left out is on another desktop's
            // work: neither drawn nor reported.
            Some(project) if !cfg.reads(&project.name) => {}
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
        read_at,
        projects: cfg.read().map(|p| p.name.clone()).collect(),
        scope: cfg.scope.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Scope;
    use crate::model::anomaly::Anomaly;
    use crate::model::badges::Badged;
    use crate::model::join::{AgentRef, BeadKey, Conflict, JoinSource};
    use crate::model::snapshot::tests::*;
    use crate::model::snapshot::{FailedProject, TrackerFailure};
    use crate::model::tree::unroll;
    use crate::model::types::{Edge, PaneStatus, Status};
    use pretty_assertions::assert_eq;
    use std::path::Path;

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
        let mut j = joined(&a.beads, &p);
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
        let joined = join::resolve(&[], &panes, &cfg);

        let snap = build(
            Collected {
                trees: Vec::new(),
                failed_projects: Vec::new(),
                read_at: std::collections::BTreeMap::new(),
            },
            &panes,
            &joined,
            &cfg,
            HerdrState::Ok,
            Filter::All,
            now(),
        );

        assert_eq!(snap.unattributed, vec![]);
        assert_eq!(snap.unconfigured, vec![]);
    }

    /// The view says on the screen when the directory chose the scope, and
    /// the snapshot is the only thing the view reads.
    #[test]
    fn the_scope_reaches_the_snapshot() {
        let cfg = cfg().scoped_to_the_project_holding(Path::new("/srv/work/ferry/src"));
        let joined = join::resolve(&[], &[], &cfg);

        let snap = build(
            Collected::default(),
            &[],
            &joined,
            &cfg,
            HerdrState::Ok,
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
                ..Default::default()
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
}
