//! A pull request out of draft, which closes the gates waiting for it to be
//! ready for review.

use serde::Deserialize;

use super::{Event, Outcome};
use crate::collect::gates::PullRequest;
use crate::collect::github::{Observed, State};
use crate::model::gate::Until;

pub const EVENT: Event = Event {
    fields: &["isDraft"],
    happening: "is ready for review",
    optional: &[],
    outcome: ready_for_review,
};

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
