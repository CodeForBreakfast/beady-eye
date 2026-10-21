//! Each review submitted on a pull request, which the beads its gates hold
//! back are told.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::{Event, Outcome, Telling};
use crate::collect::gates::PullRequest;
use crate::collect::github::{Observed, State};

/// The latest reviews, selected alike by every event that reads them so
/// that [`fields`](super::fields) asks them once.
pub(super) const REVIEWS: &str = "reviews(last:5){nodes{url state body submittedAt author{login} \
     comments(first:1){totalCount nodes{url replyTo{id}}}}}";

pub const EVENT: Event = Event {
    fields: &[REVIEWS],
    happening: "was reviewed",
    outcome: reviewed,
};

/// One of the latest reviews of a pull request.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Review {
    url: String,
    state: String,
    #[serde(default)]
    body: String,
    pub submitted_at: Option<DateTime<Utc>>,
    author: Option<Author>,
    #[serde(default)]
    comments: Comments,
}

#[derive(Deserialize)]
struct Author {
    login: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Comments {
    total_count: usize,
    nodes: Vec<InlineComment>,
}

/// An inline comment a review holds.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct InlineComment {
    pub url: String,
    reply_to: Option<serde_json::Value>,
}

impl InlineComment {
    /// Whether this replies in a review thread rather than starting one.
    pub fn is_reply(&self) -> bool {
        self.reply_to.is_some()
    }
}

impl Review {
    /// Who submitted it, or "ghost", GitHub's name for an account that has
    /// gone.
    pub fn author(&self) -> &str {
        self.author.as_ref().map_or("ghost", |author| &author.login)
    }

    fn has_body(&self) -> bool {
        !self.body.trim().is_empty()
    }

    /// The inline comment this review is no more than. GitHub reviews an
    /// inline comment made on its own as commenting, saying nothing itself
    /// and holding that one comment.
    pub fn lone_inline_comment(&self) -> Option<&InlineComment> {
        match self.comments.nodes.as_slice() {
            [comment]
                if self.state == "COMMENTED"
                    && !self.has_body()
                    && self.comments.total_count == 1 =>
            {
                Some(comment)
            }
            _ => None,
        }
    }
}

/// The latest reviews of the pull request `observed`.
pub(super) fn reviews(observed: &Observed) -> Result<Vec<Review>, serde_json::Error> {
    #[derive(Deserialize)]
    struct Fields {
        #[serde(default)]
        reviews: Reviews,
    }
    #[derive(Deserialize, Default)]
    struct Reviews {
        nodes: Vec<Review>,
    }
    Ok(Fields::deserialize(&observed.fields)?.reviews.nodes)
}

fn reviewed(pr: &PullRequest, observed: &Observed) -> Result<Option<Outcome>, serde_json::Error> {
    let told: Vec<Telling> = reviews(observed)?
        .into_iter()
        .filter(|review| !matches!(review.state.as_str(), "PENDING" | "DISMISSED"))
        .filter(|review| review.lone_inline_comment().is_none())
        .map(|review| Telling {
            text: told(pr, &review),
            happened: review.submitted_at,
        })
        .collect();
    Ok((observed.state == State::Open && !told.is_empty()).then_some(Outcome::Tell(told)))
}

/// How `review` is told: its reviewer, its state, whether it has a written
/// body and how many inline comments it holds, never what they say.
fn told(pr: &PullRequest, review: &Review) -> String {
    let state = review.state.to_ascii_lowercase().replace('_', " ");
    let inline = match review.comments.total_count {
        0 => None,
        1 => Some("1 inline comment".to_string()),
        many => Some(format!("{many} inline comments")),
    };
    let holding = match (review.has_body(), inline) {
        (false, None) => String::new(),
        (true, None) => ", with a written body".to_string(),
        (false, Some(inline)) => format!(", with {inline}"),
        (true, Some(inline)) => format!(", with a written body and {inline}"),
    };
    format!(
        "{} reviewed pull request {pr}: {state}{holding}. {}",
        review.author(),
        review.url
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::pr_events::testing::{tellings, texts, when};

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

    /// An inline comment made on its own is told as that comment, so
    /// telling its review as well would tell it twice.
    #[test]
    fn a_review_that_is_only_a_lone_inline_comment_is_not_told() {
        let lone = r#"{"url":"https://forge.invalid/example/ark/pull/7#pullrequestreview-11","state":"COMMENTED","body":"","author":{"login":"alice"},"comments":{"totalCount":1,"nodes":[{"url":"https://forge.invalid/example/ark/pull/7#discussion_r21","replyTo":null}]}}"#;
        assert_eq!(
            reviewed_on(State::Open, &reviews(&[lone.to_string()])),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_review_with_a_body_and_an_inline_comment_is_told() {
        let said = r#"{"url":"https://forge.invalid/example/ark/pull/7#pullrequestreview-11","state":"COMMENTED","body":"Looks close.","author":{"login":"alice"},"comments":{"totalCount":1,"nodes":[{"url":"https://forge.invalid/example/ark/pull/7#discussion_r21","replyTo":null}]}}"#;
        assert_eq!(
            reviewed_on(State::Open, &reviews(&[said.to_string()])).len(),
            1
        );
    }

    /// A review in `state` with `body`, holding `inline` inline comments.
    fn holding(state: &str, body: &str, inline: usize) -> String {
        format!(
            r#"{{"url":"https://forge.invalid/example/ark/pull/7#pullrequestreview-11","state":"{state}","body":"{body}","author":{{"login":"alice"}},"comments":{{"totalCount":{inline},"nodes":[{{"url":"https://forge.invalid/example/ark/pull/7#discussion_r21","replyTo":null}}]}}}}"#
        )
    }

    #[test]
    fn an_approval_holding_inline_comments_is_told_with_how_many() {
        assert_eq!(
            reviewed_on(State::Open, &reviews(&[holding("APPROVED", "", 2)])),
            [
                "alice reviewed pull request example/ark#7: approved, with 2 inline comments. \
              https://forge.invalid/example/ark/pull/7#pullrequestreview-11"
            ]
        );
        assert!(
            reviewed_on(State::Open, &reviews(&[holding("APPROVED", "", 1)]))[0]
                .contains("approved, with 1 inline comment.")
        );
    }

    #[test]
    fn an_approval_holding_a_body_only_is_told_as_having_one() {
        let told = reviewed_on(State::Open, &reviews(&[holding("APPROVED", "Nice.", 0)]));
        assert_eq!(
            told,
            [
                "alice reviewed pull request example/ark#7: approved, with a written body. \
              https://forge.invalid/example/ark/pull/7#pullrequestreview-11"
            ]
        );
        assert!(!told[0].contains("Nice"), "{}", told[0]);
    }

    #[test]
    fn a_review_holding_a_body_and_inline_comments_names_both() {
        assert!(reviewed_on(
            State::Open,
            &reviews(&[holding("CHANGES_REQUESTED", "Two things.", 3)])
        )[0]
        .contains("changes requested, with a written body and 3 inline comments."));
    }

    #[test]
    fn a_blank_body_is_not_told_as_a_written_one() {
        assert!(
            reviewed_on(State::Open, &reviews(&[holding("APPROVED", "\\n ", 0)]))[0]
                .contains("approved. ")
        );
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

    #[test]
    fn a_review_happened_when_it_was_submitted() {
        let submitted = r#"{"reviews":{"nodes":[{"url":"https://forge.invalid/example/ark/pull/7#pullrequestreview-11","state":"APPROVED","submittedAt":"2026-10-06T08:24:49Z","author":{"login":"alice"}}]}}"#;
        let told = tellings(reviewed, State::Open, submitted);

        assert_eq!(told[0].happened, when("2026-10-06T08:24:49Z"));
    }
}
