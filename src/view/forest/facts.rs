//! What layout asks of a snapshot that the snapshot alone decides, answered
//! once when the forest takes it rather than once per drawn line per
//! keystroke.
//!
//! A line's fraction, what it is shut over, whether it rests open and what a
//! run under it stands for are each a question about the beads beneath one
//! bead, and a project's line counts every bead in its trees. None of that
//! moves with a fold or the selection, so a keystroke that asked again would
//! get what the last one got.

use std::collections::BTreeMap;

use crate::model::join::BeadKey;
use crate::model::snapshot::{Counts, Snapshot, Tree};
use crate::model::tree::Link;
use crate::view::lines::{facts_of, root_key, run_size, split, split_by, BeadFacts, Place};

use super::handle::{root_handle, Handle};
use super::spine::{Chosen, Spine, Stand};

/// One snapshot's answers: a tree's for every tree it holds, shown or
/// hidden, by its root, every project's counts over all of its trees, and
/// where a way down stands at each node a rule begins on.
///
/// A hidden tree is a tree, and the filter only decides where it is drawn,
/// so it is answered as the shown ones are rather than when a reader opens
/// the group holding it.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Facts {
    trees: BTreeMap<BeadKey, TreeFacts>,
    projects: BTreeMap<String, Counts>,
    begun: BTreeMap<Handle, Stand>,
    chosen: Vec<Chosen>,
}

impl Facts {
    /// `spine` is the rule in force over the forest, and `spines` the rule
    /// the reader has put in force under each line they set one on.
    pub(super) fn of(snapshot: &Snapshot, spine: Spine, spines: &BTreeMap<Handle, Spine>) -> Self {
        let mut trees = BTreeMap::new();
        for tree in snapshot.trees.iter().chain(&snapshot.collected) {
            trees
                .entry(root_key(tree))
                .or_insert_with(|| TreeFacts::of(tree));
        }
        // The rule over the forest begins on every tree's root, and a rule
        // set on a line begins there instead of the one it stands under.
        let mut beginnings: BTreeMap<Handle, Spine> = snapshot
            .trees
            .iter()
            .chain(&snapshot.collected)
            .map(|tree| (root_handle(tree), spine))
            .collect();
        beginnings.extend(
            spines
                .iter()
                .map(|(handle, spine)| (handle.clone(), *spine)),
        );

        let mut chosen = Vec::new();
        let mut begun = BTreeMap::new();
        for (handle, spine) in beginnings {
            let Handle::Bead(place) = &handle else {
                continue;
            };
            // A rule set on a way down that leaves the tree names no line,
            // and goes with it.
            let Some((tree, at)) = stands_on(snapshot, place) else {
                continue;
            };
            let stand = spine.begins(tree, at, &mut chosen);
            begun.insert(handle, stand);
        }

        Facts {
            trees,
            begun,
            chosen,
            projects: snapshot
                .collected
                .chunk_by(|a, b| a.project == b.project)
                .map(|trees| {
                    (
                        trees[0].project.clone(),
                        Counts::over(trees.iter().flat_map(|tree| &tree.beads)),
                    )
                })
                .collect(),
        }
    }

    /// One of the snapshot's trees' answers, by its root.
    pub(super) fn tree(&self, root: &BeadKey) -> &TreeFacts {
        self.trees
            .get(root)
            .expect("every tree the snapshot holds was answered when it was taken")
    }

    /// Every bead in the project's trees, shown or hidden, counted once.
    /// Nothing for a project with no trees, which is what its line counts.
    pub(super) fn project(&self, project: &str) -> Counts {
        self.projects.get(project).cloned().unwrap_or_default()
    }

    /// Where a way down stands at the line a rule begins on, where a scope
    /// set one there. Nothing for a line that answers from the rule over it.
    pub(super) fn begun(&self, handle: &Handle) -> Option<Stand> {
        self.begun.get(handle).copied()
    }

    /// Every line a rule begins on.
    pub(super) fn beginnings(&self) -> impl Iterator<Item = &Handle> {
        self.begun.keys()
    }

    /// The ways every one-copy rule in force chose, by the index each stand
    /// under one of them holds.
    pub(super) fn chosen(&self) -> &[Chosen] {
        &self.chosen
    }

    /// One tree's answers where every bead has one, with the chosen ways
    /// beside them. None in a tree with a loop cut in it.
    pub(super) fn uniform(&self, root: &BeadKey) -> Option<Uniform<'_>> {
        self.tree(root).uniform(&self.chosen)
    }
}

/// The tree a place was drawn in and the bead it names, where the snapshot
/// still holds both.
fn stands_on<'a>(snapshot: &'a Snapshot, place: &Place) -> Option<(&'a Tree, usize)> {
    let tree = snapshot
        .trees
        .iter()
        .chain(&snapshot.collected)
        .find(|tree| root_key(tree) == place.tree)?;
    let at = tree.beads.iter().position(|bead| bead.is(place.key()))?;
    Some((tree, at))
}

/// One tree's answers, kept per bead where a bead has one answer: a tree
/// with no loop in it, where nothing beneath a bead can be above it and the
/// way down changes nothing. A tree with a loop cut in it is asked by the way
/// down, as ever, because two copies of a bead on either side of the cut
/// stand over different things.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct TreeFacts {
    beads: Option<Vec<Answered>>,
}

/// What one bead answers, and what the run its finished children make
/// stands for — nothing where they make none.
#[derive(Debug, PartialEq, Eq)]
struct Answered {
    facts: BeadFacts,
    run: usize,
}

impl TreeFacts {
    pub(super) fn of(tree: &Tree) -> Self {
        if !tree.cycles.is_empty() {
            return TreeFacts { beads: None };
        }
        let facts: Vec<BeadFacts> = (0..tree.beads.len())
            .map(|at| facts_of(tree, at, &[]))
            .collect();
        let beads = facts
            .iter()
            .enumerate()
            .map(|(at, bead)| {
                let (_, elided) = split_by(tree, at, &[], |bead| facts[bead].finished);
                Answered {
                    facts: bead.clone(),
                    run: run_size(tree, &elided, &[at]),
                }
            })
            .collect();
        TreeFacts { beads: Some(beads) }
    }

    /// The answers a bead has one of, whichever way down reaches it: every
    /// bead's, in a tree with no loop in it, and none in a tree with one.
    /// `chosen` is what a stand under a one-copy rule is read against, which
    /// the walk carries alongside them.
    fn uniform<'a>(&'a self, chosen: &'a [Chosen]) -> Option<Uniform<'a>> {
        self.beads.as_deref().map(|beads| Uniform { beads, chosen })
    }

    /// What the line at `at` says of the tree beneath it.
    pub(super) fn bead(&self, tree: &Tree, at: usize, above: &[usize]) -> BeadFacts {
        match &self.beads {
            Some(beads) => beads[at].facts.clone(),
            None => facts_of(tree, at, above),
        }
    }

    /// The children of `at` split into the ones drawn and the run that is
    /// not.
    pub(super) fn split<'a>(
        &self,
        tree: &'a Tree,
        at: usize,
        above: &[usize],
    ) -> (Vec<&'a Link>, Vec<&'a Link>) {
        match &self.beads {
            Some(beads) => split_by(tree, at, above, |bead| beads[bead].facts.finished),
            None => split(tree, at, above),
        }
    }

    /// What a run stands for. `above` is the way down to the bead the run
    /// hangs under, that bead included.
    pub(super) fn run_size(&self, tree: &Tree, members: &[&Link], above: &[usize]) -> usize {
        match &self.beads {
            Some(beads) => {
                let under = *above.last().expect("a run hangs under a bead");
                beads[under].run
            }
            None => run_size(tree, members, above),
        }
    }
}

/// One tree's answers where every bead has one answer, asked by the bead
/// alone, and the ways the rules in force chose — which the same walk reads
/// and which no bead can be asked for.
#[derive(Clone, Copy)]
pub(super) struct Uniform<'a> {
    beads: &'a [Answered],
    chosen: &'a [Chosen],
}

impl<'a> Uniform<'a> {
    pub(super) fn bead(&self, at: usize) -> &BeadFacts {
        &self.beads[at].facts
    }

    /// The children of `at` split into the ones drawn and the run that is
    /// not. No way down is needed to cut a loop, because there is none.
    pub(super) fn split<'t>(&self, tree: &'t Tree, at: usize) -> (Vec<&'t Link>, Vec<&'t Link>) {
        split_by(tree, at, &[], |bead| self.beads[bead].facts.finished)
    }

    /// What the run under `at` stands for.
    pub(super) fn run(&self, at: usize) -> usize {
        self.beads[at].run
    }

    /// The ways every one-copy rule in force chose.
    pub(super) fn chosen(&self) -> &'a [Chosen] {
        self.chosen
    }
}
