//! Everything one project's tracker is asked for, and what each answer means.
//!
//! The call sequence is the whole of it: the project's environment, the roots
//! discovery names, readiness, every bead, the top of each root's chain, and
//! the trees assembled from them. Nothing here knows that a read is kept
//! between collections, or that other projects exist.

use std::collections::{BTreeMap, BTreeSet};

use crate::collect::bd;
use crate::collect::run::{Env, FailureKind, RunFailure, Runner};
use crate::config::{Config, Project};
use crate::model::join;
use crate::model::snapshot::{Readiness, TrackerFailure};
use crate::model::tree::{self, assemble, Assembled};
use crate::model::types::{Bead, Pane};

/// One project's roots in id order, each either read or unreadable.
pub(super) struct ProjectWork {
    pub(super) readiness: Readiness,
    pub(super) roots: Vec<(String, Result<Assembled, TrackerFailure>)>,
}

/// Everything one project's tracker is asked for. A failure before the roots
/// are known has no root to name, so it becomes the project's own failure
/// rather than a tree; a failure on one root afterwards is that root's.
pub(super) fn read_project(
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

    let beads = bd::all_beads(runner, &project.path, &env)?;
    // Every bead this read of the tracker turned up. A parent chain that
    // leaves it has run off the end of what `bdi` read, and there is no tree
    // to draw from where it went — so the walk stops below that.
    let held: BTreeSet<&str> = beads
        .iter()
        .map(|bead| bead.id.as_str())
        .chain(discovered.keys().map(String::as_str))
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
    for bead in discovered.keys() {
        roots.insert(root_of(
            runner,
            project,
            &env,
            bead,
            &discovered,
            &held,
            &mut ancestors,
        )?);
    }
    for named in panes_naming_a_bead_here(panes, project, cfg) {
        // Swallowed, and it has to be. `display_agent` is free text, and
        // bd exits non-zero on an id it does not hold with nothing to tell
        // that apart from a tracker that has stopped answering — so there
        // is no failure kind to discriminate on. Propagating would cost a
        // whole tracker every time a pane was labelled with a sentence.
        if let Ok(root) = root_of(
            runner,
            project,
            &env,
            named,
            &discovered,
            &held,
            &mut ancestors,
        ) {
            roots.insert(root);
        }
    }

    let mut read: Vec<(String, Result<Assembled, TrackerFailure>)> = roots
        .into_iter()
        .map(|root| {
            let read = assemble(beads.clone(), &root).map_err(|_| TrackerFailure::Parse);
            (root, read)
        })
        .collect();
    read.extend(what_no_root_reached(&beads, &read));
    read.sort_by(|(one, _), (two, _)| one.cmp(two));

    Ok(ProjectWork {
        readiness,
        roots: read,
    })
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
    beads: &[Bead],
    read: &[(String, Result<Assembled, TrackerFailure>)],
) -> Vec<(String, Result<Assembled, TrackerFailure>)> {
    let drawn: BTreeSet<&str> = read
        .iter()
        .filter_map(|(_, read)| read.as_ref().ok())
        .flat_map(|assembled| assembled.rows.iter().map(|placed| placed.bead.id.as_str()))
        .collect();

    let tops: BTreeSet<String> = tree::adrift(beads)
        .into_iter()
        .filter(|id| !drawn.contains(id.as_str()))
        .flat_map(|id| tree::top_of(beads, &id))
        .collect();

    tops.into_iter()
        .map(|id| {
            let read = assemble(beads.to_vec(), &id).map_err(|_| TrackerFailure::Parse);
            (id, read)
        })
        .collect()
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
///
/// The climb stops below a parent `held` does not hold. `bd show` answers for
/// beads that appear in neither `bd list --all` nor discovery — three of them
/// were measured against summit-works on 2026-08-31 — and returning one names
/// a root there is no tree to draw from, which reported a tracker that had
/// answered every call as one whose answer could not be read. Nothing goes
/// missing by stopping: the bead below still names the parent the answer lost,
/// and its own tree reports that as work the tracker no longer holds.
fn root_of(
    runner: &dyn Runner,
    project: &Project,
    env: &Env,
    id: &str,
    parents: &BTreeMap<String, Option<String>>,
    held: &BTreeSet<&str>,
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
        // A refusal here propagates, and that is what `held` bought. Every id
        // this asks about is one `held` holds, bar the free text a pane's
        // `display_agent` may be — so bd refusing an id its own answer just
        // listed is a tracker that has stopped answering, not one bead's chain
        // stopping early. It used to be indistinguishable from the second, and
        // the swallow was for that; the walk no longer reaches an id the
        // answer does not hold. The pane's case is caught where a pane's root
        // is asked for, which cannot tell a bead from a sentence either.
        let parent = match parents.get(&current) {
            Some(known) => known.clone(),
            None => bd::parent_of(runner, &project.path, env, &current)?,
        };
        match parent {
            Some(parent) if held.contains(parent.as_str()) => current = parent,
            // A parent this read does not hold, or no parent at all: either
            // way the chain has nothing further this read can draw.
            Some(_) | None => break current,
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
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;

    /// A second root, reached only because config names it.
    const MAST_TREE: &str = r#"[
      {"id":"orb-4","title":"survey the mast","status":"open",
       "priority":2,"issue_type":"task"}
    ]"#;

    fn rooted_at<'a>(snap: &'a Snapshot, root: &str) -> &'a Tree {
        snap.trees
            .iter()
            .find(|t| t.root == root)
            .unwrap_or_else(|| panic!("{root} is drawn"))
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

    /// A bead whose parent the answer has lost is drawn the same way whatever
    /// `bd show` says about that parent, because the walk never asks.
    ///
    /// It used to ask, and the two answers took the bead down two paths. Where
    /// bd refused — measured against a live tracker on 2026-08-31, `bd show
    /// <missing-id> --json` exits 1 saying `no issues found matching the
    /// provided IDs`, which matches none of the classifier's phrases and so
    /// arrives as `Unavailable`, the same kind a server that is down produces
    /// — the walk stopped and the bead became its own root. Where bd answered,
    /// the walk climbed past the parent to a root whose tree could not then
    /// reach the bead. Which case a tracker is in says nothing about the bead,
    /// so neither may the picture.
    #[test]
    fn whether_bd_answers_for_a_lost_parent_decides_nothing_about_the_bead() {
        let orphan_row = r#"[{"id":"orb-7.9","title":"its parent is a digest",
                              "status":"open","parent":"orb-404"}]"#;
        let orphan_bead = r#"[{"id":"orb-7.9","title":"its parent is a digest",
                               "status":"open",
                               "dependencies":[{"depends_on_id":"orb-404","type":"parent-child"}],
                               "priority":2,"issue_type":"task"}]"#;
        let orphaned = || {
            orbital()
                .merging(&spelled(UNFINISHED_CALL), orphan_row)
                .merging(&spelled(TRACKER_CALL), orphan_bead)
        };

        for (answer, runner) in [
            (
                "bd refuses the parent",
                orphaned().failing(
                    &spelled("show orb-404 --json"),
                    failing(FailureKind::Unavailable),
                ),
            ),
            (
                "bd names a grandparent",
                orphaned().with(
                    &spelled("show orb-404 --json"),
                    r#"[{"id":"orb-404","parent":"orb-7"}]"#,
                ),
            ),
        ] {
            let snap = run(&one_project(), &runner, Filter::All, now());

            assert!(
                snap.failed_projects.is_empty(),
                "one bead bd cannot place must not take the tracker down, {answer}: {:?}",
                snap.failed_projects
            );
            let roots: Vec<&str> = snap.trees.iter().map(|t| t.root.as_str()).collect();
            assert_eq!(
                roots,
                vec!["orb-7", "orb-7.9"],
                "the bead the answer holds no way down to is its own root, {answer}"
            );
            assert_eq!(
                rooted_at(&snap, "orb-7").dangling,
                Vec::<String>::new(),
                "the tree that never reached it does not report it either, {answer}"
            );
            assert_eq!(
                rooted_at(&snap, "orb-7.9").dangling,
                vec!["orb-7.9".to_string()],
                "its own tree names the work the tracker no longer holds, {answer}"
            );
            let asked: Vec<String> = runner
                .calls()
                .into_iter()
                .map(|call| call.argv)
                .filter(|argv| argv == &spelled("show orb-404 --json"))
                .collect();
            assert!(
                asked.is_empty(),
                "a parent this read does not hold is not climbed to, so it is not asked about: \
                 {asked:?}"
            );
        }
    }

    /// A tracker that stops answering part-way through a read is reported,
    /// not drawn around.
    ///
    /// `bd list --all` has just returned `orb-8`, so `bd show orb-8` refusing
    /// is not a chain that ran off the end of the answer — the answer holds
    /// it. The two calls disagree, which is the shape of a tracker that has
    /// gone away, and the roots the walk would go on to guess at are the ones
    /// the forest is drawn from. Swallowing it drew a forest with the wrong
    /// roots and called the read a success.
    #[test]
    fn a_tracker_refusing_a_bead_its_own_answer_holds_takes_the_project_down() {
        let under_it = r#"[{"id":"orb-8.1","title":"under a bead bd will not answer for",
                            "status":"open","parent":"orb-8"}]"#;
        let both = r#"[{"id":"orb-8","title":"the bead bd will not answer for","status":"closed",
                        "priority":2,"issue_type":"epic"},
                       {"id":"orb-8.1","title":"under a bead bd will not answer for",
                        "status":"open",
                        "dependencies":[{"depends_on_id":"orb-8","type":"parent-child"}],
                        "priority":2,"issue_type":"task"}]"#;
        let runner = orbital()
            .merging(&spelled(UNFINISHED_CALL), under_it)
            .merging(&spelled(TRACKER_CALL), both)
            .failing(
                &spelled("show orb-8 --json"),
                failing(FailureKind::Unavailable),
            );

        let snap = run(&one_project(), &runner, Filter::All, now());

        assert_eq!(
            snap.failed_projects,
            vec![FailedProject {
                project: "orbital".to_string(),
                tracker: TrackerFailure::Unavailable,
            }]
        );
        assert!(
            snap.trees.is_empty(),
            "no root is guessed at from half a read: {:?}",
            snap.trees
        );
    }

    /// A closed bead is never discovered — statuses, wisps and metadata keys
    /// are all populations of unfinished work — so nothing makes it a root
    /// and no `bd show` is asked about its parent. It is the same defect with
    /// nothing else moving.
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

    /// The digest case measured against summit-works on 2026-08-31. `bd show`
    /// answers for the parent, says it has no parent of its own, and neither
    /// `bd list --all` nor discovery holds it. Climbing to it named a root no
    /// tree could be drawn from, and the project grew a tree reported as a
    /// tracker that had answered with something `bdi` could not read — when
    /// every call it made was answered correctly.
    #[test]
    fn a_parent_only_bd_show_holds_is_never_made_a_root() {
        let orphan_row = r#"[{"id":"orb-7.9","title":"its parent is a digest",
                              "status":"open","parent":"orb-404"}]"#;
        let orphan_bead = r#"[{"id":"orb-7.9","title":"its parent is a digest",
                               "status":"open",
                               "dependencies":[{"depends_on_id":"orb-404","type":"parent-child"}],
                               "priority":2,"issue_type":"task"}]"#;
        let runner = orbital()
            .merging(&spelled(UNFINISHED_CALL), orphan_row)
            .merging(&spelled(TRACKER_CALL), orphan_bead)
            .with(
                &spelled("show orb-404 --json"),
                r#"[{"id":"orb-404","parent":null}]"#,
            );

        let snap = run(&one_project(), &runner, Filter::All, now());

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
            rooted_at(&snap, "orb-7.9").dangling,
            vec!["orb-7.9".to_string()],
            "the parent chain running off the end of the answer is what is reported"
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
                .nodes
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
                .nodes
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
                .nodes
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
                .nodes
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
        let also_waiting = r#"[{"id":"orb-7.1","title":"re-point the dish","status":"in_progress",
                                "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"},
                                                {"depends_on_id":"orb-404","type":"parent-child"}],
                                "priority":2,"issue_type":"task",
                                "metadata":{"agent_pane":"w:p1"}}]"#;
        let runner = orbital().with(
            &spelled(TRACKER_CALL),
            &ORBITAL_TREE.replace(
                r#"{"id":"orb-7.1","title":"re-point the dish","status":"in_progress",
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
            lost.nodes.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
            vec!["orb-3", "orb-3.1"]
        );
        assert_eq!(
            lost.dangling,
            vec!["orb-3".to_string(), "orb-3.1".to_string()]
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
            // Both ends of the loop, because a climb stops below a parent
            // this read does not hold — and then the cycle guard, not the
            // cycle, would be what this test never reaches.
            .with(&spelled(TRACKER_CALL),
                r#"[{"id":"orb-7","title":"lift the ground station","status":"in_progress",
                     "priority":1,"issue_type":"epic"},
                    {"id":"orb-7.1","title":"re-point the dish","status":"in_progress",
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

    // ---- several projects at once --------------------------------------

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
}
