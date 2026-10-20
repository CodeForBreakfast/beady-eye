use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::{Event, Outcome, Telling};
use crate::collect::gates::PullRequest;
use crate::collect::github::{Observed, State};

pub const EVENT: Event = Event {
    fields: &["reviews(last:5){nodes{url state submittedAt author{login}}}"],
    happening: "was reviewed",
    outcome: reviewed,
};

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
