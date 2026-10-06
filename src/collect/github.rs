//! What GitHub says of a pull request now, and of the rate limit asking
//! spends, read through `gh`.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::collect::gates::PullRequest;
use crate::collect::run::{Env, RunFailure, Runner};

/// Where a pull request stands on GitHub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Open,
    /// Merged, as the commit the merge made where GitHub names one.
    Merged {
        commit: Option<String>,
    },
    /// Closed without being merged.
    Closed,
}

/// What `gh pr view --json state,mergeCommit` prints. Measured on gh
/// 2.102.0: `state` is `OPEN`, `CLOSED` or `MERGED`, and `mergeCommit` is
/// null until a merge.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Viewed {
    state: String,
    merge_commit: Option<MergeCommit>,
}

#[derive(Deserialize)]
struct MergeCommit {
    oid: String,
}

/// `pr` as GitHub has it now. The repository is always named, so the answer
/// never depends on the directory `gh` runs in. It goes to `gh` as the gate
/// wrote it, so `gh` picks the host for one that names none, `GH_HOST`
/// included, exactly as `bd gate check` has it pick.
pub fn state(runner: &dyn Runner, pr: &PullRequest) -> Result<State, RunFailure> {
    let number = pr.number.to_string();
    let out = runner.run(
        "gh",
        &[
            "pr",
            "view",
            &number,
            "--repo",
            &pr.repo,
            "--json",
            "state,mergeCommit",
        ],
        None,
        &Env::new(),
    )?;
    let viewed: Viewed = serde_json::from_str(&out).map_err(|e| RunFailure::parse("gh", e))?;
    match viewed.state.as_str() {
        "OPEN" => Ok(State::Open),
        "MERGED" => Ok(State::Merged {
            commit: viewed.merge_commit.map(|commit| commit.oid),
        }),
        "CLOSED" => Ok(State::Closed),
        unknown => Err(RunFailure::parse(
            "gh",
            format!("a pull request state bdi does not know: {unknown}"),
        )),
    }
}

/// The limits `bdi gates` spends: GraphQL for `gh pr view`, and REST.
const SPENT_BY_GATES: [&str; 2] = ["graphql", "core"];

/// What `gh api rate_limit` prints, cut to what this reads. Measured on gh
/// 2.102.0: `resources` holds one limit per API, each with `remaining` and
/// `reset` in seconds since the epoch.
#[derive(Deserialize)]
struct RateLimits {
    resources: BTreeMap<String, RateLimit>,
}

#[derive(Deserialize)]
struct RateLimit {
    remaining: u64,
    reset: i64,
}

/// When the login gh runs as can ask GitHub again: the latest reset among
/// the limits `bdi gates` spends that are used up, or `None` where neither
/// is, which is GitHub's secondary limit, whose end it does not say. Asking
/// costs nothing against any limit.
pub fn spent_until(runner: &dyn Runner) -> Result<Option<DateTime<Utc>>, RunFailure> {
    let out = runner.run("gh", &["api", "rate_limit"], None, &Env::new())?;
    let limits: RateLimits = serde_json::from_str(&out).map_err(|e| RunFailure::parse("gh", e))?;
    Ok(SPENT_BY_GATES
        .iter()
        .filter_map(|name| limits.resources.get(*name))
        .filter(|limit| limit.remaining == 0)
        .map(|limit| limit.reset)
        .max()
        .and_then(|reset| DateTime::from_timestamp(reset, 0)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::run::testing::FakeRunner;
    use crate::collect::run::FailureKind;

    const RATE_LIMIT: &str = "gh api rate_limit";

    #[test]
    fn a_spent_graphql_limit_is_waited_out_until_it_resets() {
        let runner = FakeRunner::default().with(
            RATE_LIMIT,
            include_str!("../../tests/fixtures/gh_2.102.0_api_rate_limit_graphql_spent.json"),
        );

        assert_eq!(
            spent_until(&runner),
            Ok(DateTime::from_timestamp(1767227400, 0))
        );
    }

    /// A search limit spent by someone else sharing the login is not one
    /// `bdi gates` waits on.
    #[test]
    fn with_no_limit_bdi_gates_spends_used_up_github_does_not_say_when_to_ask_again() {
        let runner = FakeRunner::default().with(
            RATE_LIMIT,
            include_str!("../../tests/fixtures/gh_2.102.0_api_rate_limit_unspent.json"),
        );

        assert_eq!(spent_until(&runner), Ok(None));
    }

    const VIEW: &str = "gh pr view 42 --repo example/ark --json state,mergeCommit";

    fn pr() -> PullRequest {
        PullRequest {
            repo: "example/ark".to_string(),
            number: 42,
        }
    }

    fn answering(out: &str) -> Result<State, RunFailure> {
        state(&FakeRunner::default().with(VIEW, out), &pr())
    }

    #[test]
    fn a_merged_pull_request_is_read_with_its_merge_commit() {
        assert_eq!(
            answering(include_str!(
                "../../tests/fixtures/gh_2.102.0_pr_view_merged.json"
            )),
            Ok(State::Merged {
                commit: Some("5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff".to_string())
            })
        );
    }

    #[test]
    fn a_pull_request_closed_without_a_merge_is_read_as_closed() {
        assert_eq!(
            answering(include_str!(
                "../../tests/fixtures/gh_2.102.0_pr_view_closed.json"
            )),
            Ok(State::Closed)
        );
    }

    #[test]
    fn an_open_pull_request_is_read_as_open() {
        assert_eq!(
            answering(include_str!(
                "../../tests/fixtures/gh_2.102.0_pr_view_open.json"
            )),
            Ok(State::Open)
        );
    }

    #[test]
    fn a_state_gh_has_not_printed_before_is_a_failure_to_read() {
        let failure = answering(r#"{"mergeCommit":null,"state":"DRAFT"}"#)
            .expect_err("DRAFT is not a state bdi knows");
        assert_eq!(failure.kind, FailureKind::Parse);
        assert!(failure.detail.contains("DRAFT"), "{}", failure.detail);
    }
}
