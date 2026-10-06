//! What GitHub says of a pull request now, read through `gh`.

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::run::testing::FakeRunner;
    use crate::collect::run::FailureKind;

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
