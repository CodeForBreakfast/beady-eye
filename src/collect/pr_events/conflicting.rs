//! A pull request whose head conflicts with its base, which the beads its
//! gates hold back are told.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::{Event, Outcome, Telling, HEAD_COMMIT};
use crate::collect::gates::PullRequest;
use crate::collect::github::{Observed, State};

pub const EVENT: Event = Event {
    fields: &[
        "mergeable",
        "headRefOid",
        HEAD_COMMIT,
        "baseRef{target{...on Commit{committedDate}}}",
    ],
    happening: "conflicts with its base",
    outcome: conflicting,
};

fn conflicting(
    pr: &PullRequest,
    observed: &Observed,
) -> Result<Option<Outcome>, serde_json::Error> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Fields {
        mergeable: Option<String>,
        head_ref_oid: Option<String>,
        head_ref: Option<Ref>,
        base_ref: Option<Ref>,
    }
    #[derive(Deserialize)]
    struct Ref {
        target: Target,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Target {
        committed_date: Option<DateTime<Utc>>,
    }
    let committed = |branch: Option<Ref>| branch.and_then(|branch| branch.target.committed_date);
    let Fields {
        mergeable,
        head_ref_oid,
        head_ref,
        base_ref,
    } = Fields::deserialize(&observed.fields)?;
    // GitHub says nothing of when a conflict began. The two commits it is
    // between have both stood since the later of them was made, and the
    // conflict with them, so that is when it happened. It may have stood
    // longer against an older base, which tells a gate made in between.
    let happened = committed(head_ref)
        .zip(committed(base_ref))
        .map(|(head, base)| head.max(base));
    Ok(match (observed.state, mergeable.as_deref(), head_ref_oid) {
        (State::Open, Some("CONFLICTING"), Some(oid)) => Some(Outcome::Tell(vec![Telling {
            text: format!(
                "The head of pull request {pr}, {oid}, conflicts with its base, so the gh:pr \
                 gate waiting on it stays open."
            ),
            happened,
        }])),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::pr_events::testing::{once, tellings, when};

    fn conflict_told_on(state: State, fields: &str) -> Option<String> {
        once(tellings(conflicting, state, fields)).map(|telling| telling.text)
    }

    fn mergeable(mergeable: &str, oid: &str) -> String {
        format!(r#"{{"mergeable":"{mergeable}","headRefOid":"{oid}"}}"#)
    }

    #[test]
    fn an_open_pull_request_whose_head_commit_conflicts_is_told_by_that_commit() {
        let told = |oid| {
            conflict_told_on(State::Open, &mergeable("CONFLICTING", oid))
                .expect("a conflict is told")
        };

        assert!(told("a1b2c3").contains("a1b2c3"));
        assert_ne!(told("a1b2c3"), told("d4e5f6"));
    }

    /// A seat that opens its pull request as a draft waits to hear it
    /// conflicts as much as one that opens it ready.
    #[test]
    fn a_draft_whose_head_commit_conflicts_is_told() {
        let draft = mergeable("CONFLICTING", "a1b2c3").replacen('{', r#"{"isDraft":true,"#, 1);
        assert!(conflict_told_on(State::Open, &draft).is_some());
    }

    /// GitHub works mergeability out in the background, so a pull request it
    /// has not got to yet says UNKNOWN, which is not a conflict.
    #[test]
    fn a_head_commit_that_merges_cleanly_or_is_not_yet_known_to_conflict_is_not_told() {
        for answer in ["MERGEABLE", "UNKNOWN"] {
            assert_eq!(
                conflict_told_on(State::Open, &mergeable(answer, "a1b2c3")),
                None,
                "{answer}"
            );
        }
    }

    #[test]
    fn a_pull_request_that_is_no_longer_open_is_not_told_of_a_conflict() {
        let conflicting = mergeable("CONFLICTING", "a1b2c3");
        assert_eq!(conflict_told_on(State::Closed, &conflicting), None);
        assert_eq!(conflict_told_on(State::Merged, &conflicting), None);
    }

    /// When a conflict between a head commit committed at `head` and a base
    /// committed at `base`, each a JSON value, happened.
    fn conflicted_at(head: &str, base: &str) -> Option<DateTime<Utc>> {
        let fields = format!(
            r#"{{"mergeable":"CONFLICTING","headRefOid":"a1b2c3","headRef":{head},"baseRef":{base}}}"#
        );
        once(tellings(conflicting, State::Open, &fields))
            .expect("a conflict is told")
            .happened
    }

    fn committed(at: &str) -> String {
        format!(r#"{{"target":{{"committedDate":"{at}"}}}}"#)
    }

    #[test]
    fn a_conflict_happened_when_the_later_of_its_two_commits_was_made() {
        let earlier = committed("2026-10-06T08:00:00Z");
        let later = committed("2026-10-06T08:30:00Z");

        assert_eq!(
            conflicted_at(&earlier, &later),
            when("2026-10-06T08:30:00Z")
        );
        assert_eq!(
            conflicted_at(&later, &earlier),
            when("2026-10-06T08:30:00Z")
        );
    }

    #[test]
    fn a_conflict_with_a_branch_that_has_gone_happened_at_no_known_time() {
        let base = committed("2026-10-06T08:00:00Z");
        assert_eq!(conflicted_at("null", &base), None);
    }
}
