//! What GitHub says of a pull request, an issue, a workflow run or a
//! repository's releases now, and of the rate limit asking spends, read through `gh`.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

use crate::collect::run::{Env, FailureKind, RunFailure, Runner};
use crate::model::gate::{self, Repository};

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
/// `state` and `fields`, which name fields of GraphQL's `PullRequest`.
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
    let args = graphql(repo, &asked.join(" "));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
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
            let mut refused: Vec<Refusal> = Vec::new();
            for (_, refusal) in refusals.iter().filter(|(refused, _)| *refused == alias) {
                if !refused.iter().any(|seen| seen.field == refusal.field) {
                    refused.push(refusal.clone());
                }
            }
            Ok(Observed {
                state: state(&fields)?,
                fields,
                refused,
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
    let listed: Vec<PullRequestAt> = rest(runner, repo, &path)?;
    Ok(listed
        .into_iter()
        .filter(|pull_request| {
            pull_request.state == "open" && pull_request.head.sha.eq_ignore_ascii_case(sha)
        })
        .map(|pull_request| pull_request.number)
        .collect())
}

/// Whether issue `number` in `repo` is closed, for whatever reason. GraphQL's
/// `issue` answers only for an issue, so the number of a pull request is one
/// GitHub does not have.
pub fn issue_closed(
    runner: &dyn Runner,
    repo: &Repository,
    number: u64,
) -> Result<bool, RunFailure> {
    let answer = asked(runner, repo, &format!("issue(number:{number}){{state}}"))?;
    match answer.pointer("/issue/state").and_then(Value::as_str) {
        Some("OPEN") => Ok(false),
        Some("CLOSED") => Ok(true),
        unknown => Err(RunFailure::parse(
            "gh",
            format!(
                "an issue state bdi does not know: {}",
                unknown.unwrap_or("none")
            ),
        )),
    }
}

/// A workflow run as GitHub has it now: its workflow's name, where it
/// stands, and how it ended, which GitHub says only once it has completed.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Run {
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
}

/// Workflow run `id` in `repo`, read from REST, since GraphQL finds a run
/// only by its node id.
pub fn workflow_run(runner: &dyn Runner, repo: &Repository, id: u64) -> Result<Run, RunFailure> {
    let path = format!("repos/{}/{}/actions/runs/{id}", repo.owner, repo.name);
    rest(runner, repo, &path)
}

/// What Actions lists of a commit's workflow runs, cut to what this reads.
#[derive(Deserialize)]
struct WorkflowRuns {
    total_count: usize,
    workflow_runs: Vec<WorkflowRun>,
}

/// One workflow run as REST gives it. GitHub gives a conclusion once the run
/// has completed, and says when it finished only as when it was last updated.
#[derive(Deserialize)]
struct WorkflowRun {
    id: u64,
    name: Option<String>,
    workflow_id: u64,
    event: String,
    conclusion: Option<String>,
    run_started_at: Option<DateTime<Utc>>,
    updated_at: Option<DateTime<Utc>>,
}

/// What GitHub combines of a commit's statuses: the latest for each context.
#[derive(Deserialize)]
struct CombinedStatus {
    total_count: usize,
    statuses: Vec<CommitStatus>,
}

#[derive(Deserialize)]
struct CommitStatus {
    state: String,
    created_at: Option<DateTime<Utc>>,
}

/// The checks on commit `sha` in `repo`, read from its workflow runs and its
/// commit statuses, which GitHub lets a token read that it refuses check
/// runs to. They are given in the shape GraphQL answers `statusCheckRollup`,
/// each run as a check run of its workflow for its event, and null where the
/// commit has neither. `sha` goes into the path, so the caller has checked
/// it is hexadecimal.
///
/// A workflow the commit triggered more than once for the same event is
/// given by its latest run, which replaced the others. The rollup fails where
/// any run or status failed, as GitHub combines statuses, is pending where
/// none failed and any has not finished or was not read, and succeeds
/// otherwise.
pub fn checks_without_check_runs(
    runner: &dyn Runner,
    repo: &Repository,
    sha: &str,
) -> Result<Value, RunFailure> {
    // ponytail: the first hundred runs and statuses only, so a commit with
    // more never passes. Follow the pages if a commit ever has more.
    let (owner, name) = (repo.owner, repo.name);
    let runs: WorkflowRuns = rest(
        runner,
        repo,
        &format!("repos/{owner}/{name}/actions/runs?head_sha={sha}&per_page=100"),
    )?;
    let statuses: CombinedStatus = rest(
        runner,
        repo,
        &format!("repos/{owner}/{name}/commits/{sha}/status?per_page=100"),
    )?;
    if runs.workflow_runs.is_empty() && statuses.statuses.is_empty() {
        return Ok(Value::Null);
    }
    let unread = runs.total_count > runs.workflow_runs.len()
        || statuses.total_count > statuses.statuses.len();
    let mut latest: BTreeMap<(u64, &str), u64> = BTreeMap::new();
    for run in &runs.workflow_runs {
        let id = latest.entry((run.workflow_id, &run.event)).or_default();
        *id = (*id).max(run.id);
    }
    let latest: Vec<u64> = latest.into_values().collect();
    let replaced = runs.workflow_runs.len() - latest.len();
    let runs_count = runs.total_count.saturating_sub(replaced);
    let runs: Vec<WorkflowRun> = runs
        .workflow_runs
        .into_iter()
        .filter(|run| latest.contains(&run.id))
        .collect();
    let finished_well = |conclusion: &str| matches!(conclusion, "success" | "neutral" | "skipped");
    let failed = runs
        .iter()
        .any(|run| run.conclusion.as_deref().is_some_and(|c| !finished_well(c)))
        || statuses
            .statuses
            .iter()
            .any(|status| matches!(status.state.as_str(), "failure" | "error"));
    let going = unread
        || runs.iter().any(|run| run.conclusion.is_none())
        || statuses
            .statuses
            .iter()
            .any(|status| status.state == "pending");
    let state = match (failed, going) {
        (true, _) => "FAILURE",
        (false, true) => "PENDING",
        (false, false) => "SUCCESS",
    };
    let nodes = runs
        .into_iter()
        .map(|run| {
            serde_json::json!({
                "name": run.name,
                "checkSuite": {"app": null, "workflowRun": {"event": run.event, "workflow": {"id": run.workflow_id}}},
                "completedAt": run.conclusion.is_some().then_some(run.updated_at).flatten(),
                "conclusion": run.conclusion.map(|conclusion| conclusion.to_uppercase()),
                "startedAt": run.run_started_at,
            })
        })
        .chain(statuses.statuses.into_iter().map(|status| {
            serde_json::json!({
                "state": status.state.to_uppercase(),
                "createdAt": status.created_at,
            })
        }))
        .collect::<Vec<_>>();
    Ok(serde_json::json!({
        "state": state,
        "contexts": {"totalCount": runs_count + statuses.total_count, "nodes": nodes},
    }))
}

/// What GitHub answers REST's `path` with in `repo`, read as `T`.
fn rest<T: serde::de::DeserializeOwned>(
    runner: &dyn Runner,
    repo: &Repository,
    path: &str,
) -> Result<T, RunFailure> {
    let mut args = vec!["api", path];
    if let Some(host) = repo.host {
        args.extend(["--hostname", host]);
    }
    let out = runner.run("gh", &args, None, &Env::new())?;
    serde_json::from_str(&out).map_err(|e| RunFailure::parse("gh", e))
}

/// A release a repository has published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    pub published: DateTime<Utc>,
}

/// How many of a repository's releases are read, the newest made first.
// ponytail: a release published from a draft made before this many newer
// ones is not seen. Page through them if a repository ever works that way.
const RELEASES_READ: usize = 10;

/// One release as GraphQL lists it. A draft has no `publishedAt`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Listed {
    tag_name: String,
    is_draft: bool,
    published_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
struct Releases {
    nodes: Vec<Listed>,
}

/// The releases `repo` has published, among the [`RELEASES_READ`] it made
/// last. A draft is not published until it stops being one.
pub fn published_releases(
    runner: &dyn Runner,
    repo: &Repository,
) -> Result<Vec<Release>, RunFailure> {
    let answer = asked(
        runner,
        repo,
        &format!(
            "releases(first:{RELEASES_READ},orderBy:{{field:CREATED_AT,direction:DESC}})\
             {{nodes{{tagName isDraft publishedAt}}}}"
        ),
    )?;
    let releases: Releases = answer
        .get("releases")
        .map(|releases| serde_json::from_value(releases.clone()))
        .ok_or_else(|| RunFailure::parse("gh", "the answer holds no releases"))?
        .map_err(|e| RunFailure::parse("gh", e))?;
    Ok(releases
        .nodes
        .into_iter()
        .filter(|listed| !listed.is_draft)
        .filter_map(|listed| {
            Some(Release {
                tag: listed.tag_name,
                published: listed.published_at?,
            })
        })
        .collect())
}

/// `repo` taken apart, or a failure saying GitHub can have no such
/// repository.
pub fn repository(repo: &str) -> Result<Repository<'_>, RunFailure> {
    gate::repository(repo).ok_or_else(|| RunFailure {
        kind: FailureKind::Gone,
        program: "gh".to_string(),
        detail: format!(
            "{repo} is not OWNER/REPO or HOST/OWNER/REPO, so GitHub has no such repository"
        ),
        unreadable: None,
    })
}

/// What GitHub answers for `fields` of `repo`, which fails as a whole on any
/// error GraphQL gives.
fn asked(runner: &dyn Runner, repo: &Repository, fields: &str) -> Result<Value, RunFailure> {
    let args = graphql(repo, fields);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = runner.run("gh", &args, None, &Env::new())?;
    let answer: Value = serde_json::from_str(&out).map_err(|e| RunFailure::parse("gh", e))?;
    answer
        .pointer("/data/repository")
        .filter(|repository| repository.is_object())
        .cloned()
        .ok_or_else(|| RunFailure::parse("gh", "the answer names no repository"))
}

/// The arguments to `gh` asking GraphQL for `fields` of `repo`. The owner and
/// name go to `gh` as variables rather than into the query, since a gate's
/// writer chose them. `gh` picks the host for a `repo` that names none,
/// `GH_HOST` included, exactly as `bd gate check` has it pick.
fn graphql(repo: &Repository, fields: &str) -> Vec<String> {
    let mut args = vec!["api".to_string(), "graphql".to_string()];
    if let Some(host) = repo.host {
        args.extend(["--hostname".to_string(), host.to_string()]);
    }
    args.extend([
        "-f".to_string(),
        format!("owner={}", repo.owner),
        "-f".to_string(),
        format!("name={}", repo.name),
        "-f".to_string(),
        format!(
            "query=query($owner:String!,$name:String!){{repository(owner:$owner,name:$name){{{fields}}}}}"
        ),
    ]);
    args
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

    const FIELDS: &str = "isDraft mergeCommit{oid}";

    const QUERY: &str = "query=query($owner:String!,$name:String!){repository(owner:$owner,\
                         name:$name){pr7:pullRequest(number:7){state isDraft mergeCommit{oid}} \
                         pr42:pullRequest(number:42){state isDraft mergeCommit{oid}}}}";

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
    /// for a fine-grained token without *Commit statuses*: the data GitHub
    /// could give, the refused field null, and an error with its path.
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

    /// An answer as gh prints it before exiting 1, measured on gh 2.102.0
    /// for a fine-grained token with *Commit statuses* but no permission for
    /// check runs: the rollup's state and count, each of its nodes null, and
    /// an error for each.
    const CHECK_RUNS_REFUSED_ON_7: &str = r#"{"data":{"repository":{"pr7":{"state":"OPEN","isDraft":true,"commits":{"nodes":[{"commit":{"oid":"5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff","statusCheckRollup":{"state":"FAILURE","contexts":{"totalCount":2,"nodes":[null,null]}}}}]}},"pr42":{"state":"MERGED","isDraft":false,"mergeCommit":{"oid":"0badc0de0badc0de0badc0de0badc0de0badc0de"}}}},"errors":[{"type":"FORBIDDEN","path":["repository","pr7","commits","nodes",0,"commit","statusCheckRollup","contexts","nodes",0],"extensions":{"saml_failure":false,"ip_allow_list_failure":false},"locations":[{"line":1,"column":217}],"message":"Resource not accessible by personal access token"},{"type":"FORBIDDEN","path":["repository","pr7","commits","nodes",0,"commit","statusCheckRollup","contexts","nodes",1],"extensions":{"saml_failure":false,"ip_allow_list_failure":false},"locations":[{"line":1,"column":217}],"message":"Resource not accessible by personal access token"}]}"#;

    #[test]
    fn a_field_github_refused_in_several_places_is_handed_back_once() {
        let runner = FakeRunner::default().failing_having_printed(
            &format!("gh api graphql -f owner=example -f name=ark -f {QUERY}"),
            CHECK_RUNS_REFUSED_ON_7,
            exited(FailureKind::Unavailable),
        );

        let each = queried(&runner, None).expect("the answer is read for what it holds");

        assert_eq!(
            each[0].as_ref().expect("#7 read").refused,
            [Refusal {
                field: "commits".to_string(),
                why: "Resource not accessible by personal access token".to_string(),
            }]
        );
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

    const ISSUE_31: &str = "gh api graphql -f owner=example -f name=ark -f \
                            query=query($owner:String!,$name:String!){repository(owner:$owner,\
                            name:$name){issue(number:31){state}}}";

    fn issue_answer(state: &str) -> String {
        format!(r#"{{"data":{{"repository":{{"issue":{{"state":"{state}"}}}}}}}}"#)
    }

    #[test]
    fn an_issue_github_says_is_closed_is_closed_and_one_it_says_is_open_is_not() {
        for (state, closed) in [("CLOSED", true), ("OPEN", false)] {
            let runner = FakeRunner::default().with(ISSUE_31, &issue_answer(state));

            assert_eq!(issue_closed(&runner, &ark(None), 31), Ok(closed), "{state}");
        }
    }

    #[test]
    fn an_issue_is_asked_of_the_host_its_repository_is_on() {
        let on_forge = ISSUE_31.replace("graphql", "graphql --hostname forge.invalid");
        let runner = FakeRunner::default().with(&on_forge, &issue_answer("CLOSED"));

        assert_eq!(
            issue_closed(&runner, &ark(Some("forge.invalid")), 31),
            Ok(true)
        );
    }

    #[test]
    fn an_issue_in_a_state_bdi_does_not_know_is_a_parse_failure() {
        let runner = FakeRunner::default().with(ISSUE_31, &issue_answer("LOCKED"));

        let failure = issue_closed(&runner, &ark(None), 31).unwrap_err();
        assert_eq!(failure.kind, FailureKind::Parse);
    }

    const RELEASES: &str = "gh api graphql -f owner=example -f name=ark -f \
                            query=query($owner:String!,$name:String!){repository(owner:$owner,\
                            name:$name){releases(first:10,orderBy:{field:CREATED_AT,direction:\
                            DESC}){nodes{tagName isDraft publishedAt}}}}";

    /// v1.3.0 is a draft, v1.2.0-rc.1 a prerelease and v1.1.0 a release, as
    /// GitHub lists them, the newest made first.
    const LISTED_RELEASES: &str = r#"{"data":{"repository":{"releases":{"nodes":[
        {"tagName":"v1.3.0","isDraft":true,"publishedAt":null},
        {"tagName":"v1.2.0-rc.1","isDraft":false,"publishedAt":"2026-10-08T09:30:00Z"},
        {"tagName":"v1.1.0","isDraft":false,"publishedAt":"2026-09-30T14:00:00Z"}]}}}}"#;

    #[test]
    fn a_repository_has_published_every_release_it_lists_but_a_draft() {
        let runner = FakeRunner::default().with(RELEASES, LISTED_RELEASES);

        assert_eq!(
            published_releases(&runner, &ark(None)),
            Ok(vec![
                Release {
                    tag: "v1.2.0-rc.1".to_string(),
                    published: "2026-10-08T09:30:00Z".parse().unwrap(),
                },
                Release {
                    tag: "v1.1.0".to_string(),
                    published: "2026-09-30T14:00:00Z".parse().unwrap(),
                },
            ])
        );
    }

    #[test]
    fn an_answer_holding_no_releases_is_a_parse_failure() {
        let runner = FakeRunner::default().with(RELEASES, r#"{"data":{"repository":null}}"#);

        let failure = published_releases(&runner, &ark(None)).unwrap_err();
        assert_eq!(failure.kind, FailureKind::Parse);
    }

    const RUN_4242: &str = "gh api repos/example/ark/actions/runs/4242";

    /// Run 4242 of example/ark's CI, cut to a few of the fields GitHub
    /// answers with.
    const RAN: &str = r#"{"id":4242,"name":"CI","head_branch":"main","run_number":17,
        "event":"push","status":"completed","conclusion":"success","workflow_id":99}"#;

    #[test]
    fn a_run_is_read_for_its_workflow_where_it_stands_and_how_it_ended() {
        let runner = FakeRunner::default().with(RUN_4242, RAN);

        assert_eq!(
            workflow_run(&runner, &ark(None), 4242),
            Ok(Run {
                name: "CI".to_string(),
                status: "completed".to_string(),
                conclusion: Some("success".to_string()),
            })
        );
    }

    #[test]
    fn a_run_still_going_has_no_conclusion() {
        let going = r#"{"id":4242,"name":"CI","status":"in_progress","conclusion":null}"#;
        let runner = FakeRunner::default().with(RUN_4242, going);

        assert_eq!(
            workflow_run(&runner, &ark(None), 4242).map(|run| run.conclusion),
            Ok(None)
        );
    }

    #[test]
    fn a_run_is_asked_of_the_host_its_repository_is_on() {
        let on_forge = format!("{RUN_4242} --hostname forge.invalid");
        let runner = FakeRunner::default().with(&on_forge, RAN);

        assert!(workflow_run(&runner, &ark(Some("forge.invalid")), 4242).is_ok());
    }

    const RUNS_OF_SHA: &str = "gh api repos/example/ark/actions/runs?head_sha=5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff&per_page=100";
    const STATUSES_OF_SHA: &str =
        "gh api repos/example/ark/commits/5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff/status?per_page=100";

    /// `runs` as Actions lists a commit's workflow runs, cut to a few of the
    /// fields GitHub answers with.
    fn runs_listed(runs: &[&str]) -> String {
        format!(
            r#"{{"total_count":{},"workflow_runs":[{}]}}"#,
            runs.len(),
            runs.join(",")
        )
    }

    /// `statuses` as GitHub combines a commit's statuses, with the combined
    /// state GitHub gives a commit with none.
    fn statuses_combined(statuses: &[&str]) -> String {
        format!(
            r#"{{"state":"pending","total_count":{},"statuses":[{}]}}"#,
            statuses.len(),
            statuses.join(",")
        )
    }

    fn run(name: &str, conclusion: &str) -> String {
        format!(
            r#"{{"id":4242,"name":"{name}","workflow_id":99,"event":"push","status":"completed","conclusion":{conclusion},"run_started_at":"2026-10-06T08:00:00Z","updated_at":"2026-10-06T08:05:00Z"}}"#
        )
    }

    const RUNNING: &str = r#"{"id":4243,"name":"CI","workflow_id":99,"event":"pull_request","status":"in_progress","conclusion":null,"run_started_at":"2026-10-06T08:00:00Z","updated_at":"2026-10-06T08:01:00Z"}"#;

    fn status(state: &str) -> String {
        format!(r#"{{"state":"{state}","context":"lint","created_at":"2026-10-06T08:10:00Z"}}"#)
    }

    fn checks_of_sha(runs: &[&str], statuses: &[&str]) -> Result<Value, RunFailure> {
        let runner = FakeRunner::default()
            .with(RUNS_OF_SHA, &runs_listed(runs))
            .with(STATUSES_OF_SHA, &statuses_combined(statuses));
        checks_without_check_runs(&runner, &ark(None), SHA)
    }

    fn rollup_state(runs: &[&str], statuses: &[&str]) -> Value {
        checks_of_sha(runs, statuses).expect("both read")["state"].clone()
    }

    #[test]
    fn a_failed_run_or_status_fails_the_checks_whatever_else_is_still_going() {
        let (failed, error) = (run("CI", r#""failure""#), status("error"));
        let (passed, success) = (run("CI", r#""success""#), status("success"));

        assert_eq!(rollup_state(&[&failed, RUNNING], &[&success]), "FAILURE");
        assert_eq!(rollup_state(&[&passed, RUNNING], &[&error]), "FAILURE");
        assert_eq!(rollup_state(&[&passed], &[&status("failure")]), "FAILURE");
    }

    /// A workflow the commit triggered again for the same event is judged by
    /// its latest run alone, which replaced the earlier ones.
    #[test]
    fn a_workflow_run_again_for_the_same_event_is_judged_by_its_latest_run() {
        let again = |id: u64, event: &str, conclusion: &str| {
            format!(
                r#"{{"id":{id},"name":"CI","workflow_id":99,"event":"{event}","status":"completed","conclusion":"{conclusion}","run_started_at":"2026-10-06T08:0{id}:00Z","updated_at":"2026-10-06T08:0{id}:30Z"}}"#
            )
        };
        let (failed, passed) = (
            again(1, "pull_request", "failure"),
            again(2, "pull_request", "success"),
        );
        let checks = checks_of_sha(&[&passed, &failed], &[]).expect("both read");

        assert_eq!(checks["state"], "SUCCESS");
        assert_eq!(
            checks["contexts"]["nodes"].as_array().map(Vec::len),
            Some(1)
        );
        let pushed = again(3, "push", "failure");
        assert_eq!(rollup_state(&[&passed, &failed, &pushed], &[]), "FAILURE");
    }

    /// The runs and statuses past the first hundred are not read, so checks
    /// that may hide a failure there are never said to pass.
    #[test]
    fn checks_with_more_runs_or_statuses_than_were_read_do_not_pass() {
        let passed = run("CI", r#""success""#);
        let more_runs = FakeRunner::default()
            .with(
                RUNS_OF_SHA,
                &runs_listed(&[&passed]).replace(r#""total_count":1"#, r#""total_count":101"#),
            )
            .with(STATUSES_OF_SHA, &statuses_combined(&[]));
        let more_statuses = FakeRunner::default()
            .with(RUNS_OF_SHA, &runs_listed(&[&passed]))
            .with(
                STATUSES_OF_SHA,
                &statuses_combined(&[&status("success")])
                    .replace(r#""total_count":1"#, r#""total_count":101"#),
            );

        for runner in [more_runs, more_statuses] {
            let checks = checks_without_check_runs(&runner, &ark(None), SHA).expect("both read");
            assert_eq!(checks["state"], "PENDING");
        }
    }

    #[test]
    fn checks_with_a_run_or_status_still_going_and_none_failed_are_pending() {
        let passed = run("CI", r#""success""#);

        assert_eq!(rollup_state(&[&passed, RUNNING], &[]), "PENDING");
        assert_eq!(rollup_state(&[&passed], &[&status("pending")]), "PENDING");
    }

    #[test]
    fn checks_whose_every_run_and_status_finished_well_pass() {
        let runs = [
            run("CI", r#""success""#),
            run("Docs", r#""skipped""#).replace(r#""workflow_id":99"#, r#""workflow_id":98"#),
            run("Lint", r#""neutral""#).replace(r#""workflow_id":99"#, r#""workflow_id":97"#),
        ];
        let runs: Vec<&str> = runs.iter().map(String::as_str).collect();

        assert_eq!(rollup_state(&runs, &[&status("success")]), "SUCCESS");
        assert_eq!(rollup_state(&[], &[&status("success")]), "SUCCESS");
    }

    /// As GraphQL has a commit with no checks, where GitHub combines no
    /// statuses as pending.
    #[test]
    fn a_commit_with_no_runs_and_no_statuses_has_no_checks() {
        assert_eq!(checks_of_sha(&[], &[]), Ok(Value::Null));
    }

    /// Each run and status as GraphQL gives a check run and a commit status,
    /// so the events reading checks read them alike.
    #[test]
    fn runs_and_statuses_are_given_as_graphql_gives_check_runs_and_statuses() {
        let checks = checks_of_sha(
            &[&run("CI", r#""cancelled""#), RUNNING],
            &[&status("success")],
        )
        .expect("both read");

        assert_eq!(
            checks["contexts"],
            serde_json::json!({
                "totalCount": 3,
                "nodes": [
                    {
                        "name": "CI",
                        "checkSuite": {"app": null, "workflowRun": {"event": "push", "workflow": {"id": 99}}},
                        "conclusion": "CANCELLED",
                        "startedAt": "2026-10-06T08:00:00Z",
                        "completedAt": "2026-10-06T08:05:00Z"
                    },
                    {
                        "name": "CI",
                        "checkSuite": {"app": null, "workflowRun": {"event": "pull_request", "workflow": {"id": 99}}},
                        "conclusion": null,
                        "startedAt": "2026-10-06T08:00:00Z",
                        "completedAt": null
                    },
                    {"state": "SUCCESS", "createdAt": "2026-10-06T08:10:00Z"}
                ]
            })
        );
    }

    #[test]
    fn a_commits_runs_and_statuses_are_asked_of_the_host_its_repository_is_on() {
        let on_forge = |argv: &str| format!("{argv} --hostname forge.invalid");
        let runner = FakeRunner::default()
            .with(&on_forge(RUNS_OF_SHA), &runs_listed(&[]))
            .with(&on_forge(STATUSES_OF_SHA), &statuses_combined(&[]));

        assert!(checks_without_check_runs(&runner, &ark(Some("forge.invalid")), SHA).is_ok());
    }

    #[test]
    fn a_commit_whose_runs_or_statuses_github_refuses_fails_as_gh_said() {
        let refused = || exited(FailureKind::Unavailable);
        let runs_refused = FakeRunner::default()
            .failing(RUNS_OF_SHA, refused())
            .with(STATUSES_OF_SHA, &statuses_combined(&[]));
        let statuses_refused = FakeRunner::default()
            .with(RUNS_OF_SHA, &runs_listed(&[]))
            .failing(STATUSES_OF_SHA, refused());

        for runner in [runs_refused, statuses_refused] {
            assert_eq!(
                checks_without_check_runs(&runner, &ark(None), SHA),
                Err(refused())
            );
        }
    }

    #[test]
    fn an_answer_that_is_not_a_run_is_a_parse_failure() {
        let runner = FakeRunner::default().with(RUN_4242, r#"{"message":"Moved"}"#);

        let failure = workflow_run(&runner, &ark(None), 4242).unwrap_err();
        assert_eq!(failure.kind, FailureKind::Parse);
    }
}
