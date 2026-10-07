//! Each thing a pull request can do that `bdi gates` acts on: the fields of
//! GraphQL's `PullRequest` it reads, and what it makes of them.
//!
//! An event either closes the gh:pr gates waiting for it, or tells the beads
//! they hold back what happened, once for each thing it has to say. A new
//! event is a new entry in [`EVENTS`], and a gate waiting for something new
//! is a new [`Until`] beside one.

use serde::Deserialize;

use crate::collect::gates::PullRequest;
use crate::collect::github::{Observed, State};
use crate::model::gate::Until;

/// One thing a pull request can do.
#[derive(Clone, Copy)]
pub struct Event {
    /// The fields of GraphQL's `PullRequest` this event reads, beside the
    /// `state` every event is given.
    pub fields: &'static str,
    /// What the pull request did, as a line reporting it says.
    pub happening: &'static str,
    /// What this event makes of the pull request, or nothing where the pull
    /// request has not done it.
    pub outcome: fn(&PullRequest, &Observed) -> Result<Option<Outcome>, serde_json::Error>,
}

/// What an event asks of the gates waiting on its pull request.
#[derive(Debug)]
pub enum Outcome {
    /// Close each gate whose wait `awaited` accepts, giving `reason`.
    Resolve {
        awaited: fn(Until) -> bool,
        reason: String,
    },
    /// Comment each of these on each bead a gate holds back, once. Two
    /// tellings with one text are one telling, so the text carries whatever
    /// tells this one from the next.
    Tell(Vec<String>),
}

/// Every event `bdi gates` acts on.
pub const EVENTS: [Event; 7] = [
    Event {
        fields: "isDraft",
        happening: "is ready for review",
        outcome: ready_for_review,
    },
    Event {
        fields: "mergeCommit{oid}",
        happening: "merged",
        outcome: merged,
    },
    Event {
        fields: "",
        happening: "closed unmerged",
        outcome: closed_unmerged,
    },
    Event {
        fields: "reviewDecision",
        happening: "is approved",
        outcome: approved,
    },
    Event {
        fields: "commits(last:1){nodes{commit{oid statusCheckRollup{state}}}}",
        happening: "has failing checks",
        outcome: checks_failed,
    },
    Event {
        fields: "reviews(last:5){nodes{url state author{login}}}",
        happening: "was reviewed",
        outcome: reviewed,
    },
    Event {
        fields: "mergeable headRefOid",
        happening: "conflicts with its base",
        outcome: conflicting,
    },
];

/// The fields of GraphQL's `PullRequest` that `events` read between them.
pub fn fields(events: &[Event]) -> String {
    events
        .iter()
        .map(|event| event.fields)
        .filter(|fields| !fields.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

impl Outcome {
    /// Whether this asks anything of a gate waiting for `until`.
    pub fn concerns(&self, until: Until) -> bool {
        match self {
            Outcome::Resolve { awaited, .. } => awaited(until),
            Outcome::Tell(_) => true,
        }
    }
}

fn ready_for_review(
    pr: &PullRequest,
    observed: &Observed,
) -> Result<Option<Outcome>, serde_json::Error> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Fields {
        is_draft: bool,
    }
    let Fields { is_draft } = Fields::deserialize(&observed.fields)?;
    Ok(
        (observed.state == State::Open && !is_draft).then(|| Outcome::Resolve {
            awaited: |until| until == Until::ReadyForReview,
            reason: format!("Pull request {pr} is ready for review."),
        }),
    )
}

fn approved(pr: &PullRequest, observed: &Observed) -> Result<Option<Outcome>, serde_json::Error> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Fields {
        review_decision: Option<String>,
    }
    let Fields { review_decision } = Fields::deserialize(&observed.fields)?;
    Ok(
        (observed.state == State::Open && review_decision.as_deref() == Some("APPROVED")).then(
            || Outcome::Resolve {
                awaited: |until| until == Until::Approved,
                reason: format!("Pull request {pr} is approved."),
            },
        ),
    )
}

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
    }
    let Fields { commits } = Fields::deserialize(&observed.fields)?;
    let failed = commits.nodes.into_iter().find_map(|Node { commit }| {
        let rollup = commit.status_check_rollup?;
        matches!(rollup.state.as_str(), "FAILURE" | "ERROR").then_some(commit.oid)
    });
    Ok(failed.filter(|_| observed.state == State::Open).map(|oid| {
        Outcome::Tell(vec![format!(
            "The checks on {oid}, the head of pull request {pr}, failed, so the gh:pr gate \
                 waiting on it stays open."
        )])
    }))
}

fn reviewed(pr: &PullRequest, observed: &Observed) -> Result<Option<Outcome>, serde_json::Error> {
    #[derive(Deserialize)]
    struct Fields {
        #[serde(default)]
        reviews: Reviews,
    }
    #[derive(Deserialize, Default)]
    struct Reviews {
        nodes: Vec<Review>,
    }
    #[derive(Deserialize)]
    struct Review {
        url: String,
        state: String,
        author: Option<Author>,
    }
    #[derive(Deserialize)]
    struct Author {
        login: String,
    }
    let Fields { reviews } = Fields::deserialize(&observed.fields)?;
    let told: Vec<String> = reviews
        .nodes
        .into_iter()
        .filter(|review| !matches!(review.state.as_str(), "PENDING" | "DISMISSED"))
        .map(|Review { url, state, author }| {
            let reviewer = author.map_or("ghost".to_string(), |author| author.login);
            let state = state.to_ascii_lowercase().replace('_', " ");
            format!("{reviewer} reviewed pull request {pr}: {state}. {url}")
        })
        .collect();
    Ok((observed.state == State::Open && !told.is_empty()).then_some(Outcome::Tell(told)))
}

fn conflicting(
    pr: &PullRequest,
    observed: &Observed,
) -> Result<Option<Outcome>, serde_json::Error> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Fields {
        mergeable: Option<String>,
        head_ref_oid: Option<String>,
    }
    let Fields {
        mergeable,
        head_ref_oid,
    } = Fields::deserialize(&observed.fields)?;
    Ok(match (observed.state, mergeable.as_deref(), head_ref_oid) {
        (State::Open, Some("CONFLICTING"), Some(oid)) => Some(Outcome::Tell(vec![format!(
            "The head of pull request {pr}, {oid}, conflicts with its base, so the gh:pr gate \
             waiting on it stays open."
        )])),
        _ => None,
    })
}

fn merged(pr: &PullRequest, observed: &Observed) -> Result<Option<Outcome>, serde_json::Error> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Fields {
        merge_commit: Option<MergeCommit>,
    }
    #[derive(Deserialize)]
    struct MergeCommit {
        oid: String,
    }
    let Fields { merge_commit } = Fields::deserialize(&observed.fields)?;
    Ok((observed.state == State::Merged).then(|| Outcome::Resolve {
        awaited: |_| true,
        reason: match merge_commit {
            Some(commit) => format!("Pull request {pr} merged as {}.", commit.oid),
            None => format!("Pull request {pr} merged."),
        },
    }))
}

fn closed_unmerged(
    pr: &PullRequest,
    observed: &Observed,
) -> Result<Option<Outcome>, serde_json::Error> {
    Ok((observed.state == State::Closed).then(|| {
        Outcome::Tell(vec![format!(
            "Pull request {pr} closed without being merged, so the gh:pr gate waiting on it \
             stays open."
        )])
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approved_on(state: State, fields: &str) -> bool {
        let pr = PullRequest {
            repo: "example/ark".to_string(),
            number: 7,
        };
        let observed = Observed {
            state,
            fields: serde_json::from_str(fields).expect("the fields parse"),
        };
        approved(&pr, &observed).expect("the fields read").is_some()
    }

    #[test]
    fn only_an_open_pull_request_with_an_approving_review_decision_is_approved() {
        let approving = r#"{"reviewDecision":"APPROVED"}"#;
        assert!(approved_on(State::Open, approving));
        assert!(!approved_on(State::Closed, approving));
        assert!(!approved_on(State::Merged, approving));
        for decision in ["null", r#""REVIEW_REQUIRED""#, r#""CHANGES_REQUESTED""#] {
            let fields = format!(r#"{{"reviewDecision":{decision}}}"#);
            assert!(!approved_on(State::Open, &fields), "{decision}");
        }
    }

    fn told_on(state: State, fields: &str) -> Option<String> {
        let pr = PullRequest {
            repo: "example/ark".to_string(),
            number: 7,
        };
        let observed = Observed {
            state,
            fields: serde_json::from_str(fields).expect("the fields parse"),
        };
        match checks_failed(&pr, &observed).expect("the fields read") {
            Some(Outcome::Tell(mut texts)) if texts.len() == 1 => texts.pop(),
            Some(other) => panic!("a failure of checks tells once, not {other:?}"),
            None => None,
        }
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

    fn reviewed_on(state: State, fields: &str) -> Vec<String> {
        let pr = PullRequest {
            repo: "example/ark".to_string(),
            number: 7,
        };
        let observed = Observed {
            state,
            fields: serde_json::from_str(fields).expect("the fields parse"),
        };
        match reviewed(&pr, &observed).expect("the fields read") {
            Some(Outcome::Tell(texts)) => texts,
            Some(other) => panic!("a review only tells, not {other:?}"),
            None => vec![],
        }
    }

    fn review(login: &str, state: &str, id: u32) -> String {
        format!(
            r#"{{"url":"https://forge.invalid/example/ark/pull/7#pullrequestreview-{id}","state":"{state}","author":{{"login":"{login}"}}}}"#
        )
    }

    fn reviews(each: &[String]) -> String {
        format!(r#"{{"reviews":{{"nodes":[{}]}}}}"#, each.join(","))
    }

    #[test]
    fn each_submitted_review_is_told_naming_its_reviewer_and_state() {
        for (state, said) in [
            ("APPROVED", "approved"),
            ("CHANGES_REQUESTED", "changes requested"),
            ("COMMENTED", "commented"),
        ] {
            let told = reviewed_on(State::Open, &reviews(&[review("alice", state, 11)]));

            assert_eq!(told.len(), 1, "{state}");
            assert!(told[0].contains("alice"), "{}", told[0]);
            assert!(told[0].contains(said), "{}", told[0]);
            assert!(told[0].contains("example/ark#7"), "{}", told[0]);
        }
    }

    /// One reviewer commenting twice, or two reviewers asking for changes,
    /// are as many reviews as there are, and each is told.
    #[test]
    fn two_reviews_that_say_the_same_thing_are_told_apart() {
        let told = reviewed_on(
            State::Open,
            &reviews(&[
                review("alice", "COMMENTED", 11),
                review("alice", "COMMENTED", 12),
            ]),
        );

        assert_eq!(told.len(), 2);
        assert_ne!(told[0], told[1]);
    }

    #[test]
    fn a_review_nobody_has_submitted_is_not_told() {
        let pending = reviews(&[review("alice", "PENDING", 11)]);
        assert_eq!(reviewed_on(State::Open, &pending), Vec::<String>::new());
    }

    /// Dismissing a review changes its state, so telling a dismissed one
    /// would tell a review already told as approved a second time.
    #[test]
    fn a_dismissed_review_is_not_told() {
        let dismissed = reviews(&[review("alice", "DISMISSED", 11)]);
        assert_eq!(reviewed_on(State::Open, &dismissed), Vec::<String>::new());
    }

    #[test]
    fn a_review_whose_author_has_gone_is_told_as_ghost() {
        let gone = reviews(&[
            r#"{"url":"https://forge.invalid/example/ark/pull/7#pullrequestreview-11","state":"APPROVED","author":null}"#
                .to_string(),
        ]);
        let told = reviewed_on(State::Open, &gone);

        assert_eq!(told.len(), 1);
        assert!(told[0].contains("ghost"), "{}", told[0]);
    }

    #[test]
    fn a_pull_request_nobody_has_reviewed_is_not_told() {
        assert_eq!(
            reviewed_on(State::Open, r#"{"reviews":{"nodes":[]}}"#),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_draft_is_told_of_its_reviews() {
        let draft =
            reviews(&[review("alice", "COMMENTED", 11)]).replacen('{', r#"{"isDraft":true,"#, 1);
        assert_eq!(reviewed_on(State::Open, &draft).len(), 1);
    }

    #[test]
    fn a_pull_request_that_is_no_longer_open_is_not_told_of_reviews() {
        let reviewed = reviews(&[review("alice", "APPROVED", 11)]);
        assert_eq!(reviewed_on(State::Closed, &reviewed), Vec::<String>::new());
        assert_eq!(reviewed_on(State::Merged, &reviewed), Vec::<String>::new());
    }

    fn conflict_told_on(state: State, fields: &str) -> Option<String> {
        let pr = PullRequest {
            repo: "example/ark".to_string(),
            number: 7,
        };
        let observed = Observed {
            state,
            fields: serde_json::from_str(fields).expect("the fields parse"),
        };
        match conflicting(&pr, &observed).expect("the fields read") {
            Some(Outcome::Tell(mut texts)) if texts.len() == 1 => texts.pop(),
            Some(other) => panic!("a conflict tells once, not {other:?}"),
            None => None,
        }
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

    /// The fields today's query asks, so adding an event that reads nothing
    /// new costs GitHub nothing new.
    #[test]
    fn the_events_read_a_draft_a_merge_commit_a_review_decision_the_head_commits_checks_recent_reviews_and_mergeability(
    ) {
        assert_eq!(
            fields(&EVENTS),
            "isDraft mergeCommit{oid} reviewDecision \
             commits(last:1){nodes{commit{oid statusCheckRollup{state}}}} \
             reviews(last:5){nodes{url state author{login}}} \
             mergeable headRefOid"
        );
    }
}
