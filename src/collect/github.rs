//! What GitHub says of a pull request now, and of the rate limit asking
//! spends, read through `gh`.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

use crate::collect::run::{Env, RunFailure, Runner};
use crate::model::gate::Repository;

/// Where a pull request stands on GitHub, which every event reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Open,
    Merged,
    /// Closed without being merged.
    Closed,
}

/// A pull request as GitHub answered for it: where it stands, and every
/// field [`pull_requests`] asked of it, under the names GraphQL gives them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observed {
    pub state: State,
    pub fields: Value,
}

/// What `gh api graphql` prints for [`pull_requests`]' query: the
/// repository, with each pull request asked about under the alias
/// `pr<number>`. Measured on gh 2.102.0, which exits 1 where the repository
/// or any one of the pull requests is not there.
#[derive(Deserialize)]
struct Queried {
    data: Data,
}

#[derive(Deserialize)]
struct Data {
    repository: Option<BTreeMap<String, Option<Value>>>,
}

/// Each of `numbers` in `repo` as GitHub has it now, in the order asked, read
/// in one query, so the caller bounds how many there are. Each is asked its
/// `state` and `fields`, which name fields of GraphQL's `PullRequest`. The
/// owner and name go to `gh` as variables rather than into the query, since
/// a gate's writer chose them. `gh` picks the host for a `repo` that names
/// none, `GH_HOST` included, exactly as `bd gate check` has it pick.
pub fn pull_requests(
    runner: &dyn Runner,
    repo: &Repository,
    numbers: &[u64],
    fields: &str,
) -> Result<Vec<Observed>, RunFailure> {
    let asked_of_each = ["state", fields].join(" ");
    let asked: Vec<String> = numbers
        .iter()
        .map(|number| {
            format!(
                "pr{number}:pullRequest(number:{number}){{{}}}",
                asked_of_each.trim_end()
            )
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
            let fields = answered
                .remove(&format!("pr{number}"))
                .flatten()
                .ok_or_else(|| {
                    RunFailure::parse("gh", format!("the answer has no pull request #{number}"))
                })?;
            Ok(Observed {
                state: state(&fields)?,
                fields,
            })
        })
        .collect()
}

/// What `gh api repos/OWNER/NAME/commits/SHA/pulls` prints, cut to what this
/// reads. It lists every pull request whose history holds the commit, merged
/// and closed ones among them.
#[derive(Deserialize)]
struct PullRequestAt {
    number: u64,
    state: String,
    head: Head,
}

#[derive(Deserialize)]
struct Head {
    sha: String,
}

/// The numbers of the open pull requests in `repo` whose head is the commit
/// `sha`, which the caller has checked is hexadecimal since it goes into the
/// path.
pub fn open_pull_requests_headed_by(
    runner: &dyn Runner,
    repo: &Repository,
    sha: &str,
) -> Result<Vec<u64>, RunFailure> {
    // ponytail: the first hundred only. A commit in more pull requests than
    // that is settled by the next look. Follow the pages if one ever is.
    let path = format!(
        "repos/{}/{}/commits/{sha}/pulls?per_page=100",
        repo.owner, repo.name
    );
    let mut args = vec!["api", &path];
    if let Some(host) = repo.host {
        args.extend(["--hostname", host]);
    }
    let out = runner.run("gh", &args, None, &Env::new())?;
    let listed: Vec<PullRequestAt> =
        serde_json::from_str(&out).map_err(|e| RunFailure::parse("gh", e))?;
    Ok(listed
        .into_iter()
        .filter(|pull_request| {
            pull_request.state == "open" && pull_request.head.sha.eq_ignore_ascii_case(sha)
        })
        .map(|pull_request| pull_request.number)
        .collect())
}

/// Where `fields` say their pull request stands. GraphQL gives `state` as
/// `OPEN`, `CLOSED` or `MERGED`.
fn state(fields: &Value) -> Result<State, RunFailure> {
    match fields.get("state").and_then(Value::as_str) {
        Some("OPEN") => Ok(State::Open),
        Some("MERGED") => Ok(State::Merged),
        Some("CLOSED") => Ok(State::Closed),
        unknown => Err(RunFailure::parse(
            "gh",
            format!(
                "a pull request state bdi does not know: {}",
                unknown.unwrap_or("none")
            ),
        )),
    }
}

/// The limits `bdi gates` spends: GraphQL for the pull requests, and REST.
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

    const SHA: &str = "5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff";
    const COMMIT_PULLS: &str =
        "gh api repos/example/ark/commits/5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff/pulls?per_page=100";

    /// #7 open with the commit as its head, #8 open with the commit only in
    /// its history, #9 closed with the commit as its head.
    const LISTED: &str = r#"[
        {"number":7,"state":"open","head":{"sha":"5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff"}},
        {"number":8,"state":"open","head":{"sha":"0badc0de0badc0de0badc0de0badc0de0badc0de"}},
        {"number":9,"state":"closed","head":{"sha":"5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff"}}
    ]"#;

    #[test]
    fn only_open_pull_requests_headed_by_the_commit_are_found() {
        let runner = FakeRunner::default().with(COMMIT_PULLS, LISTED);

        assert_eq!(
            open_pull_requests_headed_by(&runner, &ark(None), SHA),
            Ok(vec![7])
        );
    }

    #[test]
    fn the_pull_requests_of_a_commit_are_asked_of_the_host_the_repository_is_on() {
        let runner =
            FakeRunner::default().with(&format!("{COMMIT_PULLS} --hostname forge.invalid"), "[]");

        assert_eq!(
            open_pull_requests_headed_by(&runner, &ark(Some("forge.invalid")), SHA),
            Ok(vec![])
        );
    }

    #[test]
    fn an_answer_that_is_not_a_list_of_pull_requests_is_a_parse_failure() {
        let runner = FakeRunner::default().with(COMMIT_PULLS, r#"{"message":"Not Found"}"#);

        let failure = open_pull_requests_headed_by(&runner, &ark(None), SHA).unwrap_err();
        assert_eq!(failure.kind, FailureKind::Parse);
    }

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

    const FIELDS: &str = "isDraft mergeCommit{oid} reviewDecision commits(last:1){nodes{commit{oid statusCheckRollup{state contexts(last:100){nodes{...on CheckRun{conclusion completedAt} ...on StatusContext{state createdAt}}}}}}} reviews(last:5){nodes{url state submittedAt author{login}}} mergeable headRefOid headRef{target{...on Commit{committedDate}}} baseRef{target{...on Commit{committedDate}}} comments(last:5){nodes{url createdAt author{login}}}";

    const QUERY: &str = "query=query($owner:String!,$name:String!){repository(owner:$owner,\
                         name:$name){pr7:pullRequest(number:7){state isDraft mergeCommit{oid} reviewDecision commits(last:1){nodes{commit{oid statusCheckRollup{state contexts(last:100){nodes{...on CheckRun{conclusion completedAt} ...on StatusContext{state createdAt}}}}}}} reviews(last:5){nodes{url state submittedAt author{login}}} mergeable headRefOid headRef{target{...on Commit{committedDate}}} baseRef{target{...on Commit{committedDate}}} comments(last:5){nodes{url createdAt author{login}}}} \
                         pr42:pullRequest(number:42){state isDraft mergeCommit{oid} reviewDecision commits(last:1){nodes{commit{oid statusCheckRollup{state contexts(last:100){nodes{...on CheckRun{conclusion completedAt} ...on StatusContext{state createdAt}}}}}}} reviews(last:5){nodes{url state submittedAt author{login}}} mergeable headRefOid headRef{target{...on Commit{committedDate}}} baseRef{target{...on Commit{committedDate}}} comments(last:5){nodes{url createdAt author{login}}}}}}";

    fn observed(state: State, fields: &str) -> Observed {
        Observed {
            state,
            fields: serde_json::from_str(fields).expect("the fields parse"),
        }
    }

    fn ark(host: Option<&'static str>) -> Repository<'static> {
        Repository {
            host,
            owner: "example",
            name: "ark",
        }
    }

    fn queried(
        runner: &FakeRunner,
        host: Option<&'static str>,
    ) -> Result<Vec<Observed>, RunFailure> {
        pull_requests(runner, &ark(host), &[7, 42], FIELDS)
    }

    fn states(observed: Result<Vec<Observed>, RunFailure>) -> Result<Vec<State>, RunFailure> {
        observed.map(|each| each.into_iter().map(|one| one.state).collect())
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
                observed(
                    State::Open,
                    r#"{"state":"OPEN","isDraft":false,"mergeCommit":null}"#
                ),
                observed(
                    State::Merged,
                    r#"{"state":"MERGED","isDraft":false,"mergeCommit":{"oid":"5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff"}}"#
                ),
            ])
        );
        assert_eq!(runner.calls().len(), 1);
    }

    #[test]
    fn a_pull_request_closed_without_a_merge_is_read_as_closed() {
        let runner = FakeRunner::default().with(
            &format!("gh api graphql -f owner=example -f name=ark -f {QUERY}"),
            r#"{"data":{"repository":{"pr7":{"state":"CLOSED","isDraft":false,"mergeCommit":null},"pr42":{"state":"OPEN","isDraft":true,"mergeCommit":null}}}}"#,
        );

        assert_eq!(
            states(queried(&runner, None)),
            Ok(vec![State::Closed, State::Open])
        );
    }

    #[test]
    fn a_pull_request_asked_for_no_fields_is_asked_only_where_it_stands() {
        let runner = FakeRunner::default().with(
            "gh api graphql -f owner=example -f name=ark -f query=query($owner:String!,\
             $name:String!){repository(owner:$owner,name:$name){pr7:pullRequest(number:7)\
             {state}}}",
            r#"{"data":{"repository":{"pr7":{"state":"CLOSED"}}}}"#,
        );

        assert_eq!(
            pull_requests(&runner, &ark(None), &[7], ""),
            Ok(vec![observed(State::Closed, r#"{"state":"CLOSED"}"#)])
        );
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
            states(queried(&runner, Some("git.example.com"))),
            Ok(vec![State::Closed, State::Open])
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
        let runner = FakeRunner::default().with(
            &format!("gh api graphql -f owner=example -f name=ark -f {QUERY}"),
            r#"{"data":{"repository":{"pr7":{"state":"OPEN","isDraft":false,"mergeCommit":null},"pr42":{"state":"DRAFT","isDraft":false,"mergeCommit":null}}}}"#,
        );

        let failure = queried(&runner, None).expect_err("DRAFT is not a state bdi knows");
        assert_eq!(failure.kind, FailureKind::Parse);
        assert!(failure.detail.contains("DRAFT"), "{}", failure.detail);
    }
}
