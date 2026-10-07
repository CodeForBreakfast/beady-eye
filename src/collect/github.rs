//! What GitHub says of a pull request now, and of the rate limit asking
//! spends, read through `gh`.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::collect::gates::PullRequest;
use crate::collect::run::{Env, RunFailure, Runner};
use crate::model::gate::Repository;

/// Where a pull request stands on GitHub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Open, and still a draft or ready for review.
    Open { draft: bool },
    /// Merged, as the commit the merge made where GitHub names one.
    Merged { commit: Option<String> },
    /// Closed without being merged.
    Closed,
}

/// What `gh pr view --json state,isDraft,mergeCommit` prints, and what
/// [`states`]' query asks of each pull request. Measured on gh 2.102.0:
/// `state` is `OPEN`, `CLOSED` or `MERGED`, `isDraft` is always there, and
/// `mergeCommit` is null until a merge.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Viewed {
    state: String,
    is_draft: bool,
    merge_commit: Option<MergeCommit>,
}

#[derive(Deserialize)]
struct MergeCommit {
    oid: String,
}

/// What `gh api graphql` prints for [`states`]' query: the repository, with
/// each pull request asked about under the alias `pr<number>`. Measured on
/// gh 2.102.0, which exits 1 where the repository or any one of the pull
/// requests is not there.
#[derive(Deserialize)]
struct Queried {
    data: Data,
}

#[derive(Deserialize)]
struct Data {
    repository: Option<BTreeMap<String, Option<Viewed>>>,
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
            "state,isDraft,mergeCommit",
        ],
        None,
        &Env::new(),
    )?;
    let viewed: Viewed = serde_json::from_str(&out).map_err(|e| RunFailure::parse("gh", e))?;
    viewed.state()
}

/// Each of `numbers` in `repo` as GitHub has it now, in the order asked, read
/// in one query, so the caller bounds how many there are. The owner and name
/// go to `gh` as variables rather than into the query, since a gate's writer
/// chose them. `gh` picks the host for a `repo` that names none, as [`state`]
/// has it.
pub fn states(
    runner: &dyn Runner,
    repo: &Repository,
    numbers: &[u64],
) -> Result<Vec<State>, RunFailure> {
    let asked: Vec<String> = numbers
        .iter()
        .map(|number| {
            format!("pr{number}:pullRequest(number:{number}){{state isDraft mergeCommit{{oid}}}}")
        })
        .collect();
    let query = format!(
        "query=query($owner:String!,$name:String!){{repository(owner:$owner,name:$name){{{}}}}}",
        asked.join(" ")
    );
    let owner = format!("owner={}", repo.owner);
    let name = format!("name={}", repo.name);
    let mut args = vec!["api", "graphql"];
    if let Some(host) = repo.host {
        args.extend(["--hostname", host]);
    }
    args.extend(["-f", &owner, "-f", &name, "-f", &query]);
    let out = runner.run("gh", &args, None, &Env::new())?;
    let queried: Queried = serde_json::from_str(&out).map_err(|e| RunFailure::parse("gh", e))?;
    let mut answered = queried
        .data
        .repository
        .ok_or_else(|| RunFailure::parse("gh", "the answer names no repository"))?;
    numbers
        .iter()
        .map(|number| {
            answered
                .remove(&format!("pr{number}"))
                .flatten()
                .ok_or_else(|| {
                    RunFailure::parse("gh", format!("the answer has no pull request #{number}"))
                })?
                .state()
        })
        .collect()
}

impl Viewed {
    fn state(self) -> Result<State, RunFailure> {
        match self.state.as_str() {
            "OPEN" => Ok(State::Open {
                draft: self.is_draft,
            }),
            "MERGED" => Ok(State::Merged {
                commit: self.merge_commit.map(|commit| commit.oid),
            }),
            "CLOSED" => Ok(State::Closed),
            unknown => Err(RunFailure::parse(
                "gh",
                format!("a pull request state bdi does not know: {unknown}"),
            )),
        }
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

/// When the login gh runs as on `host`, or on the host gh picks where none
/// is named, can ask GitHub again: the latest reset among the limits
/// `bdi gates` spends that are used up, or `None` where neither is, which is
/// GitHub's secondary limit, whose end it does not say. Asking costs nothing
/// against any limit.
pub fn spent_until(
    runner: &dyn Runner,
    host: Option<&str>,
) -> Result<Option<DateTime<Utc>>, RunFailure> {
    let mut args = vec!["api", "rate_limit"];
    if let Some(host) = host {
        args.extend(["--hostname", host]);
    }
    let out = runner.run("gh", &args, None, &Env::new())?;
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
            spent_until(&runner, None),
            Ok(DateTime::from_timestamp(1767227400, 0))
        );
    }

    #[test]
    fn the_limit_read_is_the_one_on_the_host_the_pull_request_is_on() {
        let runner = FakeRunner::default().with(
            "gh api rate_limit --hostname forge.invalid",
            include_str!("../../tests/fixtures/gh_2.102.0_api_rate_limit_graphql_spent.json"),
        );

        assert_eq!(
            spent_until(&runner, Some("forge.invalid")),
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

        assert_eq!(spent_until(&runner, None), Ok(None));
    }

    const VIEW: &str = "gh pr view 42 --repo example/ark --json state,isDraft,mergeCommit";

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
    fn an_open_pull_request_ready_for_review_is_read_as_open_and_no_draft() {
        assert_eq!(
            answering(include_str!(
                "../../tests/fixtures/gh_2.102.0_pr_view_open.json"
            )),
            Ok(State::Open { draft: false })
        );
    }

    #[test]
    fn a_draft_pull_request_is_read_as_open_and_a_draft() {
        assert_eq!(
            answering(include_str!(
                "../../tests/fixtures/gh_2.102.0_pr_view_draft.json"
            )),
            Ok(State::Open { draft: true })
        );
    }

    const QUERY: &str = "query=query($owner:String!,$name:String!){repository(owner:$owner,\
                         name:$name){pr7:pullRequest(number:7){state isDraft mergeCommit{oid}} \
                         pr42:pullRequest(number:42){state isDraft mergeCommit{oid}}}}";

    fn ark(host: Option<&'static str>) -> Repository<'static> {
        Repository {
            host,
            owner: "example",
            name: "ark",
        }
    }

    fn queried(runner: &FakeRunner, host: Option<&'static str>) -> Result<Vec<State>, RunFailure> {
        states(runner, &ark(host), &[7, 42])
    }

    #[test]
    fn a_repositorys_pull_requests_are_read_in_one_query_in_the_order_asked() {
        let runner = FakeRunner::default().with(
            &format!("gh api graphql -f owner=example -f name=ark -f {QUERY}"),
            include_str!("../../tests/fixtures/gh_2.102.0_api_graphql_ark_42_merged.json"),
        );

        assert_eq!(
            queried(&runner, None),
            Ok(vec![
                State::Open { draft: false },
                State::Merged {
                    commit: Some("5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff".to_string())
                }
            ])
        );
        assert_eq!(runner.calls().len(), 1);
    }

    #[test]
    fn a_repository_naming_its_host_is_asked_of_that_host() {
        let runner = FakeRunner::default().with(
            &format!(
                "gh api graphql --hostname git.example.com -f owner=example -f name=ark -f {QUERY}"
            ),
            r#"{"data":{"repository":{"pr7":{"state":"CLOSED","isDraft":false,"mergeCommit":null},"pr42":{"state":"OPEN","isDraft":true,"mergeCommit":null}}}}"#,
        );

        assert_eq!(
            queried(&runner, Some("git.example.com")),
            Ok(vec![State::Closed, State::Open { draft: true }])
        );
    }

    #[test]
    fn an_answer_missing_a_pull_request_asked_about_is_a_failure_to_read() {
        let runner = FakeRunner::default().with(
            &format!("gh api graphql -f owner=example -f name=ark -f {QUERY}"),
            r#"{"data":{"repository":{"pr7":{"state":"OPEN","isDraft":false,"mergeCommit":null},"pr42":null}}}"#,
        );

        let failure = queried(&runner, None).expect_err("#42 is not in the answer");
        assert_eq!(failure.kind, FailureKind::Parse);
        assert!(failure.detail.contains("#42"), "{}", failure.detail);
    }

    #[test]
    fn a_state_gh_has_not_printed_before_is_a_failure_to_read() {
        let failure = answering(r#"{"isDraft":false,"mergeCommit":null,"state":"DRAFT"}"#)
            .expect_err("DRAFT is not a state bdi knows");
        assert_eq!(failure.kind, FailureKind::Parse);
        assert!(failure.detail.contains("DRAFT"), "{}", failure.detail);
    }
}
