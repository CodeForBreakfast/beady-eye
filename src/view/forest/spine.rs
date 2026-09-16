//! Which rule opens the spine, where a way down stands under it, and what a
//! line at a stand does.
//!
//! The fold default opens a line when it stands on the spine to work a
//! reader needs on the first screen. Layout does not know what the spine is.
//! It asks where a way down stands when a rule begins there, where a child
//! stands given its parent's stand and the link taken, and whether a line at
//! a stand rests open, and it keys what it counts on the stand. Each answer
//! is the rule's, so a rule is a variant here and an arm in each question.

use crate::model::tree::Link;
use crate::view::lines::BeadFacts;

/// A rule for which lines the spine opens to. *Spine* is coined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub(super) enum Spine {
    /// Every copy of a bead is on the spine, and the first copy the walk
    /// reaches opens while every later copy rests shut.
    #[default]
    EveryCopy,
}

/// Where one way down stands on the spine, under the rule that placed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Stand {
    EveryCopy {
        /// Whether every link on the way is the one the walk first reached
        /// its bead by.
        first: bool,
    },
}

impl Spine {
    /// Where a way down stands when this rule begins there: at a tree's
    /// root, or at the node a scope set the rule on.
    pub(super) fn begins(self) -> Stand {
        match self {
            Spine::EveryCopy => Stand::EveryCopy { first: true },
        }
    }
}

impl Stand {
    /// Where a child stands, reached by `link` from a parent standing here.
    pub(super) fn beneath(self, link: &Link) -> Stand {
        match self {
            Stand::EveryCopy { first } => Stand::EveryCopy {
                first: first && link.first,
            },
        }
    }

    /// Whether a line standing here rests open, given what its bead says of
    /// the tree beneath it.
    pub(super) fn rests_open(self, bead: &BeadFacts) -> bool {
        match self {
            Stand::EveryCopy { first } => first && bead.opens_a_fold,
        }
    }
}
