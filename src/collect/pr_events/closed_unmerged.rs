//! A pull request closed without being merged, which the beads its gates
//! hold back are told.

use super::{Event, Outcome, Telling};
use crate::collect::gates::PullRequest;
use crate::collect::github::{Observed, State};

pub const EVENT: Event = Event {
    fields: &[],
    happening: "closed unmerged",
    outcome: closed_unmerged,
};

fn closed_unmerged(
    pr: &PullRequest,
    observed: &Observed,
) -> Result<Option<Outcome>, serde_json::Error> {
    // A gate made on a pull request already closed waits for a merge that
    // cannot come, so it is told whenever it was made.
    Ok((observed.state == State::Closed).then(|| {
        Outcome::Tell(vec![Telling {
            text: format!(
                "Pull request {pr} closed without being merged, so the gh:pr gate waiting on \
                 it stays open."
            ),
            happened: None,
        }])
    }))
}
