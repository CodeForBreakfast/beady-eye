//! A pull request whose head commit failed its checks, which the beads its
//! gates hold back are told.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::{Event, Outcome, Telling, HEAD_CHECKS};
use crate::collect::gates::PullRequest;
use crate::collect::github::{Observed, State};

pub const EVENT: Event = Event {
    fields: &[HEAD_CHECKS],
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
    #[serde(rename_all = "camelCase")]
    struct Contexts {
        #[serde(default)]
        total_count: usize,
        nodes: Vec<Context>,
    }
    /// The app and workflow a check run belongs to, which with its name say
    /// which check it is.
    #[derive(Deserialize, PartialEq)]
    #[serde(rename_all = "camelCase")]
    struct CheckSuite {
        app: Option<serde_json::Value>,
        workflow_run: Option<serde_json::Value>,
    }
    /// A check run, with a conclusion once it completes, or a commit status,
    /// with a state.
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Context {
        name: Option<String>,
        check_suite: Option<CheckSuite>,
        conclusion: Option<String>,
        started_at: Option<DateTime<Utc>>,
        completed_at: Option<DateTime<Utc>>,
        state: Option<String>,
        created_at: Option<DateTime<Utc>>,
    }
    impl Context {
        /// Whether a later run of the same check has replaced this one, which
        /// was cancelled.
        fn superseded_in(&self, contexts: &[Context]) -> bool {
            self.conclusion.as_deref() == Some("CANCELLED")
                && contexts.iter().any(|other| {
                    other.name.is_some()
                        && other.name == self.name
                        && other.check_suite == self.check_suite
                        && other.started_at > self.started_at
                })
        }

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
            let complete = rollup.contexts.total_count <= rollup.contexts.nodes.len();
            let contexts = rollup.contexts.nodes;
            let replaced: Vec<bool> = contexts
                .iter()
                .map(|context| context.superseded_in(&contexts))
                .collect();
            let replaced_in_full = replaced.contains(&true);
            let first_failure = contexts
                .into_iter()
                .zip(replaced)
                .filter(|(_, replaced)| !replaced)
                .filter_map(|(context, _)| context.failed())
                .min();
            (commit.oid, first_failure, replaced_in_full && complete)
        })
    });
    Ok(failed
        .filter(|_| observed.state == State::Open)
        .filter(|(_, first_failure, replaced_in_full)| first_failure.is_some() || !replaced_in_full)
        .map(|(oid, happened, _)| {
            Outcome::Tell(vec![Telling {
                text: told(&oid, pr),
                happened,
            }])
        }))
}

/// How failing checks on the head `oid` are told, by the sweep and by a
/// delivery alike.
pub(super) fn told(oid: &str, pr: &PullRequest) -> String {
    format!(
        "The checks on {oid}, the head of pull request {pr}, failed, so the gh:pr gate waiting \
         on it stays open."
    )
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

    /// A push or a title edit cancels the runs under way, and the re-run
    /// that follows says how the checks went.
    #[test]
    fn a_cancelled_run_with_a_later_re_run_is_not_a_failure() {
        let rollup =
            |contexts: &str| format!(r#"{{"state":"FAILURE","contexts":{{"nodes":{contexts}}}}}"#);
        let told = |contexts: &str| told_on(State::Open, &head_commit("a1b2c3", &rollup(contexts)));
        let cancelled = r#"{"name":"test","conclusion":"CANCELLED","startedAt":"2026-10-06T08:00:00Z","completedAt":"2026-10-06T08:01:00Z"}"#;
        let re_run = |conclusion: &str| {
            format!(
                r#"{{"name":"test","conclusion":{conclusion},"startedAt":"2026-10-06T08:02:00Z","completedAt":"2026-10-06T08:03:00Z"}}"#
            )
        };

        for conclusion in ["\"SUCCESS\"", "null"] {
            assert_eq!(
                told(&format!("[{cancelled},{}]", re_run(conclusion))),
                None,
                "{conclusion}"
            );
        }
        assert!(told(&format!("[{cancelled}]")).is_some(), "no re-run");
        assert!(
            told(&format!(
                r#"[{cancelled},{{"name":"lint","conclusion":"SUCCESS","startedAt":"2026-10-06T08:02:00Z"}}]"#
            ))
            .is_some(),
            "a re-run of another check"
        );
        let other_workflow = r#"{"name":"test","checkSuite":{"app":null,"workflowRun":{"workflow":{"id":"W_other"}}},"conclusion":"SUCCESS","startedAt":"2026-10-06T08:02:00Z","completedAt":"2026-10-06T08:03:00Z"}"#;
        assert!(
            told(&format!("[{cancelled},{other_workflow}]")).is_some(),
            "a re-run in another workflow"
        );
        let truncated = format!(
            r#"{{"state":"FAILURE","contexts":{{"totalCount":101,"nodes":[{cancelled},{}]}}}}"#,
            re_run("\"SUCCESS\"")
        );
        assert!(
            told_on(State::Open, &head_commit("a1b2c3", &truncated)).is_some(),
            "more checks than were listed"
        );
        assert!(
            told(&format!("[{},{cancelled}]", re_run("\"FAILURE\""))).is_some(),
            "a failed re-run"
        );
    }

    /// GitHub lists the latest hundred checks, so the one that failed may not
    /// be among them.
    #[test]
    fn failed_checks_with_no_failing_check_listed_happened_at_no_known_time() {
        let passing = r#"[{"conclusion":"SUCCESS","completedAt":"2026-10-06T08:00:00Z"}]"#;
        assert_eq!(checks_failed_at(passing), None);
    }
}
