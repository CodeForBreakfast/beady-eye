//! A pull request whose head GitHub says is behind its base, which the beads
//! its gates hold back are told. GitHub says so only where the base requires
//! branches to be up to date before they merge.

use serde::Deserialize;

use super::{head_and_base_since, Event, Outcome, Telling, BASE_COMMIT, HEAD_COMMIT};
use crate::collect::gates::PullRequest;
use crate::collect::github::{Observed, State};

pub const EVENT: Event = Event {
    fields: &["mergeStateStatus", "headRefOid", HEAD_COMMIT, BASE_COMMIT],
    happening: "is behind its base",
    outcome: behind,
};

fn behind(pr: &PullRequest, observed: &Observed) -> Result<Option<Outcome>, serde_json::Error> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Fields {
        merge_state_status: Option<String>,
        head_ref_oid: Option<String>,
    }
    let Fields {
        merge_state_status,
        head_ref_oid,
    } = Fields::deserialize(&observed.fields)?;
    let happened = head_and_base_since(observed)?;
    Ok(
        match (observed.state, merge_state_status.as_deref(), head_ref_oid) {
            (State::Open, Some("BEHIND"), Some(oid)) => Some(Outcome::Tell(vec![Telling {
                text: format!(
                    "The head of pull request {pr}, {oid}, is behind its base, so the gh:pr gate \
                     waiting on it stays open."
                ),
                happened,
            }])),
            _ => None,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::pr_events::testing::{once, tellings, when};

    fn behind_told_on(state: State, fields: &str) -> Option<String> {
        once(tellings(behind, state, fields)).map(|telling| telling.text)
    }

    fn merge_state(status: &str, oid: &str) -> String {
        format!(r#"{{"mergeStateStatus":"{status}","headRefOid":"{oid}"}}"#)
    }

    #[test]
    fn an_open_pull_request_whose_head_is_behind_is_told_by_that_head() {
        let told = |oid| {
            behind_told_on(State::Open, &merge_state("BEHIND", oid)).expect("a behind head is told")
        };

        assert!(told("a1b2c3").contains("a1b2c3"));
        assert_ne!(told("a1b2c3"), told("d4e5f6"));
    }

    #[test]
    fn a_draft_whose_head_is_behind_is_told() {
        let draft = merge_state("BEHIND", "a1b2c3").replacen('{', r#"{"isDraft":true,"#, 1);
        assert!(behind_told_on(State::Open, &draft).is_some());
    }

    /// A head behind a base that does not require branches to be up to date
    /// is reported by whatever else holds of it, so it is never told.
    #[test]
    fn a_pull_request_github_does_not_say_is_behind_is_not_told() {
        for status in [
            "CLEAN",
            "UNSTABLE",
            "BLOCKED",
            "DIRTY",
            "HAS_HOOKS",
            "DRAFT",
            "UNKNOWN",
        ] {
            assert_eq!(
                behind_told_on(State::Open, &merge_state(status, "a1b2c3")),
                None,
                "{status}"
            );
        }
    }

    #[test]
    fn a_pull_request_that_is_no_longer_open_is_not_told_it_is_behind() {
        let behind = merge_state("BEHIND", "a1b2c3");
        assert_eq!(behind_told_on(State::Closed, &behind), None);
        assert_eq!(behind_told_on(State::Merged, &behind), None);
    }

    fn committed(at: &str) -> String {
        format!(r#"{{"target":{{"committedDate":"{at}"}}}}"#)
    }

    /// A seat's last push comes before its gate as often as not, and the
    /// base moving on after that is what puts the head behind.
    #[test]
    fn a_head_fell_behind_when_the_later_of_it_and_its_base_was_committed() {
        let behind_at = |head: &str, base: &str| {
            let fields = format!(
                r#"{{"mergeStateStatus":"BEHIND","headRefOid":"a1b2c3","headRef":{head},"baseRef":{base}}}"#
            );
            once(tellings(behind, State::Open, &fields))
                .expect("a behind head is told")
                .happened
        };
        let earlier = committed("2026-10-06T08:00:00Z");
        let later = committed("2026-10-06T08:30:00Z");

        assert_eq!(behind_at(&earlier, &later), when("2026-10-06T08:30:00Z"));
        assert_eq!(behind_at(&later, &earlier), when("2026-10-06T08:30:00Z"));
    }
}
