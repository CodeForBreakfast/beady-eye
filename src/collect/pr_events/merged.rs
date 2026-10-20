use serde::Deserialize;

use super::{Event, Outcome};
use crate::collect::gates::PullRequest;
use crate::collect::github::{Observed, State};

pub const EVENT: Event = Event {
    fields: &["mergeCommit{oid}"],
    happening: "merged",
    outcome: merged,
};

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
