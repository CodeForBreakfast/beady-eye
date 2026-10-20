//! Each comment on a pull request, which the beads its gates hold back are
//! told.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::{Event, Outcome, Telling};
use crate::collect::gates::PullRequest;
use crate::collect::github::{Observed, State};

pub const EVENT: Event = Event {
    fields: &["comments(last:5){nodes{url createdAt author{login}}}"],
    happening: "was commented on",
    outcome: commented,
};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::pr_events::testing::{tellings, texts, when};

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

    #[test]
    fn a_comment_happened_when_it_was_made() {
        let made = r#"{"comments":{"nodes":[{"url":"https://forge.invalid/example/ark/pull/7#issuecomment-11","createdAt":"2026-10-06T08:24:49Z","author":{"login":"alice"}}]}}"#;
        let told = tellings(commented, State::Open, made);

        assert_eq!(told[0].happened, when("2026-10-06T08:24:49Z"));
    }
}
