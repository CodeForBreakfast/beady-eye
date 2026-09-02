//! The standing set of reads, and what one collection is asked to add to it.
//!
//! A collection reads what it is asked for and draws everything standing, so
//! refreshing one project and rebuilding from nothing produce the same
//! snapshot. What a read of one project costs, and what its failures mean,
//! belongs to `tracker`.

use std::collections::BTreeMap;

use chrono::{DateTime, TimeDelta, Utc};

use crate::collect::herdr;
use crate::collect::run::{RunFailure, Runner};
use crate::config::{Config, Project};
use crate::model::join::{self, ProjectRows};
use crate::model::snapshot::{
    self, Collected, FailedProject, Filter, HerdrState, Snapshot, TrackerFailure, TrackerState,
    Tree,
};
use crate::model::types::Pane;

use super::tracker::{refresh_project, tracker_failure, ProjectWork, ReadAt, Refresh};

/// What one project's tracker last said, and when it said it.
///
/// The time belongs to the read rather than to the collection that drew it:
/// a collection naming one project leaves every other project's `Read`
/// untouched, which is how a snapshot can say how fresh each project is
/// rather than only when it was assembled.
struct Read {
    at: DateTime<Utc>,
    work: Result<ProjectWork, TrackerFailure>,
    /// What `work` was read against, where that could be established. The
    /// next refresh of this project skips the cascade only against this, so
    /// a `None` here is what has a failure retried rather than kept.
    taken_at: Option<ReadAt>,
}

/// What a collection is asked to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wanted {
    /// Every project, as a run that has read nothing yet must.
    Everything,
    /// One project. Every other keeps what its tracker last said.
    Project(String),
}

/// A read that has been asked for and has not come back: what it is to
/// read, when it was asked for, and how long it may wait before that is
/// worth saying.
///
/// Asked for rather than in flight, because only the first of them is being
/// served. A read waiting its turn is on its way as much as the one the
/// collector has, and its project's rows have been coming since it was asked
/// for rather than since it was sent.
///
/// The instant is the whole reason this is not a bare `Wanted`. A collection
/// blocks in `Command::output()`, which has no deadline, so a tracker hung
/// for an hour hands the same thing back as one asked half a second ago —
/// nothing, for as long as it takes. A read still waiting its turn hands
/// back less than that. Stamping the ask is what lets anything downstream
/// tell those apart, and it is stamped where the ask happens rather than
/// where its effects are drawn.
///
/// The patience travels beside it rather than being looked up wherever the
/// answer is wanted, so every reader of one collection answers the same way
/// about it, and the config is read once at the edge as everything else is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Awaited {
    pub wanted: Wanted,
    pub asked_at: DateTime<Utc>,
    /// How long this may go unanswered before the project it names is
    /// reported as having stopped being read rather than as being read.
    pub patience: TimeDelta,
}

impl Awaited {
    /// Whether this has gone unanswered for longer than it may.
    pub fn unanswered_at(&self, now: DateTime<Utc>) -> bool {
        now - self.asked_at >= self.patience
    }
}

impl Wanted {
    /// Whether a collection asked for this names one project.
    ///
    /// Public because the screen asks it too, of the very sequence the
    /// collector is served from: what a project line says about its rows and
    /// what will be read are decided by one predicate over one list, so the
    /// two agree by construction rather than by argument.
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

        for (project, answer) in self.refresh_together(cfg, runner, wanted, &panes, now) {
            match answer {
                Ok(Refresh::Unchanged) => {
                    // A skipped read is a successful read: `bdi` knows the
                    // tracker has not moved, so the project is as fresh as if
                    // the cascade had run and the foot must not draw it as
                    // stale.
                    if let Some(standing) = self.read.get_mut(&project.name) {
                        standing.at = now;
                    }
                }
                Ok(Refresh::Read { at, work }) => {
                    self.read.insert(
                        project.name.clone(),
                        Read {
                            at: now,
                            work: Ok(work),
                            taken_at: at,
                        },
                    );
                }
                Err(failure) => {
                    // Storing what the probe just read would make this
                    // failure sticky: the next probe would match it, the
                    // cascade that would have recovered is skipped, and the
                    // project keeps whatever partial state the failure left.
                    self.read.insert(
                        project.name.clone(),
                        Read {
                            at: now,
                            work: Err(tracker_failure(failure.kind)),
                            taken_at: None,
                        },
                    );
                }
            }
        }

        self.draw(cfg, &panes, herdr_state, filter, now)
    }

    /// One refresh of every project `wanted` names, made together rather
    /// than in turn, and handed back once the last of them has answered.
    ///
    /// The trackers are independent and a read of one is a sequence of round
    /// trips, so the reads overlap. What comes back is keyed by project and
    /// drawn in config order, so which tracker answered first is not
    /// something the screen can see. A read that panics is resumed here, on
    /// the thread that asked for it, as it would have been had the reads
    /// been made in turn.
    fn refresh_together<'a>(
        &self,
        cfg: &'a Config,
        runner: &dyn Runner,
        wanted: &Wanted,
        panes: &[Pane],
        now: DateTime<Utc>,
    ) -> Vec<(&'a Project, Result<Refresh, RunFailure>)> {
        std::thread::scope(|reads| {
            let reading: Vec<_> = cfg
                .projects
                .iter()
                .filter(|p| wanted.names(&p.name))
                .map(|project| {
                    let standing = self
                        .read
                        .get(&project.name)
                        .and_then(|read| read.taken_at.clone());
                    reads.spawn(move || {
                        let answer =
                            refresh_project(runner, project, cfg, panes, standing.as_ref(), now);
                        (project, answer)
                    })
                })
                .collect();
            reading
                .into_iter()
                .map(|read| {
                    read.join()
                        .unwrap_or_else(|panicked| std::panic::resume_unwind(panicked))
                })
                .collect()
        })
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
                        rows: &assembled.beads,
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
                    Err(why) => Tree::unread(project, root, TrackerState::from(*why)),
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
    use crate::collect::run::Env;
    use crate::collect::run::{FailureKind, RunFailure};
    use crate::model::anomaly::Anomaly;
    use pretty_assertions::assert_eq;
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::Duration;

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
      {"id":"orb-7.1","title":"re-point the dish","status":"open","parent":"orb-7",
       "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
       "priority":2,"issue_type":"task"},
      {"id":"orb-7.2","title":"lay the feeder cable","status":"open","parent":"orb-7",
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
        assert!(snap.trees[0].beads.iter().all(|n| n.agent.is_none()));
        assert!(snap.unattributed.is_empty());
    }

    #[test]
    fn a_tree_with_no_live_agent_is_reported_rather_than_dropped() {
        let runner = orbital()
            .with("herdr agent list", r#"{"result":{"agents":[]}}"#)
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

    /// A pane on a bead in one project and a session on none in the other,
    /// so the join has both directions to do across both trackers.
    const PANES_IN_BOTH: &str = r#"{"result":{"agents":[
      {"pane_id":"w:p1","cwd":"/srv/work/orbital","agent_status":"working","display_agent":"x-1.1"},
      {"pane_id":"w:p2","cwd":"/srv/work/ferry","agent_status":"idle"}
    ]}}"#;

    /// One call as `Meeting` tells it apart: the directory it was made in
    /// and its command line.
    type Made = (PathBuf, String);

    /// The probe is the one call every project's read makes exactly once,
    /// spelled for the tracker it goes to.
    fn probe_of(tracker: &str) -> Made {
        (PathBuf::from(tracker), spelled_in(tracker, PROBE_CALL))
    }

    /// How long a held call waits for the one it is waiting on before it is
    /// let go and its wait is written down as spent alone. A read in flight
    /// beside the one it waits for arrives within a thread spawn of it, so
    /// only reads made one after the other ever reach this.
    const ALONE: Duration = Duration::from_secs(5);

    /// A runner that holds a call until another call has been made, and
    /// remembers each hold that was let go by the clock rather than by the
    /// arrival it waited for.
    ///
    /// Holding each project's probe until the other project's probe has
    /// arrived is what tells reads made together from reads made in turn,
    /// without asserting against a clock: two reads in flight together meet
    /// at their probes, and one made after the other has finished waits
    /// alone. The deadline is only what a wait that would never end is
    /// reported in.
    struct Meeting {
        inner: FakeRunner,
        holds: Vec<(Made, Made)>,
        arrived: Mutex<BTreeSet<Made>>,
        someone_arrived: Condvar,
        waited_alone: Mutex<Vec<Made>>,
    }

    impl Meeting {
        fn at(inner: FakeRunner) -> Self {
            Self {
                inner,
                holds: Vec::new(),
                arrived: Mutex::new(BTreeSet::new()),
                someone_arrived: Condvar::new(),
                waited_alone: Mutex::new(Vec::new()),
            }
        }

        /// Hold `held` until `until` has been made.
        fn holding(mut self, held: Made, until: Made) -> Self {
            self.holds.push((held, until));
            self
        }

        fn waited_alone(&self) -> Vec<Made> {
            self.waited_alone.lock().unwrap().clone()
        }

        fn arrived(&self) -> BTreeSet<Made> {
            self.arrived.lock().unwrap().clone()
        }
    }

    impl Runner for Meeting {
        fn run(
            &self,
            program: &str,
            args: &[&str],
            cwd: Option<&Path>,
            env: &Env,
        ) -> Result<String, RunFailure> {
            let this: Made = (
                cwd.map(Path::to_path_buf).unwrap_or_default(),
                format!("{program} {}", args.join(" ")),
            );
            let mut arrived = self.arrived.lock().unwrap();
            arrived.insert(this.clone());
            self.someone_arrived.notify_all();
            for (_, until) in self.holds.iter().filter(|(held, _)| *held == this) {
                let (still, waited) = self
                    .someone_arrived
                    .wait_timeout_while(arrived, ALONE, |arrived| !arrived.contains(until))
                    .unwrap();
                arrived = still;
                if waited.timed_out() {
                    self.waited_alone.lock().unwrap().push(this.clone());
                }
            }
            drop(arrived);
            self.inner.run(program, args, cwd, env)
        }
    }

    /// The projects' trackers are independent, and a read of one is a
    /// sequence of round trips, so the reads are made together rather than
    /// in turn: the second project's tracker is asked while the first is
    /// still answering.
    #[test]
    fn the_projects_named_are_read_together_rather_than_in_turn() {
        let runner = Meeting::at(colliding_trackers(PANES_IN_BOTH))
            .holding(probe_of(ORBITAL), probe_of(FERRY))
            .holding(probe_of(FERRY), probe_of(ORBITAL));

        collect(&mut Collection::default(), &runner, &Wanted::Everything);

        assert!(
            runner
                .arrived()
                .is_superset(&[probe_of(ORBITAL), probe_of(FERRY)].into()),
            "both trackers were probed, so a wait spent alone would have been recorded: {:?}",
            runner.arrived()
        );
        assert_eq!(
            runner.waited_alone(),
            vec![],
            "a probe that waited alone was made after the other project's read had finished"
        );
    }

    // ---- reading one project at a time ---------------------------------

    fn collect(collection: &mut Collection, runner: &dyn Runner, wanted: &Wanted) -> Snapshot {
        collection.collect(&two_projects(), runner, wanted, Filter::All, now())
    }

    fn orbital_alone() -> Wanted {
        Wanted::Project("orbital".to_string())
    }

    /// Every `bd` invocation a runner was asked to make. What the refresh
    /// gate costs is counted in these and in nothing else: the environment
    /// capture beside them is a different program and a different bead.
    fn bd_calls(runner: &FakeRunner) -> usize {
        runner
            .calls()
            .iter()
            .filter(|c| c.argv.starts_with("bd "))
            .count()
    }

    /// The same tracker with its in-flight bead last touched 29 days before
    /// `now()`, so nothing is a stale claim at that instant and everything is
    /// two days later.
    const AGEING_TREE: &str = r#"[
      {"id":"orb-7","title":"lift the ground station","status":"in_progress",
       "priority":1,"issue_type":"epic","metadata":{"agent_pane":"w:p1"}},
      {"id":"orb-7.1","title":"re-point the dish","status":"in_progress",
       "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
       "priority":2,"issue_type":"task",
       "metadata":{"agent_pane":"w:p1"},
       "updated_at":"2026-08-01T12:00:00Z"},
      {"id":"orb-7.2","title":"lay the feeder cable","status":"open",
       "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
       "priority":2,"issue_type":"task"}
    ]"#;

    /// Whether anything on the screen is drawn as a claim that has gone
    /// stale, which is the one thing a node says that is derived from the
    /// clock rather than from what the tracker said.
    fn a_claim_is_drawn_as_stale(snap: &Snapshot) -> bool {
        snap.trees.iter().flat_map(|tree| &tree.beads).any(|node| {
            node.anomalies
                .iter()
                .any(|fired| matches!(fired, Anomaly::StaleClaim { .. }))
        })
    }

    /// The same tracker with one task bd is holding back until two hours
    /// after `now()`. `bd ready` does not name it before that instant and
    /// does after, and nothing is written when it passes.
    const DEFERRED_TREE: &str = r#"[
      {"id":"orb-7","title":"lift the ground station","status":"in_progress",
       "priority":1,"issue_type":"epic"},
      {"id":"orb-7.1","title":"re-point the dish","status":"in_progress",
       "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
       "priority":2,"issue_type":"task",
       "metadata":{"agent_pane":"w:p1"}},
      {"id":"orb-7.2","title":"lay the feeder cable","status":"open",
       "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
       "priority":2,"issue_type":"task",
       "defer_until":"2026-08-30T14:00:00Z"}
    ]"#;

    /// The instant `DEFERRED_TREE`'s held bead is due, to the second.
    fn when_it_is_due() -> DateTime<Utc> {
        "2026-08-30T14:00:00Z".parse().expect("the instant parses")
    }

    /// A pane sitting in orbital with a bead's id on it, which is a root the
    /// tracker was never asked about.
    const PANE_ON_A_BEAD: &str = r#"{"result":{"agents":[
      {"pane_id":"w:p1","cwd":"/srv/work/orbital","agent_status":"working","display_agent":"orb-7.2"}
    ]}}"#;

    // ---- the refresh gate ----------------------------------------------

    /// The whole trade: a tracker that has not moved is asked one question
    /// instead of four, and the one question is the probe.
    #[test]
    fn a_project_whose_tracker_has_not_moved_is_asked_once() {
        let runner = orbital();
        let cfg = one_project();
        let mut standing = Collection::default();

        standing.collect(&cfg, &runner, &Wanted::Everything, Filter::All, now());
        let first = bd_calls(&runner);
        standing.collect(&cfg, &runner, &orbital_alone(), Filter::All, now());

        assert_eq!(first, 5, "a project read for the first time costs both");
        assert_eq!(
            bd_calls(&runner) - first,
            1,
            "and a project that has not moved since costs the probe alone"
        );
    }

    /// The other half of the trade, and not a regression to fix: a tracker
    /// that moved costs the probe on top of the four rather than instead of
    /// them.
    #[test]
    fn a_project_whose_tracker_has_moved_is_read_in_full() {
        let cfg = one_project();
        let mut standing = Collection::default();
        standing.collect(&cfg, &orbital(), &Wanted::Everything, Filter::All, now());

        let moved = orbital().with(&spelled(PROBE_CALL), MOVED);
        let after = standing.collect(&cfg, &moved, &orbital_alone(), Filter::All, now());

        assert_eq!(bd_calls(&moved), 5);
        assert!(
            !trees_of(&after, "orbital").is_empty(),
            "and everything it read is drawn"
        );
    }

    /// A cascade only fails where one ran, so the sequence that can go wrong
    /// is a read that worked, a tracker that then moved, and the cascade that
    /// move triggered failing. The working root the probe took must not
    /// survive that: the interval after would match it, skip the cascade that
    /// recovers, and leave the project holding whatever the failure left,
    /// indefinitely.
    #[test]
    fn a_cascade_that_failed_leaves_nothing_for_the_next_interval_to_skip_against() {
        let cfg = one_project();
        let mut standing = Collection::default();
        standing.collect(&cfg, &orbital(), &Wanted::Everything, Filter::All, now());

        let refused = orbital()
            .with(&spelled(PROBE_CALL), MOVED)
            .failing(&spelled(TRACKER_CALL), failing(FailureKind::Auth));
        let failed = standing.collect(&cfg, &refused, &orbital_alone(), Filter::All, now());
        assert_eq!(failed.failed_projects.len(), 1, "the cascade failed");

        // The tracker has not moved since the failure — the same probe answer
        // the failure was taken at — and this time it answers the cascade.
        let recovered = orbital().with(&spelled(PROBE_CALL), MOVED);
        let after = standing.collect(&cfg, &recovered, &orbital_alone(), Filter::All, now());

        assert_eq!(
            bd_calls(&recovered),
            5,
            "the cascade ran again rather than being skipped against the root the failure was probed at"
        );
        assert_eq!(after.failed_projects, vec![], "so the project recovered");
    }

    /// Degrade, never disappear. `dolt_hashof_db()` is Dolt's, and a
    /// SQLite-backed tracker has no such function — so a probe that errors
    /// means "read it the slow way", never "nothing changed", every interval
    /// rather than only the first.
    #[test]
    fn a_tracker_that_cannot_answer_the_probe_is_read_in_full_every_interval() {
        let cfg = one_project();
        let blind = orbital().failing(&spelled(PROBE_CALL), failing(FailureKind::Unavailable));
        let mut standing = Collection::default();

        standing.collect(&cfg, &blind, &Wanted::Everything, Filter::All, now());
        let first = bd_calls(&blind);
        let after = standing.collect(&cfg, &blind, &orbital_alone(), Filter::All, now());

        assert_eq!(first, 5, "the probe was asked and the cascade ran anyway");
        assert_eq!(
            bd_calls(&blind) - first,
            5,
            "and again, rather than settling into a skip against a root nobody established"
        );
        assert!(
            !trees_of(&after, "orbital").is_empty(),
            "a tracker blind to the probe still draws its trees"
        );
    }

    /// The same rule where a root *is* standing to skip against, which is the
    /// way round it can silently go wrong: a tracker read once and answering
    /// the probe, then stopping. An unanswered probe is not the answer "the
    /// root you hold is still current", and reading it as one would freeze
    /// the project on that read for as long as the probe stayed broken.
    #[test]
    fn a_tracker_that_stops_answering_the_probe_is_read_in_full_again() {
        let cfg = one_project();
        let mut standing = Collection::default();
        standing.collect(&cfg, &orbital(), &Wanted::Everything, Filter::All, now());

        let blind = orbital().failing(&spelled(PROBE_CALL), failing(FailureKind::Unavailable));
        standing.collect(&cfg, &blind, &orbital_alone(), Filter::All, now());
        let first = bd_calls(&blind);
        standing.collect(&cfg, &blind, &orbital_alone(), Filter::All, now());

        assert_eq!(
            first, 5,
            "the probe went unanswered, so the cascade ran rather than the standing root being kept"
        );
        assert_eq!(
            bd_calls(&blind) - first,
            5,
            "and the read it just took left nothing for the next interval to skip against either"
        );
    }

    /// A skipped read is a successful read: `bdi` knows the tracker has not
    /// moved, so the project is as fresh as the collection that skipped it
    /// and the foot must not draw it as stale or as never-read.
    #[test]
    fn a_skipped_read_is_as_fresh_as_the_collection_that_skipped_it() {
        let cfg = one_project();
        let runner = orbital();
        let earlier = now();
        let later = earlier + chrono::Duration::seconds(30);
        let mut standing = Collection::default();
        standing.collect(&cfg, &runner, &Wanted::Everything, Filter::All, earlier);

        let after = standing.collect(&cfg, &runner, &orbital_alone(), Filter::All, later);

        assert_eq!(after.read_at["orbital"], later);
        assert!(
            !trees_of(&after, "orbital").is_empty(),
            "and everything the skipped read stood on is still drawn"
        );
    }

    /// Skipping the read must not skip the drawing. What a node says about
    /// its own age is derived from the clock at each collection rather than
    /// from what the tracker said, so a claim goes stale on the screen while
    /// the tracker it came from sits still — which is exactly when it matters.
    #[test]
    fn a_skipped_read_still_ages_what_the_screen_says_about_it() {
        let cfg = one_project();
        let runner = orbital().with(&spelled(TRACKER_CALL), AGEING_TREE);
        let earlier = now();
        let later = earlier + chrono::Duration::days(2);
        let mut standing = Collection::default();

        let before = standing.collect(&cfg, &runner, &Wanted::Everything, Filter::All, earlier);
        let after = standing.collect(&cfg, &runner, &orbital_alone(), Filter::All, later);

        assert!(
            !a_claim_is_drawn_as_stale(&before),
            "29 days is inside the window at the first collection"
        );
        assert!(
            a_claim_is_drawn_as_stale(&after),
            "and 31 days is outside it at the second, which read nothing"
        );
        assert_eq!(
            bd_calls(&runner),
            6,
            "the second collection cost the probe alone, so the ageing is the draw's and not the read's"
        );
    }

    /// The last input to the cascade's answer that is neither the tracker nor
    /// the panes is the clock, and a gate may only skip work whose answer
    /// would have been the same. A read taken while bd was holding a bead
    /// back cannot speak for the tracker once that bead is due, however still
    /// the database has been in between.
    #[test]
    fn a_read_stops_speaking_for_the_tracker_once_a_held_bead_is_due() {
        let cfg = one_project();
        let runner = orbital().with(&spelled(TRACKER_CALL), DEFERRED_TREE);
        let read_at = now();
        let mut standing = Collection::default();
        standing.collect(&cfg, &runner, &Wanted::Everything, Filter::All, read_at);
        let first = bd_calls(&runner);

        let hour = chrono::Duration::hours(1);
        standing.collect(&cfg, &runner, &orbital_alone(), Filter::All, read_at + hour);
        let while_held = bd_calls(&runner);

        standing.collect(
            &cfg,
            &runner,
            &orbital_alone(),
            Filter::All,
            read_at + hour * 3,
        );
        let once_due = bd_calls(&runner);

        standing.collect(
            &cfg,
            &runner,
            &orbital_alone(),
            Filter::All,
            read_at + hour * 4,
        );

        assert_eq!(
            while_held - first,
            1,
            "the probe alone while bd is still holding the bead back"
        );
        assert_eq!(
            once_due - while_held,
            5,
            "and the whole cascade at the first refresh past the instant it is due"
        );
        assert_eq!(
            bd_calls(&runner) - once_due,
            1,
            "after which nothing is held back, so the probe alone again rather than for ever"
        );
    }

    /// The instant itself belongs to the refresh after it, not to the read
    /// before it. A read taken while a bead was held back has already stopped
    /// speaking for the tracker at the moment the bead is due, because that
    /// is the moment `bd ready` starts naming it — waiting for the interval
    /// after would draw one refresh's worth of an answer bd would no longer
    /// give.
    #[test]
    fn a_read_has_stopped_speaking_at_the_instant_a_held_bead_is_due_rather_than_after_it() {
        let cfg = one_project();
        let runner = orbital().with(&spelled(TRACKER_CALL), DEFERRED_TREE);
        let mut standing = Collection::default();
        standing.collect(&cfg, &runner, &Wanted::Everything, Filter::All, now());
        let first = bd_calls(&runner);

        standing.collect(
            &cfg,
            &runner,
            &orbital_alone(),
            Filter::All,
            when_it_is_due(),
        );

        assert_eq!(bd_calls(&runner) - first, 5);
    }

    /// The other side of the same instant. A bead due exactly as the read was
    /// taken has nothing left to turn over: the read already saw bd naming
    /// it, so recording that instant would have the project read in full for
    /// ever after against a change that had already happened.
    #[test]
    fn a_bead_due_as_the_read_was_taken_leaves_nothing_to_turn_over() {
        let cfg = one_project();
        let runner = orbital().with(&spelled(TRACKER_CALL), DEFERRED_TREE);
        let mut standing = Collection::default();
        standing.collect(
            &cfg,
            &runner,
            &Wanted::Everything,
            Filter::All,
            when_it_is_due(),
        );
        let first = bd_calls(&runner);

        let hour = chrono::Duration::hours(1);
        standing.collect(
            &cfg,
            &runner,
            &orbital_alone(),
            Filter::All,
            when_it_is_due() + hour,
        );

        assert_eq!(bd_calls(&runner) - first, 1);
    }

    /// A root can come from a pane rather than from the tracker, so what the
    /// panes name is part of what a read was taken against. A pane that
    /// starts naming a bead has the project read again even though nothing
    /// was written — otherwise that tree stays off the screen until some
    /// unrelated write moves the tracker, which is a disappearance with
    /// nothing said.
    #[test]
    fn a_pane_that_starts_naming_a_bead_has_the_project_read_again() {
        let cfg = one_project();
        let mut standing = Collection::default();
        standing.collect(&cfg, &orbital(), &Wanted::Everything, Filter::All, now());

        let named = orbital().with("herdr agent list", PANE_ON_A_BEAD);
        standing.collect(&cfg, &named, &orbital_alone(), Filter::All, now());

        assert_eq!(
            bd_calls(&named),
            5,
            "the tracker had not moved, but what the panes name had"
        );
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
        snap.trees
            .iter()
            .filter(|t| t.project == project)
            .map(Arc::as_ref)
            .collect()
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

        // The probe is refused as well, which is what a tracker that has
        // stopped answering does: it is the same connection the cascade
        // would have used. Refusing only the cascade would stage a tracker
        // that answers one question and not the next, and the refresh would
        // rightly never ask the second.
        let refused = colliding_trackers(PANES_IN_BOTH)
            .failing(&spelled(PROBE_CALL), failing(FailureKind::Auth))
            .failing(&spelled(TRACKER_CALL), failing(FailureKind::Auth));
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

        // The probe is refused as well, which is what a tracker that has
        // stopped answering does: it is the same connection the cascade
        // would have used. Refusing only the cascade would stage a tracker
        // that answers one question and not the next, and the refresh would
        // rightly never ask the second.
        let refused = colliding_trackers(PANES_IN_BOTH)
            .failing(&spelled(PROBE_CALL), failing(FailureKind::Auth))
            .failing(&spelled(TRACKER_CALL), failing(FailureKind::Auth));
        let after = standing.collect(&cfg, &refused, &orbital_alone(), Filter::All, later);

        assert!(
            trees_of(&after, "orbital").is_empty(),
            "nothing orbital's earlier read produced is still drawn"
        );
        assert_eq!(after.read_at["orbital"], later);
    }
}
