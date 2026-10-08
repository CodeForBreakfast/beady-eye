//! What GitHub says of a pull request now, and of the rate limit asking
//! spends, read through `gh`.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

use crate::collect::run::{Env, FailureKind, RunFailure, Runner};
use crate::model::gate::Repository;

/// Where a pull request stands on GitHub, which every event reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Open,
    Merged,
    /// Closed without being merged.
    Closed,
}

/// A pull request as GitHub answered for it: where it stands, every field
/// [`pull_requests`] asked of it, under the names GraphQL gives them, and
/// the fields GitHub would not answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observed {
    pub state: State,
    pub fields: Value,
    pub refused: Vec<Refusal>,
}

/// A field of one pull request GitHub would not answer, under the name
/// GraphQL answers it, and what GitHub said of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub field: String,
    pub why: String,
}

/// What `gh api graphql` prints for [`pull_requests`]' query: the
/// repository, with each pull request asked about under the alias
/// `pr<number>`, and GraphQL's errors. Measured on gh 2.102.0, which prints
/// the whole answer and exits 1 wherever it holds an error, as it does where
/// the repository or any one of the pull requests is not there.
#[derive(Deserialize)]
struct Queried {
    data: Data,
    #[serde(default)]
    errors: Vec<QueryError>,
}

#[derive(Deserialize)]
struct Data {
    repository: Option<BTreeMap<String, Option<Value>>>,
}

/// One of GraphQL's errors: where in the answer it stands, and what GitHub
/// said.
#[derive(Deserialize)]
struct QueryError {
    #[serde(default)]
    path: Vec<Value>,
    message: String,
}

impl QueryError {
    /// The alias of the pull request this error refused a field of, and the
    /// refusal, or nothing for an error about anything larger than one
    /// field: the query, the repository, or a whole pull request.
    fn refusal(&self) -> Option<(&str, Refusal)> {
        match self.path.as_slice() {
            [repository, alias, field, ..] if repository == "repository" => Some((
                alias.as_str()?,
                Refusal {
                    field: field.as_str()?.to_string(),
                    why: self.message.clone(),
                },
            )),
            _ => None,
        }
    }
}

/// Each of `numbers` in `repo` as GitHub has it now, in the order asked, read
/// in one query, so the caller bounds how many there are. Each is asked its
/// `state` and `fields`, which name fields of GraphQL's `PullRequest`. The
/// owner and name go to `gh` as variables rather than into the query, since
/// a gate's writer chose them. `gh` picks the host for a `repo` that names
/// none, `GH_HOST` included, exactly as `bd gate check` has it pick.
///
/// An answer whose every error refuses one pull request's field is read for
/// all it does hold, and each refusal is handed back with the pull request
/// it refused. Any other error, or a rate limit, fails the query as a whole.
pub fn pull_requests(
    runner: &dyn Runner,
    repo: &Repository,
    numbers: &[u64],
    fields: &str,
) -> Result<Vec<Result<Observed, RunFailure>>, RunFailure> {
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
    let queried = match runner.run_keeping_stdout("gh", &args, None, &Env::new()) {
        Ok(out) => serde_json::from_str::<Queried>(&out).map_err(|e| RunFailure::parse("gh", e))?,
        Err(printed) => match serde_json::from_str::<Queried>(&printed.stdout) {
            Ok(queried)
                if printed.failure.kind != FailureKind::RateLimited
                    && !queried.errors.is_empty()
                    && queried.errors.iter().all(|error| error.refusal().is_some()) =>
            {
                queried
            }
            _ => return Err(printed.failure),
        },
    };
    let mut answered = queried
        .data
        .repository
        .ok_or_else(|| RunFailure::parse("gh", "the answer names no repository"))?;
    let refusals: Vec<(&str, Refusal)> = queried
        .errors
        .iter()
        .filter_map(QueryError::refusal)
        .collect();
    Ok(numbers
        .iter()
        .map(|number| {
            let alias = format!("pr{number}");
            let fields = answered.remove(&alias).flatten().ok_or_else(|| {
                RunFailure::parse("gh", format!("the answer has no pull request #{number}"))
            })?;
            Ok(Observed {
                state: state(&fields)?,
                fields,
                refused: refusals
                    .iter()
                    .filter(|(refused, _)| *refused == alias)
                    .map(|(_, refusal)| refusal.clone())
                    .collect(),
            })
        })
        .collect())
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
            refused: Vec::new(),
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
    ) -> Result<Vec<Result<Observed, RunFailure>>, RunFailure> {
        pull_requests(runner, &ark(host), &[7, 42], FIELDS)
    }

    fn states(
        observed: Result<Vec<Result<Observed, RunFailure>>, RunFailure>,
    ) -> Result<Vec<State>, RunFailure> {
        observed.and_then(|each| {
            each.into_iter()
                .map(|one| one.map(|one| one.state))
                .collect()
        })
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
                Ok(observed(
                    State::Open,
                    r#"{"state":"OPEN","isDraft":false,"mergeCommit":null}"#
                )),
                Ok(observed(
                    State::Merged,
                    r#"{"state":"MERGED","isDraft":false,"mergeCommit":{"oid":"5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff"}}"#
                )),
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
            Ok(vec![Ok(observed(State::Closed, r#"{"state":"CLOSED"}"#))])
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

    /// The one of `observed` that failed to read, which is #42.
    fn failed_42(observed: Result<Vec<Result<Observed, RunFailure>>, RunFailure>) -> RunFailure {
        let mut each = observed.expect("the query was answered");
        assert_eq!(each.len(), 2);
        assert!(each[0].is_ok(), "#7 reads: {:?}", each[0]);
        each.remove(1).expect_err("#42 does not read")
    }

    #[test]
    fn an_answer_missing_a_pull_request_asked_about_fails_to_read_only_that_one() {
        let runner = FakeRunner::default().with(
            &format!("gh api graphql -f owner=example -f name=ark -f {QUERY}"),
            r#"{"data":{"repository":{"pr7":{"state":"OPEN","isDraft":false,"mergeCommit":null},"pr42":null}}}"#,
        );

        let failure = failed_42(queried(&runner, None));
        assert_eq!(failure.kind, FailureKind::Parse);
        assert!(failure.detail.contains("#42"), "{}", failure.detail);
    }

    #[test]
    fn a_state_gh_has_not_printed_before_fails_to_read_only_that_pull_request() {
        let runner = FakeRunner::default().with(
            &format!("gh api graphql -f owner=example -f name=ark -f {QUERY}"),
            r#"{"data":{"repository":{"pr7":{"state":"OPEN","isDraft":false,"mergeCommit":null},"pr42":{"state":"DRAFT","isDraft":false,"mergeCommit":null}}}}"#,
        );

        let failure = failed_42(queried(&runner, None));
        assert_eq!(failure.kind, FailureKind::Parse);
        assert!(failure.detail.contains("DRAFT"), "{}", failure.detail);
    }

    /// What gh says on stderr for an answer holding errors, which bdi does
    /// not read past its classification.
    fn exited(kind: FailureKind) -> RunFailure {
        RunFailure {
            kind,
            program: "gh".to_string(),
            detail: "gh exited 1".to_string(),
            unreadable: None,
        }
    }

    /// An answer as gh prints it before exiting 1, measured on gh 2.102.0
    /// for an error naming a pull request: the data GitHub could give, and
    /// an error with the path to what it could not. The error refusing a
    /// field is not yet measured, and has the shape GraphQL gives every
    /// error.
    const CHECKS_REFUSED_ON_7: &str = r#"{"data":{"repository":{"pr7":{"state":"OPEN","isDraft":false,"mergeCommit":null,"commits":{"nodes":[{"commit":{"oid":"5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff","statusCheckRollup":null}}]}},"pr42":{"state":"MERGED","isDraft":false,"mergeCommit":{"oid":"0badc0de0badc0de0badc0de0badc0de0badc0de"}}}},"errors":[{"type":"FORBIDDEN","path":["repository","pr7","commits","nodes",0,"commit","statusCheckRollup"],"locations":[{"line":1,"column":200}],"message":"Resource not accessible by personal access token"}]}"#;

    #[test]
    fn a_field_github_refused_is_handed_back_with_the_pull_request_it_refused_and_the_rest_read() {
        let runner = FakeRunner::default().failing_having_printed(
            &format!("gh api graphql -f owner=example -f name=ark -f {QUERY}"),
            CHECKS_REFUSED_ON_7,
            exited(FailureKind::Unavailable),
        );

        let each = queried(&runner, None).expect("the answer is read for what it holds");
        let refused: Vec<Vec<Refusal>> = each
            .iter()
            .map(|one| one.as_ref().expect("both read").refused.clone())
            .collect();
        assert_eq!(
            refused,
            [
                vec![Refusal {
                    field: "commits".to_string(),
                    why: "Resource not accessible by personal access token".to_string(),
                }],
                vec![],
            ]
        );
        assert_eq!(states(Ok(each)), Ok(vec![State::Open, State::Merged]));
    }

    /// An error about anything larger than one pull request's field, beside
    /// one that is, leaves the query failed for what gh's exit said.
    #[test]
    fn an_answer_with_any_error_larger_than_a_field_fails_as_gh_said() {
        let larger = [
            r#"{"type":"NOT_FOUND","path":["repository","pr42"],"message":"Could not resolve to a PullRequest with the number of 42."}"#,
            r#"{"type":"NOT_FOUND","path":["repository"],"message":"Could not resolve to a Repository with the name 'example/ark'."}"#,
            r#"{"type":"RATE_LIMITED","message":"API rate limit exceeded for user ID 1."}"#,
        ];
        for error in larger {
            let printed =
                CHECKS_REFUSED_ON_7.replace(r#""errors":["#, &format!(r#""errors":[{error},"#));
            let runner = FakeRunner::default().failing_having_printed(
                &format!("gh api graphql -f owner=example -f name=ark -f {QUERY}"),
                &printed,
                exited(FailureKind::Gone),
            );

            assert_eq!(
                queried(&runner, None),
                Err(exited(FailureKind::Gone)),
                "{error}"
            );
        }
    }

    /// A rate limit is waited out whatever the answer beside it holds.
    #[test]
    fn a_rate_limit_fails_the_query_however_its_errors_are_placed() {
        let runner = FakeRunner::default().failing_having_printed(
            &format!("gh api graphql -f owner=example -f name=ark -f {QUERY}"),
            CHECKS_REFUSED_ON_7,
            exited(FailureKind::RateLimited),
        );

        assert_eq!(
            queried(&runner, None),
            Err(exited(FailureKind::RateLimited))
        );
    }

    #[test]
    fn a_failure_with_nothing_readable_on_stdout_fails_as_gh_said() {
        for printed in [
            "",
            "not json",
            r#"{"data":{"repository":{"pr7":null,"pr42":null}}}"#,
        ] {
            let runner = FakeRunner::default().failing_having_printed(
                &format!("gh api graphql -f owner=example -f name=ark -f {QUERY}"),
                printed,
                exited(FailureKind::Unavailable),
            );

            assert_eq!(
                queried(&runner, None),
                Err(exited(FailureKind::Unavailable)),
                "{printed}"
            );
        }
    }
}
