//! Each head a pull request moves to, which the beads its gates hold back
//! are told.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::{Event, Outcome, Telling, HEAD_COMMIT};
use crate::collect::gates::PullRequest;
use crate::collect::github::{Observed, State};

pub const EVENT: Event = Event {
    fields: &[HEAD_COMMIT],
    happening: "has a new head",
    outcome: new_head,
};

fn new_head(pr: &PullRequest, observed: &Observed) -> Result<Option<Outcome>, serde_json::Error> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Fields {
        head_ref: Option<Ref>,
    }
    #[derive(Deserialize)]
    struct Ref {
        target: Commit,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Commit {
        oid: Option<String>,
        url: Option<String>,
        committed_date: Option<DateTime<Utc>>,
    }
    let Fields { head_ref } = Fields::deserialize(&observed.fields)?;
    let Some(Commit {
        oid: Some(sha),
        url: Some(url),
        committed_date,
    }) = head_ref.map(|head| head.target)
    else {
        return Ok(None);
    };
    Ok((observed.state == State::Open).then(|| {
        Outcome::Tell(vec![Telling {
            text: told(pr, &sha, &url),
            happened: committed_date,
        }])
    }))
}

/// How a new head, the commit `sha` at `url`, is told, by the sweep and by a
/// delivery alike. The sha is shortened here rather than read shortened from
/// GitHub, so the two always shorten it alike.
pub(super) fn told(pr: &PullRequest, sha: &str, url: &str) -> String {
    let short: String = sha.chars().take(SHORT_SHA).collect();
    format!("Pull request {pr} has a new head, {short}. {url}")
}

/// How many characters of a sha a new head is told by.
const SHORT_SHA: usize = 7;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::pr_events::testing::{once, tellings, when};

    const SHA: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678";

    /// #7 at the head commit `oid`, committed at `at`.
    fn at_head(oid: &str, at: &str) -> String {
        format!(
            r#"{{"headRef":{{"target":{{"oid":"{oid}","url":"https://forge.invalid/example/ark/commit/{oid}","committedDate":"{at}"}}}}}}"#
        )
    }

    fn head_told_on(state: State, fields: &str) -> Option<String> {
        once(tellings(new_head, state, fields)).map(|telling| telling.text)
    }

    #[test]
    fn an_open_pull_requests_head_is_told_by_its_short_sha_and_link() {
        let told = head_told_on(State::Open, &at_head(SHA, "2026-10-06T08:24:49Z"))
            .expect("a head is told");

        assert!(told.contains("example/ark#7"), "{told}");
        assert!(told.contains("a1b2c3d."), "{told}");
        assert!(!told.contains("a1b2c3d4."), "{told}");
        assert!(
            told.contains(&format!("https://forge.invalid/example/ark/commit/{SHA}")),
            "{told}"
        );
    }

    #[test]
    fn two_heads_are_told_apart() {
        let told =
            |abbreviated| head_told_on(State::Open, &at_head(abbreviated, "2026-10-06T08:24:49Z"));

        assert_ne!(told(SHA), told("d4e5f60718293a4b5c6d7e8f9012345678a1b2c3"));
    }

    #[test]
    fn a_head_happened_when_its_commit_was_committed() {
        let told = tellings(new_head, State::Open, &at_head(SHA, "2026-10-06T08:24:49Z"));

        assert_eq!(
            once(told).expect("a head is told").happened,
            when("2026-10-06T08:24:49Z")
        );
    }

    #[test]
    fn a_drafts_head_is_told() {
        let draft = at_head(SHA, "2026-10-06T08:24:49Z").replacen('{', r#"{"isDraft":true,"#, 1);
        assert!(head_told_on(State::Open, &draft).is_some());
    }

    #[test]
    fn a_pull_request_that_is_no_longer_open_is_not_told_of_its_head() {
        let head = at_head(SHA, "2026-10-06T08:24:49Z");
        assert_eq!(head_told_on(State::Closed, &head), None);
        assert_eq!(head_told_on(State::Merged, &head), None);
    }

    /// A head branch deleted under an open pull request leaves it no head
    /// to tell.
    #[test]
    fn a_pull_request_whose_head_branch_has_gone_is_not_told() {
        assert_eq!(head_told_on(State::Open, r#"{"headRef":null}"#), None);
    }
}
