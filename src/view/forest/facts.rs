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

use crate::model::snapshot::{Counts, Snapshot, Tree};
use crate::model::tree::Link;
use crate::view::lines::{facts_of, run_size, split, split_by, BeadFacts};

/// One snapshot's answers: a tree's for each of its trees, in the same
/// order, and every project's counts over its trees.
pub(super) struct Facts {
    trees: Vec<TreeFacts>,
    projects: BTreeMap<String, Counts>,
}

impl Facts {
    pub(super) fn of(snapshot: &Snapshot) -> Self {
        Facts {
            trees: snapshot.trees.iter().map(TreeFacts::of).collect(),
            projects: snapshot
                .trees
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

    /// Beside each of the snapshot's trees.
    pub(super) fn trees(&self) -> &[TreeFacts] {
        &self.trees
    }

    /// Every bead in the project's trees, counted once. Nothing for a
    /// project with no trees, which is what its line counts.
    pub(super) fn project(&self, project: &str) -> Counts {
        self.projects.get(project).cloned().unwrap_or_default()
    }
}

/// One tree's answers, kept per bead where a bead has one answer: a tree
/// with no loop in it, where nothing beneath a bead can be above it and the
/// way down changes nothing. A tree with a loop cut in it is asked by the way
/// down, as ever, because two copies of a bead on either side of the cut
/// stand over different things.
pub(super) struct TreeFacts {
    beads: Option<Vec<Answered>>,
}

/// What one bead answers, and what the run its finished children make
/// stands for — nothing where they make none.
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
