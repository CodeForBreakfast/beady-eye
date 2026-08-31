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
use crate::model::types::Pane;

/// One project's roots in id order, each either read or unreadable.
struct ProjectWork {
    readiness: Readiness,
    roots: Vec<(String, Result<Assembled, TrackerFailure>)>,
}

/// What one project's tracker last said, and when it said it.
///
/// The time belongs to the read rather than to the collection that drew it:
/// a collection naming one project leaves every other project's `Read`
/// untouched, which is how a snapshot can say how fresh each project is
/// rather than only when it was assembled.
struct Read {
    at: DateTime<Utc>,
    work: Result<ProjectWork, TrackerFailure>,
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
    /// Whether a collection asked for this names one project.
    ///
    /// Public because the screen asks it too: what a project line says is
    /// being read now is decided by the very predicate the collector decides
    /// what to read with, so the two agree by construction rather than by
    /// argument.
    pub fn names(&self, project: &str) -> bool {
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
    read: BTreeMap<String, Read>,
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
            let work = read_project(runner, project, cfg, &panes)
                .map_err(|failure| tracker_failure(failure.kind));
            self.read
                .insert(project.name.clone(), Read { at: now, work });
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
                read.work.as_ref().err().map(|failure| FailedProject {
                    project: project.to_string(),
                    tracker: *failure,
                })
            })
            .collect();

        let read_at = self
            .standing(cfg)
            .map(|(project, read)| (project.to_string(), read.at))
            .collect();

        snapshot::build(
            Collected {
                trees,
                failed_projects,
                read_at,
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
    fn standing<'a>(&'a self, cfg: &'a Config) -> impl Iterator<Item = (&'a str, &'a Read)> {
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
            .filter_map(|(project, read)| Some((project, read.work.as_ref().ok()?)))
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
    let env = bd::tracker_env(runner, project, bd::ambient_credential().as_deref())?;
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
    for bead in discovered.keys() {
        roots.insert(root_of(
            runner,
            project,
            &env,
            bead,
            &discovered,
            &mut ancestors,
        )?);
    }
    for named in panes_naming_a_bead_here(panes, project, cfg) {
        // Swallowed, and it has to be. `display_agent` is free text, and
        // bd exits non-zero on an id it does not hold with nothing to tell
        // that apart from a tracker that has stopped answering — so there
        // is no failure kind to discriminate on. Propagating would cost a
        // whole tracker every time a pane was labelled with a sentence.
        if let Ok(root) = root_of(runner, project, &env, named, &discovered, &mut ancestors) {
            roots.insert(root);
        }
    }

    let beads = bd::all_beads(runner, &project.path, &env)?;

    Ok(ProjectWork {
        readiness,
        roots: roots
            .into_iter()
            .map(|root| {
                let read = assemble(beads.clone(), &root).map_err(|_| TrackerFailure::Parse);
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

/// The top of a bead's parent-child chain.
///
/// `bd dep tree` cannot answer this: `--direction=up` walks dependents, so
/// whatever bead it is asked about comes back as its own root. Discovery can,
/// for everything it saw — `parents` is what it brought back. What it did not
/// see costs a `bd show` each: a closed bead above open children, which is the
/// normal healthy shape of this tracker, and a bead a pane named. `ancestors`
/// carries what earlier walks found, so those cost one call each rather than
/// one per level each.
fn root_of(
    runner: &dyn Runner,
    project: &Project,
    env: &Env,
    id: &str,
    parents: &BTreeMap<String, Option<String>>,
    ancestors: &mut BTreeMap<String, String>,
) -> Result<String, RunFailure> {
    let mut climbed: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut current = id.to_string();
    // The last id the tracker has confirmed is a bead, which is what a
    // refused ancestor falls back to.
    let mut reached: Option<String> = None;

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
        let parent = match parents.get(&current) {
            Some(known) => known.clone(),
            None => match bd::parent_of(runner, &project.path, env, &current) {
                Ok(parent) => parent,
                // Above the first step every id came from a bead that named it
                // as its parent, so a refusal there is a parent the tracker no
                // longer holds: one bead's chain that stops early, not a
                // tracker that has gone away. bd answers the two the same way,
                // and the calls around this walk are what catch the second —
                // `all_beads` runs on the next line and propagates.
                Err(failure) => match &reached {
                    Some(reached) => break reached.clone(),
                    // Nothing has confirmed the id this was asked about is a
                    // bead at all: a pane's `display_agent` is free text, and
                    // bd answers a sentence exactly as it answers a bead it
                    // has lost.
                    None => return Err(failure),
                },
            },
        };
        reached = Some(current.clone());
        match parent {
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

    /// The crate's own source, with each file's tests cut away and this file
    /// left out of it.
    ///
    /// This file is left out because it builds both kinds of `Wanted` itself,
    /// in `Wanted::names` and in `run`. Counting it would satisfy the
    /// assertion below with this file's own source and stop guarding anything.
    ///
    /// It names itself through `file!()` rather than by a spelling, and
    /// checks that it found itself. A spelling stops matching the moment the
    /// file is renamed or becomes a directory module — and the assertion
    /// would then pass for the wrong reason with nothing on screen to say so.
    fn source_outside_tests() -> String {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let itself = root.join(file!());
        let mut walking = vec![root.join("src")];
        let mut read = String::new();
        let mut left_itself_out = false;

        while let Some(path) = walking.pop() {
            for entry in std::fs::read_dir(&path).expect("the crate's own source") {
                let found = entry.expect("a directory entry").path();
                if found.is_dir() {
                    walking.push(found);
                } else if found.extension().is_some_and(|kind| kind == "rs") {
                    if found == itself {
                        left_itself_out = true;
                        continue;
                    }
                    let text = std::fs::read_to_string(&found).expect("a source file");
                    read.push_str(text.split("\n#[cfg(test)]\n").next().unwrap_or_default());
                }
            }
        }

        assert!(
            left_itself_out,
            "{} was never met while walking the source, so this file was \
             read into its own assertion and the check below proves nothing",
            itself.display()
        );
        read
    }

    /// Per-project refresh is built in `tui/mod.rs` and only consumed here, so a
    /// change that emptied that file would leave this one compiling, every
    /// test passing, and `bdi` reading every tracker on every message. That
    /// happened, in `8b227ab`, and stood for an hour behind a green suite: a
    /// test cannot catch its own deletion, so this one lives beside the type
    /// rather than beside the code it guards.
    #[test]
    fn something_that_is_not_a_test_asks_for_each_kind_of_collection() {
        let source = source_outside_tests();

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
      {"id":"orb-7","title":"lift the ground station","status":"in_progress",
       "priority":1,"issue_type":"epic"},
      {"id":"orb-7.1","title":"re-point the dish","status":"in_progress",
       "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
       "priority":2,"issue_type":"task",
       "metadata":{"agent_pane":"w:p1"}},
      {"id":"orb-7.2","title":"lay the feeder cable","status":"open",
       "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
       "priority":2,"issue_type":"task"}
    ]"#;

    /// A second root, reached only because config names it.
    const MAST_TREE: &str = r#"[
      {"id":"orb-4","title":"survey the mast","status":"open",
       "priority":2,"issue_type":"task"}
    ]"#;

    /// Two trackers that chose the same id prefix, which no one coordinates.
    const COLLIDING_TREE: &str = r#"[
      {"id":"x-1","title":"the shared prefix","status":"in_progress",
       "priority":1,"issue_type":"epic"},
      {"id":"x-1.1","title":"the colliding id","status":"in_progress",
       "dependencies":[{"depends_on_id":"x-1","type":"parent-child"}],
       "priority":2,"issue_type":"task"}
    ]"#;

    /// `w:p1` is on a bead; `w:p9` is a session on none.
    const PANES: &str = r#"{"result":{"agents":[
      {"pane_id":"w:p1","cwd":"/srv/work/orbital","agent_status":"working","title":"the dish"},
      {"pane_id":"w:p9","cwd":"/srv/work/orbital","agent_status":"idle"}
    ]}}"#;

    /// A bd call as the runner spells it: the tracker named outright, and
    /// writes refused. Two projects now differ in their argv as well as their
    /// directory, so each is staged for the tracker it reads.
    fn spelled_in(tracker: &str, subcommand: &str) -> String {
        format!("bd -C {tracker} --readonly {subcommand}")
    }

    /// The same, for the single project most of these tests read.
    fn spelled(subcommand: &str) -> String {
        spelled_in(ORBITAL, subcommand)
    }

    /// The direnv call that reproduces entering a project's directory.
    fn entering(tracker: &str) -> String {
        format!("direnv exec {tracker} env -0")
    }

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

    /// The one call a project's whole forest is drawn from, spelled as bd
    /// takes it.
    const TRACKER_CALL: &str = "list --all --limit 0 --json";

    /// The one call discovery makes for statuses, spelled as bd takes it.
    const UNFINISHED_CALL: &str =
        "list --status open,in_progress,blocked,deferred --limit 0 --json";

    /// The same two questions asked of bd's ephemeral table, which `bd list`
    /// does not read.
    const WISP_CALL: &str = "query ephemeral=true --all --limit 0 --json";
    const UNFINISHED_WISP_CALL: &str = "query ephemeral=true --limit 0 --json";

    /// Every call a healthy single-project run makes. Discovery names each
    /// bead's own parent, so a healthy run climbs nothing.
    fn orbital() -> FakeRunner {
        FakeRunner::default()
            .with("herdr agent list", PANES)
            .with(&entering(ORBITAL), "")
            .with(&spelled(UNFINISHED_CALL),
                r#"[{"id":"orb-7","title":"lift the ground station","status":"in_progress","parent":""},
                    {"id":"orb-7.1","title":"re-point the dish","status":"in_progress","parent":"orb-7"},
                    {"id":"orb-7.2","title":"lay the feeder cable","status":"open","parent":"orb-7"}]"#,
            )
            .with(
                &spelled("list --has-metadata-key working_topic --limit 0 --json"),
                "[]",
            )
            .with(
                &spelled("ready --limit 0 --json"),
                r#"[{"id":"orb-7.2","title":"lay the feeder cable","status":"open"}]"#,
            )
            .with(
                &spelled("blocked --json"),
                r#"[{"id":"orb-7.1","blocked_by":["orb-9"]}]"#,
            )
            .with(&spelled(TRACKER_CALL), ORBITAL_TREE)
            .with(&spelled(WISP_CALL), "[]")
            .with(&spelled(UNFINISHED_WISP_CALL), "[]")
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
            .with(&spelled(UNFINISHED_CALL), "[]")
            .with(
                &spelled("list --has-metadata-key working_topic --limit 0 --json"),
                r#"[{"id":"orb-7.1","title":"re-point the dish","status":"open","parent":"orb-7"}]"#,
            )
            .with(&spelled("show orb-7 --json"), r#"[{"id":"orb-7","parent":null}]"#);

        let snap = run(&one_project(), &runner, Filter::All, now());

        assert_eq!(snap.trees.len(), 1);
        assert_eq!(snap.trees[0].root, "orb-7");
    }

    /// bd exits non-zero on an id it does not hold, so the walk to a root
    /// fails at a parent that was deleted — and `read_project` carries that
    /// failure out, taking every other root in the tracker with it.
    ///
    /// Measured against a live tracker on 2026-08-31: `bd show <missing-id>
    /// --json` exits 1 saying `no issues found matching the provided IDs`,
    /// which matches none of the classifier's phrases and so arrives as
    /// `Unavailable` — the same kind a server that is down produces. That is
    /// why the walk cannot simply swallow the failure.
    #[test]
    fn a_deleted_parent_costs_its_own_bead_rather_than_the_whole_tracker() {
        let orphan_row = r#"[{"id":"orb-7.9","title":"its parent was deleted",
                              "status":"open","parent":"orb-404"}]"#;
        let orphan_bead = r#"[{"id":"orb-7.9","title":"its parent was deleted",
                               "status":"open",
                               "dependencies":[{"depends_on_id":"orb-404","type":"parent-child"}],
                               "priority":2,"issue_type":"task"}]"#;
        let runner = orbital()
            .merging(&spelled(UNFINISHED_CALL), orphan_row)
            .merging(&spelled(TRACKER_CALL), orphan_bead)
            .failing(
                &spelled("show orb-404 --json"),
                failing(FailureKind::Unavailable),
            );

        let snap = run(&one_project(), &runner, Filter::All, now());

        assert!(
            snap.failed_projects.is_empty(),
            "one bead bd cannot place must not take the tracker down: {:?}",
            snap.failed_projects
        );
        let roots: Vec<&str> = snap.trees.iter().map(|t| t.root.as_str()).collect();
        assert_eq!(
            roots,
            vec!["orb-7", "orb-7.9"],
            "the readable roots are drawn, and the orphan is one of them"
        );
    }

    /// Discovery brings each bead's own parent back with it, so the walk to a
    /// root is answered from rows already in hand.
    #[test]
    fn a_run_whose_ancestors_discovery_saw_climbs_nothing() {
        let runner = orbital();

        run(&one_project(), &runner, Filter::All, now());

        // Every `bd show` is a level someone had to climb.
        let climbed: Vec<String> = runner
            .calls()
            .into_iter()
            .map(|call| call.argv)
            .filter(|argv| argv.starts_with(&spelled("show ")))
            .collect();
        assert!(climbed.is_empty(), "climbed {climbed:?}");
    }

    /// A closed bead above unfinished children is the normal healthy shape of
    /// this tree, and discovery never sees one — so `bd show` still has to
    /// reach it, once however many beads share it.
    #[test]
    fn a_closed_ancestor_is_climbed_to_once_however_many_beads_share_it() {
        let runner = orbital()
            .with(&spelled(UNFINISHED_CALL),
                r#"[{"id":"orb-7.1","title":"re-point the dish","status":"in_progress","parent":"orb-7"},
                    {"id":"orb-7.2","title":"lay the feeder cable","status":"open","parent":"orb-7"}]"#,
            )
            .with(&spelled("show orb-7 --json"), r#"[{"id":"orb-7","parent":null}]"#);

        let snap = run(&one_project(), &runner, Filter::All, now());

        assert_eq!(snap.trees[0].root, "orb-7");
        // `call` panics on a second invocation, which is the assertion.
        runner.call(&spelled("show orb-7 --json"));
    }

    /// The defect this rule replaces. Every seat stood down, so nothing was
    /// `in_progress` or `blocked`, so discovery found no bead, so no root, so
    /// the effort was not drawn at all — and whichever epic held the one bead
    /// still claimed was drawn in its place.
    #[test]
    fn an_effort_is_drawn_from_the_open_work_under_it_with_nobody_on_it() {
        let runner = orbital()
            .with("herdr agent list", r#"{"result":{"agents":[]}}"#)
            .with(&spelled(UNFINISHED_CALL),
                r#"[{"id":"orb-7","title":"lift the ground station","status":"open","parent":""},
                    {"id":"orb-7.2","title":"lay the feeder cable","status":"open","parent":"orb-7"}]"#,
            );

        let snap = run(&one_project(), &runner, Filter::All, now());

        let roots: Vec<&str> = snap.trees.iter().map(|t| t.root.as_str()).collect();
        assert_eq!(roots, vec!["orb-7"]);
    }

    /// A parent chain that loops has no top. Stopping where it repeats keeps
    /// the bead visible rather than hanging on it.
    #[test]
    fn a_parent_chain_that_loops_stops_where_it_repeats() {
        let runner = orbital()
            .with(&spelled(UNFINISHED_CALL),
                r#"[{"id":"orb-7.1","title":"re-point the dish","status":"in_progress","parent":"orb-7"}]"#,
            )
            .with(
                &spelled("show orb-7 --json"),
                r#"[{"id":"orb-7","parent":"orb-7.1"}]"#,
            )
            .with(&spelled(TRACKER_CALL),
                r#"[{"id":"orb-7.1","title":"re-point the dish","status":"in_progress",
                     "priority":2,"issue_type":"task"}]"#,
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
        let runner = orbital().merging(&spelled(TRACKER_CALL), MAST_TREE);

        let snap = run(&cfg, &runner, Filter::All, now());

        let roots: Vec<&str> = snap.trees.iter().map(|t| t.root.as_str()).collect();
        assert_eq!(
            roots,
            vec!["orb-7", "orb-4"],
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
            .merging(&spelled(TRACKER_CALL), MAST_TREE);

        let snap = run(&cfg, &runner, Filter::All, now());

        let roots: Vec<(&str, &str)> = snap
            .trees
            .iter()
            .map(|t| (t.project.as_str(), t.root.as_str()))
            .collect();
        assert_eq!(
            roots,
            vec![("orbital", "x-1"), ("orbital", "orb-4"), ("ferry", "x-1")],
            "ferry draws no tree for a root orbital was given"
        );

        // A tracker is read whole rather than per root, so no call names a
        // bead id at all — which is the stronger form of the same guarantee.
        assert!(
            runner.calls().iter().all(|c| !c.argv.contains("orb-4")),
            "no tracker was asked about an id it was never given"
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
            .with(
                &spelled("show orb-4 --json"),
                r#"[{"id":"orb-4","parent":null}]"#,
            )
            .merging(&spelled(TRACKER_CALL), MAST_TREE);

        let snap = run(&one_project(), &runner, Filter::LiveAgents, now());

        let roots: Vec<&str> = snap.trees.iter().map(|t| t.root.as_str()).collect();
        assert_eq!(
            roots,
            vec!["orb-7", "orb-4"],
            "the pane's bead joins the roots bd's own statuses found"
        );
        assert!(snap.hidden_trees.is_empty());
        let of_the_pane = snap
            .trees
            .iter()
            .find(|t| t.root == "orb-4")
            .expect("the pane's bead roots a tree");
        assert!(node(of_the_pane, "orb-4").agent.is_some());
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
                &spelled("show reviewing the docs --json"),
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
        let runner = orbital()
            .with(
                "herdr agent list",
                r#"{"result":{"agents":[
                  {"pane_id":"w:p1","cwd":"/srv/work/orbital","agent_status":"working",
                   "display_agent":"orb-7.1"}
                ]}}"#,
            )
            .with(&spelled(UNFINISHED_CALL),
                r#"[{"id":"orb-7.1","title":"re-point the dish","status":"in_progress","parent":"orb-7"}]"#,
            )
            .with(&spelled("show orb-7 --json"), r#"[{"id":"orb-7","parent":null}]"#);

        let snap = run(&one_project(), &runner, Filter::All, now());

        assert_eq!(snap.trees.len(), 1);
        // `call` panics on a second invocation, which is the assertion.
        runner.call(&spelled("show orb-7 --json"));
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
                   "display_agent":"orb-7.3"}
                ]}}"#,
            )
            // Closed, so discovery never saw it — a seat writing up the bead
            // it has just finished still sits on one.
            .with(
                &spelled("show orb-7.3 --json"),
                r#"[{"id":"orb-7.3","parent":"orb-7"}]"#,
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
        .with(
            &spelled("show orb-4 --json"),
            r#"[{"id":"orb-4","parent":null}]"#,
        )
        .merging(&spelled(TRACKER_CALL), MAST_TREE);

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
            runner.call(&spelled("show orb-4 --json")).cwd,
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
        let runner = orbital().failing(&spelled(UNFINISHED_CALL), failing(FailureKind::Auth));

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
            let runner = orbital().failing(&spelled(UNFINISHED_CALL), failing(kind));

            let snap = run(&one_project(), &runner, Filter::All, now());

            assert_eq!(snap.failed_projects[0].tracker, expected, "on {kind:?}");
        }
    }

    /// A root we already know the id of keeps it, which is what tells two
    /// failures apart, and the filter has no agent count to hide it by.
    #[test]
    fn a_root_the_answer_does_not_hold_keeps_its_id_and_is_never_hidden() {
        let runner = orbital().with(&spelled(TRACKER_CALL), MAST_TREE);

        let snap = run(&one_project(), &runner, Filter::LiveAgents, now());

        assert!(
            snap.failed_projects.is_empty(),
            "the project's own tracker answered"
        );
        assert_eq!(snap.trees.len(), 1);
        assert_eq!(snap.trees[0].root, "orb-7");
        assert_eq!(
            snap.trees[0].tracker,
            TrackerState::Unreachable(TrackerFailure::Parse)
        );
        assert!(snap.hidden_trees.is_empty());
    }

    /// A project's whole forest is drawn from one read, so that read failing
    /// is the project's failure and not any one root's. It is named with its
    /// reason rather than drawn empty, and the panes working in it are still
    /// recovered.
    #[test]
    fn the_one_tracker_read_failing_takes_the_project_down_by_name() {
        let runner = orbital().failing(&spelled(TRACKER_CALL), failing(FailureKind::Unavailable));

        let snap = run(&one_project(), &runner, Filter::LiveAgents, now());

        assert_eq!(
            snap.failed_projects,
            vec![FailedProject {
                project: "orbital".to_string(),
                tracker: TrackerFailure::Unavailable,
            }]
        );
        assert!(snap.trees.is_empty());
        assert!(snap.hidden_trees.is_empty());
        assert_eq!(
            snap.unattributed
                .iter()
                .map(|pane| pane.pane.as_str())
                .collect::<Vec<&str>>(),
            vec!["w:p1", "w:p9"],
            "the panes working in it are recovered rather than lost with it"
        );
    }

    #[test]
    fn rows_bd_could_not_have_written_are_a_parse_failure_not_a_missing_tree() {
        let runner = orbital().with(&spelled(TRACKER_CALL), "[]");

        let snap = run(&one_project(), &runner, Filter::All, now());

        assert_eq!(
            snap.trees[0].tracker,
            TrackerState::Unreachable(TrackerFailure::Parse)
        );
    }

    #[test]
    fn bds_own_words_never_reach_the_snapshot() {
        let runner = orbital().failing(
            &spelled(UNFINISHED_CALL),
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
        for call in [
            &spelled("ready --limit 0 --json"),
            &spelled("blocked --json"),
        ] {
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
        assert_eq!(snap.unattributed[0].project, "orbital");
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
            .filter(|c| c.argv.ends_with(TRACKER_CALL))
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

    /// The view draws one project line over each run of a project's trees, so
    /// a project split across two runs would draw two lines for it and a
    /// second run would look like a second project.
    ///
    /// The order is the config's because the loop is over `cfg.projects` and
    /// not over the `BTreeMap` beside it — iterating the map would keep every
    /// project's trees together and quietly re-order the projects themselves
    /// into alphabetical, which nothing else on the screen would show.
    #[test]
    fn every_projects_trees_arrive_together_and_in_the_order_the_config_names() {
        let snap = collect(
            &mut Collection::default(),
            &colliding_trackers(PANES_IN_BOTH),
            &Wanted::Everything,
        );

        let runs: Vec<&str> = snap
            .trees
            .chunk_by(|a, b| a.project == b.project)
            .map(|run| run[0].project.as_str())
            .collect();

        let distinct: BTreeSet<&str> = runs.iter().copied().collect();

        assert_eq!(runs, ["orbital", "ferry"], "{:#?}", snap.trees);
        assert_eq!(
            runs.len(),
            distinct.len(),
            "a project in two runs draws two project lines: {:#?}",
            snap.trees
        );
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

        let refused = colliding_trackers(PANES_IN_BOTH)
            .failing(&spelled(UNFINISHED_CALL), failing(FailureKind::Auth));
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

    /// `generated_at` is when a snapshot was drawn, and a refresh naming one
    /// project draws every project — including the ones it did not read. So
    /// the freshness of a project's rows is its own fact, and reading it off
    /// the snapshot's own clock would date rows nothing touched to a read
    /// that never happened.
    #[test]
    fn a_refresh_naming_one_project_dates_that_project_and_leaves_the_rest_alone() {
        let runner = colliding_trackers(PANES_IN_BOTH);
        let mut standing = Collection::default();
        let cfg = two_projects();
        let earlier = now();
        let later = earlier + chrono::Duration::seconds(30);

        standing.collect(&cfg, &runner, &Wanted::Everything, Filter::All, earlier);
        let after = standing.collect(&cfg, &runner, &orbital_alone(), Filter::All, later);

        assert_eq!(
            after.read_at,
            BTreeMap::from([
                ("orbital".to_string(), later),
                ("ferry".to_string(), earlier),
            ]),
            "only the project the refresh named was read again"
        );
        assert_eq!(
            after.generated_at, later,
            "the snapshot is still drawn at the later instant"
        );
    }

    /// A read that failed is stamped like any other, because a tracker that
    /// refused takes its trees down with it — `after` has no rows of
    /// orbital's left to be stale. Keeping the last read that *worked* would
    /// drag the whole view's freshness back to it over rows nothing on the
    /// screen came from, which is the same false claim as `generated_at` in
    /// the other direction.
    #[test]
    fn a_read_that_failed_is_still_dated_by_the_attempt_that_failed() {
        let mut standing = Collection::default();
        let cfg = two_projects();
        let earlier = now();
        let later = earlier + chrono::Duration::seconds(30);
        standing.collect(
            &cfg,
            &colliding_trackers(PANES_IN_BOTH),
            &Wanted::Everything,
            Filter::All,
            earlier,
        );

        let refused = colliding_trackers(PANES_IN_BOTH)
            .failing(&spelled(UNFINISHED_CALL), failing(FailureKind::Auth));
        let after = standing.collect(&cfg, &refused, &orbital_alone(), Filter::All, later);

        assert!(
            trees_of(&after, "orbital").is_empty(),
            "nothing orbital's earlier read produced is still drawn"
        );
        assert_eq!(after.read_at["orbital"], later);
    }

    fn colliding_trackers(panes: &str) -> FakeRunner {
        let mut runner = FakeRunner::default()
            .with("herdr agent list", panes)
            .with("sh -c secret orbital", "orbital-password")
            .with("sh -c secret ferry", "ferry-password");
        for tracker in [ORBITAL, FERRY] {
            runner = runner
                .with(&spelled_in(tracker, UNFINISHED_CALL),
                    r#"[{"id":"x-1","title":"the shared prefix","status":"in_progress","parent":""},
                        {"id":"x-1.1","title":"the colliding id","status":"in_progress","parent":"x-1"}]"#,
                )
                .with(&spelled_in(tracker, "ready --limit 0 --json"), "[]")
                .with(&spelled_in(tracker, "blocked --json"), "[]")
                .with(&spelled_in(tracker, TRACKER_CALL), COLLIDING_TREE)
                .with(&spelled_in(tracker, WISP_CALL), "[]")
                .with(&spelled_in(tracker, UNFINISHED_WISP_CALL), "[]");
        }
        runner
    }
}
