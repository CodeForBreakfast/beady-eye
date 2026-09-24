//! One project's rows drawn as a tree, and every project's trees gathered
//! into the snapshot, with the live panes that belong to none of them.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::config::{Badge, Config};
use crate::model::anomaly;
use crate::model::badges;
use crate::model::edges::Relations;
use crate::model::join::{self, BeadKey, Conflict, Joined};
use crate::model::tree::Assembled;
use crate::model::types::{Edge, Pane, PaneKey};

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
            let tied = said
                .and_then(|said| said.relations.get(&bead.id))
                .cloned()
                .unwrap_or_default();
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
                    &cfg.anomalies,
                    now,
                ),
                agent,
                orphaned_dependencies: assembled.orphaned.get(&at).cloned().unwrap_or_default(),
                description: bead.description.clone().unwrap_or_default(),
                notes: bead.notes.clone().unwrap_or_default(),
                created_by: bead.created_by.clone(),
                assignee: bead.assignee.clone(),
                labels: bead.labels.clone(),
                created_at: bead.created_at,
                updated_at: bead.updated_at,
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

/// The ids of the beads in another project that the bead at `at` waits on and
/// that still block it, by bd's own rule: a finished blocker blocks nothing,
/// and a finished bead waits on nothing.
///
/// bd reads no edge to another project's bead, so its answer never names
/// these. A tree assembled across projects hangs each beneath the bead
/// waiting on it, so every tree drawing the bead finds the same ones.
fn blockers_elsewhere<'a>(
    assembled: &'a Assembled,
    project_of: &'a impl Fn(usize) -> &'a str,
    at: usize,
) -> impl Iterator<Item = String> + 'a {
    let waiting = !assembled.beads[at].status.is_finished();
    assembled.children[at]
        .iter()
        .filter(move |link| {
            waiting
                && link.edge == Edge::Blocks
                && project_of(link.bead) != project_of(at)
                && !assembled.beads[link.bead].status.is_finished()
        })
        .map(|link| assembled.beads[link.bead].id.clone())
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
        speaks_until,
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

    #[test]
    fn the_root_names_the_tree() {
        let t = tree();
        assert_eq!(t.project, "dunwich");
        assert_eq!(t.root, "dun-7");
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
                finished: 1,
                live_agents: 2,
                anomalies: 2,
            }
        );
    }

    /// Prefixes are uncoordinated, so a tree that reaches into another
    /// project can hold two beads of one id, one from each. They are two
    /// beads, and the tree counts both.
    #[test]
    fn a_tree_counts_two_projects_beads_of_one_id_as_two() {
        let harbour = parse_beads(
            r#"[{"id":"hbr-1","title":"clear the berth","status":"open",
                 "dependencies":[{"depends_on_id":"dun-7","type":"blocks"}]},
                {"id":"x-1","title":"harbour's x-1","status":"open",
                 "dependencies":[{"depends_on_id":"hbr-1","type":"parent-child"}]}]"#,
        )
        .expect("the rows parse");
        let dunwich = parse_beads(
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
        let harbour = parse_beads(
            r#"[{"id":"hbr-1","title":"clear the berth","status":"closed",
                 "dependencies":[{"depends_on_id":"dun-7","type":"blocks"}]}]"#,
        )
        .expect("the rows parse");
        let dunwich = parse_beads(
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

    #[test]
    fn a_bead_firing_two_rules_is_counted_once() {
        let t = tree();
        assert_eq!(
            node(&t, "dun-7.3").anomalies,
            vec![
                Anomaly::OrphanClaim { refused: None },
                Anomaly::StaleClaim { days: 60 }
            ],
            "an old claim whose agent died is both"
        );
        assert_eq!(node(&t, "dun-7.2").anomalies, vec![Anomaly::StalePane]);
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

    #[test]
    fn ready_comes_from_bd_rather_than_the_bead_s_status() {
        let t = tree();
        assert!(node(&t, "dun-7.4").ready);
        assert!(
            !node(&t, "dun-7.1").ready,
            "open is not ready; only bd knows which"
        );
        assert!(!node(&t, "dun-7").ready);
    }

    #[test]
    fn blocked_by_comes_from_bd_rather_than_the_tree() {
        let t = tree();
        assert_eq!(
            node(&t, "dun-7.1").blocked_by,
            ["dun-9", "dun-7.3"],
            "a blocker outside the tree is still a blocker"
        );
        assert!(node(&t, "dun-7.4").blocked_by.is_empty());
    }

    #[test]
    fn the_contract_fields_reach_the_node() {
        let t = tree();
        let root = node(&t, "dun-7");
        assert_eq!(root.status, Status::InProgress);
        assert_eq!(root.issue_type, "epic");
        assert_eq!(root.priority, 1);
        assert_eq!(
            root.started_at,
            Some("2026-08-20T09:00:00Z".parse().unwrap())
        );
        assert_eq!(root.closed_at, None);
        assert_eq!(
            node(&t, "dun-7.2").closed_at,
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
        let beads = parse_beads(json).expect("the rows parse");
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
        assert_eq!(root.description, "lift the whole station");
        assert_eq!(root.notes, "the crane is booked");
        assert_eq!(root.created_by.as_deref(), Some("Mira Vance"));
        assert_eq!(root.assignee.as_deref(), Some("Rowan Ash"));
        assert_eq!(root.labels, ["mast", "weather"]);
        assert_eq!(root.created_at, "2026-03-14T09:00:00Z".parse().ok());
        assert_eq!(root.updated_at, "2026-03-16T09:00:00Z".parse().ok());
        assert_eq!(root.parent, None);
        assert_eq!(root.depends_on, vec![]);
        assert_eq!(root.blocks, vec![]);

        let waiting = node(&t, "dun-6.1");
        assert_eq!(waiting.description, "", "a row with nothing to say");
        assert_eq!(waiting.notes, "");
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

    #[test]
    fn the_badges_reach_the_node() {
        let t = tree();
        assert_eq!(
            node(&t, "dun-7.1").badges,
            vec![Badged {
                key: "metadata.blocked_on".to_string(),
                text: "⏸ waiting".to_string(),
                link: None,
                short: None,
                colour: None,
            }]
        );
        assert!(node(&t, "dun-7").badges.is_empty());
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
        let beads = parse_beads(json).expect("the rows parse");

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
        let beads = parse_beads(json).expect("the rows parse");

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
            }],
            "the shared entry still reads the shape the project said nothing about"
        );
    }

    #[test]
    fn the_agent_reaches_the_node_with_the_direction_that_resolved_it() {
        let t = tree();
        let claimed = node(&t, "dun-7")
            .agent
            .as_ref()
            .expect("the bead named a pane");
        assert_eq!(claimed.pane, key("w:p1"));
        assert_eq!(claimed.source, JoinSource::AgentPane);
        assert_eq!(claimed.title.as_deref(), Some("lifting the mast"));

        let inferred = node(&t, "dun-7.2")
            .agent
            .as_ref()
            .expect("the pane named the bead");
        assert_eq!(inferred.source, JoinSource::DisplayAgent);
        assert!(node(&t, "dun-7.1").agent.is_none());
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
    fn a_bead_whose_parent_is_absent_is_reported_and_kept() {
        let json = r#"[
          {"id":"dun-4","title":"root","status":"open"},
          {"id":"dun-4.2","title":"waiting on a bead bd did not return","status":"open",
           "dependencies":[{"depends_on_id":"dun-4","type":"parent-child"},
                           {"depends_on_id":"dun-4.1","type":"blocks"}]}
        ]"#;
        let a = assembled(json);
        let t = build_tree(
            "dunwich",
            &a,
            &Joined::default(),
            &crate::model::snapshot::said_by("dunwich", &Readiness::default(), &BTreeMap::new()),
            ProviderState::Answering,
            &cfg(),
            now(),
        );

        assert_eq!(t.orphaned_dependencies, ["dun-4.2"]);
        assert_eq!(t.counts.total, 2, "a reported bead is still drawn");
        assert_eq!(rows(&t)[1], ("dun-4.2", 1, Some(Edge::ParentChild)));
    }

    /// The nesting is the whole of what this tree says, so each row has to
    /// carry which kind of edge put it where it is. A copy reached because a
    /// bead blocks its forebear and a copy reached because it is that
    /// forebear's child are drawn the same way and mean different things.
    #[test]
    fn a_node_carries_the_kind_of_edge_its_copy_was_reached_by() {
        let json = r#"[
          {"id":"dun-9","title":"root","status":"open"},
          {"id":"dun-9.1","title":"waiting","status":"open",
           "dependencies":[{"depends_on_id":"dun-9","type":"parent-child"},
                           {"depends_on_id":"dun-9.2","type":"blocks"}]},
          {"id":"dun-9.2","title":"what it waits on","status":"closed",
           "dependencies":[{"depends_on_id":"dun-9","type":"parent-child"}]}
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

        assert_eq!(
            rows(&t),
            vec![
                ("dun-9", 0, None),
                ("dun-9.1", 1, Some(Edge::ParentChild)),
                ("dun-9.2", 2, Some(Edge::Blocks)),
                ("dun-9.2", 1, Some(Edge::ParentChild)),
            ]
        );
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

    #[test]
    fn a_cycle_is_reported_and_its_beads_kept() {
        // `dun-5.1` hangs under `dun-5` and is blocked by it, so each must
        // finish before the other.
        let json = r#"[
          {"id":"dun-5","title":"root","status":"open"},
          {"id":"dun-5.1","title":"one","status":"open",
           "dependencies":[{"depends_on_id":"dun-5","type":"parent-child"},
                           {"depends_on_id":"dun-5","type":"blocks"}]},
          {"id":"dun-5.2","title":"two","status":"open",
           "dependencies":[{"depends_on_id":"dun-5.1","type":"parent-child"}]}
        ]"#;
        let a = assembled(json);
        let t = build_tree(
            "dunwich",
            &a,
            &Joined::default(),
            &crate::model::snapshot::said_by("dunwich", &Readiness::default(), &BTreeMap::new()),
            ProviderState::Answering,
            &cfg(),
            now(),
        );

        assert_eq!(t.cycles, ["dun-5"]);
        assert_eq!(t.counts.total, 3);
    }

    #[test]
    fn a_pane_on_no_bead_lands_in_unattributed_with_its_project() {
        let snap = snapshot(vec![tree()]);

        assert_eq!(
            snap.unattributed,
            vec![LoosePane {
                pane: key("w:p9"),
                project: "dunwich".to_string(),
                cwd: "/srv/work/dunwich".to_string(),
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
          {"id":"dun-1","title":"root","status":"open"},
          {"id":"dun-1.1","title":"one","status":"in_progress",
           "metadata":{"agent_pane":"w:p5"},
           "dependencies":[{"depends_on_id":"dun-1","type":"parent-child"}]},
          {"id":"dun-1.2","title":"two","status":"in_progress",
           "metadata":{"agent_pane":"w:p5"},
           "dependencies":[{"depends_on_id":"dun-1","type":"parent-child"}]}
        ]"#;
        let panes = panes(
            r#"{"result":{"agents":[
              {"pane_id":"w:p5","cwd":"/srv/work/dunwich","agent_status":"working"},
              {"pane_id":"w:p9","cwd":"/srv/work/dunwich","agent_status":"idle"}
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
    fn the_join_s_conflicts_reach_the_snapshot() {
        let disagreeing = r#"{"result":{"agents":[
          {"pane_id":"w:p1","cwd":"/srv/work/dunwich","agent_status":"working"},
          {"pane_id":"w:p2","cwd":"/srv/work/dunwich","agent_status":"idle",
           "display_agent":"dun-7"}
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
                    project: "dunwich".to_string(),
                    id: "dun-7".to_string(),
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
}
