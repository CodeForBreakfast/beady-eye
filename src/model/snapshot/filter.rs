//! Which of a snapshot's trees are shown, and the order a reader meets them
//! in.

use std::sync::Arc;

use super::{Filter, HerdrState, HiddenTree, Snapshot, TrackerState, Tree};

impl Tree {
    /// What the default filter keeps: a tree somebody is working in, a tree
    /// carrying something `bdi` has already called wrong, or a tree it could
    /// not read.
    ///
    /// An anomaly counts because a claim with no pane behind it is the only
    /// sign that work is under way, and counting agents alone loses the whole
    /// project rather than folding it. That is the same reckoning `quiet`
    /// makes of one bead — an agent or an anomaly is something to show — so
    /// the fold and the filter agree about what a claim means.
    ///
    /// A tree we could not read has no agents to count, so the filter would
    /// hide it for the one reason it must not: an unreadable tracker and a
    /// tracker with no work would look the same.
    fn survives(&self, filter: Filter) -> bool {
        match filter {
            Filter::All => true,
            Filter::LiveAgents => {
                self.counts.live_agents > 0
                    || self.counts.anomalies > 0
                    || self.tracker != TrackerState::Ok
            }
        }
    }
}

/// A project's trees in the order a reader meets them.
///
/// A tracker that files loose beads in bulk gives hundreds of roots holding
/// one bead each — 530 of 562, measured on one such tracker — and none of
/// them may be dropped, so the order they come in is the whole of what the
/// reader has:
///
/// - a tree `bdi` could not read leads, for the reason `Tree::survives` shows
///   it whatever the filter says — a root buried under hundreds of others has
///   disappeared as surely as one that was dropped;
/// - then the staffed trees, so that showing every tree adds the rest of the
///   forest below what the reader was already looking at rather than
///   shuffling it;
/// - then the trees with the most unfinished work in them;
/// - then the root's id, so a redraw moves nothing.
///
/// Projects keep the order the config named them in and a project's trees
/// stay together, so this orders within each project's run rather than across
/// the forest.
pub(super) fn in_flight_first(trees: &mut [Tree]) {
    for project in trees.chunk_by_mut(|a, b| a.project == b.project) {
        project.sort_by(|a, b| {
            (b.tracker != TrackerState::Ok)
                .cmp(&(a.tracker != TrackerState::Ok))
                .then(b.counts.live_agents.cmp(&a.counts.live_agents))
                .then(b.counts.unfinished().cmp(&a.counts.unfinished()))
                .then_with(|| a.root.cmp(&b.root))
        });
    }
}

/// Divide the collected trees into the ones the filter shows and the ones it
/// hides. With no herdr there are no panes, so there is no agent to filter on
/// and every tree renders.
pub(super) fn partition(
    trees: &[Arc<Tree>],
    herdr: HerdrState,
    filter: Filter,
) -> (Vec<Arc<Tree>>, Vec<HiddenTree>) {
    let filter = match herdr {
        HerdrState::Ok => filter,
        HerdrState::Unavailable => Filter::All,
    };
    let (shown, hidden): (Vec<&Arc<Tree>>, Vec<&Arc<Tree>>) =
        trees.iter().partition(|t| t.survives(filter));

    (
        shown.into_iter().cloned().collect(),
        hidden.into_iter().map(|t| HiddenTree::of(t)).collect(),
    )
}

impl HiddenTree {
    /// What the filter leaves of a tree it hides: enough to name it, why it
    /// went, and whether it took findings out of the forest with it — counted
    /// off the whole tree, because the hidden tree draws nothing and this is
    /// all a reader gets of it.
    pub(crate) fn of(tree: &Tree) -> Self {
        HiddenTree {
            project: tree.project.clone(),
            root: tree.root.clone(),
            title: tree.title.clone(),
            reason: "no-live-agent",
            findings: !tree.dangling.is_empty()
                || !tree.cycles.is_empty()
                || tree.counts.anomalies > 0,
        }
    }
}

impl Snapshot {
    /// Re-apply the filter to a snapshot already in hand. Which trees show is
    /// a display choice over what was collected, so nothing is read again and
    /// the answer is the one `build` would have given for that filter.
    pub fn refilter(&mut self, filter: Filter) {
        let (trees, hidden_trees) = partition(&self.collected, self.herdr, filter);
        self.filter = filter;
        self.trees = trees;
        self.hidden_trees = hidden_trees;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::join::{BeadKey, Conflict, Joined};
    use crate::model::snapshot::tests::*;
    use crate::model::snapshot::{
        build, build_tree, Collected, Counts, FailedProject, Readiness, TrackerFailure,
    };
    use pretty_assertions::assert_eq;
    use std::collections::BTreeMap;

    /// The snapshot as `refilter` leaves it, with the one in hand kept.
    fn refiltered(snapshot: &Snapshot, filter: Filter) -> Snapshot {
        let mut refiltered = snapshot.clone();
        refiltered.refilter(filter);
        refiltered
    }

    /// One project's tree with nothing claimed in it: a root waiting on work
    /// elsewhere, over open and closed tasks. No pane can be on it and no
    /// anomaly rule can fire on it, which is the one state the default filter
    /// folds away.
    const UNSTAFFED: &str = r#"[
      {"id":"orb-2","title":"quiet work","status":"blocked",
       "priority":2,"issue_type":"epic"},
      {"id":"orb-2.1","title":"read the almanac","status":"open",
       "dependencies":[{"depends_on_id":"orb-2","type":"parent-child"}],
       "priority":2,"issue_type":"task"},
      {"id":"orb-2.2","title":"log the pass","status":"closed",
       "dependencies":[{"depends_on_id":"orb-2","type":"parent-child"}],
       "priority":2,"issue_type":"task","closed_at":"2026-08-28T09:00:00Z"}
    ]"#;

    /// A tree nobody is working in, told apart from its neighbours by its root.
    fn quiet(root: &str, title: &str) -> Tree {
        let mut t = build_tree(
            "orbital",
            &assembled(UNSTAFFED),
            &Joined::default(),
            &Readiness::default(),
            &BTreeMap::new(),
            &cfg(),
            now(),
        );
        t.root = root.to_string();
        t.title = title.to_string();
        t
    }

    /// A quiet tree that has something to report: a bead waiting on work bd
    /// never returned, and a bead blocked by its own forebear.
    fn quiet_with_reports() -> Tree {
        let json = r#"[
          {"id":"orb-6","title":"the far side","status":"open"},
          {"id":"orb-6.2","title":"waiting on a bead bd did not return","status":"open",
           "dependencies":[{"depends_on_id":"orb-6","type":"parent-child"},
                           {"depends_on_id":"orb-6.1","type":"blocks"}]},
          {"id":"orb-6.3","title":"one","status":"open",
           "dependencies":[{"depends_on_id":"orb-6","type":"parent-child"},
                           {"depends_on_id":"orb-6","type":"blocks"}]},
          {"id":"orb-6.4","title":"two","status":"open",
           "dependencies":[{"depends_on_id":"orb-6.3","type":"parent-child"}]}
        ]"#;
        build_tree(
            "orbital",
            &assembled(json),
            &Joined::default(),
            &Readiness::default(),
            &BTreeMap::new(),
            &cfg(),
            now(),
        )
    }

    /// One project's tree whose only activity is a bead a seat has claimed
    /// and no pane has joined yet: `orb-4.1` is `in_progress`, freshly
    /// touched, and named by no pane.
    const CLAIMED_WITH_NO_PANE: &str = r#"[
      {"id":"orb-4","title":"raise the mast","status":"open",
       "priority":1,"issue_type":"epic"},
      {"id":"orb-4.1","title":"seat the guy wires","status":"in_progress",
       "dependencies":[{"depends_on_id":"orb-4","type":"parent-child"}],
       "priority":1,"issue_type":"task","updated_at":"2026-08-30T11:00:00Z"}
    ]"#;

    fn claimed_with_no_pane() -> Tree {
        build_tree(
            "orbital",
            &assembled(CLAIMED_WITH_NO_PANE),
            &Joined::default(),
            &Readiness::default(),
            &BTreeMap::new(),
            &cfg(),
            now(),
        )
    }

    /// A quiet tree, a live one, and another quiet one: the interleaving a
    /// lifted filter has to put back.
    fn interleaved() -> Vec<Tree> {
        vec![quiet("orb-2", "quiet work"), tree(), quiet_with_reports()]
    }

    /// A tree with nothing in it but the numbers the order is made from.
    fn counted(project: &str, root: &str, live_agents: usize, total: usize, closed: usize) -> Tree {
        Tree {
            project: project.to_string(),
            root: root.to_string(),
            title: String::new(),
            counts: Counts {
                total,
                closed,
                live_agents,
                anomalies: 0,
            },
            tracker: TrackerState::Ok,
            beads: Vec::new(),
            children: Vec::new(),
            dangling: Vec::new(),
            cycles: Vec::new(),
        }
    }

    fn ordered(mut trees: Vec<Tree>) -> Vec<String> {
        in_flight_first(&mut trees);
        trees
            .into_iter()
            .map(|t| format!("{}:{}", t.project, t.root))
            .collect()
    }

    /// Which project comes first is the config's to say, so the order runs
    /// within a project rather than over the whole forest.
    #[test]
    fn a_projects_trees_stay_together_where_the_config_put_them() {
        assert_eq!(
            ordered(vec![
                counted("orbital", "orb-1", 0, 2, 0),
                counted("orbital", "orb-2", 0, 9, 0),
                counted("ferry", "fer-1", 1, 1, 0),
                counted("ferry", "fer-2", 0, 40, 0),
            ]),
            [
                "orbital:orb-2",
                "orbital:orb-1",
                "ferry:fer-1",
                "ferry:fer-2"
            ]
        );
    }

    /// A closed bead is a row, not work, so a long-finished effort does not
    /// outrank a small one still going.
    #[test]
    fn a_tree_of_finished_beads_does_not_outrank_a_smaller_one_still_going() {
        assert_eq!(
            ordered(vec![
                counted("orbital", "orb-done", 0, 90, 88),
                counted("orbital", "orb-going", 0, 5, 0),
            ]),
            ["orbital:orb-going", "orbital:orb-done"]
        );
    }

    /// A tracker that could not be read has no counts to sort on, and a root
    /// buried under hundreds of others has disappeared as surely as a dropped
    /// one.
    #[test]
    fn a_tree_bdi_could_not_read_leads_the_forest() {
        let mut trees = vec![
            counted("orbital", "orb-1", 1, 40, 0),
            Tree::tracker_unreachable("orbital", "orb-9", TrackerFailure::Auth),
        ];
        in_flight_first(&mut trees);

        assert_eq!(trees[0].root, "orb-9");
    }

    /// Nothing tells the beads filed in bulk apart, so the tail is at least
    /// the same tail on every redraw.
    #[test]
    fn trees_holding_the_same_work_come_in_id_order() {
        assert_eq!(
            ordered(vec![
                counted("orbital", "orb-c", 0, 1, 0),
                counted("orbital", "orb-a", 0, 1, 0),
                counted("orbital", "orb-b", 0, 1, 0),
            ]),
            ["orbital:orb-a", "orbital:orb-b", "orbital:orb-c"]
        );
    }

    #[test]
    fn a_tree_with_no_live_agent_is_hidden_and_reported() {
        let snap = snapshot(vec![tree(), quiet("orb-2", "quiet work")]);

        assert_eq!(snap.trees.len(), 1);
        assert_eq!(snap.trees[0].root, "orb-7");
        assert_eq!(
            snap.hidden_trees,
            vec![HiddenTree {
                project: "orbital".to_string(),
                root: "orb-2".to_string(),
                title: "quiet work".to_string(),
                reason: "no-live-agent",
                findings: false,
            }],
            "a filtered tree is reported, never dropped"
        );
    }

    /// A hidden tree's findings leave the forest with it, and the group that
    /// holds it admits to them. That is settled here, once, as the tree is
    /// hidden: the view asks the hidden tree and never the forest.
    #[test]
    fn a_hidden_tree_waiting_on_a_bead_bd_never_returned_has_a_finding() {
        let mut waiting = quiet("orb-2", "quiet work");
        waiting.dangling = vec!["orb-2.9".to_string()];
        let snap = snapshot(vec![tree(), waiting]);

        assert!(snap.hidden_trees[0].findings);
    }

    #[test]
    fn a_hidden_tree_blocked_by_its_own_forebear_has_a_finding() {
        let mut looping = quiet("orb-2", "quiet work");
        looping.cycles = vec!["orb-2".to_string()];
        let snap = snapshot(vec![tree(), looping]);

        assert!(snap.hidden_trees[0].findings);
    }

    /// No filter today hides a tree with an anomaly in it, so the tree is
    /// hidden here the way `partition` hides one. What is under test is what
    /// the hidden tree admits to, not which trees get hidden.
    #[test]
    fn a_hidden_tree_whose_only_finding_is_an_anomaly_has_a_finding() {
        let claimed = claimed_with_no_pane();
        assert!(claimed.dangling.is_empty() && claimed.cycles.is_empty());
        assert_eq!(claimed.counts.anomalies, 1);

        assert!(HiddenTree::of(&claimed).findings);
    }

    /// `fleet-launch` makes a pane and boots for some time before the agent
    /// in it writes `agent_pane`, and a bead whose agent never writes the key
    /// stays that way for good. The claim is the only sign of the work, and a
    /// filter that hides the project loses every trace of it.
    #[test]
    fn a_project_whose_only_claim_has_no_pane_is_still_drawn() {
        let snap = snapshot(vec![claimed_with_no_pane()]);

        assert!(
            snap.hidden_trees.is_empty(),
            "a claim bdi has already called an anomaly is not nothing: {:?}",
            snap.hidden_trees
        );
        assert_eq!(snap.trees.len(), 1);
        assert_eq!(snap.trees[0].root, "orb-4");
    }

    #[test]
    fn a_tree_whose_tracker_failed_is_never_hidden() {
        let broken = Tree::tracker_unreachable("ferry", "fry-3", TrackerFailure::Auth);
        let snap = snapshot(vec![broken]);

        assert_eq!(
            snap.trees.len(),
            1,
            "a tracker we could not read has no agents to count, so hiding it \
             would be indistinguishable from having no work"
        );
        assert!(snap.hidden_trees.is_empty());
    }

    #[test]
    fn without_herdr_nothing_is_filtered() {
        let mut quiet = tree();
        quiet.counts.live_agents = 0;
        quiet.beads.iter_mut().for_each(|n| n.agent = None);

        let snap = build(
            Collected {
                trees: vec![quiet],
                ..Collected::default()
            },
            &[],
            &Joined::default(),
            &cfg(),
            HerdrState::Unavailable,
            Filter::LiveAgents,
            now(),
        );

        assert_eq!(
            snap.trees.len(),
            1,
            "with no panes there is no filter to apply"
        );
        assert!(snap.hidden_trees.is_empty());
        assert!(snap.unattributed.is_empty());
    }

    #[test]
    fn asking_for_every_tree_hides_none() {
        let mut quiet = tree();
        quiet.counts.live_agents = 0;
        quiet.beads.iter_mut().for_each(|n| n.agent = None);

        let snap = build(
            Collected {
                trees: vec![quiet],
                ..Collected::default()
            },
            &panes(PANES),
            &Joined::default(),
            &cfg(),
            HerdrState::Ok,
            Filter::All,
            now(),
        );

        assert_eq!(snap.trees.len(), 1);
        assert!(snap.hidden_trees.is_empty());
    }

    #[test]
    fn lifting_the_filter_gives_what_a_fresh_collection_would_have() {
        let filtered = built(interleaved(), Filter::LiveAgents);

        assert_eq!(
            refiltered(&filtered, Filter::All),
            built(interleaved(), Filter::All),
            "a display change must not read differently from a collection"
        );
    }

    #[test]
    fn re_applying_the_filter_gives_what_a_fresh_collection_would_have() {
        let all = built(interleaved(), Filter::All);

        assert_eq!(
            refiltered(&all, Filter::LiveAgents),
            built(interleaved(), Filter::LiveAgents)
        );
    }

    #[test]
    fn a_lifted_filter_puts_the_hidden_trees_back_among_the_shown_ones() {
        let lifted = refiltered(&built(interleaved(), Filter::LiveAgents), Filter::All);

        let roots: Vec<&str> = lifted.trees.iter().map(|t| t.root.as_str()).collect();
        assert_eq!(
            roots,
            ["orb-7", "orb-6", "orb-2"],
            "the forest's own order, not the shown ones followed by the hidden ones"
        );
        assert!(lifted.hidden_trees.is_empty());
        assert_eq!(lifted.filter, Filter::All);
    }

    #[test]
    fn the_filter_goes_off_and_on_again_without_drift() {
        let filtered = built(interleaved(), Filter::LiveAgents);

        assert_eq!(
            refiltered(&refiltered(&filtered, Filter::All), Filter::LiveAgents),
            filtered
        );
    }

    /// A filter is a display choice over what was collected, so what it
    /// shows is the collected trees themselves. A copy of every shown tree
    /// was most of what a collection cost to land, paid again on `a`.
    #[test]
    fn a_refilter_shows_the_collected_trees_themselves_rather_than_copies() {
        let mut lifted = built(interleaved(), Filter::LiveAgents);
        let collected: Vec<Arc<Tree>> = lifted.collected.clone();

        lifted.refilter(Filter::All);

        assert_eq!(lifted.trees.len(), collected.len());
        for (shown, was) in lifted.trees.iter().zip(&collected) {
            assert!(Arc::ptr_eq(shown, was), "{} was copied", shown.root);
            assert_eq!(
                Arc::strong_count(was),
                3,
                "{} is held by collected, trees and this test, and nothing else",
                was.root
            );
        }
    }

    #[test]
    fn a_hidden_tree_comes_back_whole() {
        let lifted = refiltered(&built(interleaved(), Filter::LiveAgents), Filter::All);
        let back = lifted
            .trees
            .iter()
            .find(|t| t.root == "orb-6")
            .expect("the hidden tree is back");

        assert_eq!(
            back.as_ref(),
            &quiet_with_reports(),
            "what a filter hid it must be able to show again"
        );
        assert_eq!(back.dangling, ["orb-6.2"]);
        assert_eq!(back.cycles, ["orb-6"]);
    }

    #[test]
    fn what_belongs_to_no_tree_survives_a_refilter() {
        let mut before = built(interleaved(), Filter::LiveAgents);
        before.failed_projects = vec![FailedProject {
            project: "ferry".to_string(),
            tracker: TrackerFailure::Auth,
        }];
        before.conflicts = vec![Conflict::BeadAndPaneDisagree {
            bead: BeadKey {
                project: "orbital".to_string(),
                id: "orb-7".to_string(),
            },
            named_by_bead: "w:p1".to_string(),
            named_by_pane: "w:p2".to_string(),
        }];
        assert!(!before.unattributed.is_empty(), "there are panes to lose");

        let after = refiltered(&refiltered(&before, Filter::All), Filter::LiveAgents);

        assert_eq!(after.failed_projects, before.failed_projects);
        assert_eq!(after.unattributed, before.unattributed);
        assert_eq!(after.conflicts, before.conflicts);
        assert_eq!(
            after.generated_at, before.generated_at,
            "a refilter is not a new reading"
        );
    }

    #[test]
    fn a_tracker_that_could_not_be_read_is_never_hidden_by_a_refilter() {
        let broken = Tree::tracker_unreachable("ferry", "fry-3", TrackerFailure::Auth);

        let filtered = refiltered(
            &built(vec![broken.clone()], Filter::All),
            Filter::LiveAgents,
        );

        assert_eq!(filtered.trees, vec![Arc::new(broken)]);
        assert!(filtered.hidden_trees.is_empty());
    }

    #[test]
    fn without_herdr_a_refilter_hides_nothing() {
        let blind = build(
            Collected {
                trees: vec![quiet("orb-2", "quiet work")],
                ..Collected::default()
            },
            &[],
            &Joined::default(),
            &cfg(),
            HerdrState::Unavailable,
            Filter::All,
            now(),
        );

        let filtered = refiltered(&blind, Filter::LiveAgents);

        assert_eq!(
            filtered.trees.len(),
            1,
            "with no panes there is no filter to apply"
        );
        assert!(filtered.hidden_trees.is_empty());
    }

    #[test]
    fn what_a_refilter_needs_is_not_part_of_the_contract() {
        let json: serde_json::Value =
            serde_json::to_value(snapshot(interleaved())).expect("the snapshot serialises");

        assert!(
            json.get("collected").is_none(),
            "the trees kept for a refilter are not emitted"
        );
        assert_eq!(json["trees"][0]["root"], "orb-7");
        assert_eq!(json["hidden_trees"][0]["root"], "orb-6");
    }
}
