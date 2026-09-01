//! What more than one of the loop's tests is built out of.
//!
//! A fixture used on one side of a seam belongs in the module that side is
//! in; these are the ones both sides want.

use std::collections::BTreeMap;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};

use crate::app::{InFlight, Wanted};
use crate::model::snapshot::{Filter, HerdrState, Snapshot, TrackerFailure, Tree};

/// Long enough that a thread which was going to report has, and short
/// enough that a test waiting in vain is not a hang.
pub(in crate::tui) const A_MOMENT: Duration = Duration::from_secs(5);

pub(in crate::tui) fn atlas() -> Wanted {
    Wanted::Project("atlas".to_string())
}

pub(in crate::tui) fn ferry() -> Wanted {
    Wanted::Project("ferry".to_string())
}

/// A collection reading `wanted`, asked for at `asked_at`.
///
/// The instant is handed in rather than taken from the clock, because what a
/// project line says about a collection turns on how long it has been waiting
/// — so a test that could not place the ask against the instant it draws at
/// could not say which mark it expected.
pub(in crate::tui) fn reading(wanted: Wanted, asked_at: DateTime<Utc>) -> InFlight {
    InFlight {
        wanted,
        asked_at,
        patience: PATIENCE,
    }
}

/// How long the collections these tests build may go unanswered. A round
/// number the instants are written against, rather than the configured
/// default: what they assert is which mark a wait produces, not what the
/// deadline is.
pub(in crate::tui) const PATIENCE: TimeDelta = TimeDelta::seconds(30);

/// A snapshot of one unremarkable tree. Nothing the loop does depends on
/// what is in one, only on when it arrives.
pub(in crate::tui) fn a_snapshot() -> Snapshot {
    let tree = Tree {
        project: "atlas".to_string(),
        root: "a-1".to_string(),
        title: "the only tree there is".to_string(),
        ..Tree::tracker_unreachable("atlas", "a-1", TrackerFailure::Unavailable)
    };

    Snapshot {
        generated_at: Utc::now(),
        herdr: HerdrState::Ok,
        filter: Filter::LiveAgents,
        trees: vec![tree.clone()],
        hidden_trees: Vec::new(),
        failed_projects: Vec::new(),
        unattributed: Vec::new(),
        unconfigured: Vec::new(),
        conflicts: Vec::new(),
        read_at: BTreeMap::new(),
        collected: vec![tree],
    }
}
