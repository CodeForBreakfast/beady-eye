//! Each thing a pull request can do that `bdi gates` acts on: the fields of
//! GraphQL's `PullRequest` it reads, and what it makes of them.
//!
//! An event either closes the gh:pr gates waiting for it, or tells the beads
//! they hold back what happened, once for each thing it has to say. A new
//! event is a new entry in [`EVENTS`], and a gate waiting for something new
//! is a new [`Until`] beside one.

use serde::Deserialize;

use crate::collect::gates::PullRequest;
use crate::collect::github::{Observed, State};
use crate::model::gate::Until;

/// One thing a pull request can do.
#[derive(Clone, Copy)]
pub struct Event {
    /// The fields of GraphQL's `PullRequest` this event reads, beside the
    /// `state` every event is given.
    pub fields: &'static str,
    /// What the pull request did, as a line reporting it says.
    pub happening: &'static str,
    /// What this event makes of the pull request, or nothing where the pull
    /// request has not done it.
    pub outcome: fn(&PullRequest, &Observed) -> Result<Option<Outcome>, serde_json::Error>,
}

/// What an event asks of the gates waiting on its pull request.
#[derive(Debug)]
pub enum Outcome {
    /// Close each gate whose wait `awaited` accepts, giving `reason`.
    Resolve {
        awaited: fn(Until) -> bool,
        reason: String,
    },
    /// Comment this on each bead a gate holds back, once. Two tellings with
    /// one text are one telling, so the text carries whatever tells this one
    /// from the next.
    Tell(String),
}

/// Every event `bdi gates` acts on.
pub const EVENTS: [Event; 3] = [
    Event {
        fields: "isDraft",
        happening: "is ready for review",
        outcome: ready_for_review,
    },
    Event {
        fields: "mergeCommit{oid}",
        happening: "merged",
        outcome: merged,
    },
    Event {
        fields: "",
        happening: "closed unmerged",
        outcome: closed_unmerged,
    },
];

/// The fields of GraphQL's `PullRequest` that `events` read between them.
pub fn fields(events: &[Event]) -> String {
    events
        .iter()
        .map(|event| event.fields)
        .filter(|fields| !fields.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

impl Outcome {
    /// Whether this asks anything of a gate waiting for `until`.
    pub fn concerns(&self, until: Until) -> bool {
        match self {
            Outcome::Resolve { awaited, .. } => awaited(until),
            Outcome::Tell(_) => true,
        }
    }
}

fn ready_for_review(
    pr: &PullRequest,
    observed: &Observed,
) -> Result<Option<Outcome>, serde_json::Error> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Fields {
        is_draft: bool,
    }
    let Fields { is_draft } = Fields::deserialize(&observed.fields)?;
    Ok(
        (observed.state == State::Open && !is_draft).then(|| Outcome::Resolve {
            awaited: |until| until == Until::ReadyForReview,
            reason: format!("Pull request {pr} is ready for review."),
        }),
    )
}

fn merged(pr: &PullRequest, observed: &Observed) -> Result<Option<Outcome>, serde_json::Error> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Fields {
        merge_commit: Option<MergeCommit>,
    }
    #[derive(Deserialize)]
    struct MergeCommit {
        oid: String,
    }
    let Fields { merge_commit } = Fields::deserialize(&observed.fields)?;
    Ok((observed.state == State::Merged).then(|| Outcome::Resolve {
        awaited: |_| true,
        reason: match merge_commit {
            Some(commit) => format!("Pull request {pr} merged as {}.", commit.oid),
            None => format!("Pull request {pr} merged."),
        },
    }))
}

fn closed_unmerged(
    pr: &PullRequest,
    observed: &Observed,
) -> Result<Option<Outcome>, serde_json::Error> {
    Ok((observed.state == State::Closed).then(|| {
        Outcome::Tell(format!(
            "Pull request {pr} closed without being merged, so the gh:pr gate waiting on it \
             stays open."
        ))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fields today's query asks, so adding an event that reads nothing
    /// new costs GitHub nothing new.
    #[test]
    fn the_events_read_a_draft_and_a_merge_commit_and_nothing_else() {
        assert_eq!(fields(&EVENTS), "isDraft mergeCommit{oid}");
    }
}
