//! A pull request whose head commit passed its checks, which the beads its
//! gates hold back are told.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::{Event, Outcome, Telling, HEAD_CHECKS};
use crate::collect::gates::PullRequest;
use crate::collect::github::{Observed, State};

pub const EVENT: Event = Event {
    fields: &[HEAD_CHECKS],
    happening: "has passing checks",
    optional: &[],
    outcome: checks_passed,
};

fn checks_passed(
    pr: &PullRequest,
    observed: &Observed,
) -> Result<Option<Outcome>, serde_json::Error> {
    #[derive(Deserialize)]
    struct Fields {
        #[serde(default)]
        commits: Commits,
    }
    #[derive(Deserialize, Default)]
    struct Commits {
        nodes: Vec<Node>,
    }
    #[derive(Deserialize)]
    struct Node {
        commit: Commit,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Commit {
        oid: String,
        status_check_rollup: Option<Rollup>,
    }
    #[derive(Deserialize)]
    struct Rollup {
        state: String,
        #[serde(default)]
        contexts: Contexts,
    }
    #[derive(Deserialize, Default)]
    struct Contexts {
        nodes: Vec<Context>,
    }
    /// A check run, which completes, or a commit status, which is created
    /// finished.
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Context {
        completed_at: Option<DateTime<Utc>>,
        created_at: Option<DateTime<Utc>>,
    }
    let Fields { commits } = Fields::deserialize(&observed.fields)?;
    let passed = commits.nodes.into_iter().find_map(|Node { commit }| {
        let rollup = commit.status_check_rollup?;
        (rollup.state == "SUCCESS").then(|| {
            let last_finished = rollup
                .contexts
                .nodes
                .into_iter()
                .filter_map(|context| context.completed_at.or(context.created_at))
                .max();
            (commit.oid, last_finished)
        })
    });
    Ok(passed
        .filter(|_| observed.state == State::Open)
        .map(|(oid, happened)| {
            Outcome::Tell(vec![Telling {
                text: told(&oid, pr),
                happened,
            }])
        }))
}

/// How passing checks on the head `oid` are told.
fn told(oid: &str, pr: &PullRequest) -> String {
    format!("The checks on {oid}, the head of pull request {pr}, passed.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::pr_events::testing::{once, tellings, when};

    fn told_on(state: State, fields: &str) -> Option<String> {
        once(tellings(checks_passed, state, fields)).map(|telling| telling.text)
    }

    fn head_commit(oid: &str, rollup: &str) -> String {
        format!(
            r#"{{"commits":{{"nodes":[{{"commit":{{"oid":"{oid}","statusCheckRollup":{rollup}}}}}]}}}}"#
        )
    }

    const PASSED: &str = r#"{"state":"SUCCESS"}"#;

    #[test]
    fn an_open_pull_request_whose_head_commit_passed_its_checks_is_told_by_that_commit() {
        let told = |oid| told_on(State::Open, &head_commit(oid, PASSED)).expect("a pass is told");

        assert!(told("a1b2c3").contains("a1b2c3"));
        assert_ne!(told("a1b2c3"), told("d4e5f6"));
    }

    #[test]
    fn a_draft_whose_head_commit_passed_its_checks_is_told() {
        let draft = head_commit("a1b2c3", PASSED).replacen('{', r#"{"isDraft":true,"#, 1);
        assert!(told_on(State::Open, &draft).is_some());
    }

    #[test]
    fn a_head_commit_whose_checks_have_not_all_passed_is_not_told() {
        for rollup in [
            "null",
            r#"{"state":"FAILURE"}"#,
            r#"{"state":"ERROR"}"#,
            r#"{"state":"PENDING"}"#,
            r#"{"state":"EXPECTED"}"#,
        ] {
            assert_eq!(
                told_on(State::Open, &head_commit("a1b2c3", rollup)),
                None,
                "{rollup}"
            );
        }
        assert_eq!(told_on(State::Open, r#"{"commits":{"nodes":[]}}"#), None);
        assert_eq!(told_on(State::Open, "{}"), None);
    }

    #[test]
    fn a_pull_request_that_is_no_longer_open_is_not_told_of_passed_checks() {
        let passed = head_commit("a1b2c3", PASSED);
        assert_eq!(told_on(State::Closed, &passed), None);
        assert_eq!(told_on(State::Merged, &passed), None);
    }

    /// When the checks on a head commit whose checks are `contexts`, a JSON
    /// array, passed.
    fn checks_passed_at(contexts: &str) -> Option<DateTime<Utc>> {
        let rollup = format!(r#"{{"state":"SUCCESS","contexts":{{"nodes":{contexts}}}}}"#);
        once(tellings(
            checks_passed,
            State::Open,
            &head_commit("a1b2c3", &rollup),
        ))
        .expect("a pass is told")
        .happened
    }

    /// The checks pass as the last of them finishes, whether a check run or a
    /// commit status.
    #[test]
    fn passed_checks_happened_when_the_last_check_finished() {
        let contexts = r#"[
            {"conclusion":"SUCCESS","completedAt":"2026-10-06T08:00:00Z"},
            {"conclusion":"SKIPPED","completedAt":"2026-10-06T08:30:00Z"},
            {"state":"SUCCESS","createdAt":"2026-10-06T08:20:00Z"}
        ]"#;
        let statuses = r#"[
            {"conclusion":"NEUTRAL","completedAt":"2026-10-06T08:10:00Z"},
            {"state":"SUCCESS","createdAt":"2026-10-06T08:40:00Z"}
        ]"#;

        assert_eq!(checks_passed_at(contexts), when("2026-10-06T08:30:00Z"));
        assert_eq!(checks_passed_at(statuses), when("2026-10-06T08:40:00Z"));
    }

    #[test]
    fn passed_checks_with_no_check_listed_happened_at_no_known_time() {
        assert_eq!(checks_passed_at("[]"), None);
    }
}
