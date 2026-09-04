//! The standing set of reads, and what one collection is asked to add to it.
//!
//! A collection reads what it is asked for and draws everything standing, so
//! refreshing one project and rebuilding from nothing produce the same
//! snapshot — for as long as every session answers. A collection that has
//! watched one answer knows which panes it was holding when it stops, and a
//! collection built from nothing has never seen it: that is the one thing
//! the two can disagree about, and `every_pane` is where it is decided.
//! What a read of one project costs, and what its failures mean, belongs to
//! `tracker`.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, TimeDelta, Utc};

use crate::collect::agents::Agents;
use crate::collect::run::FailureKind;
use crate::collect::tracker::{OpenFailure, Trackers};
use crate::collect::worktree;
use crate::config::{Config, Project};
use crate::model::join::{self, Listed, ProjectRows};
use crate::model::snapshot::{
    self, AgentProvider, Collected, FailedProject, Filter, ProviderState, Session, SessionState,
    Snapshot, TrackerFailure, TrackerState, Tree,
};
use crate::model::types::Pane;

use super::tracker::{open_failure, refresh_project, ProjectWork, ReadAt, Refresh};

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

/// Everything the collector is asked for.
///
/// A read is one of two things rather than the only one, because a config
/// the reader has written is not a read: it draws nothing by itself, and
/// what it changes is what every read after it reads. Sending it down the
/// same channel is what puts it in order against them — the collector works
/// to it before the collection that reads under it, without either having to
/// know when the other was decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Asked {
    Read(Wanted),
    /// The config the collector works to from here on.
    Reloaded(Box<Config>),
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

/// A pane herdr listed, told where its directory sits in the main working
/// tree of the repository it is in.
///
/// herdr says only where the pane is. A pane in a linked worktree of a
/// project this run never asked where it is worked — one the scope left
/// out — is held by no configured path as it stands, and only the
/// filesystem can say which project it belongs to. That read happens here,
/// once per listing, because `model` is pure over what it is handed: it
/// places panes at paths that exist on no machine, and a placement that
/// asked the disk would answer differently on a machine where one of them
/// happened to exist.
fn placed(pane: Pane) -> Pane {
    let main_tree = worktree::in_the_main_working_tree(&pane.cwd);
    pane.with_cwd_in_the_main_working_tree(main_tree)
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
    /// Which pane ids each session last answered with. A session that
    /// answers replaces its own entry and no other's, and one the provider
    /// has stopped running loses its.
    ///
    /// Kept for the collection where a session does *not* answer, which is
    /// the only time it is read. A seat writes a bare pane id, and an id
    /// names a pane only within its session, so nothing in the run's own
    /// answer can place the id a silent session was holding. What it last
    /// answered with can.
    panes_last_answered: BTreeMap<String, BTreeSet<String>>,
}

impl Collection {
    /// Read what `wanted` names, and draw everything standing.
    pub fn collect(
        &mut self,
        cfg: &Config,
        agents: &dyn Agents,
        trackers: &dyn Trackers,
        wanted: &Wanted,
        filter: Filter,
        now: DateTime<Utc>,
    ) -> Snapshot {
        // The provider is the second tier: without its panes there is no agent
        // to join and no filter to apply, and every tracker still reads.
        //
        // Read again however few projects `wanted` names: it is a few local
        // calls, the join it feeds is across every project, and a project
        // with a producer is never polled — so a refresh naming it is the
        // only chance the agent join gets.
        let (panes, provider, out_of_reach) = self.every_pane(agents);

        for (project, answer) in self.refresh_together(cfg, trackers, wanted, &panes, now) {
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
                            work: Ok(*work),
                            taken_at: at.map(|at| *at),
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
                            work: Err(open_failure(&failure)),
                            taken_at: None,
                        },
                    );
                }
            }
        }

        self.draw(cfg, &panes, &out_of_reach, provider, filter, now)
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
        trackers: &dyn Trackers,
        wanted: &Wanted,
        panes: &[Pane],
        now: DateTime<Utc>,
    ) -> Vec<(&'a Project, Result<Refresh, OpenFailure>)> {
        std::thread::scope(|reads| {
            let reading: Vec<_> = cfg
                .read()
                .filter(|p| wanted.names(&p.name))
                .map(|project| {
                    let standing = self
                        .read
                        .get(&project.name)
                        .and_then(|read| read.taken_at.clone());
                    reads.spawn(move || {
                        let answer =
                            refresh_project(trackers, project, cfg, panes, standing.as_ref(), now);
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
        out_of_reach: &BTreeSet<String>,
        agents: AgentProvider,
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
        let joined = &join::resolve(
            &rows,
            Listed {
                panes,
                out_of_reach,
            },
            cfg,
        );

        let trees = self
            .that_answered(cfg)
            .flat_map(|(project, work)| {
                work.roots.iter().map(move |(root, read)| match read {
                    Ok(assembled) => snapshot::build_tree(
                        project,
                        assembled,
                        joined,
                        &work.readiness,
                        &work.relations,
                        agents.state,
                        cfg,
                        now,
                    ),
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
            agents,
            filter,
            now,
        )
    }

    /// What has been read, in the order the config names the projects. The
    /// order a snapshot draws in belongs to the config, not to how a
    /// collection happened to store what it read.
    fn standing<'a>(&'a self, cfg: &'a Config) -> impl Iterator<Item = (&'a str, &'a Read)> {
        cfg.read()
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

    /// Every pane on the machine, how asking for them went, and the ids of the
    /// panes this run is missing.
    ///
    /// The provider is asked which sessions it runs, once, and then each session
    /// for its panes. A session that will not answer is a finding about that
    /// session and takes nothing from the others: its state is carried beside
    /// the panes of the sessions that did answer, which are drawn as they would
    /// be had it never existed. A provider that will not say which sessions it
    /// runs is the older, whole failure, and holds no session to report.
    ///
    /// It is still `Answering`, and that is the report rather than a gap in it.
    /// A partial answer is not a fact about the provider — the provider said
    /// which sessions it runs and then answered for most of them — it is a fact
    /// about each session, which `AgentProvider::sessions` carries one by one
    /// and the foot says one by one. Collapsing those into a fourth provider
    /// state would reach every reader of `ProviderState::answered()`: the live-
    /// agents filter would fall back to showing everything and the tail would
    /// refuse to read any pane on the machine, both because one session of five
    /// hiccuped. So what a partial answer costs is carried at the grain it
    /// happened at, and the third value here is that grain: the panes the
    /// missing sessions were holding when they last answered, which is what
    /// tells a claim this run cannot speak for from a seat that has died.
    ///
    /// A session that has never answered contributes nothing, so a claim naming
    /// a pane in one is reported as an orphan the way it was before this. That
    /// is bounded — a session that wedges while `bdi` watches is placed within
    /// one collection — but it never closes for `bdi --json`, which builds a
    /// collection per invocation and so has watched nothing. That divergence
    /// between the screen and the published snapshot is `bdi-0tp.15`.
    fn every_pane(&mut self, agents: &dyn Agents) -> (Vec<Pane>, AgentProvider, BTreeSet<String>) {
        let sessions = match agents.sessions() {
            Ok(sessions) => sessions,
            Err(failure) => {
                return (
                    Vec::new(),
                    AgentProvider {
                        provider: agents.name(),
                        state: unlistable(failure.kind),
                        sessions: Vec::new(),
                    },
                    BTreeSet::new(),
                )
            }
        };
        self.panes_last_answered
            .retain(|session, _| sessions.contains(session));

        let mut panes = Vec::new();
        let mut read = Vec::with_capacity(sessions.len());
        let mut out_of_reach = BTreeSet::new();
        for name in sessions {
            let state = match agents.list(&name) {
                Ok(listed) => {
                    self.panes_last_answered.insert(
                        name.clone(),
                        listed.iter().map(|pane| pane.pane_id.clone()).collect(),
                    );
                    panes.extend(listed.into_iter().map(placed));
                    SessionState::Answering
                }
                Err(_) => {
                    if let Some(held) = self.panes_last_answered.get(&name) {
                        out_of_reach.extend(held.iter().cloned());
                    }
                    SessionState::NotAnswering
                }
            };
            read.push(Session { name, state });
        }
        (
            panes,
            AgentProvider::answering(agents.name(), read),
            out_of_reach,
        )
    }
}

/// What a failed listing says about the provider that failed it.
///
/// `NotInstalled` is the one failure that means nothing was installed to
/// run, and it is the only one that is not a finding. A provider that is
/// there and would not start is something the reader had and lost, so it
/// stands with the ones that ran and did not answer.
fn unlistable(kind: FailureKind) -> ProviderState {
    match kind {
        FailureKind::NotInstalled => ProviderState::Absent,
        FailureKind::Unstartable
        | FailureKind::InstalledUnstartable
        | FailureKind::Auth
        | FailureKind::Unavailable
        | FailureKind::Gone
        | FailureKind::Busy
        | FailureKind::Parse
        | FailureKind::Unsupported
        | FailureKind::UnknownFlag => ProviderState::NotAnswering,
    }
}

/// Read every configured tracker and the agent provider, and draw the result.
pub fn run(
    cfg: &Config,
    agents: &dyn Agents,
    trackers: &dyn Trackers,
    filter: Filter,
    now: DateTime<Utc>,
) -> Snapshot {
    Collection::default().collect(cfg, agents, trackers, &Wanted::Everything, filter, now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::fixtures::*;
    use crate::collect::agents::testing::{
        in_session, named, pane, titled, Asked as AskedOfTheProvider, Fake as Provider, THE_FAKE,
    };
    use crate::collect::run::{FailureKind, RunFailure};
    use crate::collect::tracker::testing::{Asked, Fake, Fakes};
    use crate::collect::tracker::Tracker;
    use crate::collect::worktree::testing::a_linked_worktree_git_made;
    use crate::model::anomaly::Anomaly;
    use crate::model::join::{BeadKey, Conflict};
    use crate::model::snapshot::LoosePane;
    use crate::model::types::testing::{key, A_SESSION};
    use crate::model::types::PaneStatus;
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

    /// A provider that is installed and would not answer, which is the only
    /// state that costs the foot a notice.
    #[test]
    fn a_provider_that_will_not_answer_is_said_and_every_tree_still_draws() {
        let no_session = Provider::unlistable(RunFailure {
            kind: FailureKind::Unavailable,
            program: "a provider".to_string(),
            detail: "no such session".to_string(),
        });

        let snap = run(
            &one_project(),
            &no_session,
            &orbital(),
            Filter::LiveAgents,
            now(),
        );

        assert_eq!(snap.agents.state, ProviderState::NotAnswering);
        assert_eq!(snap.trees.len(), 1, "trees draw without liveness");
        assert!(snap.trees[0].beads.iter().all(|n| n.agent.is_none()));
        assert!(snap.unattributed.is_empty());
    }

    /// A machine with nothing to provide agents. It draws exactly as the
    /// unanswering one does and says something else about why, because a
    /// provider nobody installed is not a provider that broke.
    #[test]
    fn a_provider_that_was_never_installed_is_absent_and_every_tree_still_draws() {
        let nothing = Provider::unlistable(RunFailure::not_installed(
            THE_FAKE,
            "No such file or directory",
        ));

        let snap = run(
            &one_project(),
            &nothing,
            &orbital(),
            Filter::LiveAgents,
            now(),
        );

        assert_eq!(snap.agents.state, ProviderState::Absent);
        assert_eq!(snap.agents.provider, THE_FAKE);
        assert_eq!(snap.trees.len(), 1, "trees draw with no provider at all");
        assert!(snap.trees[0].beads.iter().all(|n| n.agent.is_none()));
        assert!(snap.unattributed.is_empty());
    }

    /// The state this bead exists for: a provider that is installed and
    /// would not start. It is not a machine that never had one, so it does
    /// not get that machine's silence — it is a finding, said at the foot,
    /// like every other provider that is there and did not answer.
    #[test]
    fn a_provider_that_is_installed_and_will_not_start_is_a_finding_not_an_absence() {
        let broken = Provider::unlistable(RunFailure::unstartable(
            THE_FAKE,
            "Permission denied (os error 13)",
        ));

        let snap = run(
            &one_project(),
            &broken,
            &orbital(),
            Filter::LiveAgents,
            now(),
        );

        assert_eq!(snap.agents.state, ProviderState::NotAnswering);
        assert_eq!(snap.trees.len(), 1, "trees draw without liveness");
    }

    /// A provider that answered and holds no pane is neither of those: it
    /// answered, so the panes it gave are all the panes there are.
    #[test]
    fn a_provider_holding_no_pane_has_still_answered() {
        let snap = run(&one_project(), &no_panes(), &orbital(), Filter::All, now());

        assert_eq!(snap.agents.state, ProviderState::Answering);
    }

    /// Every bead a run reported as a claim that has lost its agent.
    fn orphaned(snap: &Snapshot) -> Vec<&str> {
        snap.trees
            .iter()
            .flat_map(|tree| tree.beads.iter())
            .filter(|node| {
                node.anomalies
                    .iter()
                    .any(|fired| matches!(fired, Anomaly::OrphanClaim { .. }))
            })
            .map(|node| node.id.as_str())
            .collect()
    }

    /// Reading a claim's missing pane as an agent that died is only sound
    /// where something answered for panes, and the collection is what carries
    /// that from the provider it read to the rules. Both of orbital's claims
    /// are orphaned where a provider answered and holds no pane, and neither
    /// is where nothing answered — a run that cannot tell reports the
    /// provider rather than the beads.
    #[test]
    fn a_claim_is_only_orphaned_against_a_provider_that_answered() {
        let answered = run(&one_project(), &no_panes(), &orbital(), Filter::All, now());
        assert_eq!(orphaned(&answered), ["orb-7", "orb-7.1"]);

        for silent in [
            Provider::unlistable(RunFailure::not_installed(
                THE_FAKE,
                "No such file or directory",
            )),
            Provider::unlistable(RunFailure::unstartable(
                THE_FAKE,
                "Permission denied (os error 13)",
            )),
        ] {
            let snap = run(&one_project(), &silent, &orbital(), Filter::All, now());

            assert_eq!(
                orphaned(&snap),
                Vec::<&str>::new(),
                "{:?} knows no more about panes than the other",
                snap.agents.state
            );
        }
    }

    #[test]
    fn a_tree_with_no_live_agent_is_reported_rather_than_dropped() {
        let nobody = no_panes();
        let trackers = orbital_with(
            Fake::holding(beads(UNSTAFFED_TREE))
                .ready(["orb-7.2"])
                .blocked("orb-7", &["orb-9"]),
        );

        let filtered = run(
            &one_project(),
            &nobody,
            &trackers,
            Filter::LiveAgents,
            now(),
        );
        assert!(filtered.trees.is_empty());
        assert_eq!(filtered.hidden_trees.len(), 1);
        assert_eq!(filtered.hidden_trees[0].root, "orb-7");

        let all = run(&one_project(), &nobody, &trackers, Filter::All, now());
        assert_eq!(all.trees.len(), 1);
        assert!(all.hidden_trees.is_empty());
    }

    #[test]
    fn a_pane_on_no_bead_is_reported_under_the_project_it_sits_in() {
        let snap = run(
            &one_project(),
            &panes(),
            &orbital(),
            Filter::LiveAgents,
            now(),
        );

        let loose: Vec<&str> = snap
            .unattributed
            .iter()
            .map(|p| p.pane.id.as_str())
            .collect();
        assert_eq!(loose, vec!["w:p9"]);
        assert_eq!(snap.unattributed[0].project, "orbital");
    }

    // ---- several projects at once --------------------------------------

    /// The join runs once over every project's rows, so a pane resolves
    /// against the tracker its directory sits in and no other.
    #[test]
    fn a_pane_joins_only_the_project_its_directory_sits_in() {
        let panes = Provider::holding(vec![named(
            pane("w:p1", ORBITAL, PaneStatus::Working),
            "x-1.1",
        )]);

        let snap = run(
            &two_projects(),
            &panes,
            &colliding_trackers(),
            Filter::All,
            now(),
        );

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
    fn panes_in_both() -> Provider {
        Provider::holding(vec![
            named(pane("w:p1", ORBITAL, PaneStatus::Working), "x-1.1"),
            pane("w:p2", FERRY, PaneStatus::Idle),
        ])
    }

    /// How long a held fingerprint waits for the one it is waiting on before
    /// it is let go and its wait is written down as spent alone. A read in
    /// flight beside the one it waits for arrives within a thread spawn of
    /// it, so only reads made one after the other ever reach this.
    const ALONE: Duration = Duration::from_secs(5);

    /// Trackers that hold one project's fingerprint until another project's
    /// has been asked for, and remember each hold that was let go by the
    /// clock rather than by the arrival it waited for.
    ///
    /// The fingerprint is the one question every project's read asks exactly
    /// once, and first. Holding each project's until the other project's has
    /// arrived is what tells reads made together from reads made in turn,
    /// without asserting against a clock: two reads in flight together meet
    /// at their fingerprints, and one made after the other has finished waits
    /// alone. The deadline is only what a wait that would never end is
    /// reported in.
    struct Meeting {
        inner: Fakes,
        holds: Vec<(String, String)>,
        arrived: Mutex<BTreeSet<String>>,
        someone_arrived: Condvar,
        waited_alone: Mutex<Vec<String>>,
    }

    impl Meeting {
        fn at(inner: Fakes) -> Self {
            Self {
                inner,
                holds: Vec::new(),
                arrived: Mutex::new(BTreeSet::new()),
                someone_arrived: Condvar::new(),
                waited_alone: Mutex::new(Vec::new()),
            }
        }

        /// Hold `held`'s fingerprint until `until`'s has been asked for.
        fn holding(mut self, held: &str, until: &str) -> Self {
            self.holds.push((held.to_string(), until.to_string()));
            self
        }

        fn waited_alone(&self) -> Vec<String> {
            self.waited_alone.lock().unwrap().clone()
        }

        fn arrived(&self) -> BTreeSet<String> {
            self.arrived.lock().unwrap().clone()
        }

        /// `project`'s fingerprint has been asked for; wait here for whoever
        /// it was told to wait for.
        fn met_by(&self, project: &str) {
            let mut arrived = self.arrived.lock().unwrap();
            arrived.insert(project.to_string());
            self.someone_arrived.notify_all();
            for (_, until) in self.holds.iter().filter(|(held, _)| held == project) {
                let (still, waited) = self
                    .someone_arrived
                    .wait_timeout_while(arrived, ALONE, |arrived| !arrived.contains(until))
                    .unwrap();
                arrived = still;
                if waited.timed_out() {
                    self.waited_alone.lock().unwrap().push(project.to_string());
                }
            }
        }
    }

    impl Trackers for Meeting {
        fn of(&self, project: &Project) -> Result<Box<dyn Tracker + '_>, OpenFailure> {
            Ok(Box::new(Held {
                meeting: self,
                project: project.name.clone(),
                inner: self.inner.of(project)?,
            }))
        }
    }

    /// One project's tracker as `Meeting` hands it out.
    struct Held<'m> {
        meeting: &'m Meeting,
        project: String,
        inner: Box<dyn Tracker + 'm>,
    }

    impl Tracker for Held<'_> {
        fn fingerprint(&self) -> Option<Result<String, RunFailure>> {
            self.meeting.met_by(&self.project);
            self.inner.fingerprint()
        }

        fn all(&self) -> Result<Vec<crate::model::types::Bead>, RunFailure> {
            self.inner.all()
        }

        fn ready(&self) -> Result<BTreeSet<String>, RunFailure> {
            self.inner.ready()
        }

        fn blocked(&self) -> Result<BTreeMap<String, Vec<String>>, RunFailure> {
            self.inner.blocked()
        }
    }

    /// The projects' trackers are independent, and a read of one is a
    /// sequence of round trips, so the reads are made together rather than
    /// in turn: the second project's tracker is asked while the first is
    /// still answering.
    #[test]
    fn the_projects_named_are_read_together_rather_than_in_turn() {
        let trackers = Meeting::at(colliding_trackers())
            .holding("orbital", "ferry")
            .holding("ferry", "orbital");

        collect(
            &mut Collection::default(),
            &panes_in_both(),
            &trackers,
            &Wanted::Everything,
        );

        assert_eq!(
            trackers.arrived(),
            BTreeSet::from(["orbital".to_string(), "ferry".to_string()]),
            "both trackers were asked for a fingerprint, so a wait spent alone would have been recorded"
        );
        assert_eq!(
            trackers.waited_alone(),
            Vec::<String>::new(),
            "a fingerprint that waited alone was asked for after the other project's read had finished"
        );
    }

    // ---- reading one project at a time ---------------------------------

    fn collect(
        collection: &mut Collection,
        agents: &dyn Agents,
        trackers: &dyn Trackers,
        wanted: &Wanted,
    ) -> Snapshot {
        collection.collect(
            &two_projects(),
            agents,
            trackers,
            wanted,
            Filter::All,
            now(),
        )
    }

    fn orbital_alone() -> Wanted {
        Wanted::Project("orbital".to_string())
    }

    /// Every question `project`'s tracker was asked. What the refresh gate
    /// costs is counted in these and in nothing else.
    fn asked_of(trackers: &Fakes, project: &str) -> usize {
        trackers.tracker(project).asked().len()
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
    fn pane_on_a_bead() -> Provider {
        Provider::holding(vec![named(
            pane("w:p1", ORBITAL, PaneStatus::Working),
            "orb-7.2",
        )])
    }

    // ---- the refresh gate ----------------------------------------------

    /// The whole trade: a tracker that has not moved is asked one question
    /// instead of four, and the one question is its fingerprint.
    #[test]
    fn a_project_whose_tracker_has_not_moved_is_asked_once() {
        let trackers = orbital();
        let cfg = one_project();
        let mut standing = Collection::default();

        standing.collect(
            &cfg,
            &panes(),
            &trackers,
            &Wanted::Everything,
            Filter::All,
            now(),
        );
        let first = asked_of(&trackers, "orbital");
        standing.collect(
            &cfg,
            &panes(),
            &trackers,
            &orbital_alone(),
            Filter::All,
            now(),
        );

        assert_eq!(first, 4, "a project read for the first time costs both");
        assert_eq!(
            asked_of(&trackers, "orbital") - first,
            1,
            "and a project that has not moved since costs the fingerprint alone"
        );
    }

    /// The other half of the trade, and not a regression to fix: a tracker
    /// that moved costs the fingerprint on top of the three rather than
    /// instead of them.
    #[test]
    fn a_project_whose_tracker_has_moved_is_read_in_full() {
        let cfg = one_project();
        let mut standing = Collection::default();
        standing.collect(
            &cfg,
            &panes(),
            &orbital(),
            &Wanted::Everything,
            Filter::All,
            now(),
        );

        let moved = orbital_with(orbital_tracker().moved());
        let after = standing.collect(&cfg, &panes(), &moved, &orbital_alone(), Filter::All, now());

        assert_eq!(asked_of(&moved, "orbital"), 4);
        assert!(
            !trees_of(&after, "orbital").is_empty(),
            "and everything it read is drawn"
        );
    }

    /// A project the reader has rewritten in the file is read again, however
    /// still its tracker has stood.
    ///
    /// The fingerprint cannot cover this and is not meant to: it answers for
    /// the tracker, and what changed is how the tracker is reached and what
    /// is asked of it. The rows in hand were taken under the settings the
    /// reader has just replaced, so a skip against them keeps a read nothing
    /// in the config now asks for — and keeps it for as long as the tracker
    /// stands still, which is the reader's own idle project.
    #[test]
    fn a_project_the_reader_has_rewritten_is_read_again_though_its_tracker_has_not_moved() {
        let trackers = orbital();
        let mut standing = Collection::default();
        standing.collect(
            &one_project(),
            &panes(),
            &trackers,
            &Wanted::Everything,
            Filter::All,
            now(),
        );
        let first = asked_of(&trackers, "orbital");

        let after = standing.collect(
            &one_project_reached_with_a_credential(),
            &panes(),
            &trackers,
            &Wanted::Everything,
            Filter::All,
            now(),
        );

        assert_eq!(
            asked_of(&trackers, "orbital") - first,
            4,
            "the cascade ran again rather than being skipped against a read taken \
             under the settings the reader has just replaced"
        );
        assert!(
            !trees_of(&after, "orbital").is_empty(),
            "and what it read under the new settings is drawn"
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
        standing.collect(
            &cfg,
            &panes(),
            &orbital(),
            &Wanted::Everything,
            Filter::All,
            now(),
        );

        let refused = orbital_with(
            orbital_tracker()
                .moved()
                .failing(Asked::All, failing(FailureKind::Auth)),
        );
        let failed = standing.collect(
            &cfg,
            &panes(),
            &refused,
            &orbital_alone(),
            Filter::All,
            now(),
        );
        assert_eq!(failed.failed_projects.len(), 1, "the cascade failed");

        // The tracker has not moved since the failure — the same fingerprint
        // the failure was taken at — and this time it answers the cascade.
        let recovered = orbital_with(orbital_tracker().moved());
        let after = standing.collect(
            &cfg,
            &panes(),
            &recovered,
            &orbital_alone(),
            Filter::All,
            now(),
        );

        assert_eq!(
            asked_of(&recovered, "orbital"),
            4,
            "the cascade ran again rather than being skipped against the fingerprint the failure was taken at"
        );
        assert_eq!(after.failed_projects, vec![], "so the project recovered");
    }

    /// Degrade, never disappear. A fingerprint that errors means "read it
    /// the slow way", never "nothing changed", every interval rather than
    /// only the first.
    #[test]
    fn a_tracker_that_cannot_answer_its_fingerprint_is_read_in_full_every_interval() {
        let cfg = one_project();
        let blind = orbital_with(
            orbital_tracker().failing(Asked::Fingerprint, failing(FailureKind::Unavailable)),
        );
        let mut standing = Collection::default();

        standing.collect(
            &cfg,
            &panes(),
            &blind,
            &Wanted::Everything,
            Filter::All,
            now(),
        );
        let first = asked_of(&blind, "orbital");
        let after = standing.collect(&cfg, &panes(), &blind, &orbital_alone(), Filter::All, now());

        assert_eq!(
            first, 4,
            "the fingerprint was asked for and the cascade ran anyway"
        );
        assert_eq!(
            asked_of(&blind, "orbital") - first,
            4,
            "and again, rather than settling into a skip against a fingerprint nobody established"
        );
        assert!(
            !trees_of(&after, "orbital").is_empty(),
            "a tracker blind to its fingerprint still draws its trees"
        );
    }

    /// A tracker with nothing to fingerprint by says so rather than failing,
    /// and is read in full every interval the same way: `None` is not the
    /// answer "the read you hold is still current".
    #[test]
    fn a_tracker_with_no_fingerprint_is_read_in_full_every_interval() {
        let cfg = one_project();
        let unfingerprinted = orbital_with(orbital_tracker().without_a_fingerprint());
        let mut standing = Collection::default();

        standing.collect(
            &cfg,
            &panes(),
            &unfingerprinted,
            &Wanted::Everything,
            Filter::All,
            now(),
        );
        let first = asked_of(&unfingerprinted, "orbital");
        let after = standing.collect(
            &cfg,
            &panes(),
            &unfingerprinted,
            &orbital_alone(),
            Filter::All,
            now(),
        );

        assert_eq!(first, 4);
        assert_eq!(asked_of(&unfingerprinted, "orbital") - first, 4);
        assert!(!trees_of(&after, "orbital").is_empty());
    }

    /// The same rule where a fingerprint *is* standing to skip against, which
    /// is the way round it can silently go wrong: a tracker read once and
    /// answering, then stopping. An unanswered fingerprint is not the answer
    /// "the read you hold is still current", and reading it as one would
    /// freeze the project on that read for as long as it stayed broken.
    #[test]
    fn a_tracker_that_stops_answering_its_fingerprint_is_read_in_full_again() {
        let cfg = one_project();
        let mut standing = Collection::default();
        standing.collect(
            &cfg,
            &panes(),
            &orbital(),
            &Wanted::Everything,
            Filter::All,
            now(),
        );

        let blind = orbital_with(
            orbital_tracker().failing(Asked::Fingerprint, failing(FailureKind::Unavailable)),
        );
        standing.collect(&cfg, &panes(), &blind, &orbital_alone(), Filter::All, now());
        let first = asked_of(&blind, "orbital");
        standing.collect(&cfg, &panes(), &blind, &orbital_alone(), Filter::All, now());

        assert_eq!(
            first, 4,
            "the fingerprint went unanswered, so the cascade ran rather than the standing one being kept"
        );
        assert_eq!(
            asked_of(&blind, "orbital") - first,
            4,
            "and the read it just took left nothing for the next interval to skip against either"
        );
    }

    /// A skipped read is a successful read: `bdi` knows the tracker has not
    /// moved, so the project is as fresh as the collection that skipped it
    /// and the foot must not draw it as stale or as never-read.
    #[test]
    fn a_skipped_read_is_as_fresh_as_the_collection_that_skipped_it() {
        let cfg = one_project();
        let trackers = orbital();
        let earlier = now();
        let later = earlier + chrono::Duration::seconds(30);
        let mut standing = Collection::default();
        standing.collect(
            &cfg,
            &panes(),
            &trackers,
            &Wanted::Everything,
            Filter::All,
            earlier,
        );

        let after = standing.collect(
            &cfg,
            &panes(),
            &trackers,
            &orbital_alone(),
            Filter::All,
            later,
        );

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
        let trackers = orbital_with(orbital_holding(AGEING_TREE));
        let earlier = now();
        let later = earlier + chrono::Duration::days(2);
        let mut standing = Collection::default();

        let before = standing.collect(
            &cfg,
            &panes(),
            &trackers,
            &Wanted::Everything,
            Filter::All,
            earlier,
        );
        let after = standing.collect(
            &cfg,
            &panes(),
            &trackers,
            &orbital_alone(),
            Filter::All,
            later,
        );

        assert!(
            !a_claim_is_drawn_as_stale(&before),
            "29 days is inside the window at the first collection"
        );
        assert!(
            a_claim_is_drawn_as_stale(&after),
            "and 31 days is outside it at the second, which read nothing"
        );
        assert_eq!(
            asked_of(&trackers, "orbital"),
            5,
            "the second collection cost the fingerprint alone, so the ageing is the draw's and not the read's"
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
        let trackers = orbital_with(orbital_holding(DEFERRED_TREE));
        let read_at = now();
        let mut standing = Collection::default();
        standing.collect(
            &cfg,
            &panes(),
            &trackers,
            &Wanted::Everything,
            Filter::All,
            read_at,
        );
        let first = asked_of(&trackers, "orbital");

        let hour = chrono::Duration::hours(1);
        standing.collect(
            &cfg,
            &panes(),
            &trackers,
            &orbital_alone(),
            Filter::All,
            read_at + hour,
        );
        let while_held = asked_of(&trackers, "orbital");

        standing.collect(
            &cfg,
            &panes(),
            &trackers,
            &orbital_alone(),
            Filter::All,
            read_at + hour * 3,
        );
        let once_due = asked_of(&trackers, "orbital");

        standing.collect(
            &cfg,
            &panes(),
            &trackers,
            &orbital_alone(),
            Filter::All,
            read_at + hour * 4,
        );

        assert_eq!(
            while_held - first,
            1,
            "the fingerprint alone while the tracker is still holding the bead back"
        );
        assert_eq!(
            once_due - while_held,
            4,
            "and the whole cascade at the first refresh past the instant it is due"
        );
        assert_eq!(
            asked_of(&trackers, "orbital") - once_due,
            1,
            "after which nothing is held back, so the fingerprint alone again rather than for ever"
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
        let trackers = orbital_with(orbital_holding(DEFERRED_TREE));
        let mut standing = Collection::default();
        standing.collect(
            &cfg,
            &panes(),
            &trackers,
            &Wanted::Everything,
            Filter::All,
            now(),
        );
        let first = asked_of(&trackers, "orbital");

        standing.collect(
            &cfg,
            &panes(),
            &trackers,
            &orbital_alone(),
            Filter::All,
            when_it_is_due(),
        );

        assert_eq!(asked_of(&trackers, "orbital") - first, 4);
    }

    /// The other side of the same instant. A bead due exactly as the read was
    /// taken has nothing left to turn over: the read already saw bd naming
    /// it, so recording that instant would have the project read in full for
    /// ever after against a change that had already happened.
    #[test]
    fn a_bead_due_as_the_read_was_taken_leaves_nothing_to_turn_over() {
        let cfg = one_project();
        let trackers = orbital_with(orbital_holding(DEFERRED_TREE));
        let mut standing = Collection::default();
        standing.collect(
            &cfg,
            &panes(),
            &trackers,
            &Wanted::Everything,
            Filter::All,
            when_it_is_due(),
        );
        let first = asked_of(&trackers, "orbital");

        let hour = chrono::Duration::hours(1);
        standing.collect(
            &cfg,
            &panes(),
            &trackers,
            &orbital_alone(),
            Filter::All,
            when_it_is_due() + hour,
        );

        assert_eq!(asked_of(&trackers, "orbital") - first, 1);
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
        standing.collect(
            &cfg,
            &panes(),
            &orbital(),
            &Wanted::Everything,
            Filter::All,
            now(),
        );

        let again = orbital();
        standing.collect(
            &cfg,
            &pane_on_a_bead(),
            &again,
            &orbital_alone(),
            Filter::All,
            now(),
        );

        assert_eq!(
            asked_of(&again, "orbital"),
            4,
            "the tracker had not moved, but what the panes name had"
        );
    }

    /// Both trackers, with orbital's refusing its credential. The fingerprint
    /// is refused as well, which is what a tracker that has stopped answering
    /// does: it is the same connection the cascade would have used. Refusing
    /// only the cascade would stage a tracker that answers one question and
    /// not the next, and the refresh would rightly never ask the second.
    fn orbital_refusing() -> Fakes {
        Fakes::default()
            .with(
                "orbital",
                colliding_tracker()
                    .failing(Asked::Fingerprint, failing(FailureKind::Auth))
                    .failing(Asked::All, failing(FailureKind::Auth)),
            )
            .with("ferry", colliding_tracker())
    }

    fn trees_of<'a>(snap: &'a Snapshot, project: &str) -> Vec<&'a Tree> {
        snap.trees
            .iter()
            .filter(|t| t.project == project)
            .map(Arc::as_ref)
            .collect()
    }

    /// The pane on ferry's desktop is neither drawn nor reported by a run
    /// scoped to orbital, and in particular is not reported as unconfigured:
    /// the config still names ferry, and the pane is placed against the
    /// config as written.
    #[test]
    fn a_pane_under_a_project_the_scope_left_out_is_not_reported() {
        let scoped = two_projects()
            .scoped_to(&["orbital".to_string()])
            .expect("orbital is configured");

        let snap = Collection::default().collect(
            &scoped,
            &panes_in_both(),
            &colliding_trackers(),
            &Wanted::Everything,
            Filter::All,
            now(),
        );

        assert_eq!(snap.unconfigured, vec![]);
        assert!(
            !snap.unattributed.iter().any(|pane| pane.pane.id == "w:p2"),
            "ferry's pane was reported by a run reading orbital: {:#?}",
            snap.unattributed
        );
        let named: Vec<&str> = scoped.projects.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            named,
            ["orbital", "ferry"],
            "the config as written is still reachable"
        );
    }

    /// orbital, and a second project at a checkout that is really on disk so
    /// that a linked worktree of it can be too.
    fn orbital_and_ferry_at(checkout: &Path) -> Config {
        Config::from_toml(&format!(
            r#"
[[projects]]
name = "orbital"
path = "{ORBITAL}"

[[projects]]
name = "ferry"
path = "{}"
"#,
            checkout.display()
        ))
        .expect("the config parses")
    }

    /// A provider answering with one idle pane sitting in `cwd`, with no
    /// main-tree place on it — which is every pane as a provider answers,
    /// and what the collection has to translate.
    fn a_pane_sitting_in(cwd: &Path) -> Provider {
        Provider::holding(vec![pane(
            "w:p2",
            &cwd.display().to_string(),
            PaneStatus::Idle,
        )])
    }

    /// A pane in a linked worktree, outside every configured path: nothing
    /// in the config holds where it sits, and where it sits in ferry's main
    /// working tree is what says it is ferry's.
    ///
    /// This is the only test that runs the read the way `bdi` does. Every
    /// pane in `tests/fixtures/` comes off the wire with no main-tree place
    /// at all, and the tests in `join` and `snapshot` fill the field by
    /// hand — so a run that never translated the listing would satisfy all
    /// of them and still report this pane as `unconfigured`.
    #[test]
    fn a_pane_in_a_linked_worktree_is_reported_in_the_project_its_main_working_tree_is_under() {
        let fixture = a_linked_worktree_git_made("collection-linked-worktree");
        let cfg = orbital_and_ferry_at(&fixture.checkout);

        let snap = run(
            &cfg,
            &a_pane_sitting_in(&fixture.linked),
            &colliding_trackers(),
            Filter::All,
            now(),
        );

        assert_eq!(
            snap.unattributed,
            vec![LoosePane {
                pane: key("w:p2"),
                project: "ferry".to_string(),
                cwd: fixture.linked.display().to_string(),
                pane_status: PaneStatus::Idle,
                display_agent: None,
                title: None,
                claim_refused: false,
            }],
            "reported where it sits, placed by where its main working tree is"
        );
        assert_eq!(snap.unconfigured, vec![]);
    }

    /// The same pane under a run scoped to orbital. ferry is never asked
    /// where it is worked, so its linked worktrees are unknown and the pane
    /// is held by nothing as the config stands; placing it by its main
    /// working tree is what keeps another desktop's work off this screen
    /// instead of reporting it as a directory nobody configured.
    ///
    /// And nothing was asked to work that out beyond the listing. The
    /// collection is handed a provider and no runner, so what the provider
    /// recorded is the whole of what `bdi` put to anything.
    #[test]
    fn a_pane_in_an_excluded_projects_linked_worktree_is_placed_without_running_anything() {
        let fixture = a_linked_worktree_git_made("collection-linked-worktree-excluded");
        let cfg = orbital_and_ferry_at(&fixture.checkout)
            .scoped_to(&["orbital".to_string()])
            .expect("orbital is configured");
        let provider = a_pane_sitting_in(&fixture.linked);

        let snap = run(&cfg, &provider, &colliding_trackers(), Filter::All, now());

        assert_eq!(snap.unattributed, vec![]);
        assert_eq!(snap.unconfigured, vec![]);
        assert_eq!(provider.asked(), one_session_read());
    }

    /// What a collection asks a provider running the one session a test's
    /// panes are in: which sessions there are, and then that session's panes.
    fn one_session_read() -> Vec<AskedOfTheProvider> {
        vec![
            AskedOfTheProvider::Sessions,
            AskedOfTheProvider::List {
                session: A_SESSION.to_string(),
            },
        ]
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
            &panes_in_both(),
            &colliding_trackers(),
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
        let panes = panes_in_both();
        let trackers = colliding_trackers();
        let mut standing = Collection::default();
        collect(&mut standing, &panes, &trackers, &Wanted::Everything);

        let refreshed = collect(&mut standing, &panes, &trackers, &orbital_alone());
        let rebuilt = collect(
            &mut Collection::default(),
            &panes,
            &trackers,
            &Wanted::Everything,
        );

        assert_eq!(refreshed, rebuilt);
    }

    /// The saving the change channel exists for: a project nothing said had
    /// changed is not read again.
    #[test]
    fn refreshing_one_project_asks_no_other_projects_tracker() {
        let panes = panes_in_both();
        let trackers = colliding_trackers();
        let mut standing = Collection::default();
        collect(&mut standing, &panes, &trackers, &Wanted::Everything);
        let (orbital, ferry) = (asked_of(&trackers, "orbital"), asked_of(&trackers, "ferry"));

        collect(&mut standing, &panes, &trackers, &orbital_alone());

        assert_eq!(
            asked_of(&trackers, "ferry"),
            ferry,
            "ferry was not named, so its tracker was not asked again"
        );
        assert!(
            asked_of(&trackers, "orbital") > orbital,
            "orbital was named, so it was read"
        );
    }

    /// The refresh gate reaches the trackers and stops there. A provider
    /// reports on the sessions on the machine rather than on a project, so a
    /// collection asks it which sessions there are once and each session for
    /// its panes once — reading two projects, and reading the one a refresh
    /// named.
    #[test]
    fn a_collection_asks_the_provider_for_its_sessions_once_and_each_session_once() {
        let panes = Provider::holding(vec![
            named(pane("w:p1", ORBITAL, PaneStatus::Working), "x-1.1"),
            in_session(pane("w:p2", FERRY, PaneStatus::Idle), "beacon"),
        ]);
        let trackers = colliding_trackers();
        let mut standing = Collection::default();
        let both_sessions_read = vec![
            AskedOfTheProvider::Sessions,
            AskedOfTheProvider::List {
                session: A_SESSION.to_string(),
            },
            AskedOfTheProvider::List {
                session: "beacon".to_string(),
            },
        ];

        collect(&mut standing, &panes, &trackers, &Wanted::Everything);

        assert_eq!(
            panes.asked(),
            both_sessions_read,
            "two projects were read, and each session on the machine was asked once"
        );

        collect(&mut standing, &panes, &trackers, &orbital_alone());

        assert_eq!(
            panes.asked(),
            [both_sessions_read.clone(), both_sessions_read].concat(),
            "a refresh naming one project reads every session on the machine, once each"
        );
    }

    /// A session that will not answer takes nothing from the others: its
    /// panes are unknown and it is named as such, and every other session's
    /// seats are drawn as they would be had it never existed.
    #[test]
    fn a_session_that_will_not_answer_is_named_and_the_others_still_draw() {
        let panes = Provider::holding(vec![named(
            in_session(pane("w:p1", ORBITAL, PaneStatus::Working), "beacon"),
            "x-1.1",
        )])
        .not_answering_for("persistent-agents", wedged());

        let snap = run(
            &two_projects(),
            &panes,
            &colliding_trackers(),
            Filter::All,
            now(),
        );

        assert_eq!(snap.agents.state, ProviderState::Answering);
        assert_eq!(
            snap.agents.sessions,
            vec![
                Session {
                    name: A_SESSION.to_string(),
                    state: SessionState::Answering,
                },
                Session {
                    name: "beacon".to_string(),
                    state: SessionState::Answering,
                },
                Session {
                    name: "persistent-agents".to_string(),
                    state: SessionState::NotAnswering,
                },
            ]
        );
        assert_eq!(
            snap.agents.unanswered().collect::<Vec<_>>(),
            ["persistent-agents"]
        );
        let seat = node(tree_of(&snap, "orbital"), "x-1.1")
            .agent
            .as_ref()
            .expect("the seat in beacon is on its bead");
        assert_eq!(seat.pane.session, "beacon");
    }

    /// A session that is running and will not answer for its panes.
    fn wedged() -> RunFailure {
        RunFailure {
            kind: FailureKind::Unavailable,
            program: THE_FAKE.to_string(),
            detail: "no socket".to_string(),
        }
    }

    /// Three claims, each naming the pane its seat sits in, so a collection
    /// over two sessions can be asked about each of them separately.
    const SEATED_TREE: &str = r#"[
      {"id":"orb-7","title":"lift the ground station","status":"open",
       "priority":1,"issue_type":"epic"},
      {"id":"orb-7.1","title":"re-point the dish","status":"in_progress","parent":"orb-7",
       "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
       "priority":2,"issue_type":"task","metadata":{"agent_pane":"w:p1"}},
      {"id":"orb-7.2","title":"lay the feeder cable","status":"in_progress","parent":"orb-7",
       "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
       "priority":2,"issue_type":"task","metadata":{"agent_pane":"w:p2"}},
      {"id":"orb-7.3","title":"tune the receiver","status":"in_progress","parent":"orb-7",
       "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
       "priority":2,"issue_type":"task","metadata":{"agent_pane":"w:p3"}}
    ]"#;

    /// What a session that has gone quiet is allowed to take with it, and
    /// what it is not.
    ///
    /// A run holding no pane of a session cannot say whether the seat in it
    /// is alive, so the claim naming that seat keeps its silence. It can
    /// still say so about every other claim, and a seat that died in a
    /// session which answered is exactly what `orphan-claim` is for — so the
    /// two are asked in one collection, because a change that suppresses
    /// both is the whole rule going quiet on one session's hiccup.
    #[test]
    fn a_claim_whose_seat_is_in_a_session_that_went_quiet_is_not_orphaned_and_the_rest_still_are() {
        let cfg = one_project();
        let trackers = orbital_with(orbital_holding(SEATED_TREE));
        let mut standing = Collection::default();

        let seated = Provider::holding(vec![
            pane("w:p1", ORBITAL, PaneStatus::Working),
            in_session(pane("w:p2", ORBITAL, PaneStatus::Working), "beacon"),
            pane("w:p3", ORBITAL, PaneStatus::Working),
        ]);
        let before = standing.collect(
            &cfg,
            &seated,
            &trackers,
            &Wanted::Everything,
            Filter::All,
            now(),
        );
        assert_eq!(
            orphaned(&before),
            Vec::<&str>::new(),
            "every seat is on its pane while both sessions answer"
        );

        // beacon stops answering, and `w:p3` leaves the session that still does.
        let quiet = Provider::holding(vec![pane("w:p1", ORBITAL, PaneStatus::Working)])
            .not_answering_for("beacon", wedged());
        let after = standing.collect(
            &cfg,
            &quiet,
            &trackers,
            &Wanted::Everything,
            Filter::All,
            now(),
        );

        assert_eq!(
            after.agents.unanswered().collect::<Vec<_>>(),
            ["beacon"],
            "the session that went quiet is still a finding of its own"
        );
        assert_eq!(
            orphaned(&after),
            ["orb-7.3"],
            "the seat that died in the session that answered is reported, and \
             the one in the session that did not is not"
        );
        assert!(
            node(tree_of(&after, "orbital"), "orb-7.1").agent.is_some(),
            "the seat that answered is still drawn on its bead"
        );
    }

    /// The bound on that, said as a test rather than only in prose: a session
    /// that has never answered has left nothing behind to place its panes, so
    /// a claim naming one is reported as an orphan exactly as it was before.
    ///
    /// A session that wedges while `bdi` watches is placed within one
    /// collection, so on the screen this is a frame. A run that has watched
    /// nothing never places one at all, which is `bdi --json` every time it
    /// is invoked — tracked as `bdi-0tp.15`.
    #[test]
    fn a_claim_in_a_session_that_has_never_answered_is_orphaned_as_before() {
        let quiet = Provider::holding(vec![pane("w:p1", ORBITAL, PaneStatus::Working)])
            .not_answering_for("beacon", wedged());

        let snap = run(
            &one_project(),
            &quiet,
            &orbital_with(orbital_holding(SEATED_TREE)),
            Filter::All,
            now(),
        );

        assert_eq!(orphaned(&snap), ["orb-7.2", "orb-7.3"]);
    }

    /// What a session that stops being run takes with it. A name the provider
    /// no longer lists holds no pane, and a session running under that name
    /// later is not the one that went: what the old one was holding says
    /// nothing about the new one's seats, so it goes when the name does.
    #[test]
    fn a_session_the_provider_has_stopped_running_takes_what_it_was_holding() {
        let cfg = one_project();
        let trackers = orbital_with(orbital_holding(SEATED_TREE));
        let mut standing = Collection::default();

        let both = Provider::holding(vec![
            pane("w:p1", ORBITAL, PaneStatus::Working),
            in_session(pane("w:p2", ORBITAL, PaneStatus::Working), "beacon"),
        ]);
        standing.collect(
            &cfg,
            &both,
            &trackers,
            &Wanted::Everything,
            Filter::All,
            now(),
        );

        // beacon stops being run at all, and starts again under the same name.
        let alone = Provider::holding(vec![pane("w:p1", ORBITAL, PaneStatus::Working)]);
        standing.collect(
            &cfg,
            &alone,
            &trackers,
            &Wanted::Everything,
            Filter::All,
            now(),
        );
        let quiet = Provider::holding(vec![pane("w:p1", ORBITAL, PaneStatus::Working)])
            .not_answering_for("beacon", wedged());
        let after = standing.collect(
            &cfg,
            &quiet,
            &trackers,
            &Wanted::Everything,
            Filter::All,
            now(),
        );

        assert_eq!(
            orphaned(&after),
            ["orb-7.2", "orb-7.3"],
            "the pane list of the session that went is not the new one's"
        );
    }

    /// A pane id names a pane only within its session, and a seat writes the
    /// id alone. Where one session holds it the bead gets that pane; where two
    /// do, the bead gets neither and the disagreement says which sessions.
    #[test]
    fn a_pane_id_two_sessions_hold_is_awarded_to_nobody_and_reported() {
        let panes = Provider::holding(vec![
            titled(pane("w:p1", ORBITAL, PaneStatus::Working), "in default"),
            titled(
                in_session(pane("w:p1", ORBITAL, PaneStatus::Idle), "beacon"),
                "in beacon",
            ),
        ]);

        let snap = run(&one_project(), &panes, &orbital(), Filter::All, now());

        let claimed = node(tree_of(&snap, "orbital"), "orb-7.1");
        assert_eq!(claimed.agent, None, "neither pane is awarded");
        assert_eq!(
            claimed.anomalies,
            vec![Anomaly::OrphanClaim {
                refused: Some(Conflict::PaneIdInSeveralSessions {
                    bead: BeadKey {
                        project: "orbital".to_string(),
                        id: "orb-7.1".to_string(),
                    },
                    pane_id: "w:p1".to_string(),
                    sessions: vec![A_SESSION.to_string(), "beacon".to_string()],
                }),
            }]
        );
        assert_eq!(snap.conflicts.len(), 1);
        let loose: Vec<(&str, &str)> = snap
            .unattributed
            .iter()
            .map(|pane| (pane.pane.session.as_str(), pane.pane.id.as_str()))
            .collect();
        assert_eq!(
            loose,
            vec![(A_SESSION, "w:p1"), ("beacon", "w:p1")],
            "both panes are still drawn, each under its session"
        );
    }

    /// The property scoping exists for, one layer up from the refresh: a
    /// project the config no longer names is a project whose tracker is never
    /// asked, however whole the collection asking is.
    #[test]
    fn a_project_a_scope_left_out_has_its_tracker_unasked() {
        let trackers = colliding_trackers();
        let scoped = two_projects()
            .scoped_to(&["orbital".to_string()])
            .expect("orbital is configured");

        Collection::default().collect(
            &scoped,
            &panes_in_both(),
            &trackers,
            &Wanted::Everything,
            Filter::All,
            now(),
        );

        assert_eq!(
            asked_of(&trackers, "ferry"),
            0,
            "ferry was scoped out, so nothing should have gone near its tracker"
        );
        assert!(
            asked_of(&trackers, "orbital") > 0,
            "orbital was scoped in, so it was read"
        );
    }

    /// Degrade, never disappear, with further to reach than the whole-snapshot
    /// path ever had to: the projects already drawn are not the ones being
    /// read, so a tracker that fails mid-refresh cannot cost them anything.
    #[test]
    fn a_tracker_that_fails_while_one_project_refreshes_leaves_the_others_drawn() {
        let panes = panes_in_both();
        let mut standing = Collection::default();
        let before = collect(
            &mut standing,
            &panes,
            &colliding_trackers(),
            &Wanted::Everything,
        );

        let after = collect(&mut standing, &panes, &orbital_refusing(), &orbital_alone());

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
    /// it is the only chance the agent join gets. The provider is read
    /// again whatever a refresh names — it is one local call, and the join it
    /// feeds runs across every project by design. What a refresh leaves alone
    /// is the trackers it did not name, not the panes.
    #[test]
    fn a_refresh_naming_one_project_still_reads_the_provider() {
        let trackers = colliding_trackers();
        let mut standing = Collection::default();
        let before = collect(&mut standing, &no_panes(), &trackers, &Wanted::Everything);
        assert!(node(tree_of(&before, "ferry"), "x-1.1").agent.is_none());

        let arrived = Provider::holding(vec![named(
            pane("w:p2", FERRY, PaneStatus::Working),
            "x-1.1",
        )]);
        let after = collect(&mut standing, &arrived, &trackers, &orbital_alone());

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
        let panes = panes_in_both();
        let trackers = colliding_trackers();
        let mut standing = Collection::default();
        let cfg = two_projects();
        let earlier = now();
        let later = earlier + chrono::Duration::seconds(30);

        standing.collect(
            &cfg,
            &panes,
            &trackers,
            &Wanted::Everything,
            Filter::All,
            earlier,
        );
        let after = standing.collect(
            &cfg,
            &panes,
            &trackers,
            &orbital_alone(),
            Filter::All,
            later,
        );

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
        let panes = panes_in_both();
        let mut standing = Collection::default();
        let cfg = two_projects();
        let earlier = now();
        let later = earlier + chrono::Duration::seconds(30);
        standing.collect(
            &cfg,
            &panes,
            &colliding_trackers(),
            &Wanted::Everything,
            Filter::All,
            earlier,
        );

        let after = standing.collect(
            &cfg,
            &panes,
            &orbital_refusing(),
            &orbital_alone(),
            Filter::All,
            later,
        );

        assert!(
            trees_of(&after, "orbital").is_empty(),
            "nothing orbital's earlier read produced is still drawn"
        );
        assert_eq!(after.read_at["orbital"], later);
    }
}
