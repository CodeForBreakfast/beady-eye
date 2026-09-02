//! Everything one project's tracker is asked for, and what each answer means.
//!
//! The call sequence is the whole of it: the project's environment, the roots
//! discovery names, readiness, every bead, the top of each root's chain, and
//! the trees assembled from them. Nothing here knows that a read is kept
//! between collections, or that other projects exist.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};

use crate::collect::bd;
use crate::collect::environment;
use crate::collect::run::{Env, FailureKind, RunFailure, Runner};
use crate::config::{Config, Project};
use crate::model::join;
use crate::model::snapshot::{Readiness, TrackerFailure, TrackerState};
use crate::model::tree::{Assembled, Nesting};
use crate::model::types::{Bead, Pane};

/// One project's roots in id order, each either read or unreadable.
pub(super) struct ProjectWork {
    pub(super) readiness: Readiness,
    pub(super) roots: Vec<(String, Result<Assembled, RootUnread>)>,
}

/// Why a root drew no rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RootUnread {
    /// The tracker could not be read.
    Tracker(TrackerFailure),
    /// The tracker answered, and holds no bead of this id.
    NotFound,
}

impl From<RootUnread> for TrackerState {
    fn from(why: RootUnread) -> Self {
        match why {
            RootUnread::Tracker(failure) => TrackerState::Unreachable(failure),
            RootUnread::NotFound => TrackerState::RootNotFound,
        }
    }
}

/// What a project's rows were last read against, and when they stop speaking
/// for the tracker whatever anyone writes.
///
/// A skip is only allowed where the cascade would have answered the same, so
/// this has to cover every input to that answer and not only the tracker.
///
/// The panes are one of the others: a root can come from a pane rather than
/// from the tracker, because `panes_naming_a_bead_here` reaches beads
/// discovery never names. Gating on the tracker alone would keep such a tree
/// off the screen until some unrelated write moved the tracker, and the panes
/// cost nothing to compare — they are already in hand when the refresh starts.
///
/// The clock is the last of them. `bd ready` does not name a bead before its
/// `defer_until` and does after, and nothing is written when that instant
/// passes — measured 2026-09-01 against this project's own tracker, a bead
/// left `open` with `defer_until` two minutes out went from absent to present
/// across it with its row untouched. So a read carries the soonest instant at
/// which it stops being able to speak for the tracker.
#[derive(Clone)]
pub(super) struct ReadAt {
    working_root: String,
    named: BTreeSet<String>,
    speaks_until: Option<DateTime<Utc>>,
}

impl ReadAt {
    /// Whether a read taken against this still says what the tracker would
    /// say now.
    fn still_speaks_for(
        &self,
        working_root: &str,
        named: &BTreeSet<String>,
        now: DateTime<Utc>,
    ) -> bool {
        self.working_root == working_root
            && self.named == *named
            && self.speaks_until.is_none_or(|until| now < until)
    }
}

/// The soonest instant after `read_at` at which this tracker would answer
/// differently with nothing written, or `None` where it holds nothing back.
fn speaks_until(beads: &[Bead], read_at: DateTime<Utc>) -> Option<DateTime<Utc>> {
    beads
        .iter()
        .filter_map(|bead| bead.defer_until)
        .filter(|until| *until > read_at)
        .min()
}

/// What one refresh of one project did.
pub(super) enum Refresh {
    /// Nothing has moved since the read that is standing, so there is nothing
    /// to replace it with.
    Unchanged,
    /// What the tracker says now, and what it was read against. `None` where
    /// the probe could not answer, which has every later refresh read in full
    /// rather than compare against a state nobody established.
    Read {
        at: Option<ReadAt>,
        work: ProjectWork,
    },
}

/// One project's refresh: what it looks like now, and the cascade only if
/// that differs from what the standing read was taken against.
///
/// The probe cannot come first. It goes to the tracker, and reaching the
/// tracker needs the credential the environment capture produces — so an
/// unchanged project costs that capture and one `bd` invocation, against the
/// four a changed one still costs on top of them.
///
/// A tracker that cannot answer the probe is read the slow way.
/// `dolt_hashof_db()` is Dolt's and a SQLite-backed tracker has no such
/// function, so an error there means "read it the slow way", never "nothing
/// changed".
pub(super) fn refresh_project(
    runner: &dyn Runner,
    project: &Project,
    cfg: &Config,
    panes: &[Pane],
    standing: Option<&ReadAt>,
    now: DateTime<Utc>,
) -> Result<Refresh, RunFailure> {
    let env = environment::tracker_env(
        runner,
        project,
        environment::ambient_credential().as_deref(),
    )?;

    let probed = bd::working_root(runner, &project.path, &env).ok();
    let named: BTreeSet<String> = panes_naming_a_bead_here(panes, project, cfg)
        .map(str::to_string)
        .collect();

    if let (Some(working_root), Some(standing)) = (probed.as_deref(), standing) {
        if standing.still_speaks_for(working_root, &named, now) {
            return Ok(Refresh::Unchanged);
        }
    }

    let (work, beads) = read_project(runner, project, cfg, panes, &env)?;
    let at = probed.map(|working_root| ReadAt {
        working_root,
        named,
        speaks_until: speaks_until(&beads, now),
    });
    Ok(Refresh::Read { at, work })
}

/// Everything one project's tracker is asked for, and every bead it said it
/// with — the rows go back as well because what they hold decides how long
/// this read speaks for the tracker, which is not a question about the trees.
///
/// A failure before the roots are known has no root to name, so it becomes
/// the project's own failure rather than a tree; a failure on one root
/// afterwards is that root's.
fn read_project(
    runner: &dyn Runner,
    project: &Project,
    cfg: &Config,
    panes: &[Pane],
    env: &Env,
) -> Result<(ProjectWork, Vec<Bead>), RunFailure> {
    let beads = bd::all_beads(runner, &project.path, env)?;

    // An empty readiness set reads as "nothing here is ready", so a tracker
    // that cannot answer must not leave one behind.
    let readiness = Readiness {
        ready: bd::ready_ids(runner, &project.path, env)?,
        blocked_by: bd::blocked_by(runner, &project.path, env)?,
    };

    // Every bead this read of the tracker turned up, and the bead each one
    // hangs under. A parent chain that leaves it has run off the end of what
    // `bdi` read, and there is no tree to draw from where it went — so the
    // walk stops below that.
    let parents: BTreeMap<&str, Option<&str>> = beads
        .iter()
        .map(|bead| (bead.id.as_str(), bead.parent.as_deref()))
        .collect();

    let mut ancestors: BTreeMap<String, String> = BTreeMap::new();
    let mut roots: BTreeSet<String> = cfg
        .roots
        .explicit
        .get(&project.name)
        .into_iter()
        .flatten()
        .cloned()
        .collect();
    for bead in unfinished(&beads) {
        roots.extend(root_of(bead, &parents, &mut ancestors));
    }
    for named in panes_naming_a_bead_here(panes, project, cfg) {
        roots.extend(root_of(named, &parents, &mut ancestors));
    }

    // A root the answer does not hold is one config named: every other root
    // came out of the answer itself, so a tree cannot fail to assemble on
    // it.
    let nesting = Nesting::of(&beads);
    let mut read: Vec<(String, Result<Assembled, RootUnread>)> = roots
        .into_iter()
        .map(|root| {
            let read = nesting.assemble(&root).map_err(|_| RootUnread::NotFound);
            (root, read)
        })
        .collect();
    read.extend(what_no_root_reached(&nesting, &read));
    read.sort_by(|(one, _), (two, _)| one.cmp(two));

    Ok((
        ProjectWork {
            readiness,
            roots: read,
        },
        beads,
    ))
}

/// The beads the discovered roots left off the screen, each drawn from the
/// top of its own component.
///
/// A bead depending on work the tracker no longer holds keeps no way down to
/// it, and discovery only ever names the roots of unfinished work — so a bead
/// that lost its place is drawn by no tree unless a rule reaches it, and a
/// tree reports what it drew: absent from the picture and absent from the
/// report both. Drawing its component is what leaves it somewhere to be
/// reported from, and the top is where a tree that reaches it has to start:
/// the edge it kept may nest it under a bead nothing discovered, and that bead
/// lost nothing of its own to be found by.
///
/// Which is why `drawn` filters the bead and not the top. A bead a tree
/// already draws needs nothing standing up over it.
fn what_no_root_reached(
    nesting: &Nesting,
    read: &[(String, Result<Assembled, RootUnread>)],
) -> Vec<(String, Result<Assembled, RootUnread>)> {
    let drawn: BTreeSet<&str> = read
        .iter()
        .filter_map(|(_, read)| read.as_ref().ok())
        .flat_map(|assembled| assembled.beads.iter().map(|bead| bead.id.as_str()))
        .collect();

    let tops: BTreeSet<String> = nesting
        .adrift()
        .into_iter()
        .filter(|id| !drawn.contains(id.as_str()))
        .flat_map(|id| nesting.top_of(&id))
        .collect();

    tops.into_iter()
        .map(|id| {
            let read = nesting
                .assemble(&id)
                .map_err(|_| RootUnread::Tracker(TrackerFailure::Parse));
            (id, read)
        })
        .collect()
}

/// Every bead that marks unfinished work: the beads discovery climbs to a
/// root from, read off the listing rather than asked of bd, which would
/// answer with a subset of the rows already in hand.
///
/// Unfinished rather than claimed: an effort holds the work it has left after
/// its last seat stands down, and a rule that noticed only a claim lost the
/// whole tree at that moment. `closed` is the only status bd stores that this
/// leaves out, and that is the whole of the rule.
///
/// Wisps are in the listing too, and one with no parent is a root of its own:
/// every step of a bd molecule hangs under it, so the one rootless row is the
/// whole run.
fn unfinished(beads: &[Bead]) -> impl Iterator<Item = &str> {
    beads
        .iter()
        .filter(|bead| !bead.status.is_closed())
        .map(|bead| bead.id.as_str())
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

/// The top of a bead's parent-child chain, or `None` for an id this read
/// does not hold.
///
/// `bd dep tree` cannot answer this: `--direction=up` walks dependents, so
/// whatever bead it is asked about comes back as its own root. The rows can —
/// every one carries the bead's own `parent` — so the climb is answered from
/// `parents`, the read already in hand, and asks the tracker nothing.
/// `ancestors` carries what earlier walks found, so a parent many beads
/// share is climbed past once.
///
/// The climb stops below a parent `parents` does not hold. A row can name one
/// the answer lacks — three were measured against summit-works on 2026-08-31
/// — and climbing to it names a root there is no tree to draw from, which
/// reported a tracker that had answered every call as one whose answer could
/// not be read. Nothing goes missing by stopping: the bead below still names
/// the parent the answer lost, and its own tree reports that as work the
/// tracker no longer holds.
///
/// A miss on `id` itself is the other side of the same rule. Every id
/// discovery names is in `parents`; the one that need not be is the free text
/// a pane's `display_agent` may hold, and a sentence has no chain to climb.
fn root_of<'a>(
    id: &'a str,
    parents: &BTreeMap<&'a str, Option<&'a str>>,
    ancestors: &mut BTreeMap<String, String>,
) -> Option<String> {
    if !parents.contains_key(id) {
        return None;
    }
    let mut climbed: Vec<&str> = Vec::new();
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut current = id;

    let root = loop {
        if let Some(known) = ancestors.get(current) {
            break known.clone();
        }
        // A parent chain that loops has no top. Stopping where it repeats
        // keeps the bead visible rather than hanging on it.
        if !seen.insert(current) {
            break current.to_string();
        }
        climbed.push(current);
        match parents[current] {
            Some(parent) if parents.contains_key(parent) => current = parent,
            // A parent this read does not hold, or no parent at all: either
            // way the chain has nothing further this read can draw.
            Some(_) | None => break current.to_string(),
        }
    };

    for climbed in climbed {
        ancestors.insert(climbed.to_string(), root.clone());
    }
    Some(root)
}

/// The kinds bd's collector can produce, in the model's own vocabulary.
/// `RunFailure.detail` stops here: bd names the database and the SQL user
/// when it refuses a credential, and the words for a failure belong to
/// whatever draws it.
///
/// `Gone` and `Busy` are herdr's, and a tracker cannot answer with either.
/// `TrackerFailure` stays as it is rather than learning a word for a pane.
pub(super) fn tracker_failure(kind: FailureKind) -> TrackerFailure {
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
    use crate::app::fixtures::*;
    use crate::app::run;
    use crate::model::snapshot::{FailedProject, Filter, Snapshot, TrackerState, Tree};
    use crate::model::tree::nestings_on_this_thread;
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;

    /// A second root, reached only because config or a pane names it: closed,
    /// so no status does.
    const MAST_TREE: &str = r#"[
      {"id":"orb-4","title":"survey the mast","status":"closed",
       "priority":2,"issue_type":"task"}
    ]"#;

    /// The orbital tracker with its epic finished and its tasks not: the
    /// shape discovery never sees the top of.
    const CLOSED_OVER_OPEN_WORK: &str = r#"[
      {"id":"orb-7","title":"lift the ground station","status":"closed",
       "priority":1,"issue_type":"epic"},
      {"id":"orb-7.1","title":"re-point the dish","status":"in_progress","parent":"orb-7",
       "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
       "priority":2,"issue_type":"task",
       "metadata":{"agent_pane":"w:p1"}},
      {"id":"orb-7.2","title":"lay the feeder cable","status":"open","parent":"orb-7",
       "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
       "priority":2,"issue_type":"task"}
    ]"#;

    fn rooted_at<'a>(snap: &'a Snapshot, root: &str) -> &'a Tree {
        snap.trees
            .iter()
            .find(|t| t.root == root)
            .unwrap_or_else(|| panic!("{root} is drawn"))
    }

    // ---- discovery ----------------------------------------------------

    /// Discovery reads the listing the forest is drawn from, so a changed
    /// refresh lists the tracker once and its wisps once, and asks bd for no
    /// subset of either.
    #[test]
    fn a_changed_refresh_lists_the_tracker_once_and_its_wisps_once() {
        let runner = orbital();

        run(&one_project(), &runner, Filter::All, now());

        let listings: Vec<String> = runner
            .calls()
            .into_iter()
            .map(|call| call.argv)
            .filter(|argv| {
                argv.starts_with(&spelled("list ")) || argv.starts_with(&spelled("query "))
            })
            .collect();
        assert_eq!(listings, vec![spelled(TRACKER_CALL), spelled(WISP_CALL)]);
    }

    /// The same edges nest the same beads whichever root is walked, so one
    /// read of a tracker reads them once, however many trees it draws from
    /// them — three here: the discovered epic, a root only config names, and
    /// the top of a component no root reached.
    #[test]
    fn one_read_nests_the_tracker_once_however_many_roots_it_draws() {
        let cfg = Config::from_toml(&format!(
            r#"
[[projects]]
name = "orbital"
path = "{ORBITAL}"

[roots.explicit]
orbital = ["orb-4"]
"#
        ))
        .expect("the config parses");
        let lost = r#"[{"id":"orb-3","title":"its parent was deleted","status":"closed",
                        "dependencies":[{"depends_on_id":"orb-404","type":"parent-child"}],
                        "priority":2,"issue_type":"task"}]"#;
        let runner = orbital()
            .merging(&spelled(TRACKER_CALL), MAST_TREE)
            .merging(&spelled(TRACKER_CALL), lost);

        let before = nestings_on_this_thread();
        let (work, _) = read_project(&runner, &cfg.projects[0], &cfg, &[], &Env::new())
            .expect("the tracker answers every call");

        let roots: Vec<&str> = work.roots.iter().map(|(root, _)| root.as_str()).collect();
        assert_eq!(roots, vec!["orb-3", "orb-4", "orb-7"]);
        assert_eq!(nestings_on_this_thread() - before, 1);
    }

    /// The whole point of the ancestor walk: an in-flight task is drawn as
    /// the tree it hangs under, not as a root of its own.
    #[test]
    fn a_discovered_bead_is_drawn_as_the_tree_it_hangs_under() {
        let snap = run(&one_project(), &orbital(), Filter::LiveAgents, now());

        assert_eq!(snap.trees.len(), 1);
        assert_eq!(snap.trees[0].root, "orb-7");
        assert_eq!(snap.trees[0].title, "lift the ground station");
        assert_eq!(snap.trees[0].beads.len(), 3);
    }

    /// The shapes this project's own tracker held on 2026-09-02, drawn from
    /// the one listing: a closed epic over open work, an epic finished whole,
    /// a deferred bead, a wisp left open under no parent, a wisp closed, and
    /// a closed bead still carrying a `working_topic`.
    #[test]
    fn discovery_names_every_unfinished_bead_and_wisp_and_nothing_closed() {
        let listing = r#"[
          {"id":"orb-7","title":"lift the ground station","status":"closed",
           "priority":1,"issue_type":"epic"},
          {"id":"orb-7.1","title":"re-point the dish","status":"open","parent":"orb-7",
           "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
           "priority":2,"issue_type":"task"},
          {"id":"orb-8","title":"decommission the old mast","status":"closed",
           "priority":1,"issue_type":"epic"},
          {"id":"orb-8.1","title":"cut the guy lines","status":"closed","parent":"orb-8",
           "dependencies":[{"depends_on_id":"orb-8","type":"parent-child"}],
           "priority":2,"issue_type":"task",
           "metadata":{"working_topic":"orbital/v1-orb-8.1"}},
          {"id":"orb-9","title":"wait for the permit","status":"deferred",
           "priority":3,"issue_type":"task"}
        ]"#;
        let wisps = r#"[
          {"id":"orb-wisp-a1","title":"heartbeat","status":"open",
           "priority":2,"issue_type":"task"},
          {"id":"orb-wisp-b2","title":"a run that finished","status":"closed",
           "priority":2,"issue_type":"molecule"}
        ]"#;
        let runner = orbital()
            .with("herdr agent list", r#"{"result":{"agents":[]}}"#)
            .with(&spelled(TRACKER_CALL), listing)
            .with(&spelled(WISP_CALL), wisps);

        let snap = run(&one_project(), &runner, Filter::All, now());

        assert!(
            snap.failed_projects.is_empty(),
            "{:?}",
            snap.failed_projects
        );
        let roots: BTreeSet<&str> = snap.trees.iter().map(|t| t.root.as_str()).collect();
        assert_eq!(
            roots,
            BTreeSet::from(["orb-7", "orb-9", "orb-wisp-a1"]),
            "the open task climbs to its closed epic; the deferred bead and the open wisp are roots of their own"
        );
    }

    /// The digest case measured against summit-works on 2026-08-31: a bead
    /// names a parent that `bd list --all` does not hold. The chain stops at
    /// the last bead this read holds, which is a missing ancestor and not a
    /// tracker outage — every call was answered, so no tree calls the tracker
    /// unreadable, and the bead is top of its own tree, which is where the
    /// parent the answer lost is reported from.
    ///
    /// Climbing to the parent named a root no tree could be drawn from, and
    /// the project grew a tree reported as a tracker that had answered with
    /// something `bdi` could not read.
    #[test]
    fn a_bead_whose_parent_the_answer_has_lost_is_top_of_its_own_tree() {
        let orphan_bead = r#"[{"id":"orb-7.9","title":"its parent is a digest",
                               "status":"open","parent":"orb-404",
                               "dependencies":[{"depends_on_id":"orb-404","type":"parent-child"}],
                               "priority":2,"issue_type":"task"}]"#;
        let runner = orbital().merging(&spelled(TRACKER_CALL), orphan_bead);

        let snap = run(&one_project(), &runner, Filter::All, now());

        assert!(
            snap.failed_projects.is_empty(),
            "one bead the read cannot place must not take the tracker down: {:?}",
            snap.failed_projects
        );
        let roots: Vec<&str> = snap.trees.iter().map(|t| t.root.as_str()).collect();
        assert_eq!(
            roots,
            vec!["orb-7", "orb-7.9"],
            "the chain stops at the last bead this read holds, and names no root above it"
        );
        for tree in &snap.trees {
            assert_eq!(
                tree.tracker,
                TrackerState::Ok,
                "{} answered every call, so no tree of its calls it unreadable",
                tree.root
            );
        }
        assert_eq!(
            rooted_at(&snap, "orb-7").dangling,
            Vec::<String>::new(),
            "the tree that never reached it does not report it"
        );
        assert_eq!(
            rooted_at(&snap, "orb-7.9").dangling,
            vec!["orb-7.9".to_string()],
            "its own tree names the work the tracker no longer holds"
        );
    }

    /// A closed bead is never discovered — statuses, wisps and metadata keys
    /// are all populations of unfinished work — so nothing makes it a root
    /// and nothing climbs from it. It is the same defect with nothing else
    /// moving.
    #[test]
    fn a_closed_bead_the_answer_holds_no_way_down_to_is_still_drawn() {
        let lost = r#"[{"id":"orb-3","title":"its parent was deleted","status":"closed",
                        "dependencies":[{"depends_on_id":"orb-404","type":"parent-child"}],
                        "priority":2,"issue_type":"task"}]"#;
        let runner = orbital().merging(&spelled(TRACKER_CALL), lost);

        let snap = run(&one_project(), &runner, Filter::All, now());

        // The tree somebody is working leads, as it does whatever else is
        // drawn beside it.
        let roots: Vec<&str> = snap.trees.iter().map(|t| t.root.as_str()).collect();
        assert_eq!(roots, vec!["orb-7", "orb-3"]);
        assert_eq!(
            rooted_at(&snap, "orb-3").dangling,
            vec!["orb-3".to_string()]
        );
    }

    /// The gap `codex review` found in `bdi-7ao.49`, and the other end of the
    /// same shortfall. A closed bead hangs under two parents: one the answer
    /// has lost, one it holds. The edge it kept places it, so nothing nests
    /// it *nowhere* — and the parent that kept it is closed, so no rule of
    /// unfinished work discovers that either. The whole component sat off the
    /// screen, drawn in no tree and reported in none.
    #[test]
    fn a_component_nothing_discovered_is_drawn_from_its_top() {
        let component = r#"[{"id":"orb-5","title":"the parent it was moved to","status":"closed",
                             "priority":2,"issue_type":"task"},
                            {"id":"orb-5.1","title":"moved off a parent that is gone",
                             "status":"closed",
                             "dependencies":[{"depends_on_id":"orb-404","type":"parent-child"},
                                             {"depends_on_id":"orb-5","type":"parent-child"}],
                             "priority":2,"issue_type":"task"}]"#;
        let runner = orbital().merging(&spelled(TRACKER_CALL), component);

        let snap = run(&one_project(), &runner, Filter::All, now());

        let roots: Vec<&str> = snap.trees.iter().map(|t| t.root.as_str()).collect();
        assert_eq!(roots, vec!["orb-7", "orb-5"]);
        let component = rooted_at(&snap, "orb-5");
        assert_eq!(
            component
                .beads
                .iter()
                .map(|n| n.id.as_str())
                .collect::<Vec<_>>(),
            vec!["orb-5", "orb-5.1"],
            "the bead that lost its place is drawn where its surviving edge puts it"
        );
        assert_eq!(
            component.dangling,
            vec!["orb-5.1".to_string()],
            "and the tree that draws it is the one that reports it"
        );
    }

    /// The same component, with a loop in it above the bead that lost its
    /// place. A loop has no top, and the bead the climb started from is not
    /// one: a tree rooted there draws what is under it and leaves the loop it
    /// hangs from off the screen — the whole thing this rule exists to stop.
    #[test]
    fn a_component_whose_top_is_a_loop_is_drawn_from_inside_the_loop() {
        let looping = r#"[{"id":"orb-9","title":"each other's parent","status":"closed",
                           "dependencies":[{"depends_on_id":"orb-9b","type":"parent-child"}],
                           "priority":2,"issue_type":"epic"},
                          {"id":"orb-9b","title":"and the other way round","status":"closed",
                           "dependencies":[{"depends_on_id":"orb-9","type":"parent-child"}],
                           "priority":2,"issue_type":"epic"},
                          {"id":"orb-9.1","title":"under the loop, and off a parent that is gone",
                           "status":"closed",
                           "dependencies":[{"depends_on_id":"orb-9","type":"parent-child"},
                                           {"depends_on_id":"orb-404","type":"parent-child"}],
                           "priority":2,"issue_type":"task"}]"#;
        let runner = orbital().merging(&spelled(TRACKER_CALL), looping);

        let snap = run(&one_project(), &runner, Filter::All, now());

        let roots: Vec<&str> = snap.trees.iter().map(|t| t.root.as_str()).collect();
        assert_eq!(roots, vec!["orb-7", "orb-9"]);
        let component = rooted_at(&snap, "orb-9");
        assert_eq!(
            component
                .beads
                .iter()
                .map(|n| n.id.as_str())
                .collect::<Vec<_>>(),
            vec!["orb-9", "orb-9.1", "orb-9b"],
            "every bead in the component is drawn, loop included"
        );
        assert_eq!(
            component.cycles,
            vec!["orb-9".to_string()],
            "and the loop is reported where it was cut"
        );
        assert_eq!(component.dangling, vec!["orb-9.1".to_string()]);
    }

    /// The same again, where the bead hangs under a loop *and* under
    /// something with a top of its own. The top draws the bead, so the bead
    /// is placed and reported — and the loop it also hangs from is still
    /// nowhere, because a tree only ever walks downward. Every bead the climb
    /// reached has to end up on the screen, not just the one it started at.
    #[test]
    fn a_loop_over_a_bead_is_drawn_even_where_another_parent_has_a_top() {
        let both_ways = r#"[{"id":"orb-6","title":"a top of its own","status":"closed",
                             "priority":2,"issue_type":"epic"},
                            {"id":"orb-6c","title":"each other's parent","status":"closed",
                             "dependencies":[{"depends_on_id":"orb-6d","type":"parent-child"}],
                             "priority":2,"issue_type":"epic"},
                            {"id":"orb-6d","title":"and the other way round","status":"closed",
                             "dependencies":[{"depends_on_id":"orb-6c","type":"parent-child"}],
                             "priority":2,"issue_type":"epic"},
                            {"id":"orb-6.1","title":"under both, and off a parent that is gone",
                             "status":"closed",
                             "dependencies":[{"depends_on_id":"orb-6","type":"parent-child"},
                                             {"depends_on_id":"orb-6c","type":"parent-child"},
                                             {"depends_on_id":"orb-404","type":"parent-child"}],
                             "priority":2,"issue_type":"task"}]"#;
        let runner = orbital().merging(&spelled(TRACKER_CALL), both_ways);

        let snap = run(&one_project(), &runner, Filter::All, now());

        let ids = |root: &str| {
            rooted_at(&snap, root)
                .beads
                .iter()
                .map(|n| n.id.to_string())
                .collect::<Vec<_>>()
        };
        let roots: Vec<&str> = snap.trees.iter().map(|t| t.root.as_str()).collect();
        assert_eq!(roots, vec!["orb-7", "orb-6", "orb-6c"]);
        assert_eq!(ids("orb-6"), vec!["orb-6", "orb-6.1"]);
        assert_eq!(
            ids("orb-6c"),
            vec!["orb-6c", "orb-6.1", "orb-6d"],
            "the loop the top could not reach is drawn from inside itself"
        );
    }

    /// And no more of them than that. Reaching a loop means standing a bead
    /// up for it, and the bead the loop stands over can be the one reached
    /// first — leaving two roots where the second's tree already draws the
    /// first, and every bead in it on the screen twice.
    #[test]
    fn a_top_another_top_already_draws_is_not_a_root_as_well() {
        let under_a_loop = r#"[{"id":"orb-2a","title":"under the loop, off a parent that is gone",
                                "status":"closed",
                                "dependencies":[{"depends_on_id":"orb-2c","type":"parent-child"},
                                                {"depends_on_id":"orb-404","type":"parent-child"}],
                                "priority":2,"issue_type":"task"},
                               {"id":"orb-2c","title":"each other's parent","status":"closed",
                                "dependencies":[{"depends_on_id":"orb-2d","type":"parent-child"}],
                                "priority":2,"issue_type":"epic"},
                               {"id":"orb-2d","title":"and the other way round","status":"closed",
                                "dependencies":[{"depends_on_id":"orb-2c","type":"parent-child"}],
                                "priority":2,"issue_type":"epic"}]"#;
        let runner = orbital().merging(&spelled(TRACKER_CALL), under_a_loop);

        let snap = run(&one_project(), &runner, Filter::All, now());

        let roots: Vec<&str> = snap.trees.iter().map(|t| t.root.as_str()).collect();
        assert_eq!(
            roots,
            vec!["orb-7", "orb-2c"],
            "one root draws the whole component, so it is the only one"
        );
        assert_eq!(
            rooted_at(&snap, "orb-2c")
                .beads
                .iter()
                .map(|n| n.id.as_str())
                .collect::<Vec<_>>(),
            vec!["orb-2c", "orb-2a", "orb-2d"]
        );
    }

    /// An edge kind `bdi` does not know nests nothing, so a target it names
    /// and the answer has lost takes no place away. The bead is where it
    /// always was — nowhere, if nothing discovered it — and standing it up as
    /// a root for having named a lost id would put a bead no rule found on
    /// the screen.
    #[test]
    fn a_bead_whose_absent_dependency_would_have_nested_nothing_is_not_a_root() {
        let unrelated = r#"[{"id":"orb-2","title":"found by work that is gone","status":"closed",
                             "dependencies":[{"depends_on_id":"orb-404","type":"discovered-by"}],
                             "priority":2,"issue_type":"task"}]"#;
        let runner = orbital().merging(&spelled(TRACKER_CALL), unrelated);

        let snap = run(&one_project(), &runner, Filter::All, now());

        let roots: Vec<&str> = snap.trees.iter().map(|t| t.root.as_str()).collect();
        assert_eq!(roots, vec!["orb-7"]);
    }

    /// A blocker the answer has lost would have been drawn *under* the bead
    /// waiting on it, not over it. So nothing about where that bead is drawn
    /// went missing with it, and it is where it always was.
    #[test]
    fn a_bead_whose_absent_dependency_would_have_hung_beneath_it_is_not_a_root() {
        let waiting = r#"[{"id":"orb-2","title":"waiting on work that is gone","status":"closed",
                           "dependencies":[{"depends_on_id":"orb-404","type":"blocks"}],
                           "priority":2,"issue_type":"task"}]"#;
        let runner = orbital().merging(&spelled(TRACKER_CALL), waiting);

        let snap = run(&one_project(), &runner, Filter::All, now());

        let roots: Vec<&str> = snap.trees.iter().map(|t| t.root.as_str()).collect();
        assert_eq!(roots, vec!["orb-7"]);
    }

    /// The narrowing this rule turns on. A bead that lost one edge and kept
    /// another is placed by the one it kept, and a tree already draws it —
    /// so drawing it again as a root of its own would put it on the screen
    /// twice and count it twice.
    #[test]
    fn a_bead_a_tree_already_draws_is_not_made_a_root_as_well() {
        let also_waiting = r#"[{"id":"orb-7.1","title":"re-point the dish","status":"in_progress","parent":"orb-7",
                                "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"},
                                                {"depends_on_id":"orb-404","type":"parent-child"}],
                                "priority":2,"issue_type":"task",
                                "metadata":{"agent_pane":"w:p1"}}]"#;
        let runner = orbital().with(
            &spelled(TRACKER_CALL),
            &ORBITAL_TREE.replace(
                r#"{"id":"orb-7.1","title":"re-point the dish","status":"in_progress","parent":"orb-7",
       "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
       "priority":2,"issue_type":"task",
       "metadata":{"agent_pane":"w:p1"}}"#,
                also_waiting
                    .trim()
                    .trim_start_matches('[')
                    .trim_end_matches(']'),
            ),
        );

        let snap = run(&one_project(), &runner, Filter::All, now());

        let roots: Vec<&str> = snap.trees.iter().map(|t| t.root.as_str()).collect();
        assert_eq!(
            roots,
            vec!["orb-7"],
            "one tree draws it, so one tree reports it"
        );
        assert_eq!(snap.trees[0].dangling, vec!["orb-7.1".to_string()]);
    }

    /// A bead the lost bead's own tree draws is not a second root either,
    /// however many edges it lost of its own.
    #[test]
    fn a_bead_under_a_lost_bead_is_drawn_under_it_rather_than_beside_it() {
        let lost = r#"[{"id":"orb-3","title":"its parent was deleted","status":"closed",
                        "dependencies":[{"depends_on_id":"orb-404","type":"parent-child"}],
                        "priority":2,"issue_type":"task"},
                       {"id":"orb-3.1","title":"under it, waiting on more","status":"closed",
                        "dependencies":[{"depends_on_id":"orb-3","type":"parent-child"},
                                        {"depends_on_id":"orb-405","type":"parent-child"}],
                        "priority":2,"issue_type":"task"}]"#;
        let runner = orbital().merging(&spelled(TRACKER_CALL), lost);

        let snap = run(&one_project(), &runner, Filter::All, now());

        let roots: Vec<&str> = snap.trees.iter().map(|t| t.root.as_str()).collect();
        assert_eq!(roots, vec!["orb-7", "orb-3"]);
        let lost = rooted_at(&snap, "orb-3");
        assert_eq!(
            lost.beads.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
            vec!["orb-3", "orb-3.1"]
        );
        assert_eq!(
            lost.dangling,
            vec!["orb-3".to_string(), "orb-3.1".to_string()]
        );
    }

    /// A closed bead above unfinished children is the normal healthy shape of
    /// this tree, and discovery never sees one. `bd list --all` carries every
    /// bead's own parent, so the climb past it is answered from the read
    /// already in hand: it costs no process, however many beads share it.
    #[test]
    fn a_closed_parent_over_open_work_costs_no_bd_show() {
        let runner = orbital()
            .with(&spelled(TRACKER_CALL), CLOSED_OVER_OPEN_WORK)
            // Answered, so that the fake does not panic: what is counted is
            // whether it is asked at all.
            .with(
                &spelled("show orb-7 --json"),
                r#"[{"id":"orb-7","parent":null}]"#,
            );

        let snap = run(&one_project(), &runner, Filter::All, now());

        assert_eq!(snap.trees[0].root, "orb-7");
        let shown: Vec<String> = runner
            .calls()
            .into_iter()
            .map(|call| call.argv)
            .filter(|argv| argv.starts_with(&spelled("show ")))
            .collect();
        assert_eq!(
            shown,
            Vec::<String>::new(),
            "a parent the listing already carries is not asked for again"
        );
    }

    /// The defect this rule replaces. Every seat stood down, so nothing was
    /// `in_progress` or `blocked`, so discovery found no bead, so no root, so
    /// the effort was not drawn at all — and whichever epic held the one bead
    /// still claimed was drawn in its place.
    #[test]
    fn an_effort_is_drawn_from_the_open_work_under_it_with_nobody_on_it() {
        let runner = orbital()
            .with("herdr agent list", r#"{"result":{"agents":[]}}"#)
            .with(
                &spelled(TRACKER_CALL),
                r#"[{"id":"orb-7","title":"lift the ground station","status":"open",
                     "priority":1,"issue_type":"epic"},
                    {"id":"orb-7.2","title":"lay the feeder cable","status":"open","parent":"orb-7",
                     "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
                     "priority":2,"issue_type":"task"}]"#,
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
            // Both ends of the loop, because a climb stops below a parent
            // this read does not hold — and then the cycle guard, not the
            // cycle, would be what this test never reaches.
            .with(&spelled(TRACKER_CALL),
                r#"[{"id":"orb-7","title":"lift the ground station","status":"closed","parent":"orb-7.1",
                     "priority":1,"issue_type":"epic"},
                    {"id":"orb-7.1","title":"re-point the dish","status":"in_progress","parent":"orb-7",
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

    /// What this project's own tracker answered `bd list --all --json` with,
    /// captured rather than written: the ids a root is checked against are
    /// the ones a real answer carries.
    const A_CAPTURED_ANSWER: &str = include_str!("../../tests/fixtures/bd_list.json");

    /// The tracker answered every call and holds no bead of that id. That is
    /// a fact about the config, and a reader sent to look at the tracker is
    /// sent to the one thing that did nothing wrong.
    #[test]
    fn a_root_named_in_config_that_the_answer_does_not_hold_is_not_an_unreadable_tracker() {
        let cfg = Config::from_toml(&format!(
            r#"
[[projects]]
name = "orbital"
path = "{ORBITAL}"

[roots.explicit]
orbital = ["bdi-404"]
"#
        ))
        .expect("the config parses");
        let runner = orbital()
            .with(&spelled("ready --limit 0 --json"), "[]")
            .with(&spelled("blocked --json"), "[]")
            .with(&spelled(TRACKER_CALL), A_CAPTURED_ANSWER);

        let snap = run(&cfg, &runner, Filter::All, now());

        assert!(
            snap.failed_projects.is_empty(),
            "the project's own tracker answered: {:?}",
            snap.failed_projects
        );
        assert_eq!(
            rooted_at(&snap, "bdi-404").tracker,
            TrackerState::RootNotFound
        );
        assert!(
            snap.trees
                .iter()
                .all(|tree| tree.tracker != TrackerState::Unreachable(TrackerFailure::Parse)),
            "no tree blames the tracker's answer: {:?}",
            snap.trees
                .iter()
                .map(|tree| (tree.root.as_str(), tree.tracker))
                .collect::<Vec<_>>()
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
    /// A sentence is an id the read does not hold, so the guess costs no
    /// call and names no root: the pane belongs in `unattributed`, and
    /// taking the whole tracker down for one is the opposite of degrading.
    #[test]
    fn a_pane_labelled_with_something_that_is_not_a_bead_costs_the_project_nothing() {
        let runner = orbital().with(
            "herdr agent list",
            r#"{"result":{"agents":[
              {"pane_id":"w:p4","cwd":"/srv/work/orbital","agent_status":"working",
               "display_agent":"reviewing the docs"}
            ]}}"#,
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
        // The fake panics on a call it has no response for, so the label
        // never reaching bd is what lets this get to its assertions.
        assert!(
            runner
                .calls()
                .iter()
                .all(|call| !call.argv.contains("reviewing the docs")),
            "the label is looked up in the read, not sent to the tracker"
        );
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
            .merging(
                &spelled(TRACKER_CALL),
                r#"[{"id":"orb-7.3","title":"written up","status":"closed","parent":"orb-7",
                     "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
                     "priority":2,"issue_type":"task"}]"#,
            );

        let snap = run(&one_project(), &runner, Filter::All, now());

        let roots: Vec<&str> = snap.trees.iter().map(|t| t.root.as_str()).collect();
        assert_eq!(
            roots,
            vec!["orb-7"],
            "climbed to its root, and deduped there"
        );
    }

    /// A pane names its bead by id alone, and prefixes are uncoordinated
    /// across trackers — so the root it contributes belongs to the project
    /// its directory sits in, and no other tracker's read is asked about the
    /// id. Both trackers hold a bead of that id, because a tracker that did
    /// not would draw no root for it whichever project the pane was put in.
    #[test]
    fn a_pane_contributes_its_root_only_to_the_project_it_sits_in() {
        let runner = colliding_trackers(
            r#"{"result":{"agents":[
              {"pane_id":"w:p4","cwd":"/srv/work/orbital","agent_status":"working",
               "display_agent":"orb-4"}
            ]}}"#,
        )
        .merging(&spelled_in(ORBITAL, TRACKER_CALL), MAST_TREE)
        .merging(&spelled_in(FERRY, TRACKER_CALL), MAST_TREE);

        let snap = run(&two_projects(), &runner, Filter::All, now());

        let roots: Vec<(&str, &str)> = snap
            .trees
            .iter()
            .map(|t| (t.project.as_str(), t.root.as_str()))
            .collect();
        assert_eq!(
            roots,
            vec![("orbital", "orb-4"), ("orbital", "x-1"), ("ferry", "x-1")],
            "ferry draws no tree for a bead no pane of its own named"
        );
    }

    /// A pane has to resolve to a project before the id it names means
    /// anything, because there is no tracker to ask otherwise. The tracker
    /// holds a bead of that id, so a pane wrongly placed in the project
    /// would draw a root for it.
    #[test]
    fn a_pane_under_no_configured_project_contributes_no_root() {
        let runner = orbital()
            .with(
                "herdr agent list",
                r#"{"result":{"agents":[
                  {"pane_id":"w:p4","cwd":"/srv/elsewhere","agent_status":"working",
                   "display_agent":"orb-4"}
                ]}}"#,
            )
            .merging(&spelled(TRACKER_CALL), MAST_TREE);

        let snap = run(&one_project(), &runner, Filter::All, now());

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
    fn a_project_whose_listing_fails_is_named_not_dropped() {
        let runner = orbital().failing(&spelled(TRACKER_CALL), failing(FailureKind::Auth));

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
            let runner = orbital().failing(&spelled(TRACKER_CALL), failing(kind));

            let snap = run(&one_project(), &runner, Filter::All, now());

            assert_eq!(snap.failed_projects[0].tracker, expected, "on {kind:?}");
        }
    }

    /// A root config names that the answer does not hold keeps its id, which
    /// is what sends a reader to the right config entry, and the filter has
    /// no agent count to hide it by. It is the one root that can be unheld:
    /// every other comes out of the answer itself.
    #[test]
    fn a_root_the_answer_does_not_hold_keeps_its_id_and_is_never_hidden() {
        let cfg = Config::from_toml(&format!(
            r#"
[[projects]]
name = "orbital"
path = "{ORBITAL}"

[roots.explicit]
orbital = ["orb-404"]
"#
        ))
        .expect("the config parses");

        let snap = run(&cfg, &orbital(), Filter::LiveAgents, now());

        assert!(
            snap.failed_projects.is_empty(),
            "the project's own tracker answered"
        );
        let roots: BTreeSet<&str> = snap.trees.iter().map(|t| t.root.as_str()).collect();
        assert_eq!(roots, BTreeSet::from(["orb-404", "orb-7"]));
        assert_eq!(
            rooted_at(&snap, "orb-404").tracker,
            TrackerState::RootNotFound
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

    /// A tracker holding no bead answers `[]`, and that is an empty forest
    /// rather than a failure: every root is read off the answer, so there is
    /// nothing to draw and nothing whose absence to report.
    #[test]
    fn a_tracker_holding_no_bead_draws_nothing_and_fails_nothing() {
        let runner = orbital().with(&spelled(TRACKER_CALL), "[]");

        let snap = run(&one_project(), &runner, Filter::All, now());

        assert!(snap.trees.is_empty());
        assert!(
            snap.failed_projects.is_empty(),
            "{:?}",
            snap.failed_projects
        );
    }

    #[test]
    fn bds_own_words_never_reach_the_snapshot() {
        let runner = orbital().failing(
            &spelled(TRACKER_CALL),
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

    // ---- several projects at once --------------------------------------

    /// A set rather than a sequence: the projects are read together, so
    /// which tracker was asked first is not something a read promises.
    #[test]
    fn each_project_reads_its_tracker_in_its_own_directory_with_its_own_credential() {
        let runner = colliding_trackers(r#"{"result":{"agents":[]}}"#);

        run(&two_projects(), &runner, Filter::All, now());

        let reads: BTreeSet<(Option<PathBuf>, Option<String>)> = runner
            .calls()
            .iter()
            .filter(|c| c.argv.ends_with(TRACKER_CALL))
            .map(|c| (c.cwd.clone(), c.env.get("BEADS_DOLT_PASSWORD").cloned()))
            .collect();

        assert_eq!(
            reads,
            BTreeSet::from([
                (
                    Some(PathBuf::from(ORBITAL)),
                    Some("orbital-password".to_string())
                ),
                (
                    Some(PathBuf::from(FERRY)),
                    Some("ferry-password".to_string())
                ),
            ])
        );
    }
}
