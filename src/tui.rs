//! The terminal's lifecycle and the loop that keeps the view live.

use crate::model::snapshot::Snapshot;

/// Draw the snapshot until the user quits, re-collecting on a refresh.
pub fn run(_collect: Box<dyn Fn() -> Snapshot + Send>) -> anyhow::Result<()> {
    todo!()
}
