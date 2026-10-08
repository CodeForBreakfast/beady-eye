//! Each thing a pull request can do that `bdi gates` acts on: the fields of
//! GraphQL's `PullRequest` it reads, and what it makes of them.
//!
//! An event either closes the gh:pr gates waiting for it, or tells the beads
//! they hold back what happened, once for each thing it has to say. A new
//! event is a new entry in [`EVENTS`], and a gate waiting for something new
//! is a new [`Until`] beside one.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::collect::gates::PullRequest;
use crate::collect::github::{Observed, State};
use crate::model::gate::Until;

/// One thing a pull request can do.
#[derive(Clone, Copy)]
pub struct Event {
    /// The fields of GraphQL's `PullRequest` this event reads, beside the
    /// `state` every event is given, each with what it selects.
    pub fields: &'static [&'static str],
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
    /// Comment each of these on each bead a gate holds back, once.
    Tell(Vec<Telling>),
}

/// One thing to comment on the beads a gate holds back.
#[derive(Debug)]
pub struct Telling {
    /// Two tellings with one text are one telling, so the text carries
    /// whatever tells this one from the next.
    pub text: String,
    /// When what it tells happened. A gate made after that does not tell it,
    /// since whoever made the gate could already see it. One with no time is
    /// told by every gate.
    pub happened: Option<DateTime<Utc>>,
}

impl Telling {
    /// Whether a gate made at `made` tells this.
    pub fn is_news_to(&self, made: Option<DateTime<Utc>>) -> bool {
        match (self.happened, made) {
            (Some(happened), Some(made)) => happened >= made,
            _ => true,
        }
    }
}

/// Every event `bdi gates` acts on.
pub const EVENTS: [Event; 8] = [
    Event {
        fields: &["isDraft"],
        happening: "is ready for review",
        outcome: ready_for_review,
    },
    Event {
        fields: &["mergeCommit{oid}"],
        happening: "merged",
        outcome: merged,
    },
    Event {
        fields: &[],
        happening: "closed unmerged",
        outcome: closed_unmerged,
    },
    Event {
        fields: &["reviewDecision"],
        happening: "is approved",
        outcome: approved,
    },
    Event {
        fields: &["commits(last:1){nodes{commit{oid statusCheckRollup{state contexts(last:100){nodes{...on CheckRun{conclusion completedAt} ...on StatusContext{state createdAt}}}}}}}"],
        happening: "has failing checks",
        outcome: checks_failed,
    },
    Event {
        fields: &["reviews(last:5){nodes{url state submittedAt author{login}}}"],
        happening: "was reviewed",
        outcome: reviewed,
    },
    Event {
        fields: &[
            "mergeable",
            "headRefOid",
            "headRef{target{...on Commit{committedDate}}}",
            "baseRef{target{...on Commit{committedDate}}}",
        ],
        happening: "conflicts with its base",
        outcome: conflicting,
    },
    Event {
        fields: &["comments(last:5){nodes{url createdAt author{login}}}"],
        happening: "was commented on",
        outcome: commented,
    },
];

/// The fields of GraphQL's `PullRequest` that `events` read between them.
pub fn fields(events: &[Event]) -> String {
    events
        .iter()
        .flat_map(|event| event.fields)
        .copied()
        .collect::<Vec<_>>()
        .join(" ")
}

impl Event {
    /// Whether this event reads the field GraphQL answers under `name`.
    pub fn reads(&self, name: &str) -> bool {
        self.fields
            .iter()
            .any(|field| field.split(['(', '{']).next() == Some(name))
    }
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
    #[serde(rename_all = "camelCase")]
    struct Review {
        url: String,
        state: String,
        submitted_at: Option<DateTime<Utc>>,
        author: Option<Author>,
    }
    #[derive(Deserialize)]
    struct Author {
        login: String,
    }
    let Fields { reviews } = Fields::deserialize(&observed.fields)?;
    let told: Vec<Telling> = reviews
        .nodes
        .into_iter()
        .filter(|review| !matches!(review.state.as_str(), "PENDING" | "DISMISSED"))
        .map(
            |Review {
                 url,
                 state,
                 submitted_at,
                 author,
             }| {
                let reviewer = author.map_or("ghost".to_string(), |author| author.login);
                let state = state.to_ascii_lowercase().replace('_', " ");
                Telling {
                    text: format!("{reviewer} reviewed pull request {pr}: {state}. {url}"),
                    happened: submitted_at,
                }
            },
        )
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

fn commented(pr: &PullRequest, observed: &Observed) -> Result<Option<Outcome>, serde_json::Error> {
    #[derive(Deserialize)]
    struct Fields {
        #[serde(default)]
        comments: Comments,
    }
    #[derive(Deserialize, Default)]
    struct Comments {
        nodes: Vec<Comment>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Comment {
        url: String,
        created_at: Option<DateTime<Utc>>,
        author: Option<Author>,
    }
    #[derive(Deserialize)]
    struct Author {
        login: String,
    }
    let Fields { comments } = Fields::deserialize(&observed.fields)?;
    let told: Vec<Telling> = comments
        .nodes
        .into_iter()
        .map(
            |Comment {
                 url,
                 created_at,
                 author,
             }| {
                let commenter = author.map_or("ghost".to_string(), |author| author.login);
                Telling {
                    text: format!("{commenter} commented on pull request {pr}. {url}"),
                    happened: created_at,
                }
            },
        )
        .collect();
    Ok((observed.state == State::Open && !told.is_empty()).then_some(Outcome::Tell(told)))
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
    // A gate made on a pull request already closed waits for a merge that
    // cannot come, so it is told whenever it was made.
    Ok((observed.state == State::Closed).then(|| {
        Outcome::Tell(vec![Telling {
            text: format!(
                "Pull request {pr} closed without being merged, so the gh:pr gate waiting on \
                 it stays open."
            ),
            happened: None,
        }])
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
            refused: Vec::new(),
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

    type Outcomes = fn(&PullRequest, &Observed) -> Result<Option<Outcome>, serde_json::Error>;

    /// What `event` tells of #7 in `state` with `fields`.
    fn tellings(event: Outcomes, state: State, fields: &str) -> Vec<Telling> {
        let pr = PullRequest {
            repo: "example/ark".to_string(),
            number: 7,
        };
        let observed = Observed {
            state,
            fields: serde_json::from_str(fields).expect("the fields parse"),
            refused: Vec::new(),
        };
        match event(&pr, &observed).expect("the fields read") {
            Some(Outcome::Tell(tellings)) => tellings,
            Some(other) => panic!("this event only tells, not {other:?}"),
            None => vec![],
        }
    }

    /// The one telling of an event that tells once, if it tells.
    fn once(mut tellings: Vec<Telling>) -> Option<Telling> {
        assert!(
            tellings.len() <= 1,
            "this event tells once, not {tellings:?}"
        );
        tellings.pop()
    }

    fn texts(tellings: Vec<Telling>) -> Vec<String> {
        tellings.into_iter().map(|telling| telling.text).collect()
    }

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

    fn reviewed_on(state: State, fields: &str) -> Vec<String> {
        texts(tellings(reviewed, state, fields))
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

    fn commented_on(state: State, fields: &str) -> Vec<String> {
        texts(tellings(commented, state, fields))
    }

    fn comment(login: &str, id: u32) -> String {
        format!(
            r#"{{"url":"https://forge.invalid/example/ark/pull/7#issuecomment-{id}","author":{{"login":"{login}"}}}}"#
        )
    }

    fn comments(each: &[String]) -> String {
        format!(r#"{{"comments":{{"nodes":[{}]}}}}"#, each.join(","))
    }

    #[test]
    fn each_comment_is_told_naming_its_author() {
        let told = commented_on(State::Open, &comments(&[comment("alice", 11)]));

        assert_eq!(told.len(), 1);
        assert!(told[0].contains("alice"), "{}", told[0]);
        assert!(told[0].contains("example/ark#7"), "{}", told[0]);
    }

    /// One author commenting twice is two comments, and each is told.
    #[test]
    fn two_comments_by_one_author_are_told_apart() {
        let told = commented_on(
            State::Open,
            &comments(&[comment("alice", 11), comment("alice", 12)]),
        );

        assert_eq!(told.len(), 2);
        assert_ne!(told[0], told[1]);
    }

    #[test]
    fn a_comment_whose_author_has_gone_is_told_as_ghost() {
        let gone = comments(&[
            r#"{"url":"https://forge.invalid/example/ark/pull/7#issuecomment-11","author":null}"#
                .to_string(),
        ]);
        let told = commented_on(State::Open, &gone);

        assert_eq!(told.len(), 1);
        assert!(told[0].contains("ghost"), "{}", told[0]);
    }

    #[test]
    fn a_pull_request_nobody_has_commented_on_is_not_told() {
        assert_eq!(
            commented_on(State::Open, r#"{"comments":{"nodes":[]}}"#),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_draft_is_told_of_its_comments() {
        let draft = comments(&[comment("alice", 11)]).replacen('{', r#"{"isDraft":true,"#, 1);
        assert_eq!(commented_on(State::Open, &draft).len(), 1);
    }

    #[test]
    fn a_pull_request_that_is_no_longer_open_is_not_told_of_comments() {
        let commented = comments(&[comment("alice", 11)]);
        assert_eq!(
            commented_on(State::Closed, &commented),
            Vec::<String>::new()
        );
        assert_eq!(
            commented_on(State::Merged, &commented),
            Vec::<String>::new()
        );
    }

    fn when(at: &str) -> Option<DateTime<Utc>> {
        Some(at.parse().expect("the time parses"))
    }

    #[test]
    fn a_telling_is_news_to_a_gate_made_no_later_than_it_happened() {
        let telling = |happened| Telling {
            text: String::new(),
            happened,
        };
        let at = when("2026-10-06T08:24:49Z");

        assert!(telling(at).is_news_to(when("2026-10-06T08:24:48Z")));
        assert!(telling(at).is_news_to(at));
        assert!(!telling(at).is_news_to(when("2026-10-06T08:24:50Z")));
        assert!(telling(None).is_news_to(at));
        assert!(telling(at).is_news_to(None));
    }

    #[test]
    fn a_review_happened_when_it_was_submitted() {
        let submitted = r#"{"reviews":{"nodes":[{"url":"https://forge.invalid/example/ark/pull/7#pullrequestreview-11","state":"APPROVED","submittedAt":"2026-10-06T08:24:49Z","author":{"login":"alice"}}]}}"#;
        let told = tellings(reviewed, State::Open, submitted);

        assert_eq!(told[0].happened, when("2026-10-06T08:24:49Z"));
    }

    #[test]
    fn a_comment_happened_when_it_was_made() {
        let made = r#"{"comments":{"nodes":[{"url":"https://forge.invalid/example/ark/pull/7#issuecomment-11","createdAt":"2026-10-06T08:24:49Z","author":{"login":"alice"}}]}}"#;
        let told = tellings(commented, State::Open, made);

        assert_eq!(told[0].happened, when("2026-10-06T08:24:49Z"));
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

    /// The fields today's query asks, so adding an event that reads nothing
    /// new costs GitHub nothing new.
    #[test]
    fn the_events_read_a_draft_a_merge_commit_a_review_decision_the_head_commits_checks_recent_reviews_mergeability_and_comments(
    ) {
        assert_eq!(
            fields(&EVENTS),
            "isDraft mergeCommit{oid} reviewDecision \
             commits(last:1){nodes{commit{oid statusCheckRollup{state \
             contexts(last:100){nodes{...on CheckRun{conclusion completedAt} \
             ...on StatusContext{state createdAt}}}}}}} \
             reviews(last:5){nodes{url state submittedAt author{login}}} \
             mergeable headRefOid headRef{target{...on Commit{committedDate}}} \
             baseRef{target{...on Commit{committedDate}}} \
             comments(last:5){nodes{url createdAt author{login}}}"
        );
    }
}
