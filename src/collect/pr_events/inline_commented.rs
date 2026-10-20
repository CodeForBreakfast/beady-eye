//! Each inline comment made on its own, a reply in a review thread among
//! them, which the beads a pull request's gates hold back are told.
//!
//! GitHub makes every inline comment part of a review. One made on its own
//! is a review of its own, commenting, with no body and that one comment, so
//! this event reads the reviews and tells those as their comment.

use super::reviewed::{self, REVIEWS};
use super::{Event, Outcome, Telling};
use crate::collect::gates::PullRequest;
use crate::collect::github::{Observed, State};

pub const EVENT: Event = Event {
    fields: &[REVIEWS],
    happening: "was commented on inline",
    outcome: inline_commented,
};

/// What a bead is told of `author`'s inline comment at `url` on `pr`, a
/// reply in a review thread where `reply` says so.
fn telling(pr: &PullRequest, author: &str, url: &str, reply: bool) -> String {
    if reply {
        format!("{author} replied in a review thread on pull request {pr}. {url}")
    } else {
        format!("{author} commented inline on pull request {pr}. {url}")
    }
}

fn inline_commented(
    pr: &PullRequest,
    observed: &Observed,
) -> Result<Option<Outcome>, serde_json::Error> {
    let told: Vec<Telling> = reviewed::reviews(observed)?
        .iter()
        .filter_map(|review| {
            let comment = review.lone_inline_comment()?;
            Some(Telling {
                text: telling(pr, review.author(), &comment.url, comment.is_reply()),
                happened: review.submitted_at,
            })
        })
        .collect();
    Ok((observed.state == State::Open && !told.is_empty()).then_some(Outcome::Tell(told)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::pr_events::testing::{pr, tellings, texts, when};

    fn commented_inline_on(state: State, fields: &str) -> Vec<String> {
        texts(tellings(inline_commented, state, fields))
    }

    /// A review by `login` in `state` with `body`, holding the inline
    /// comment `discussion` where there is one, replying where `reply`
    /// says so.
    fn review(
        login: &str,
        state: &str,
        body: &str,
        discussion: Option<u32>,
        reply: bool,
    ) -> String {
        let reply_to = if reply {
            r#"{"id":"PRRC_root"}"#
        } else {
            "null"
        };
        let comments = discussion.map_or(r#"{"totalCount":0,"nodes":[]}"#.to_string(), |id| {
            format!(
                r#"{{"totalCount":1,"nodes":[{{"url":"https://forge.invalid/example/ark/pull/7#discussion_r{id}","replyTo":{reply_to}}}]}}"#
            )
        });
        format!(
            r#"{{"url":"https://forge.invalid/example/ark/pull/7#pullrequestreview-1","state":"{state}","body":"{body}","author":{{"login":"{login}"}},"comments":{comments}}}"#
        )
    }

    fn reviews(each: &[String]) -> String {
        format!(r#"{{"reviews":{{"nodes":[{}]}}}}"#, each.join(","))
    }

    #[test]
    fn a_reply_in_a_review_thread_is_told_naming_its_author_and_linking_it() {
        let told = commented_inline_on(
            State::Open,
            &reviews(&[review("alice", "COMMENTED", "", Some(21), true)]),
        );

        assert_eq!(
            told,
            [telling(
                &pr(),
                "alice",
                "https://forge.invalid/example/ark/pull/7#discussion_r21",
                true
            )]
        );
        assert!(told[0].contains("alice"), "{}", told[0]);
        assert!(told[0].contains("replied"), "{}", told[0]);
        assert!(told[0].contains("example/ark#7"), "{}", told[0]);
        assert!(told[0].contains("#discussion_r21"), "{}", told[0]);
    }

    #[test]
    fn a_lone_inline_comment_is_told_naming_its_author_and_linking_it() {
        let told = commented_inline_on(
            State::Open,
            &reviews(&[review("alice", "COMMENTED", "", Some(21), false)]),
        );

        assert_eq!(told.len(), 1);
        assert!(told[0].contains("alice"), "{}", told[0]);
        assert!(told[0].contains("commented inline"), "{}", told[0]);
        assert!(told[0].contains("example/ark#7"), "{}", told[0]);
        assert!(told[0].contains("#discussion_r21"), "{}", told[0]);
    }

    #[test]
    fn a_reply_and_a_lone_comment_are_told_apart() {
        assert_ne!(
            telling(&pr(), "alice", "https://forge.invalid/x", true),
            telling(&pr(), "alice", "https://forge.invalid/x", false)
        );
    }

    #[test]
    fn two_inline_comments_by_one_author_are_told_apart() {
        let told = commented_inline_on(
            State::Open,
            &reviews(&[
                review("alice", "COMMENTED", "", Some(21), true),
                review("alice", "COMMENTED", "", Some(22), true),
            ]),
        );

        assert_eq!(told.len(), 2);
        assert_ne!(told[0], told[1]);
    }

    /// A review with something to say of its own is a review, which the
    /// review event tells, whatever inline comments it holds.
    #[test]
    fn a_review_is_not_told_as_an_inline_comment() {
        for review in [
            review("alice", "COMMENTED", "Looks close.", Some(21), false),
            review("alice", "APPROVED", "", Some(21), false),
            review("alice", "CHANGES_REQUESTED", "", Some(21), false),
            review("alice", "COMMENTED", "", None, false),
            review("alice", "PENDING", "", Some(21), false),
        ] {
            assert_eq!(
                commented_inline_on(State::Open, &reviews(std::slice::from_ref(&review))),
                Vec::<String>::new(),
                "{review}"
            );
        }
    }

    #[test]
    fn a_review_of_several_inline_comments_is_not_told_as_one_of_them() {
        let several = review("alice", "COMMENTED", "", Some(21), false)
            .replace(r#""totalCount":1"#, r#""totalCount":3"#);
        assert_eq!(
            commented_inline_on(State::Open, &reviews(&[several])),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_review_whose_body_is_blank_is_told_as_its_inline_comment() {
        let blank = reviews(&[review("alice", "COMMENTED", "\\n ", Some(21), false)]);
        assert_eq!(commented_inline_on(State::Open, &blank).len(), 1);
    }

    #[test]
    fn an_inline_comment_whose_author_has_gone_is_told_as_ghost() {
        let gone = review("alice", "COMMENTED", "", Some(21), true)
            .replace(r#"{"login":"alice"}"#, "null");
        let told = commented_inline_on(State::Open, &reviews(&[gone]));

        assert_eq!(told.len(), 1);
        assert!(told[0].contains("ghost"), "{}", told[0]);
    }

    #[test]
    fn a_draft_is_told_of_its_inline_comments() {
        let draft = reviews(&[review("alice", "COMMENTED", "", Some(21), true)]).replacen(
            '{',
            r#"{"isDraft":true,"#,
            1,
        );
        assert_eq!(commented_inline_on(State::Open, &draft).len(), 1);
    }

    #[test]
    fn a_pull_request_that_is_no_longer_open_is_not_told_of_inline_comments() {
        let commented = reviews(&[review("alice", "COMMENTED", "", Some(21), true)]);
        assert_eq!(
            commented_inline_on(State::Closed, &commented),
            Vec::<String>::new()
        );
        assert_eq!(
            commented_inline_on(State::Merged, &commented),
            Vec::<String>::new()
        );
    }

    #[test]
    fn an_inline_comment_happened_when_its_review_was_submitted() {
        let submitted = reviews(&[review("alice", "COMMENTED", "", Some(21), true).replacen(
            '{',
            r#"{"submittedAt":"2026-10-06T08:24:49Z","#,
            1,
        )]);
        let told = tellings(inline_commented, State::Open, &submitted);

        assert_eq!(told[0].happened, when("2026-10-06T08:24:49Z"));
    }
}
