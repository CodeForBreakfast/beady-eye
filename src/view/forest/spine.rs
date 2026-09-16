//! Where a way down stands on the spine, and what a line at a stand does.
//!
//! The fold default opens a line when it stands on the spine to work a
//! reader needs on the first screen. Layout does not know what the spine is.
//! It asks where a tree's root stands, where a child stands given its
//! parent's stand and the link taken, and whether a line at a stand rests
//! open, and it keys what it counts on the stand. The rule here is the one
//! there is: the first copy of a bead opens, and every later copy rests shut.

use crate::model::tree::Link;
use crate::view::lines::BeadFacts;

/// Where one way down stands on the spine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct Stand {
    /// Whether every link on the way is the one the walk first reached its
    /// bead by.
    first: bool,
}

/// Where a tree's root stands.
pub(super) fn root() -> Stand {
    Stand { first: true }
}

/// Where a child stands, reached by `link` from a parent at `stand`.
pub(super) fn beneath(stand: Stand, link: &Link) -> Stand {
    Stand {
        first: stand.first && link.first,
    }
}

/// Whether a line at `stand` rests open, given what its bead says of the
/// tree beneath it.
pub(super) fn rests_open(stand: Stand, bead: &BeadFacts) -> bool {
    stand.first && bead.opens_a_fold
}
