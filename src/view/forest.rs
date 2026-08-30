//! The snapshot, flattened into the lines the screen shows.

use crate::model::join::BeadKey;
use crate::model::snapshot::Snapshot;
use crate::view::Action;

/// One snapshot's rows in render order, with the fold state and the selection
/// that decide which of them are visible and which one is current.
pub struct Forest;

/// Flatten a snapshot into its rows.
pub fn flatten(_snapshot: &Snapshot) -> Forest {
    todo!()
}

impl Forest {
    /// Apply one action, reporting whether it changed anything.
    pub fn apply(&mut self, _action: Action) -> bool {
        todo!()
    }

    /// The bead the selection sits on, where it sits on one.
    pub fn selected(&self) -> Option<&BeadKey> {
        todo!()
    }
}
