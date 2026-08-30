use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};

use crate::collect::herdr::Pane;
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
    readiness: Readiness,
    roots: Vec<(String, Result<Assembled, TrackerFailure>)>,
}

/// What a collection is asked to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wanted {
    /// Every project, as a run that has read nothing yet must.
    Everything,
    /// One project. Every other keeps what its tracker last said.
    Project(String),
}

impl Wanted {
    fn names(&self, project: &str) -> bool {
        match self {
            Wanted::Everything => true,
            Wanted::Project(named) => named == project,
        }
    }
}

/// What each project's tracker last said, kept between collections.
///
/// Reading a tracker is dozens of round trips; joining what came back is a
/// pass over rows already in hand. Keeping the reads is what lets a project a
/// change message named be read on its own, and the join then runs over the
/// standing set exactly as it runs over a set read all at once — so refreshing
/// one project and rebuilding everything agree by construction rather than by
/// argument.
#[derive(Default)]
pub struct Collection {
    read: BTreeMap<String, Result<ProjectWork, TrackerFailure>>,
}

impl Collection {
    /// Read what `wanted` names, and draw everything standing.
    pub fn collect(
        &mut self,
        cfg: &Config,
        runner: &dyn Runner,
        wanted: &Wanted,
        filter: Filter,
        now: DateTime<Utc>,
    ) -> Snapshot {
        // herdr is the second tier: without it there is no agent to join and
        // no filter to apply, and every tracker still reads.
        //
        // Read again however few projects `wanted` names: it is one local
        // call, the join it feeds is across every project, and a project with
        // a producer is never polled — so a refresh naming it is the only
        // chance the agent join gets.
        let (panes, herdr_state) = match herdr::agent_list(runner) {
            Ok(panes) => (panes, HerdrState::Ok),
            Err(_) => (Vec::new(), HerdrState::Unavailable),
        };

        for project in cfg.projects.iter().filter(|p| wanted.names(&p.name)) {
            let read = read_project(runner, project, cfg, &panes)
                .map_err(|failure| tracker_failure(failure.kind));
            self.read.insert(project.name.clone(), read);
        }

        self.draw(cfg, &panes, herdr_state, filter, now)
    }

    /// Everything standing, in config order, however much of it this
    /// collection just read.
    fn draw(
        &self,
        cfg: &Config,
        panes: &[Pane],
        herdr_state: HerdrState,
        filter: Filter,
        now: DateTime<Utc>,
    ) -> Snapshot {
        // One resolve over every project's rows at once. A pane names its bead
        // by id alone, and only the whole set tells a match from a prefix
        // collision.
        let rows: Vec<ProjectRows<'_>> = self
            .that_answered(cfg)
            .flat_map(|(project, work)| {
                work.roots.iter().filter_map(move |(_, read)| {
                    read.as_ref().ok().map(|assembled| ProjectRows {
                        project,
                        rows: &assembled.rows,
                    })
                })
            })
            .collect();
        let joined = &join::resolve(&rows, panes, &cfg.projects, &cfg.join);

        let trees = self
            .that_answered(cfg)
            .flat_map(|(project, work)| {
                work.roots.iter().map(move |(root, read)| match read {
                    Ok(assembled) => {
                        snapshot::build_tree(project, assembled, joined, &work.readiness, cfg, now)
                    }
                    Err(failure) => Tree::tracker_unreachable(project, root, *failure),
                })
            })
            .collect();

        let failed_projects = self
            .standing(cfg)
            .filter_map(|(project, read)| {
                read.as_ref().err().map(|failure| FailedProject {
                    project: project.to_string(),
                    tracker: *failure,
                })
            })
            .collect();

        snapshot::build(
            Collected {
                trees,
                failed_projects,
            },
            panes,
            joined,
            cfg,
            herdr_state,
            filter,
            now,
        )
    }

    /// What has been read, in the order the config names the projects. The
    /// order a snapshot draws in belongs to the config, not to how a
    /// collection happened to store what it read.
    fn standing<'a>(
        &'a self,
        cfg: &'a Config,
    ) -> impl Iterator<Item = (&'a str, &'a Result<ProjectWork, TrackerFailure>)> {
        cfg.projects
            .iter()
            .filter_map(|p| Some((p.name.as_str(), self.read.get(&p.name)?)))
    }

    /// The projects whose trackers answered, in the same order.
    fn that_answered<'a>(
        &'a self,
        cfg: &'a Config,
    ) -> impl Iterator<Item = (&'a str, &'a ProjectWork)> {
        self.standing(cfg)
            .filter_map(|(project, read)| Some((project, read.as_ref().ok()?)))
    }
}

/// Read every configured tracker and, where there is one, the herdr session,
/// and draw the result.
pub fn run(cfg: &Config, runner: &dyn Runner, filter: Filter, now: DateTime<Utc>) -> Snapshot {
    Collection::default().collect(cfg, runner, &Wanted::Everything, filter, now)
}

/// Everything one project's tracker is asked for. A failure before the roots
/// are known has no root to name, so it becomes the project's own failure
/// rather than a tree; a failure on one root afterwards is that root's.
fn read_project(
    runner: &dyn Runner,
    project: &Project,
    cfg: &Config,
    panes: &[Pane],
) -> Result<ProjectWork, RunFailure> {
    let env = bd::credential_env(runner, project, bd::ambient_credential().as_deref())?;
    let discovered = bd::discover_roots(runner, &project.path, &env, &cfg.roots.metadata_keys)?;

    // An empty readiness set reads as "nothing here is ready", so a tracker
    // that cannot answer must not leave one behind.
    let readiness = Readiness {
        ready: bd::ready_ids(runner, &project.path, &env)?,
        blocked_by: bd::blocked_by(runner, &project.path, &env)?,
    };

    let mut ancestors: BTreeMap<String, String> = BTreeMap::new();
    let mut roots: BTreeSet<String> = cfg
        .roots
        .explicit
        .get(&project.name)
        .into_iter()
        .flatten()
        .cloned()
        .collect();
    for bead in &discovered {
        roots.insert(root_of(runner, project, &env, &bead.id, &mut ancestors)?);
    }
    for named in panes_naming_a_bead_here(panes, project, cfg) {
        // Swallowed, and it has to be. `display_agent` is free text, and
        // bd exits non-zero on an id it does not hold with nothing to tell
        // that apart from a tracker that has stopped answering — so there
        // is no failure kind to discriminate on. Propagating would cost a
        // whole tracker every time a pane was labelled with a sentence.
        if let Ok(root) = root_of(runner, project, &env, named, &mut ancestors) {
            roots.insert(root);
        }
    }

    Ok(ProjectWork {
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

/// What the live panes in this project's directory name. A pane placed in no
/// configured project has no tracker to ask, and one placed in another
/// project names an id in that tracker's namespace, not this one's.
fn panes_naming_a_bead_here<'a>(
    panes: &'a [Pane],
    project: &'a Project,
    cfg: &'a Config,
) -> impl Iterator<Item = &'a str> {
    panes
        .iter()
        .filter(|pane| {
            join::project_of(&pane.cwd, &cfg.projects).is_some_and(|p| p.name == project.name)
        })
        .filter_map(|pane| pane.display_agent.as_deref())
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

/// The kinds bd's collector can produce, in the model's own vocabulary.
/// `RunFailure.detail` stops here: bd names the database and the SQL user
/// when it refuses a credential, and the words for a failure belong to
/// whatever draws it.
///
/// `Gone` and `Busy` are herdr's, and a tracker cannot answer with either.
/// `TrackerFailure` stays as it is rather than learning a word for a pane.
fn tracker_failure(kind: FailureKind) -> TrackerFailure {
    match kind {
        FailureKind::Auth => TrackerFailure::Auth,
        FailureKind::Unavailable | FailureKind::Gone | FailureKind::Busy => {
            TrackerFailure::Unavailable
        }
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

    /// Every way a collection can be asked for, named where a test that reads
    /// the sources can enumerate them.
    fn every_wanted() -> [&'static str; 2] {
        match Wanted::Everything {
            Wanted::Everything | Wanted::Project(_) => ["Wanted::Everything", "Wanted::Project"],
        }
    }

    /// The crate's own source, with each file's tests cut away.
    fn source_outside_tests(except: &str) -> String {
        let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut walking = vec![src];
        let mut read = String::new();

        while let Some(path) = walking.pop() {
            for entry in std::fs::read_dir(&path).expect("the crate's own source") {
                let found = entry.expect("a directory entry").path();
                if found.is_dir() {
                    walking.push(found);
                } else if found.extension().is_some_and(|kind| kind == "rs")
                    && found.file_name().is_some_and(|name| name != except)
                {
                    let text = std::fs::read_to_string(&found).expect("a source file");
                    read.push_str(text.split("\n#[cfg(test)]\n").next().unwrap_or_default());
                }
            }
        }
        read
    }

    /// Per-project refresh is built in `tui.rs` and only consumed here, so a
    /// change that emptied that file would leave this one compiling, every
    /// test passing, and `bdi` reading every tracker on every message. That
    /// happened, in `8b227ab`, and stood for an hour behind a green suite: a
    /// test cannot catch its own deletion, so this one lives beside the type
    /// rather than beside the code it guards.
    #[test]
    fn something_that_is_not_a_test_asks_for_each_kind_of_collection() {
        let source = source_outside_tests("app.rs");

        for wanted in every_wanted() {
            assert!(
                source.contains(wanted),
                "{wanted} is built nowhere but in tests"
            );
        }
    }

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

[roots.explicit]
orbital = ["orb-7", "orb-4"]
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

    /// The key is `(project, id)`: a root named in config belongs to one
    /// tracker, and no other is asked about an id it was never given. Asking
    /// them all drew a tree per project claiming a healthy tracker was
    /// unreachable.
    #[test]
    fn a_root_named_in_config_is_read_only_from_the_project_it_is_named_under() {
        let cfg = Config::from_toml(&format!(
            r#"
[[projects]]
name = "orbital"
path = "{ORBITAL}"
credential_command = "secret orbital"

[[projects]]
name = "ferry"
path = "{FERRY}"
credential_command = "secret ferry"

[roots.explicit]
orbital = ["orb-4"]
"#
        ))
        .expect("the config parses");
        let runner = colliding_trackers(r#"{"result":{"agents":[]}}"#)
            .with("bd dep tree orb-4 --direction=up --json", MAST_TREE);

        let snap = run(&cfg, &runner, Filter::All, now());

        let roots: Vec<(&str, &str)> = snap
            .trees
            .iter()
            .map(|t| (t.project.as_str(), t.root.as_str()))
            .collect();
        assert_eq!(
            roots,
            vec![("orbital", "orb-4"), ("orbital", "x-1"), ("ferry", "x-1")],
            "ferry draws no tree for a root orbital was given"
        );

        // `call` panics on a second invocation, so this is the assertion that
        // ferry's tracker was asked about orb-4 exactly no times.
        assert_eq!(
            runner.call("bd dep tree orb-4 --direction=up --json").cwd,
            Some(PathBuf::from(ORBITAL))
        );
    }

    // ---- discovery rule 4: a root only a live pane names ----------------

    /// The only root herdr contributes, and the reason it exists: an agent
    /// working off-tree still appears, on a bead no bd status and no
    /// configured key reached. Such a tree has a live agent by construction,
    /// so the live-agent filter can never be what hides it.
    #[test]
    fn a_bead_named_only_by_a_live_pane_becomes_a_root() {
        let runner = orbital()
            .with(
                "herdr agent list",
                r#"{"result":{"agents":[
                  {"pane_id":"w:p1","cwd":"/srv/work/orbital","agent_status":"working"},
                  {"pane_id":"w:p4","cwd":"/srv/work/orbital","agent_status":"working",
                   "display_agent":"orb-4"}
                ]}}"#,
            )
            .with("bd show orb-4 --json", r#"[{"id":"orb-4","parent":null}]"#)
            .with("bd dep tree orb-4 --direction=up --json", MAST_TREE);

        let snap = run(&one_project(), &runner, Filter::LiveAgents, now());

        let roots: Vec<&str> = snap.trees.iter().map(|t| t.root.as_str()).collect();
        assert_eq!(
            roots,
            vec!["orb-4", "orb-7"],
            "the pane's bead joins the roots bd's own statuses found"
        );
        assert!(snap.hidden_trees.is_empty());
        assert!(node(tree_of(&snap, "orbital"), "orb-4").agent.is_some());
    }

    /// `display_agent` is free text, so reading it as a bead id is a guess.
    /// bd exits non-zero on an id it does not hold, saying nothing that tells
    /// it apart from a tracker that has stopped answering — so rule 4 cannot
    /// discriminate on the kind and swallows the lookup's failure whole. A
    /// pane labelled with a sentence belongs in `unattributed`, and taking
    /// the whole tracker down for one is the opposite of degrading.
    #[test]
    fn a_pane_labelled_with_something_that_is_not_a_bead_costs_the_project_nothing() {
        let runner = orbital()
            .with(
                "herdr agent list",
                r#"{"result":{"agents":[
                  {"pane_id":"w:p4","cwd":"/srv/work/orbital","agent_status":"working",
                   "display_agent":"reviewing the docs"}
                ]}}"#,
            )
            .failing(
                "bd show reviewing the docs --json",
                failing(FailureKind::Unavailable),
            );

        let snap = run(&one_project(), &runner, Filter::All, now());

        assert!(
            snap.failed_projects.is_empty(),
            "a mislabelled pane is not a tracker outage"
        );
        let roots: Vec<&str> = snap.trees.iter().map(|t| t.root.as_str()).collect();
        assert_eq!(roots, vec!["orb-7"], "rules 1 to 3 are untouched");
        let loose: Vec<&str> = snap.unattributed.iter().map(|p| p.pane.as_str()).collect();
        assert_eq!(loose, vec!["w:p4"], "the pane is reported, not dropped");
    }

    #[test]
    fn a_pane_naming_a_bead_discovery_already_walked_costs_no_second_climb() {
        let runner = orbital().with(
            "herdr agent list",
            r#"{"result":{"agents":[
              {"pane_id":"w:p1","cwd":"/srv/work/orbital","agent_status":"working",
               "display_agent":"orb-7.1"}
            ]}}"#,
        );

        let snap = run(&one_project(), &runner, Filter::All, now());

        assert_eq!(snap.trees.len(), 1);
        // `call` panics on a second invocation, which is the assertion.
        runner.call("bd show orb-7.1 --json");
    }

    /// The pane names a bead, not a root. What joins the root set is the top
    /// of that bead's parent-child chain, so a pane sitting on a task deep in
    /// a tree draws the tree rather than a stray one-node root beside it.
    #[test]
    fn a_pane_naming_a_bead_inside_a_tree_contributes_that_tree_not_the_bead() {
        let runner = orbital()
            .with(
                "herdr agent list",
                r#"{"result":{"agents":[
                  {"pane_id":"w:p4","cwd":"/srv/work/orbital","agent_status":"working",
                   "display_agent":"orb-7.2"}
                ]}}"#,
            )
            .with(
                "bd show orb-7.2 --json",
                r#"[{"id":"orb-7.2","parent":"orb-7"}]"#,
            );

        let snap = run(&one_project(), &runner, Filter::All, now());

        // The fake panics on a call it has no response for, so no `bd dep
        // tree orb-7.2` is half of this assertion.
        let roots: Vec<&str> = snap.trees.iter().map(|t| t.root.as_str()).collect();
        assert_eq!(
            roots,
            vec!["orb-7"],
            "climbed to its root, and deduped there"
        );
    }

    /// A pane names its bead by id alone, and prefixes are uncoordinated
    /// across trackers — so the root it contributes belongs to the project
    /// its directory sits in, and no other tracker is asked about the id.
    #[test]
    fn a_pane_contributes_its_root_only_to_the_project_it_sits_in() {
        let runner = colliding_trackers(
            r#"{"result":{"agents":[
              {"pane_id":"w:p4","cwd":"/srv/work/orbital","agent_status":"working",
               "display_agent":"orb-4"}
            ]}}"#,
        )
        .with("bd show orb-4 --json", r#"[{"id":"orb-4","parent":null}]"#)
        .with("bd dep tree orb-4 --direction=up --json", MAST_TREE);

        let snap = run(&two_projects(), &runner, Filter::All, now());

        let roots: Vec<(&str, &str)> = snap
            .trees
            .iter()
            .map(|t| (t.project.as_str(), t.root.as_str()))
            .collect();
        assert_eq!(
            roots,
            vec![("orbital", "orb-4"), ("orbital", "x-1"), ("ferry", "x-1")]
        );

        // `call` panics on a second invocation, so this is also the assertion
        // that ferry was never asked about an id no pane of its own named.
        assert_eq!(
            runner.call("bd show orb-4 --json").cwd,
            Some(PathBuf::from(ORBITAL))
        );
    }

    /// A pane has to resolve to a project before the id it names means
    /// anything, because there is no tracker to ask otherwise.
    #[test]
    fn a_pane_under_no_configured_project_contributes_no_root() {
        let runner = orbital().with(
            "herdr agent list",
            r#"{"result":{"agents":[
              {"pane_id":"w:p4","cwd":"/srv/elsewhere","agent_status":"working",
               "display_agent":"orb-4"}
            ]}}"#,
        );

        let snap = run(&one_project(), &runner, Filter::All, now());

        // The fake panics on a call it has no response for, so `bd show
        // orb-4` never being made is what lets this reach its assertions.
        let roots: Vec<&str> = snap.trees.iter().map(|t| t.root.as_str()).collect();
        assert_eq!(roots, vec!["orb-7"]);
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

    // ---- reading one project at a time ---------------------------------

    /// A pane on a bead in one project and a session on none in the other,
    /// so the join has both directions to do across both trackers.
    const PANES_IN_BOTH: &str = r#"{"result":{"agents":[
      {"pane_id":"w:p1","cwd":"/srv/work/orbital","agent_status":"working","display_agent":"x-1.1"},
      {"pane_id":"w:p2","cwd":"/srv/work/ferry","agent_status":"idle"}
    ]}}"#;

    fn collect(collection: &mut Collection, runner: &dyn Runner, wanted: &Wanted) -> Snapshot {
        collection.collect(&two_projects(), runner, wanted, Filter::All, now())
    }

    fn orbital_alone() -> Wanted {
        Wanted::Project("orbital".to_string())
    }

    /// What one project's tracker was asked, however it was reached.
    fn tracker_calls(runner: &FakeRunner, path: &str) -> usize {
        runner
            .calls()
            .iter()
            .filter(|c| c.cwd == Some(PathBuf::from(path)))
            .count()
    }

    fn trees_of<'a>(snap: &'a Snapshot, project: &str) -> Vec<&'a Tree> {
        snap.trees.iter().filter(|t| t.project == project).collect()
    }

    /// The whole of the split, and the thing it would be worst to get wrong:
    /// reading one project on its own and rebuilding everything must not be
    /// able to disagree about what is on the screen.
    #[test]
    fn refreshing_one_project_gives_the_snapshot_a_whole_rebuild_would_have() {
        let runner = colliding_trackers(PANES_IN_BOTH);
        let mut standing = Collection::default();
        collect(&mut standing, &runner, &Wanted::Everything);

        let refreshed = collect(&mut standing, &runner, &orbital_alone());
        let rebuilt = collect(&mut Collection::default(), &runner, &Wanted::Everything);

        assert_eq!(refreshed, rebuilt);
    }

    /// The saving the change channel exists for: a project nothing said had
    /// changed is not read again.
    #[test]
    fn refreshing_one_project_asks_no_other_projects_tracker() {
        let runner = colliding_trackers(PANES_IN_BOTH);
        let mut standing = Collection::default();
        collect(&mut standing, &runner, &Wanted::Everything);
        let (orbital, ferry) = (
            tracker_calls(&runner, ORBITAL),
            tracker_calls(&runner, FERRY),
        );

        collect(&mut standing, &runner, &orbital_alone());

        assert_eq!(
            tracker_calls(&runner, FERRY),
            ferry,
            "ferry was not named, so its tracker was not asked again"
        );
        assert!(
            tracker_calls(&runner, ORBITAL) > orbital,
            "orbital was named, so it was read"
        );
    }

    /// Degrade, never disappear, with further to reach than the whole-snapshot
    /// path ever had to: the projects already drawn are not the ones being
    /// read, so a tracker that fails mid-refresh cannot cost them anything.
    #[test]
    fn a_tracker_that_fails_while_one_project_refreshes_leaves_the_others_drawn() {
        let mut standing = Collection::default();
        let before = collect(
            &mut standing,
            &colliding_trackers(PANES_IN_BOTH),
            &Wanted::Everything,
        );

        let refused = colliding_trackers(PANES_IN_BOTH).failing(
            "bd list --status in_progress --limit 0 --json",
            failing(FailureKind::Auth),
        );
        let after = collect(&mut standing, &refused, &orbital_alone());

        assert_eq!(
            after.failed_projects,
            vec![FailedProject {
                project: "orbital".to_string(),
                tracker: TrackerFailure::Auth,
            }]
        );
        assert!(
            trees_of(&after, "orbital").is_empty(),
            "orbital's trees went with the tracker that could not be read"
        );
        assert_eq!(
            trees_of(&after, "ferry"),
            trees_of(&before, "ferry"),
            "ferry is exactly what it was before orbital's outage"
        );
    }

    /// A project something reports for is never polled, so a refresh naming
    /// it is the only chance the agent join gets. The herdr session is read
    /// again whatever a refresh names — it is one local call, and the join it
    /// feeds runs across every project by design. What a refresh leaves alone
    /// is the trackers it did not name, not the panes.
    #[test]
    fn a_refresh_naming_one_project_still_reads_the_herdr_session() {
        let mut standing = Collection::default();
        let before = collect(
            &mut standing,
            &colliding_trackers(r#"{"result":{"agents":[]}}"#),
            &Wanted::Everything,
        );
        assert!(node(tree_of(&before, "ferry"), "x-1.1").agent.is_none());

        let arrived = colliding_trackers(
            r#"{"result":{"agents":[
              {"pane_id":"w:p2","cwd":"/srv/work/ferry","agent_status":"working",
               "display_agent":"x-1.1"}
            ]}}"#,
        );
        let after = collect(&mut standing, &arrived, &orbital_alone());

        assert!(
            node(tree_of(&after, "ferry"), "x-1.1").agent.is_some(),
            "the pane reached the project the refresh did not name"
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
