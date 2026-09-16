//! Which rule opens the spine, where a way down stands under it, and what a
//! line at a stand does.
//!
//! The fold default opens a line when it stands on the spine to work a
//! reader needs on the first screen. Layout does not know what the spine is.
//! It asks where a way down stands when a rule begins there, where a child
//! stands given its parent's stand and the link taken, and whether a line at
//! a stand rests open, and it keys what it counts on the stand. Each answer
//! is the rule's, so a rule is a variant here and an arm in each question.
//!
//! A one-copy rule works the tree beneath the line it begins on out once, and
//! layout keeps that answer beside the stands that read it. What the rule
//! works out is one way down per bead — which is why a stand can stay a few
//! bits and the bead the way came through is asked for alongside it.

use crate::model::snapshot::{Node, Tree};
use crate::model::tree::{links_from, Link};
use crate::view::lines::{quiet, BeadFacts};

/// A rule for which lines the spine opens to. *Spine* is coined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Spine {
    /// Every copy of a bead is on the spine, and the first copy the walk
    /// reaches opens while every later copy rests shut.
    #[default]
    EveryCopy,
    /// One copy of each bead is on the spine: the one at the end of the
    /// longest way down to it.
    Deepest,
}

/// Where one way down stands on the spine, under the rule that placed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Stand {
    EveryCopy {
        /// Whether every link on the way is the one the walk first reached
        /// its bead by.
        first: bool,
    },
    OneCopy {
        /// Which of the ways chosen beneath the lines rules begin on this
        /// way down is read against.
        chosen: usize,
        /// Whether every link on the way is one that rule chose.
        on: bool,
        /// Whether that rule chose a way on down from this bead to work a
        /// reader needs.
        over_work: bool,
    },
}

impl Spine {
    /// Every rule there is, in the order a reader cycles them.
    pub const EVERY: &'static [Spine] = &[Spine::EveryCopy, Spine::Deepest];

    /// The rule after this one, back round to the first at the end.
    pub fn next(self) -> Spine {
        let at = Spine::EVERY
            .iter()
            .position(|rule| *rule == self)
            .expect("every rule is in the cycle");
        Spine::EVERY[(at + 1) % Spine::EVERY.len()]
    }

    /// Where a way down stands when this rule begins on the bead at `at`: at
    /// a tree's root, or at the node a scope set the rule on.
    ///
    /// What a one-copy rule works out about the tree beneath that bead goes
    /// into `chosen`, and the stand says where, so the two questions below
    /// can be asked of the stand alone and a few bits.
    pub(super) fn begins(self, tree: &Tree, at: usize, chosen: &mut Vec<Chosen>) -> Stand {
        match self {
            Spine::EveryCopy => Stand::EveryCopy { first: true },
            Spine::Deepest => {
                let ways = Chosen::deepest(tree, at);
                let stand = Stand::OneCopy {
                    chosen: chosen.len(),
                    on: true,
                    over_work: ways.over_work(at),
                };
                chosen.push(ways);
                stand
            }
        }
    }
}

impl Stand {
    /// Where a way down stands with no tree beneath it to place it in.
    ///
    /// A root whose tracker refused holds no beads: it draws one line, folds
    /// nothing, and no rule has anything to work out about it.
    pub(super) fn over_nothing() -> Stand {
        Stand::EveryCopy { first: true }
    }

    /// Where a child stands, reached by `link` from the bead at `from`
    /// standing here.
    pub(super) fn beneath(self, from: usize, link: &Link, chosen: &[Chosen]) -> Stand {
        match self {
            Stand::EveryCopy { first } => Stand::EveryCopy {
                first: first && link.first,
            },
            Stand::OneCopy {
                chosen: which, on, ..
            } => {
                let ways = &chosen[which];
                Stand::OneCopy {
                    chosen: which,
                    on: on && ways.chose(from, link.bead),
                    over_work: ways.over_work(link.bead),
                }
            }
        }
    }

    /// Whether a line standing here rests open, given what its bead says of
    /// the tree beneath it.
    pub(super) fn rests_open(self, bead: &BeadFacts) -> bool {
        match self {
            Stand::EveryCopy { first } => first && bead.opens_a_fold,
            // What the bead says is about every way down to the work beneath
            // it, and this rule opens on one of them. A sibling off the way
            // chosen has the same work beneath it and rests shut over it.
            Stand::OneCopy { on, over_work, .. } => on && over_work,
        }
    }
}

/// The ways one one-copy rule chose beneath the line it begins on: one way
/// down per bead it reaches, each held as the bead it steps down from.
///
/// Holding the step rather than the whole way is what makes the ways a tree
/// over the beads, so what a bead is opened for can be asked of the bead.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Chosen {
    /// The bead each chosen way steps down from. Nothing for the bead the
    /// rule begins on, and for a bead no way down from there reaches.
    from: Vec<Option<usize>>,
    /// Whether the rule chose a way on down from this bead to work a reader
    /// needs.
    over_work: Vec<bool>,
}

impl Chosen {
    /// The longest way down to each bead the tree's links reach from `at`,
    /// ties going to the way through the earlier-placed parent.
    ///
    /// A link back to a bead the way down came through is skipped, as every
    /// walk here skips one, so a looped tree answers with the longest way
    /// that skips such a link rather than with no way at all.
    fn deepest(tree: &Tree, at: usize) -> Self {
        let mut deepest: Vec<Option<usize>> = vec![None; tree.beads.len()];
        let mut from: Vec<Option<usize>> = vec![None; tree.beads.len()];
        deepest[at] = Some(0);
        deepen(tree, at, 0, &mut vec![at], &mut deepest, &mut from);
        let over_work = work_beneath(tree, at, &from);
        Chosen { from, over_work }
    }

    /// Whether the way chosen for the bead at `to` steps down from `from`.
    fn chose(&self, from: usize, to: usize) -> bool {
        self.from[to] == Some(from)
    }

    /// Whether the ways run on down from `at` to work a reader needs.
    fn over_work(&self, at: usize) -> bool {
        self.over_work[at]
    }
}

/// Take every way down from `at` that is deeper than the one already found
/// to the bead it reaches, and go on down each.
///
/// A way no deeper than the one found ends here: the beads beneath it were
/// offered this depth when that way was taken, and offering it again would
/// walk the same subtree to change nothing. A way that is exactly as deep
/// changes which bead it steps down from where that bead was placed earlier,
/// which is the tie-break and costs no walk.
fn deepen(
    tree: &Tree,
    at: usize,
    depth: usize,
    path: &mut Vec<usize>,
    deepest: &mut Vec<Option<usize>>,
    from: &mut Vec<Option<usize>>,
) {
    for link in links_from(&tree.children, at, path) {
        let below = depth + 1;
        match deepest[link.bead] {
            Some(found) if found > below => {}
            Some(found) if found == below => {
                if from[link.bead] > Some(at) {
                    from[link.bead] = Some(at);
                }
            }
            _ => {
                deepest[link.bead] = Some(below);
                from[link.bead] = Some(at);
                path.push(link.bead);
                deepen(tree, link.bead, below, path, deepest, from);
                path.pop();
            }
        }
    }
}

/// Whether the chosen ways run on down from each bead to work a reader
/// needs.
fn work_beneath(tree: &Tree, at: usize, from: &[Option<usize>]) -> Vec<bool> {
    let mut under: Vec<Vec<usize>> = vec![Vec::new(); from.len()];
    for (bead, stepped) in from.iter().enumerate() {
        if let Some(stepped) = stepped {
            under[*stepped].push(bead);
        }
    }
    let mut over_work = vec![false; from.len()];
    work_under(
        tree,
        at,
        &under,
        &mut vec![false; from.len()],
        &mut over_work,
    );
    over_work
}

/// Fill in `over_work` beneath `at`, and answer whether `at` itself or
/// anything the ways reach beneath it is work a reader needs.
///
/// `seen` is what keeps a looped tree's degraded answer from walking for
/// ever: the ways chosen in one are not always a tree, and a bead reached
/// twice holds nothing the first reach did not.
fn work_under(
    tree: &Tree,
    at: usize,
    under: &[Vec<usize>],
    seen: &mut Vec<bool>,
    over_work: &mut Vec<bool>,
) -> bool {
    if seen[at] {
        return false;
    }
    seen[at] = true;
    let mut found = false;
    for bead in &under[at] {
        found |= work_under(tree, *bead, under, seen, over_work);
    }
    over_work[at] = found;
    found || wanted(&tree.beads[at])
}

/// Whether a bead is itself work a reader needs on the first screen: a live
/// agent or an anomaly on it, or a bead `bd` calls ready.
///
/// The same set `opens_a_fold` asks after over every way down at once.
fn wanted(bead: &Node) -> bool {
    !quiet(bead) || bead.ready
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// The cycle takes in every rule and comes back round.
    #[test]
    fn every_rule_is_one_step_from_the_next_and_the_cycle_closes() {
        let mut reached = vec![Spine::default()];
        while reached.len() < Spine::EVERY.len() {
            let next = reached.last().expect("the cycle starts somewhere").next();
            assert!(
                !reached.contains(&next),
                "the cycle closed early: {reached:?}"
            );
            reached.push(next);
        }

        assert_eq!(
            reached.last().expect("the cycle ends somewhere").next(),
            Spine::default(),
            "the cycle did not come back round"
        );
    }
}
