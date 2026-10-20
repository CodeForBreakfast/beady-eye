//! A pull request whose head commit failed its checks, which the beads its
//! gates hold back are told.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::{Event, Outcome, Telling};
use crate::collect::gates::PullRequest;
use crate::collect::github::{Observed, State};

pub const EVENT: Event = Event {
    fields: &["commits(last:1){nodes{commit{oid statusCheckRollup{state contexts(last:100){nodes{...on CheckRun{conclusion completedAt} ...on StatusContext{state createdAt}}}}}}}"],
    happening: "has failing checks",
    outcome: checks_failed,
};

fn checks_failed(
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
    /// A check run, with a conclusion once it completes, or a commit status,
    /// with a state.
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Context {
        conclusion: Option<String>,
        completed_at: Option<DateTime<Utc>>,
        state: Option<String>,
        created_at: Option<DateTime<Utc>>,
    }
    impl Context {
        /// When this failed, or nothing where it has not.
        fn failed(self) -> Option<DateTime<Utc>> {
            let failed = match (self.conclusion, self.state) {
                (Some(conclusion), _) => {
                    !matches!(conclusion.as_str(), "SUCCESS" | "NEUTRAL" | "SKIPPED")
                }
                (None, Some(state)) => matches!(state.as_str(), "FAILURE" | "ERROR"),
                (None, None) => false,
            };
            failed
                .then_some(self.completed_at.or(self.created_at))
                .flatten()
        }
    }
    let Fields { commits } = Fields::deserialize(&observed.fields)?;
    let failed = commits.nodes.into_iter().find_map(|Node { commit }| {
        let rollup = commit.status_check_rollup?;
        matches!(rollup.state.as_str(), "FAILURE" | "ERROR").then(|| {
            let first_failure = rollup
                .contexts
                .nodes
                .into_iter()
                .filter_map(Context::failed)
                .min();
            (commit.oid, first_failure)
        })
    });
    Ok(failed
        .filter(|_| observed.state == State::Open)
        .map(|(oid, happened)| {
            Outcome::Tell(vec![Telling {
                text: format!(
                    "The checks on {oid}, the head of pull request {pr}, failed, so the gh:pr \
                     gate waiting on it stays open."
                ),
                happened,
            }])
        }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::pr_events::testing::{once, tellings, when};

    fn told_on(state: State, fields: &str) -> Option<String> {
        once(tellings(checks_failed, state, fields)).map(|telling| telling.text)
    }

    fn head_commit(oid: &str, rollup: &str) -> String {
        format!(
            r#"{{"commits":{{"nodes":[{{"commit":{{"oid":"{oid}","statusCheckRollup":{rollup}}}}}]}}}}"#
        )
    }

    #[test]
    fn an_open_pull_request_whose_head_commit_failed_its_checks_is_told_by_that_commit() {
        for state in ["FAILURE", "ERROR"] {
            let rollup = format!(r#"{{"state":"{state}"}}"#);
            let told =
                |oid| told_on(State::Open, &head_commit(oid, &rollup)).expect("a failure is told");

            assert!(told("a1b2c3").contains("a1b2c3"), "{state}");
            assert_ne!(told("a1b2c3"), told("d4e5f6"), "{state}");
        }
    }

    /// A seat that opens its pull request as a draft waits to hear its
    /// checks fail as much as one that opens it ready.
    #[test]
    fn a_draft_whose_head_commit_failed_its_checks_is_told() {
        let draft =
            head_commit("a1b2c3", r#"{"state":"FAILURE"}"#).replacen('{', r#"{"isDraft":true,"#, 1);
        assert!(told_on(State::Open, &draft).is_some());
    }

    #[test]
    fn a_head_commit_whose_checks_have_not_failed_is_not_told() {
        for rollup in [
            "null",
            r#"{"state":"SUCCESS"}"#,
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
    }

    #[test]
    fn a_pull_request_that_is_no_longer_open_is_not_told_of_failed_checks() {
        let failed = head_commit("a1b2c3", r#"{"state":"FAILURE"}"#);
        assert_eq!(told_on(State::Closed, &failed), None);
        assert_eq!(told_on(State::Merged, &failed), None);
    }

    /// When the checks on a head commit whose checks are `contexts`, a JSON
    /// array, failed.
    fn checks_failed_at(contexts: &str) -> Option<DateTime<Utc>> {
        let rollup = format!(r#"{{"state":"FAILURE","contexts":{{"nodes":{contexts}}}}}"#);
        once(tellings(
            checks_failed,
            State::Open,
            &head_commit("a1b2c3", &rollup),
        ))
        .expect("a failure is told")
        .happened
    }

    /// The checks fail as the first check to fail finishes. A check still
    /// running, or one that passed, says nothing of when.
    #[test]
    fn failed_checks_happened_when_the_first_check_to_fail_finished() {
        let check_runs = r#"[
            {"conclusion":"SUCCESS","completedAt":"2026-10-06T08:00:00Z"},
            {"conclusion":"NEUTRAL","completedAt":"2026-10-06T08:05:00Z"},
            {"conclusion":"SKIPPED","completedAt":"2026-10-06T08:06:00Z"},
            {"conclusion":null,"completedAt":null},
            {"conclusion":"FAILURE","completedAt":"2026-10-06T08:30:00Z"},
            {"conclusion":"CANCELLED","completedAt":"2026-10-06T08:20:00Z"},
            {"state":"ERROR","createdAt":"2026-10-06T08:25:00Z"}
        ]"#;
        let statuses = r#"[
            {"state":"SUCCESS","createdAt":"2026-10-06T08:10:00Z"},
            {"state":"PENDING","createdAt":"2026-10-06T08:11:00Z"},
            {"state":"FAILURE","createdAt":"2026-10-06T08:25:00Z"}
        ]"#;

        assert_eq!(checks_failed_at(check_runs), when("2026-10-06T08:20:00Z"));
        assert_eq!(checks_failed_at(statuses), when("2026-10-06T08:25:00Z"));
    }

    /// GitHub lists the latest hundred checks, so the one that failed may not
    /// be among them.
    #[test]
    fn failed_checks_with_no_failing_check_listed_happened_at_no_known_time() {
        let passing = r#"[{"conclusion":"SUCCESS","completedAt":"2026-10-06T08:00:00Z"}]"#;
        assert_eq!(checks_failed_at(passing), None);
    }
}
