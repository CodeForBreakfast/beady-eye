//! A pull request whose reviews approve it, which closes the gates waiting
//! for an approval.

use serde::Deserialize;

use super::{Event, Outcome};
use crate::collect::gates::PullRequest;
use crate::collect::github::{Observed, State};
use crate::model::gate::Until;

pub const EVENT: Event = Event {
    fields: &["reviewDecision"],
    happening: "is approved",
    outcome: approved,
};

fn approved(pr: &PullRequest, observed: &Observed) -> Result<Option<Outcome>, serde_json::Error> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Fields {
        review_decision: Option<String>,
    }
    let Fields { review_decision } = Fields::deserialize(&observed.fields)?;
    Ok(
        (observed.state == State::Open && review_decision.as_deref() == Some("APPROVED")).then(
            || Outcome::Resolve {
                awaited: |until| until == Until::Approved,
                reason: format!("Pull request {pr} is approved."),
            },
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::pr_events::testing::{observed, pr};

    fn approved_on(state: State, fields: &str) -> bool {
        approved(&pr(), &observed(state, fields))
            .expect("the fields read")
            .is_some()
    }

    #[test]
    fn only_an_open_pull_request_with_an_approving_review_decision_is_approved() {
        let approving = r#"{"reviewDecision":"APPROVED"}"#;
        assert!(approved_on(State::Open, approving));
        assert!(!approved_on(State::Closed, approving));
        assert!(!approved_on(State::Merged, approving));
        for decision in ["null", r#""REVIEW_REQUIRED""#, r#""CHANGES_REQUESTED""#] {
            let fields = format!(r#"{{"reviewDecision":{decision}}}"#);
            assert!(!approved_on(State::Open, &fields), "{decision}");
        }
    }
}
