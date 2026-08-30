use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};

use crate::collect::run::{Env, FailureKind, RunFailure, Runner};
use crate::collect::{bd, herdr};
use crate::config::{Config, Project};
use crate::model::join::{self, ProjectRows};
use crate::model::snapshot::{
    self, Collected, FailedProject, Filter, HerdrState, Readiness, Snapshot, TrackerFailure, Tree,
};
use crate::model::tree::{assemble, Assembled};

/// One project's roots in id order, each either read or unreadable.
struct ProjectWork {
    project: String,
    readiness: Readiness,
    roots: Vec<(String, Result<Assembled, TrackerFailure>)>,
}

/// Read every configured tracker and, where there is one, the herdr session,
/// and draw the result.
pub fn run(cfg: &Config, runner: &dyn Runner, filter: Filter, now: DateTime<Utc>) -> Snapshot {
    // herdr is the second tier: without it there is no agent to join and no
    // filter to apply, and every tracker still reads.
    let (panes, herdr_state) = match herdr::agent_list(runner) {
        Ok(panes) => (panes, HerdrState::Ok),
        Err(_) => (Vec::new(), HerdrState::Unavailable),
    };

    let mut read: Vec<ProjectWork> = Vec::new();
    let mut failed_projects: Vec<FailedProject> = Vec::new();
    for project in &cfg.projects {
        match read_project(runner, project, cfg) {
            Ok(work) => read.push(work),
            Err(failure) => failed_projects.push(FailedProject {
                project: project.name.clone(),
                tracker: tracker_failure(failure.kind),
            }),
        }
    }

    // One resolve over every project's rows at once. A pane names its bead by
    // id alone, and only the whole set tells a match from a prefix collision.
    let rows: Vec<ProjectRows<'_>> = read
        .iter()
        .flat_map(|work| {
            work.roots.iter().filter_map(|(_, read)| {
                read.as_ref().ok().map(|assembled| ProjectRows {
                    project: work.project.as_str(),
                    rows: &assembled.rows,
                })
            })
        })
        .collect();
    let joined = join::resolve(&rows, &panes, &cfg.projects, &cfg.join);

    let trees = read
        .iter()
        .flat_map(|work| {
            work.roots.iter().map(|(root, read)| match read {
                Ok(assembled) => snapshot::build_tree(
                    &work.project,
                    assembled,
                    &joined,
                    &work.readiness,
                    cfg,
                    now,
                ),
                Err(failure) => Tree::tracker_unreachable(&work.project, root, *failure),
            })
        })
        .collect();

    snapshot::build(
        Collected {
            trees,
            failed_projects,
        },
        &panes,
        &joined,
        cfg,
        herdr_state,
        filter,
        now,
    )
}

/// Everything one project's tracker is asked for. A failure before the roots
/// are known has no root to name, so it becomes the project's own failure
/// rather than a tree; a failure on one root afterwards is that root's.
fn read_project(
    runner: &dyn Runner,
    project: &Project,
    cfg: &Config,
) -> Result<ProjectWork, RunFailure> {
    let env = bd::credential_env(runner, project)?;
    let discovered = bd::discover_roots(runner, &project.path, &env, &cfg.roots.metadata_keys)?;

    // An empty readiness set reads as "nothing here is ready", so a tracker
    // that cannot answer must not leave one behind.
    let readiness = Readiness {
        ready: bd::ready_ids(runner, &project.path, &env)?,
        blocked_by: bd::blocked_by(runner, &project.path, &env)?,
    };

    let mut ancestors: BTreeMap<String, String> = BTreeMap::new();
    let mut roots: BTreeSet<String> = cfg.roots.explicit.iter().cloned().collect();
    for bead in &discovered {
        roots.insert(root_of(runner, project, &env, &bead.id, &mut ancestors)?);
    }

    Ok(ProjectWork {
        project: project.name.clone(),
        readiness,
        roots: roots
            .into_iter()
            .map(|root| {
                let read = bd::dep_tree(runner, &project.path, &env, &root)
                    .map_err(|failure| tracker_failure(failure.kind))
                    .and_then(|rows| assemble(rows).map_err(|_| TrackerFailure::Parse));
                (root, read)
            })
            .collect(),
    })
}

/// The top of a bead's parent-child chain, asked of bd one level at a time.
///
/// `bd dep tree` cannot answer this: `--direction=up` walks dependents, so
/// whatever bead it is asked about comes back as its own root. `ancestors`
/// carries what earlier walks found, so an epic's beads cost one call each
/// rather than one per level each.
fn root_of(
    runner: &dyn Runner,
    project: &Project,
    env: &Env,
    id: &str,
    ancestors: &mut BTreeMap<String, String>,
) -> Result<String, RunFailure> {
    let mut climbed: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut current = id.to_string();

    let root = loop {
        if let Some(known) = ancestors.get(&current) {
            break known.clone();
        }
        // A parent chain that loops has no top. Stopping where it repeats
        // keeps the bead visible rather than hanging on it.
        if !seen.insert(current.clone()) {
            break current;
        }
        climbed.push(current.clone());
        match bd::parent_of(runner, &project.path, env, &current)? {
            Some(parent) => current = parent,
            None => break current,
        }
    };

    for climbed in climbed {
        ancestors.insert(climbed, root.clone());
    }
    Ok(root)
}

/// The four kinds bd's collector classifies, in the model's own vocabulary.
/// `RunFailure.detail` stops here: bd names the database and the SQL user
/// when it refuses a credential, and the words for a failure belong to
/// whatever draws it.
fn tracker_failure(kind: FailureKind) -> TrackerFailure {
    match kind {
        FailureKind::Auth => TrackerFailure::Auth,
        FailureKind::Unavailable => TrackerFailure::Unavailable,
        FailureKind::Exec => TrackerFailure::Exec,
        FailureKind::Parse => TrackerFailure::Parse,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::run::testing::FakeRunner;
    use crate::model::snapshot::{Node, TrackerState};
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;

    const ORBITAL: &str = "/srv/work/orbital";
    const FERRY: &str = "/srv/work/ferry";

    /// One project's tracker as bd answers for the root: an epic over two
    /// tasks, one of them naming the pane working it.
    const ORBITAL_TREE: &str = r#"[
      {"id":"orb-7","title":"lift the ground station","status":"in_progress","parent_id":"",
       "priority":1,"issue_type":"epic"},
      {"id":"orb-7.1","title":"re-point the dish","status":"in_progress","parent_id":"orb-7",
       "priority":2,"issue_type":"task","edge_from_parent":"parent-child",
       "metadata":{"agent_pane":"w:p1"}},
      {"id":"orb-7.2","title":"lay the feeder cable","status":"open","parent_id":"orb-7",
       "priority":2,"issue_type":"task","edge_from_parent":"parent-child"}
    ]"#;

    /// A second root, reached only because config names it.
    const MAST_TREE: &str = r#"[
      {"id":"orb-4","title":"survey the mast","status":"open","parent_id":"",
       "priority":2,"issue_type":"task"}
    ]"#;

    /// Two trackers that chose the same id prefix, which no one coordinates.
    const COLLIDING_TREE: &str = r#"[
      {"id":"x-1","title":"the shared prefix","status":"in_progress","parent_id":"",
       "priority":1,"issue_type":"epic"},
      {"id":"x-1.1","title":"the colliding id","status":"in_progress","parent_id":"x-1",
       "priority":2,"issue_type":"task","edge_from_parent":"parent-child"}
    ]"#;

    /// `w:p1` is on a bead; `w:p9` is a session on none.
    const PANES: &str = r#"{"result":{"agents":[
      {"pane_id":"w:p1","cwd":"/srv/work/orbital","agent_status":"working","title":"the dish"},
      {"pane_id":"w:p9","cwd":"/srv/work/orbital","agent_status":"idle"}
    ]}}"#;

    fn now() -> DateTime<Utc> {
        "2026-08-30T12:00:00Z".parse().expect("the instant parses")
    }

    fn one_project() -> Config {
        Config::from_toml(&format!(
            r#"
[[projects]]
name = "orbital"
path = "{ORBITAL}"

[roots]
metadata_keys = ["working_topic"]
"#
        ))
        .expect("the config parses")
    }

    fn two_projects() -> Config {
        Config::from_toml(&format!(
            r#"
[[projects]]
name = "orbital"
path = "{ORBITAL}"
credential_command = "secret orbital"

[[projects]]
name = "ferry"
path = "{FERRY}"
credential_command = "secret ferry"
"#
        ))
        .expect("the config parses")
    }

    /// Every call a healthy single-project run makes.
    fn orbital() -> FakeRunner {
        FakeRunner::default()
            .with("herdr agent list", PANES)
            .with(
                "bd list --status in_progress --limit 0 --json",
                r#"[{"id":"orb-7.1","title":"re-point the dish","status":"in_progress"}]"#,
            )
            .with("bd list --status blocked --limit 0 --json", "[]")
            .with(
                "bd list --has-metadata-key working_topic --limit 0 --json",
                "[]",
            )
            .with(
                "bd ready --limit 0 --json",
                r#"[{"id":"orb-7.2","title":"lay the feeder cable","status":"open"}]"#,
            )
            .with(
                "bd blocked --json",
                r#"[{"id":"orb-7.1","blocked_by":["orb-9"]}]"#,
            )
            .with(
                "bd show orb-7.1 --json",
                r#"[{"id":"orb-7.1","parent":"orb-7"}]"#,
            )
            .with("bd show orb-7 --json", r#"[{"id":"orb-7","parent":null}]"#)
            .with("bd dep tree orb-7 --direction=up --json", ORBITAL_TREE)
    }

    fn failing(kind: FailureKind) -> RunFailure {
        RunFailure {
            kind,
            program: "bd".to_string(),
            detail: "bd could not read the tracker".to_string(),
        }
    }

    fn node<'a>(tree: &'a Tree, id: &str) -> &'a Node {
        tree.nodes
            .iter()
            .find(|n| n.id == id)
            .unwrap_or_else(|| panic!("{id} is among the nodes"))
    }

    fn tree_of<'a>(snap: &'a Snapshot, project: &str) -> &'a Tree {
        snap.trees
            .iter()
            .find(|t| t.project == project)
            .unwrap_or_else(|| panic!("{project} has a tree"))
    }

    // ---- discovery ----------------------------------------------------

    /// The whole point of the ancestor walk: an in-flight task is drawn as
    /// the tree it hangs under, not as a root of its own.
    #[test]
    fn a_discovered_bead_is_drawn_as_the_tree_it_hangs_under() {
        let snap = run(&one_project(), &orbital(), Filter::LiveAgents, now());

        assert_eq!(snap.trees.len(), 1);
        assert_eq!(snap.trees[0].root, "orb-7");
        assert_eq!(snap.trees[0].title, "lift the ground station");
        assert_eq!(snap.trees[0].nodes.len(), 3);
    }

    #[test]
    fn a_configured_metadata_key_discovers_a_root_bds_statuses_would_miss() {
        let runner = orbital()
            .with("bd list --status in_progress --limit 0 --json", "[]")
            .with(
                "bd list --has-metadata-key working_topic --limit 0 --json",
                r#"[{"id":"orb-7.1","title":"re-point the dish","status":"open"}]"#,
            );

        let snap = run(&one_project(), &runner, Filter::All, now());

        assert_eq!(snap.trees.len(), 1);
        assert_eq!(snap.trees[0].root, "orb-7");
    }

    #[test]
    fn one_ancestor_is_walked_once_however_many_beads_share_it() {
        let runner = orbital()
            .with(
                "bd list --status blocked --limit 0 --json",
                r#"[{"id":"orb-7.2","title":"lay the feeder cable","status":"blocked"}]"#,
            )
            .with(
                "bd show orb-7.2 --json",
                r#"[{"id":"orb-7.2","parent":"orb-7"}]"#,
            );

        run(&one_project(), &runner, Filter::All, now());

        // `call` panics on a second invocation, which is the assertion.
        runner.call("bd show orb-7 --json");
    }

    /// A parent chain that loops has no top. Stopping where it repeats keeps
    /// the bead visible rather than hanging on it.
    #[test]
    fn a_parent_chain_that_loops_stops_where_it_repeats() {
        let runner = orbital()
            .with(
                "bd show orb-7 --json",
                r#"[{"id":"orb-7","parent":"orb-7.1"}]"#,
            )
            .with(
                "bd dep tree orb-7.1 --direction=up --json",
                r#"[{"id":"orb-7.1","title":"re-point the dish","status":"in_progress",
                     "parent_id":"","priority":2,"issue_type":"task"}]"#,
            );

        let snap = run(&one_project(), &runner, Filter::All, now());

        assert_eq!(snap.trees.len(), 1);
        assert_eq!(snap.trees[0].root, "orb-7.1");
    }

    #[test]
    fn a_root_named_in_config_joins_the_discovered_ones_without_duplicating_them() {
        let cfg = Config::from_toml(&format!(
            r#"
[[projects]]
name = "orbital"
path = "{ORBITAL}"

[roots]
explicit = ["orb-7", "orb-4"]
"#
        ))
        .expect("the config parses");
        let runner = orbital().with("bd dep tree orb-4 --direction=up --json", MAST_TREE);

        let snap = run(&cfg, &runner, Filter::All, now());

        let roots: Vec<&str> = snap.trees.iter().map(|t| t.root.as_str()).collect();
        assert_eq!(
            roots,
            vec!["orb-4", "orb-7"],
            "the root config and discovery both name is drawn once"
        );
    }

    // ---- what bd knows that the tree does not --------------------------

    #[test]
    fn readiness_and_blockers_come_from_bd_rather_than_from_status() {
        let snap = run(&one_project(), &orbital(), Filter::LiveAgents, now());
        let tree = &snap.trees[0];

        assert!(node(tree, "orb-7.2").ready, "bd ready named it");
        assert!(!node(tree, "orb-7.1").ready);
        assert_eq!(
            node(tree, "orb-7.1").blocked_by,
            vec!["orb-9".to_string()],
            "a blocker outside the tree still reaches the node"
        );
        assert!(node(tree, "orb-7.2").blocked_by.is_empty());
    }

    // ---- degradation ---------------------------------------------------

    #[test]
    fn a_project_whose_discovery_fails_is_named_not_dropped() {
        let runner = orbital().failing(
            "bd list --status in_progress --limit 0 --json",
            failing(FailureKind::Auth),
        );

        let snap = run(&one_project(), &runner, Filter::LiveAgents, now());

        assert!(snap.trees.is_empty());
        assert_eq!(
            snap.failed_projects,
            vec![FailedProject {
                project: "orbital".to_string(),
                tracker: TrackerFailure::Auth,
            }]
        );
    }

    #[test]
    fn every_way_a_tracker_fails_keeps_its_own_kind() {
        let kinds = [
            (FailureKind::Auth, TrackerFailure::Auth),
            (FailureKind::Unavailable, TrackerFailure::Unavailable),
            (FailureKind::Exec, TrackerFailure::Exec),
            (FailureKind::Parse, TrackerFailure::Parse),
        ];

        for (kind, expected) in kinds {
            let runner = orbital().failing(
                "bd list --status in_progress --limit 0 --json",
                failing(kind),
            );

            let snap = run(&one_project(), &runner, Filter::All, now());

            assert_eq!(snap.failed_projects[0].tracker, expected, "on {kind:?}");
        }
    }

    /// A root we already know the id of keeps it, which is what tells two
    /// failures apart, and the filter has no agent count to hide it by.
    #[test]
    fn a_root_whose_tree_cannot_be_read_keeps_its_id_and_is_never_hidden() {
        let runner = orbital().failing(
            "bd dep tree orb-7 --direction=up --json",
            failing(FailureKind::Unavailable),
        );

        let snap = run(&one_project(), &runner, Filter::LiveAgents, now());

        assert!(
            snap.failed_projects.is_empty(),
            "the project's own tracker answered"
        );
        assert_eq!(snap.trees.len(), 1);
        assert_eq!(snap.trees[0].root, "orb-7");
        assert_eq!(
            snap.trees[0].tracker,
            TrackerState::Unreachable(TrackerFailure::Unavailable)
        );
        assert!(snap.hidden_trees.is_empty());
    }

    #[test]
    fn rows_bd_could_not_have_written_are_a_parse_failure_not_a_missing_tree() {
        let runner = orbital().with("bd dep tree orb-7 --direction=up --json", "[]");

        let snap = run(&one_project(), &runner, Filter::All, now());

        assert_eq!(
            snap.trees[0].tracker,
            TrackerState::Unreachable(TrackerFailure::Parse)
        );
    }

    #[test]
    fn bds_own_words_never_reach_the_snapshot() {
        let runner = orbital().failing(
            "bd list --status in_progress --limit 0 --json",
            RunFailure {
                kind: FailureKind::Auth,
                program: "bd".to_string(),
                detail: "Access denied for user 'orbital' at db.example.invalid:3306".to_string(),
            },
        );

        let snap = run(&one_project(), &runner, Filter::All, now());
        let json = serde_json::to_string(&snap).expect("the snapshot serialises");

        for leak in ["Access denied", "db.example.invalid", "'orbital'", "3306"] {
            assert!(!json.contains(leak), "{leak:?} survived into {json}");
        }
    }

    /// An empty readiness set is indistinguishable from nothing being ready,
    /// so a tracker that cannot answer must not leave one behind.
    #[test]
    fn a_tracker_that_cannot_answer_readiness_fails_rather_than_calling_every_bead_unready() {
        for call in ["bd ready --limit 0 --json", "bd blocked --json"] {
            let runner = orbital().failing(call, failing(FailureKind::Unavailable));

            let snap = run(&one_project(), &runner, Filter::All, now());

            assert!(snap.trees.is_empty(), "on {call}");
            assert_eq!(
                snap.failed_projects[0].tracker,
                TrackerFailure::Unavailable,
                "on {call}"
            );
        }
    }

    #[test]
    fn without_herdr_the_snapshot_says_so_and_still_draws_every_tree() {
        let runner = orbital().failing(
            "herdr agent list",
            RunFailure::exec("herdr", "no such session"),
        );

        let snap = run(&one_project(), &runner, Filter::LiveAgents, now());

        assert_eq!(snap.herdr, HerdrState::Unavailable);
        assert_eq!(snap.trees.len(), 1, "trees draw without liveness");
        assert!(snap.trees[0].nodes.iter().all(|n| n.agent.is_none()));
        assert!(snap.unattributed.is_empty());
    }

    #[test]
    fn a_tree_with_no_live_agent_is_reported_rather_than_dropped() {
        let runner = orbital().with("herdr agent list", r#"{"result":{"agents":[]}}"#);

        let filtered = run(&one_project(), &runner, Filter::LiveAgents, now());
        assert!(filtered.trees.is_empty());
        assert_eq!(filtered.hidden_trees.len(), 1);
        assert_eq!(filtered.hidden_trees[0].root, "orb-7");

        let all = run(&one_project(), &runner, Filter::All, now());
        assert_eq!(all.trees.len(), 1);
        assert!(all.hidden_trees.is_empty());
    }

    #[test]
    fn a_pane_on_no_bead_is_reported_under_the_project_it_sits_in() {
        let snap = run(&one_project(), &orbital(), Filter::LiveAgents, now());

        let loose: Vec<&str> = snap.unattributed.iter().map(|p| p.pane.as_str()).collect();
        assert_eq!(loose, vec!["w:p9"]);
        assert_eq!(snap.unattributed[0].project.as_deref(), Some("orbital"));
    }

    // ---- several projects at once --------------------------------------

    /// The join runs once over every project's rows, so a pane resolves
    /// against the tracker its directory sits in and no other.
    #[test]
    fn a_pane_joins_only_the_project_its_directory_sits_in() {
        let panes = r#"{"result":{"agents":[
          {"pane_id":"w:p1","cwd":"/srv/work/orbital","agent_status":"working",
           "display_agent":"x-1.1"}
        ]}}"#;
        let runner = colliding_trackers(panes);

        let snap = run(&two_projects(), &runner, Filter::All, now());

        assert!(
            node(tree_of(&snap, "orbital"), "x-1.1").agent.is_some(),
            "the pane's own project"
        );
        assert!(
            node(tree_of(&snap, "ferry"), "x-1.1").agent.is_none(),
            "the same id in a tracker the pane is nowhere near"
        );
    }

    #[test]
    fn each_project_reads_its_tracker_in_its_own_directory_with_its_own_credential() {
        let runner = colliding_trackers(r#"{"result":{"agents":[]}}"#);

        run(&two_projects(), &runner, Filter::All, now());

        let reads: Vec<(Option<PathBuf>, Option<String>)> = runner
            .calls()
            .iter()
            .filter(|c| c.argv == "bd dep tree x-1 --direction=up --json")
            .map(|c| (c.cwd.clone(), c.env.get("BEADS_DOLT_PASSWORD").cloned()))
            .collect();

        assert_eq!(
            reads,
            vec![
                (
                    Some(PathBuf::from(ORBITAL)),
                    Some("orbital-password".to_string())
                ),
                (
                    Some(PathBuf::from(FERRY)),
                    Some("ferry-password".to_string())
                ),
            ]
        );
    }

    fn colliding_trackers(panes: &str) -> FakeRunner {
        FakeRunner::default()
            .with("herdr agent list", panes)
            .with("sh -c secret orbital", "orbital-password")
            .with("sh -c secret ferry", "ferry-password")
            .with(
                "bd list --status in_progress --limit 0 --json",
                r#"[{"id":"x-1.1","title":"the colliding id","status":"in_progress"}]"#,
            )
            .with("bd list --status blocked --limit 0 --json", "[]")
            .with("bd ready --limit 0 --json", "[]")
            .with("bd blocked --json", "[]")
            .with("bd show x-1.1 --json", r#"[{"id":"x-1.1","parent":"x-1"}]"#)
            .with("bd show x-1 --json", r#"[{"id":"x-1","parent":null}]"#)
            .with("bd dep tree x-1 --direction=up --json", COLLIDING_TREE)
    }
}
