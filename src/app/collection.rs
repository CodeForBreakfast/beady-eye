//! The standing set of reads, and what one collection is asked to add to it.
//!
//! A collection reads what it is asked for and draws everything standing, so
//! refreshing one project and rebuilding from nothing produce the same
//! snapshot. What a read of one project costs, and what its failures mean,
//! belongs to `tracker`.

use std::collections::BTreeMap;

use chrono::{DateTime, TimeDelta, Utc};

use crate::collect::herdr;
use crate::collect::run::Runner;
use crate::config::Config;
use crate::model::join::{self, ProjectRows};
use crate::model::snapshot::{
    self, Collected, FailedProject, Filter, HerdrState, Snapshot, TrackerFailure, Tree,
};
use crate::model::types::Pane;

use super::tracker::{read_project, tracker_failure, ProjectWork};

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

/// A collection that has been asked for and has not come back: what it is
/// reading, when it was asked, and how long it may wait before that is worth
/// saying.
///
/// The instant is the whole reason this is not a bare `Wanted`. A collection
/// blocks in `Command::output()`, which has no deadline, so a tracker hung
/// for an hour hands the same thing back as one asked half a second ago —
/// nothing, for as long as it takes. Stamping the ask is what lets anything
/// downstream tell those apart, and it is stamped where the ask happens
/// rather than where its effects are drawn.
///
/// The patience travels beside it rather than being looked up wherever the
/// answer is wanted, so every reader of one collection answers the same way
/// about it, and the config is read once at the edge as everything else is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InFlight {
    pub wanted: Wanted,
    pub asked_at: DateTime<Utc>,
    /// How long this may go unanswered before the tracker is reported as
    /// having stopped answering rather than as being read.
    pub patience: TimeDelta,
}

impl InFlight {
    /// Whether this has gone unanswered for longer than it may.
    pub fn unanswered_at(&self, now: DateTime<Utc>) -> bool {
        now - self.asked_at >= self.patience
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::fixtures::*;
    use crate::collect::run::testing::FakeRunner;
    use crate::collect::run::{FailureKind, RunFailure};
    use pretty_assertions::assert_eq;
    use std::collections::BTreeSet;
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

    /// The same tracker with nothing claimed in it: a root waiting on work
    /// elsewhere, over open tasks. No pane can be on it and no anomaly rule
    /// can fire on it, which is the one state the default filter folds away.
    const UNSTAFFED_TREE: &str = r#"[
      {"id":"orb-7","title":"lift the ground station","status":"blocked",
       "priority":1,"issue_type":"epic"},
      {"id":"orb-7.1","title":"re-point the dish","status":"open",
       "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
       "priority":2,"issue_type":"task"},
      {"id":"orb-7.2","title":"lay the feeder cable","status":"open",
       "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
       "priority":2,"issue_type":"task"}
    ]"#;

    fn tree_of<'a>(snap: &'a Snapshot, project: &str) -> &'a Tree {
        snap.trees
            .iter()
            .find(|t| t.project == project)
            .unwrap_or_else(|| panic!("{project} has a tree"))
    }

    // ---- degradation ---------------------------------------------------

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
        let runner = orbital()
            .with("herdr agent list", r#"{"result":{"agents":[]}}"#)
            .with(
                &spelled(UNFINISHED_CALL),
                r#"[{"id":"orb-7","title":"lift the ground station","status":"blocked","parent":""},
                    {"id":"orb-7.1","title":"re-point the dish","status":"open","parent":"orb-7"},
                    {"id":"orb-7.2","title":"lay the feeder cable","status":"open","parent":"orb-7"}]"#,
            )
            .with(&spelled(TRACKER_CALL), UNSTAFFED_TREE)
            .with(
                &spelled("blocked --json"),
                r#"[{"id":"orb-7","blocked_by":["orb-9"]}]"#,
            );

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

    /// The property scoping exists for, one layer up from the refresh: a
    /// project the config no longer names is a project whose tracker is never
    /// asked, however whole the collection asking is.
    #[test]
    fn a_project_a_scope_left_out_has_its_tracker_unasked() {
        let runner = colliding_trackers(PANES_IN_BOTH);
        let scoped = two_projects()
            .scoped_to(&["orbital".to_string()])
            .expect("orbital is configured");

        Collection::default().collect(&scoped, &runner, &Wanted::Everything, Filter::All, now());

        assert_eq!(
            tracker_calls(&runner, FERRY),
            0,
            "ferry was scoped out, so nothing should have gone near its tracker"
        );
        assert!(
            tracker_calls(&runner, ORBITAL) > 0,
            "orbital was scoped in, so it was read"
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
}
