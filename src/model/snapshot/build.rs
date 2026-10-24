//! One project's rows drawn as a tree, and every project's trees gathered
//! into the snapshot, with the live panes that belong to none of them.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::config::{Badge, Config};
use crate::model::anomaly;
use crate::model::badges;
use crate::model::edges::{Elsewhere, Named, Related, Relations};
use crate::model::join::{self, BeadKey, Conflict, Joined};
use crate::model::tree::Assembled;
use crate::model::types::{Edge, Pane, PaneKey};
use crate::model::watching::{Watchers, Watching};

use super::filter::{in_flight_first, partition};
use super::{
    AgentProvider, Collected, Counts, Filter, LoosePane, Node, ProviderState, Readiness, Snapshot,
    TrackerState, Tree, UnconfiguredPane,
};

/// What one project's tracker said of its beads beyond their rows.
///
/// `relations` is read for the whole answer rather than for one tree: what a
/// bead blocks is found on the beads that wait on it, and those can sit in
/// another tree.
#[derive(Debug, Clone, Copy)]
pub struct Said<'a> {
    pub readiness: &'a Readiness,
    pub relations: &'a BTreeMap<String, Relations>,
    /// The beads in other projects waiting on each of this project's beads,
    /// which no answer from this project names. Nothing where no other
    /// project was read.
    pub waiting: Option<&'a BTreeMap<String, Vec<Related>>>,
    /// The bead in another project each id this project's beads name by an
    /// edge stands for, where this project's answer lacks it. Nothing where
    /// no other project was read.
    pub elsewhere: Option<&'a BTreeMap<Named, Elsewhere>>,
    /// The watchers of each of this project's beads, by id. Nothing where
    /// no watcher answered for the project, which leaves who watches them
    /// unknown.
    pub watching: Option<&'a BTreeMap<String, Watchers>>,
}

/// What one project said, as the only project a tree is drawn from.
#[cfg(feature = "testing")]
pub fn said_by<'a>(
    project: &'a str,
    readiness: &'a Readiness,
    relations: &'a BTreeMap<String, Relations>,
) -> BTreeMap<&'a str, Said<'a>> {
    BTreeMap::from([(
        project,
        Said {
            readiness,
            relations,
            waiting: None,
            elsewhere: None,
            watching: None,
        },
    )])
}

/// Draw one project's assembled rows as a tree, with the agents already
/// resolved across every project.
///
/// A tree can hold another project's beads, so what was `said` is by
/// project, and each bead reads what its own project said of it. `agents`
/// is how the run went for panes, which the anomaly rules need: a pane the
/// join did not award and a pane nothing was asked about are different
/// facts, and `joined` alone reads the same for both.
pub fn build_tree(
    project: &str,
    assembled: &Assembled,
    joined: &Joined,
    said: &BTreeMap<&str, Said>,
    agents: ProviderState,
    cfg: &Config,
    now: DateTime<Utc>,
) -> Tree {
    let badges: BTreeMap<&str, Vec<Badge>> = std::iter::once(project)
        .chain(assembled.external.values().map(String::as_str))
        .map(|project| (project, cfg.badges_for_project(project)))
        .collect();
    let project_of = |at: usize| assembled.external.get(&at).map_or(project, String::as_str);
    let beads: Vec<Node> = assembled
        .beads
        .iter()
        .enumerate()
        .map(|(at, bead)| {
            let own = project_of(at);
            let key = BeadKey {
                project: own.to_string(),
                id: bead.id.clone(),
            };
            let said = said.get(own);
            let readiness = said.map(|said| said.readiness);
            let agent = joined.agents.get(&key).cloned();
            let refused = joined.refused.get(&key);
            let out_of_reach = joined.out_of_reach.contains(&key);
            let mut tied = said
                .and_then(|said| said.relations.get(&bead.id))
                .cloned()
                .unwrap_or_default();
            if let Some(elsewhere) = said.and_then(|said| said.elsewhere) {
                held_elsewhere(&mut tied, elsewhere);
            }
            if let Some(waiting) = said
                .and_then(|said| said.waiting)
                .and_then(|waiting| waiting.get(&bead.id))
            {
                tied.blocks.extend(waiting.iter().cloned());
            }
            let badged = badges::badges_for(bead, &badges[own]);
            let mut blocked_by = readiness
                .and_then(|r| r.blocked_by.get(&bead.id))
                .cloned()
                .unwrap_or_default();
            let unseen_by_bd: Vec<String> = blockers_elsewhere(assembled, &project_of, at)
                .filter(|id| !blocked_by.contains(id))
                .collect();
            let ready =
                readiness.is_some_and(|r| r.ready.contains(&bead.id)) && unseen_by_bd.is_empty();
            blocked_by.extend(unseen_by_bd);
            let orphaned = assembled.orphaned.get(&at).cloned().unwrap_or_default();
            blocked_by.retain(|id| {
                !orphaned
                    .iter()
                    .any(|orphan| orphan.id == *id && orphan.why.is_unread())
            });
            Node {
                project: own.to_string(),
                id: bead.id.clone(),
                title: bead.title.clone(),
                status: bead.status.clone(),
                issue_type: bead.issue_type.clone(),
                priority: bead.priority,
                ready,
                blocked_by,
                started_at: bead.started_at,
                closed_at: bead.closed_at,
                badges: badged.drawn,
                undrawn: badged.undrawn,
                anomalies: anomaly::detect(
                    bead,
                    agent.as_ref(),
                    refused,
                    agents,
                    out_of_reach,
                    joined.under_a_seat.contains(&key),
                    &cfg.anomalies,
                    now,
                ),
                watching: Watching::of(
                    said.and_then(|said| said.watching),
                    &bead.id,
                    agent
                        .as_ref()
                        .and_then(|agent| agent.agent_session.as_deref()),
                ),
                agent,
                orphaned_dependencies: orphaned,
                row: bead.row.clone(),
                created_by: bead.created_by.clone(),
                assignee: bead.assignee.clone(),
                labels: bead.labels.clone(),
                created_at: bead.created_at,
                updated_at: bead.updated_at,
                defer_until: bead.defer_until,
                due_at: bead.due_at,
                lease_expires_at: bead.lease_expires_at,
                metadata: bead.metadata.clone(),
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
        orphaned_dependencies: assembled.orphaned_dependencies.clone(),
        cycles: assembled.cycles.clone(),
    }
}

/// The ids of the blockers outside the bead at `at`'s own answer that may
/// still block it: each bead in another project it waits on that is not
/// finished, by bd's own rule, and each blocker no answer holds that may be
/// such a bead. A finished bead waits on nothing.
///
/// bd reads no edge to another project's bead, so its answer never names
/// these. A tree assembled across projects hangs each beneath the bead
/// waiting on it, so every tree drawing the bead finds the same ones.
fn blockers_elsewhere<'a>(
    assembled: &'a Assembled,
    project_of: &'a impl Fn(usize) -> &'a str,
    at: usize,
) -> impl Iterator<Item = String> + 'a {
    let held = assembled.children[at]
        .iter()
        .filter(move |link| {
            link.edge == Edge::Blocks
                && project_of(link.bead) != project_of(at)
                && !assembled.beads[link.bead].status.is_finished()
        })
        .map(|link| assembled.beads[link.bead].id.clone());
    let unheld = assembled
        .orphaned
        .get(&at)
        .into_iter()
        .flatten()
        .filter(|orphaned| orphaned.may_block)
        .map(|orphaned| orphaned.id.clone());
    let waiting = !assembled.beads[at].status.is_finished();
    held.chain(unheld).filter(move |_| waiting)
}

/// Name each bead a bead is tied to that its own project's answer does not
/// hold as the bead in another project the id stands for.
///
/// A project's relations are read from its answer alone, and `elsewhere` is
/// what read every answer together to find the other project's bead.
fn held_elsewhere(tied: &mut Relations, elsewhere: &BTreeMap<Named, Elsewhere>) {
    for related in tied
        .parent
        .iter_mut()
        .chain(&mut tied.depends_on)
        .filter(|related| related.status.is_none())
    {
        if let Some(Elsewhere { project, bead }) =
            elsewhere.get(&(related.project.clone(), related.id.clone()))
        {
            related.project = Some(project.clone());
            related.status = Some(bead.status.clone());
            related.title = Some(bead.title.clone());
        }
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
        speaks_until,
        read_for_reach,
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
            Some(project)
                if !cfg.reads(&project.name) || read_for_reach.contains(&project.name) => {}
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
        unanswered_by_the_watcher: Vec::new(),
        collected: trees,
        out_of_focus: BTreeSet::new(),
        read_at,
        speaks_until,
        projects: cfg.read().map(|p| p.name.clone()).collect(),
        scope: cfg.scope.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::bd::parse_shared_beads;
    use crate::config::{DrawnOn, Scope};
    use crate::model::anomaly::Anomaly;
    use crate::model::badges::Badged;
    use crate::model::edges::relations;
    use crate::model::join::{AgentRef, BeadKey, JoinSource, Listed};
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
            ["dun-7", "dun-7.3", "dun-7.1", "dun-7.4", "dun-7.2"],
            "work in flight leads, finished work trails"
        );
        assert_eq!(
            rows(&t),
            vec![
                ("dun-7", 0, None),
                ("dun-7.3", 1, Some(Edge::ParentChild)),
                ("dun-7.1", 1, Some(Edge::ParentChild)),
                ("dun-7.4", 1, Some(Edge::ParentChild)),
                ("dun-7.2", 1, Some(Edge::ParentChild)),
            ]
        );
    }

    /// Prefixes are uncoordinated, so a tree that reaches into another
    /// project can hold two beads of one id, one from each. They are two
    /// beads, and the tree counts both.
    #[test]
    fn a_tree_counts_two_projects_beads_of_one_id_as_two() {
        let harbour = parse_shared_beads(
            r#"[{"id":"hbr-1","title":"clear the berth","status":"open",
                 "dependencies":[{"depends_on_id":"dun-7","type":"blocks"}]},
                {"id":"x-1","title":"harbour's x-1","status":"open",
                 "dependencies":[{"depends_on_id":"hbr-1","type":"parent-child"}]}]"#,
        )
        .expect("the rows parse");
        let dunwich = parse_shared_beads(
            r#"[{"id":"dun-7","title":"lift the ground station","status":"open"},
                {"id":"x-1","title":"dunwich's x-1","status":"open",
                 "dependencies":[{"depends_on_id":"dun-7","type":"parent-child"}]}]"#,
        )
        .expect("the rows parse");
        let assembled = crate::model::tree::Across::of(
            [
                ("harbour", crate::model::tree::Nesting::of(&harbour)),
                ("dunwich", crate::model::tree::Nesting::of(&dunwich)),
            ],
            [],
        )
        .assemble("harbour", "hbr-1")
        .expect("the rows assemble");

        let t = build_tree(
            "harbour",
            &assembled,
            &Joined::default(),
            &BTreeMap::new(),
            ProviderState::Answering,
            &cfg(),
            now(),
        );

        assert_eq!(t.counts.total, 4);
    }

    /// bd names no blocker against a finished bead, and neither does the edge
    /// bd cannot read.
    #[test]
    fn a_finished_bead_is_blocked_by_nothing_in_another_project() {
        let harbour = parse_shared_beads(
            r#"[{"id":"hbr-1","title":"clear the berth","status":"closed",
                 "dependencies":[{"depends_on_id":"dun-7","type":"blocks"}]}]"#,
        )
        .expect("the rows parse");
        let dunwich = parse_shared_beads(
            r#"[{"id":"dun-7","title":"lift the ground station","status":"in_progress"}]"#,
        )
        .expect("the rows parse");
        let assembled = crate::model::tree::Across::of(
            [
                ("harbour", crate::model::tree::Nesting::of(&harbour)),
                ("dunwich", crate::model::tree::Nesting::of(&dunwich)),
            ],
            [],
        )
        .assemble("harbour", "hbr-1")
        .expect("the rows assemble");

        let t = build_tree(
            "harbour",
            &assembled,
            &Joined::default(),
            &BTreeMap::new(),
            ProviderState::Answering,
            &cfg(),
            now(),
        );

        assert!(node(&t, "hbr-1").blocked_by.is_empty());
    }

    /// One pane, working on the work in `dun-1`'s tree.
    const SEAT_PANES: &str = r#"{"result":{"agents":[
      {"pane_id":"w:p1","cwd":"/srv/work/dunwich","agent_status":"working"}
    ]}}"#;

    /// A bead in flight, parented where `parent` says, and blocked by each of
    /// `blockers`. `pane` is the one thing a seat writes onto its bead.
    fn in_flight(id: &str, parent: Option<&str>, blockers: &[&str], pane: bool) -> String {
        let deps: Vec<String> = parent
            .map(|p| format!(r#"{{"depends_on_id":"{p}","type":"parent-child"}}"#))
            .into_iter()
            .chain(
                blockers
                    .iter()
                    .map(|b| format!(r#"{{"depends_on_id":"{b}","type":"blocks"}}"#)),
            )
            .collect();
        let metadata = if pane {
            r#"{"agent_pane":"w:p1"}"#
        } else {
            "{}"
        };
        format!(
            r#"{{"id":"{id}","title":"{id}","status":"in_progress",
                "updated_at":"2026-08-29T12:00:00Z","metadata":{metadata},
                "dependencies":[{}]}}"#,
            deps.join(",")
        )
    }

    /// The trees under each of `roots`, all read together with `out_of_reach`
    /// as the beads whose panes the run could not ask about.
    fn trees_under(roots: &[&str], beads: &[String], out_of_reach: &[&str]) -> Vec<Tree> {
        let beads = parse_shared_beads(&format!("[{}]", beads.join(","))).expect("rows parse");
        let nesting = crate::model::tree::Nesting::of(&beads);
        let assembled: Vec<Assembled> = roots
            .iter()
            .map(|root| nesting.assemble(root).expect("the rows assemble"))
            .collect();
        let mut joined = joined(&beads, &panes(SEAT_PANES));
        joined.out_of_reach = out_of_reach
            .iter()
            .map(|id| BeadKey {
                project: "dunwich".to_string(),
                id: id.to_string(),
            })
            .collect();
        for tree in &assembled {
            joined.cover("dunwich", tree, now());
        }
        let relations = relations(&beads);
        assembled
            .iter()
            .map(|tree| {
                build_tree(
                    "dunwich",
                    tree,
                    &joined,
                    &crate::model::snapshot::said_by("dunwich", &readiness(), &relations),
                    ProviderState::Answering,
                    &cfg(),
                    now(),
                )
            })
            .collect()
    }

    fn tree_under(root: &str, beads: &[String], out_of_reach: &[&str]) -> Tree {
        trees_under(&[root], beads, out_of_reach).remove(0)
    }

    /// Every anomaly fired in the tree under `root`, by the bead it fired on.
    fn fired_under(
        root: &str,
        beads: &[String],
        out_of_reach: &[&str],
    ) -> BTreeMap<String, Vec<Anomaly>> {
        tree_under(root, beads, out_of_reach)
            .beads
            .into_iter()
            .map(|n| (n.id, n.anomalies))
            .filter(|(_, fired)| !fired.is_empty())
            .collect()
    }

    #[test]
    fn a_bead_firing_two_rules_is_counted_once() {
        let old_claim = r#"{"id":"dun-8","title":"dun-8","status":"in_progress",
                            "updated_at":"2026-07-01T12:00:00Z"}"#;
        let t = tree_under("dun-8", &[old_claim.to_string()], &[]);

        assert_eq!(
            node(&t, "dun-8").anomalies,
            vec![
                Anomaly::OrphanClaim { refused: None },
                Anomaly::StaleClaim { days: 60 }
            ],
            "an old claim whose agent died is both"
        );
        assert_eq!(
            t.counts.anomalies, 1,
            "the count is beads to look at, not rules that fired"
        );
    }

    fn orphan() -> Vec<Anomaly> {
        vec![Anomaly::OrphanClaim { refused: None }]
    }

    /// The seat convention: the pane is on the bead covering the work, and the
    /// beads beneath it, however deep, are in progress without one.
    #[test]
    fn a_step_in_progress_beneath_a_seated_bead_is_not_orphaned() {
        let fired = fired_under(
            "dun-1",
            &[
                in_flight("dun-1", None, &[], true),
                in_flight("dun-1.1", Some("dun-1"), &[], false),
                in_flight("dun-1.1.1", Some("dun-1.1"), &[], false),
            ],
            &[],
        );
        assert_eq!(fired, BTreeMap::new());
    }

    /// A molecule's steps hang beneath the bead waiting on its outcome step
    /// by a blocks edge, which nests them just as a parent-child edge does.
    #[test]
    fn a_blocker_in_progress_beneath_a_seated_bead_is_not_orphaned() {
        let fired = fired_under(
            "dun-2",
            &[
                in_flight("dun-2", None, &["dun-2.1"], true),
                in_flight("dun-2.1", None, &[], false),
            ],
            &[],
        );
        assert_eq!(fired, BTreeMap::new());
    }

    #[test]
    fn a_claim_with_no_seat_on_it_or_above_it_is_still_orphaned() {
        let fired = fired_under(
            "dun-3",
            &[
                in_flight("dun-3", None, &[], false),
                in_flight("dun-3.1", Some("dun-3"), &[], false),
            ],
            &[],
        );
        assert_eq!(
            fired,
            BTreeMap::from([
                ("dun-3".to_string(), orphan()),
                ("dun-3.1".to_string(), orphan())
            ])
        );
    }

    #[test]
    fn a_seat_beneath_a_claim_does_not_cover_it() {
        let fired = fired_under(
            "dun-4",
            &[
                in_flight("dun-4", None, &[], false),
                in_flight("dun-4.1", Some("dun-4"), &[], true),
            ],
            &[],
        );
        assert_eq!(fired, BTreeMap::from([("dun-4".to_string(), orphan())]));
    }

    /// `dun-5.3` is a step of two beads, only one of which has a seat.
    #[test]
    fn a_seat_on_any_one_of_a_beads_parents_covers_it() {
        let fired = fired_under(
            "dun-5",
            &[
                in_flight("dun-5", None, &[], false),
                in_flight("dun-5.1", Some("dun-5"), &["dun-5.3"], true),
                in_flight("dun-5.2", Some("dun-5"), &["dun-5.3"], false),
                in_flight("dun-5.3", None, &[], false),
            ],
            &[],
        );
        assert_eq!(
            fired,
            BTreeMap::from([
                ("dun-5".to_string(), orphan()),
                ("dun-5.2".to_string(), orphan()),
            ]),
            "dun-5.3 is covered by dun-5.1, and the unseated beads are not"
        );
    }

    /// `dun-9.1` is drawn in both trees, and the seat is in only one of them.
    /// A claim is covered or it is not, whichever tree it is read in.
    #[test]
    fn a_seat_in_one_tree_covers_a_claim_drawn_in_another() {
        let trees = trees_under(
            &["dun-9", "dun-10"],
            &[
                in_flight("dun-9", None, &["dun-9.1"], true),
                in_flight("dun-10", None, &["dun-9.1"], false),
                in_flight("dun-9.1", None, &[], false),
            ],
            &[],
        );

        let fired: Vec<Vec<(&str, &[Anomaly])>> = trees
            .iter()
            .map(|tree| {
                tree.beads
                    .iter()
                    .map(|n| (n.id.as_str(), n.anomalies.as_slice()))
                    .filter(|(_, fired)| !fired.is_empty())
                    .collect()
            })
            .collect();
        assert_eq!(
            fired,
            vec![vec![], vec![("dun-10", orphan().as_slice())]],
            "dun-10 has no seat on it or above it, and dun-9.1 is under dun-9's"
        );
    }

    /// A seat whose pane the run could not ask about may be alive, so a claim
    /// beneath it is not known to be orphaned.
    #[test]
    fn a_claim_beneath_a_seat_out_of_reach_is_not_orphaned() {
        let fired = fired_under(
            "dun-6",
            &[
                in_flight("dun-6", None, &[], false),
                in_flight("dun-6.1", Some("dun-6"), &[], false),
            ],
            &["dun-6"],
        );
        assert_eq!(fired, BTreeMap::new());
    }

    /// A seat on another machine has no pane here, but the lease on the bead
    /// it sits on is still live, so the claims beneath it are covered.
    #[test]
    fn a_claim_beneath_a_bead_holding_a_live_lease_is_not_orphaned() {
        let leased = r#"{"id":"dun-6","title":"dun-6","status":"in_progress",
                         "updated_at":"2026-08-29T12:00:00Z","assignee":"Rowan Ash",
                         "lease_expires_at":"2026-08-30T12:05:00Z"}"#;
        let fired = fired_under(
            "dun-6",
            &[
                leased.to_string(),
                in_flight("dun-6.1", Some("dun-6"), &[], false),
            ],
            &[],
        );
        assert_eq!(fired, BTreeMap::new());
    }

    /// The same forest read on a machine with no agent provider: nothing
    /// answers for panes, so the join is handed none.
    fn read_without_a_provider(state: ProviderState) -> Snapshot {
        let assembled = assembled(BEADS);
        let joined = joined(&assembled.beads, &[]);
        let relations = relations(&assembled.beads);
        let tree = build_tree(
            "dunwich",
            &assembled,
            &joined,
            &crate::model::snapshot::said_by("dunwich", &readiness(), &relations),
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
    /// every claim in flight reads as an agent that died. `dun-7` is such a
    /// claim, and `bd` alone holds no fact against it — it is fresh, so even
    /// its age says nothing. The four rules that key on a pane being *there*
    /// fall silent on their own, and asserting the whole set is what keeps
    /// that true of the rule somebody adds next.
    #[test]
    fn with_no_provider_only_the_rule_bd_answers_alone_still_fires() {
        let snap = read_without_a_provider(ProviderState::Absent);

        assert_eq!(
            anomalies(&snap),
            vec![("dun-7.3", [Anomaly::StaleClaim { days: 60 }].as_slice())],
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
            vec![("dun-7.3", [Anomaly::StaleClaim { days: 60 }].as_slice())]
        );
    }

    /// What `bd show` says of a bead, carried on the node so the screen can
    /// say it without asking the tracker again: the words the row holds, and
    /// what the answer ties the bead to — each with the status and title the
    /// answer gave it, including a bead in no tree of its own.
    #[test]
    fn the_bead_as_bd_show_gives_it_reaches_the_node() {
        let json = r#"[
          {"id":"dun-6","title":"root","status":"open",
           "owner":"mira@dunwich.invalid","created_by":"Mira Vance",
           "assignee":"Rowan Ash","labels":["mast","weather"],
           "created_at":"2026-03-14T09:00:00Z","updated_at":"2026-03-16T09:00:00Z",
           "description":"lift the whole station","notes":"the crane is booked"},
          {"id":"dun-6.1","title":"waiting","status":"open",
           "dependencies":[{"depends_on_id":"dun-6","type":"parent-child"},
                           {"depends_on_id":"dun-6.2","type":"blocks"}]},
          {"id":"dun-6.2","title":"what it waits on","status":"closed",
           "dependencies":[{"depends_on_id":"dun-6","type":"parent-child"}]}
        ]"#;
        let beads = parse_shared_beads(json).expect("the rows parse");
        let relations = relations(&beads);
        let t = build_tree(
            "dunwich",
            &assembled(json),
            &Joined::default(),
            &crate::model::snapshot::said_by("dunwich", &Readiness::default(), &relations),
            ProviderState::Answering,
            &cfg(),
            now(),
        );

        let root = node(&t, "dun-6");
        assert_eq!(
            root.description().as_deref(),
            Some("lift the whole station")
        );
        assert_eq!(root.notes().as_deref(), Some("the crane is booked"));
        assert_eq!(root.created_by.as_deref(), Some("Mira Vance"));
        assert_eq!(root.assignee.as_deref(), Some("Rowan Ash"));
        assert_eq!(root.labels, ["mast", "weather"]);
        assert_eq!(root.created_at, "2026-03-14T09:00:00Z".parse().ok());
        assert_eq!(root.updated_at, "2026-03-16T09:00:00Z".parse().ok());
        assert_eq!(root.parent, None);
        assert_eq!(root.depends_on, vec![]);
        assert_eq!(root.blocks, vec![]);

        let waiting = node(&t, "dun-6.1");
        assert_eq!(waiting.description(), None, "a row with nothing to say");
        assert_eq!(waiting.notes(), None);
        assert_eq!(waiting.created_by, None);
        assert_eq!(waiting.assignee, None);
        assert_eq!(waiting.labels, [] as [String; 0]);
        assert_eq!(waiting.created_at, None);
        assert_eq!(waiting.updated_at, None);
        assert_eq!(
            waiting.parent.as_ref().map(|p| p.id.as_str()),
            Some("dun-6")
        );
        assert_eq!(
            waiting
                .depends_on
                .iter()
                .map(|r| (r.id.as_str(), r.status.clone(), r.title.as_deref()))
                .collect::<Vec<_>>(),
            [("dun-6.2", Some(Status::Closed), Some("what it waits on"))]
        );

        let waited_on = node(&t, "dun-6.2");
        assert_eq!(
            waited_on
                .blocks
                .iter()
                .map(|r| (r.id.as_str(), r.status.clone(), r.title.as_deref()))
                .collect::<Vec<_>>(),
            [("dun-6.1", Some(Status::Open), Some("waiting"))]
        );
    }

    /// The two shapes a reference is held in that a pattern written for the
    /// qualified form cannot read: a bare number with no repository to build
    /// a URL out of, and a URL held whole where the parts were expected.
    ///
    /// The config asked for the qualified form and got its answer, so the two
    /// it declined are silent the whole way through the build rather than only
    /// at the badge. A reader wanting them writes a second entry for the key.
    #[test]
    fn a_value_no_badge_on_its_key_reads_reaches_the_node_drawing_nothing() {
        let json = r#"[
          {"id":"dun-8","title":"root","status":"open"},
          {"id":"dun-8.1","title":"a bare number","status":"blocked",
           "metadata":{"delivery_pr":"30"},
           "dependencies":[{"depends_on_id":"dun-8","type":"parent-child"}]},
          {"id":"dun-8.2","title":"a whole URL","status":"blocked",
           "metadata":{"delivery_pr":"https://forge.invalid/dunwich/arkham/pull/30"},
           "dependencies":[{"depends_on_id":"dun-8","type":"parent-child"}]},
          {"id":"dun-8.3","title":"the shape it was written for","status":"blocked",
           "metadata":{"delivery_pr":"dunwich/arkham#30"},
           "dependencies":[{"depends_on_id":"dun-8","type":"parent-child"}]}
        ]"#;
        let cfg = Config::from_toml(
            r#"
[[projects]]
name = "dunwich"
path = "/srv/work/dunwich"

[[badges]]
key    = "metadata.delivery_pr"
match  = "(?<owner>[^/]+)/(?<repo>[^#]+)#(?<number>[0-9]+)"
render = "⇢ {repo} #{number}"
link   = "https://forge.invalid/{owner}/{repo}/pull/{number}"
"#,
        )
        .expect("the config parses");
        let beads = parse_shared_beads(json).expect("the rows parse");

        let t = build_tree(
            "dunwich",
            &assembled(json),
            &Joined::default(),
            &crate::model::snapshot::said_by("dunwich", &Readiness::default(), &relations(&beads)),
            ProviderState::Answering,
            &cfg,
            now(),
        );

        for unread in ["dun-8.1", "dun-8.2"] {
            assert!(node(&t, unread).badges.is_empty(), "drew on {unread}");
            assert_eq!(node(&t, unread).undrawn, Vec::new(), "reported on {unread}");
        }

        assert_eq!(
            node(&t, "dun-8.3").undrawn,
            Vec::new(),
            "the shape the pattern was written for has nothing to report"
        );
        assert_eq!(
            node(&t, "dun-8.3").badges,
            vec![Badged {
                key: "metadata.delivery_pr".to_string(),
                text: "⇢ arkham #30".to_string(),
                link: Some("https://forge.invalid/dunwich/arkham/pull/30".to_string()),
                short: None,
                colour: None,
                drawn_on: DrawnOn::Own,
            }]
        );

        assert_eq!(
            node(&t, "dun-8").undrawn,
            Vec::new(),
            "a bead carrying the key at all is the only one this is about"
        );
    }

    /// A project's own entry for a key is tried before the shared entries for
    /// it, so a value both read draws the project's words.
    ///
    /// Where the two rules meet: the shared entry is still in the list and
    /// still reads this value, and what keeps it off the row is only that the
    /// project's entry was tried first and stopped the chain.
    #[test]
    fn a_projects_own_badge_draws_the_value_a_global_one_would_have() {
        let assembled = assembled(BEADS);
        let panes = panes(PANES);
        let joined = joined(&assembled.beads, &panes);
        let relations = relations(&assembled.beads);
        let cfg = Config::from_toml(
            r#"
[[projects]]
name = "dunwich"
path = "/srv/work/dunwich"

[[projects.badges]]
key    = "metadata.blocked_on"
match  = "human"
render = "⏸ ask the ground station"

[[badges]]
key    = "metadata.blocked_on"
match  = "human"
render = "⏸ waiting"
"#,
        )
        .expect("the config parses");

        let t = build_tree(
            "dunwich",
            &assembled,
            &joined,
            &crate::model::snapshot::said_by("dunwich", &readiness(), &relations),
            ProviderState::Answering,
            &cfg,
            now(),
        );

        assert_eq!(
            node(&t, "dun-7.1").badges,
            vec![Badged {
                key: "metadata.blocked_on".to_string(),
                text: "⏸ ask the ground station".to_string(),
                link: None,
                short: None,
                colour: None,
                drawn_on: DrawnOn::Own,
            }]
        );
    }

    /// The bare-number case `docs/configuration.md` works through, and what
    /// precedence buys over replacing the shared entries: the project supplies
    /// the shape its own tracker writes, and a bead written in the shared shape
    /// is still read by the shared entry underneath it.
    ///
    /// Neither pattern reads the other's value, so each bead names which entry
    /// drew it. A project that replaced the shared entry rather than going in
    /// front of it would leave the second bead with no badge at all.
    #[test]
    fn a_value_a_projects_badge_does_not_read_falls_through_to_the_shared_one() {
        let json = r#"[
          {"id":"dun-9","title":"root","status":"open"},
          {"id":"dun-9.1","title":"a bare number","status":"blocked",
           "metadata":{"delivery_pr":"12"},
           "dependencies":[{"depends_on_id":"dun-9","type":"parent-child"}]},
          {"id":"dun-9.2","title":"the shape the shared list reads","status":"blocked",
           "metadata":{"delivery_pr":"dunwich/arkham#30"},
           "dependencies":[{"depends_on_id":"dun-9","type":"parent-child"}]}
        ]"#;
        let cfg = Config::from_toml(
            r#"
[[projects]]
name = "dunwich"
path = "/srv/work/dunwich"

[[projects.badges]]
key    = "metadata.delivery_pr"
match  = "(?<number>[0-9]+)"
render = "⇢ #{number}"

[[badges]]
key    = "metadata.delivery_pr"
match  = "(?<owner>[^/]+)/(?<repo>[^#]+)#(?<number>[0-9]+)"
render = "⇢ {repo} #{number}"
"#,
        )
        .expect("the config parses");
        let beads = parse_shared_beads(json).expect("the rows parse");

        let t = build_tree(
            "dunwich",
            &assembled(json),
            &Joined::default(),
            &crate::model::snapshot::said_by("dunwich", &Readiness::default(), &relations(&beads)),
            ProviderState::Answering,
            &cfg,
            now(),
        );

        assert_eq!(
            node(&t, "dun-9.1").badges,
            vec![Badged {
                key: "metadata.delivery_pr".to_string(),
                text: "⇢ #12".to_string(),
                link: None,
                short: None,
                colour: None,
                drawn_on: DrawnOn::Own,
            }],
            "the project's own entry is the one that reads a bare number"
        );
        assert_eq!(
            node(&t, "dun-9.2").badges,
            vec![Badged {
                key: "metadata.delivery_pr".to_string(),
                text: "⇢ arkham #30".to_string(),
                link: None,
                short: None,
                colour: None,
                drawn_on: DrawnOn::Own,
            }],
            "the shared entry still reads the shape the project said nothing about"
        );
    }

    #[test]
    fn an_agent_on_a_colliding_id_in_another_project_is_not_this_tree_s() {
        let a = assembled(BEADS);
        let p = panes(PANES);
        let mut j = joined(&a.beads, &p);
        j.agents.insert(
            BeadKey {
                project: "ferry".to_string(),
                id: "dun-7.4".to_string(),
            },
            AgentRef {
                pane: key("w:pB"),
                pane_status: PaneStatus::Working,
                title: None,
                source: JoinSource::AgentPane,
                agent_session: None,
            },
        );

        let t = build_tree(
            "dunwich",
            &a,
            &j,
            &crate::model::snapshot::said_by("dunwich", &readiness(), &BTreeMap::new()),
            ProviderState::Answering,
            &cfg(),
            now(),
        );

        assert!(
            node(&t, "dun-7.4").agent.is_none(),
            "prefixes are per-tracker; an id alone does not name a bead"
        );
        assert_eq!(t.counts.live_agents, 2);
    }

    #[test]
    fn a_header_counts_beads_rather_than_the_rows_they_are_drawn_on() {
        // `dun-8.9` blocks both of the root's children, so it is drawn three
        // times. A header saying five would send a reader looking for a bead
        // that is not there.
        let json = r#"[
          {"id":"dun-8","title":"root","status":"open"},
          {"id":"dun-8.1","title":"one","status":"open",
           "dependencies":[{"depends_on_id":"dun-8","type":"parent-child"},
                           {"depends_on_id":"dun-8.9","type":"blocks"}]},
          {"id":"dun-8.2","title":"two","status":"open",
           "dependencies":[{"depends_on_id":"dun-8","type":"parent-child"},
                           {"depends_on_id":"dun-8.9","type":"blocks"}]},
          {"id":"dun-8.9","title":"what both wait on","status":"closed",
           "dependencies":[{"depends_on_id":"dun-8","type":"parent-child"}]}
        ]"#;
        let t = build_tree(
            "dunwich",
            &assembled(json),
            &Joined::default(),
            &crate::model::snapshot::said_by("dunwich", &Readiness::default(), &BTreeMap::new()),
            ProviderState::Answering,
            &cfg(),
            now(),
        );

        assert_eq!(rows(&t).len(), 6, "a copy per way down");
        assert_eq!(t.beads.len(), 4, "held once each");
        assert_eq!(t.counts.total, 4);
        assert_eq!(t.counts.finished, 1);
    }

    /// A pane under a project the scope left out is neither drawn nor
    /// reported: not loose, because it is on another desktop's work, and not
    /// unconfigured, because the config still names its project.
    #[test]
    fn a_pane_under_a_project_the_scope_left_out_is_neither_loose_nor_unconfigured() {
        let cfg = cfg()
            .scoped_to(&["dunwich".to_string()])
            .expect("dunwich is configured");
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
                speaks_until: std::collections::BTreeMap::new(),
                read_for_reach: std::collections::BTreeSet::new(),
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
            .scoped_to(&["dunwich".to_string()])
            .expect("dunwich is configured");

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
    fn a_project_whose_tracker_failed_is_reported_apart_from_the_trees() {
        let failed = vec![
            FailedProject {
                project: "ferry".to_string(),
                tracker: TrackerFailure::Auth,
            },
            FailedProject {
                project: "dunwich".to_string(),
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
name = "dunwich"
path = "/tmp/bdi-ground/dunwich"
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
            .expect("in dunwich's directory and on no bead");
        assert_eq!(p6.display_agent.as_deref(), Some("dun-2kd.5"));
        assert_eq!(
            p6.title.as_deref(),
            Some("writing the parser and its tests")
        );
    }

    #[test]
    fn a_snapshot_of_a_long_nesting_whose_every_bead_is_blocked_by_its_root_serialises() {
        const DEPTH: usize = 6000;
        let mut beads = vec![in_flight("dun-1", None, &[], false)];
        for at in 0..DEPTH {
            let parent = if at == 0 {
                "dun-1".to_string()
            } else {
                format!("dun-1.{}", at - 1)
            };
            beads.push(in_flight(
                &format!("dun-1.{at}"),
                Some(&parent),
                &["dun-1"],
                false,
            ));
        }
        let t = tree_under("dun-1", &beads, &[]);

        let json = serde_json::to_value(&t).expect("the tree serialises");

        let deepest = json["nodes"]
            .as_array()
            .expect("the tree has nodes")
            .iter()
            .find(|row| row["id"] == format!("dun-1.{}", DEPTH - 1))
            .expect("the deepest bead is drawn");
        assert_eq!(deepest["depth"], DEPTH);
    }
}
