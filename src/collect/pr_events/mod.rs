//! Each thing a pull request can do that `bdi gates` acts on: the fields of
//! GraphQL's `PullRequest` it reads, and what it makes of them.
//!
//! An event either closes the gh:pr gates waiting for it, or tells the beads
//! they hold back what happened, once for each thing it has to say. A webhook
//! delivery tells them what it says happened, too. A new
//! event is a module of its own here, holding its [`Event`] and its tests,
//! and an entry in [`EVENTS`]. A gate waiting for something new is a new
//! [`Until`] beside one.

use chrono::{DateTime, Utc};

use crate::collect::gates::PullRequest;
use crate::collect::github::Observed;
use crate::collect::webhook::Said;
use crate::model::gate::Until;

mod approved;
mod checks_failed;
mod checks_passed;
mod closed_unmerged;
mod commented;
mod conflicting;
mod inline_commented;
mod merged;
mod new_head;
mod ready_for_review;
mod reviewed;

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

/// The pull request's head commit, selected alike by every event that reads
/// it so that [`fields`] asks it once.
const HEAD_COMMIT: &str = "headRef{target{...on Commit{committedDate oid url}}}";

/// The checks on the pull request's head commit, selected alike by every
/// event that reads them so that [`fields`] asks them once.
const HEAD_CHECKS: &str = "commits(last:1){nodes{commit{oid statusCheckRollup{state contexts(last:100){nodes{...on CheckRun{conclusion completedAt} ...on StatusContext{state createdAt}}}}}}}";

/// Every event `bdi gates` acts on.
pub const EVENTS: &[Event] = &[
    ready_for_review::EVENT,
    merged::EVENT,
    closed_unmerged::EVENT,
    approved::EVENT,
    checks_failed::EVENT,
    checks_passed::EVENT,
    reviewed::EVENT,
    conflicting::EVENT,
    commented::EVENT,
    inline_commented::EVENT,
    new_head::EVENT,
];

/// What a delivery that came with no word of its own did, as a line
/// reporting it says.
const ACTED: &str = "was acted on";

/// What a delivery that `said` something of `pr` tells the beads its gates
/// hold back, beside what it did, as a line reporting it says. Where the
/// sweep tells the same thing, the delivery tells it in the sweep's words, so
/// a bead told by both is told once.
pub fn delivered(said: &Said, pr: &PullRequest) -> (&'static str, Telling) {
    let (happening, text, happened) = match said {
        Said::Commented { by, url, at } => (
            commented::EVENT.happening,
            commented::told(by, pr, url),
            *at,
        ),
        Said::Reviewed { by, state, url, at } => (
            reviewed::EVENT.happening,
            reviewed::told(by, pr, state, url),
            *at,
        ),
        Said::NewHead { sha, url } => (
            new_head::EVENT.happening,
            new_head::told(pr, sha, url),
            None,
        ),
        Said::ChecksFailed { sha } => (
            checks_failed::EVENT.happening,
            checks_failed::told(sha, pr),
            None,
        ),
        Said::Acted {
            event,
            action,
            by,
            url,
        } => (
            ACTED,
            format!("{by} acted on pull request {pr}: {event} {action}. {url}"),
            None,
        ),
    };
    (happening, Telling { text, happened })
}

/// The fields of GraphQL's `PullRequest` that `events` read between them,
/// each asked once however many events read it.
pub fn fields(events: &[Event]) -> String {
    let mut asked: Vec<&str> = Vec::new();
    for field in events.iter().flat_map(|event| event.fields) {
        if !asked.contains(field) {
            asked.push(field);
        }
    }
    asked.join(" ")
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

/// What every event's tests read #7 through.
#[cfg(test)]
mod testing {
    use chrono::{DateTime, Utc};

    use super::{Outcome, Telling};
    use crate::collect::gates::PullRequest;
    use crate::collect::github::{Observed, State};

    pub fn pr() -> PullRequest {
        PullRequest {
            repo: "example/ark".to_string(),
            number: 7,
        }
    }

    pub fn observed(state: State, fields: &str) -> Observed {
        Observed {
            state,
            fields: serde_json::from_str(fields).expect("the fields parse"),
            refused: Vec::new(),
        }
    }

    type Outcomes = fn(&PullRequest, &Observed) -> Result<Option<Outcome>, serde_json::Error>;

    /// What `event` tells of #7 in `state` with `fields`.
    pub fn tellings(event: Outcomes, state: State, fields: &str) -> Vec<Telling> {
        match event(&pr(), &observed(state, fields)).expect("the fields read") {
            Some(Outcome::Tell(tellings)) => tellings,
            Some(other) => panic!("this event only tells, not {other:?}"),
            None => vec![],
        }
    }

    /// The one telling of an event that tells once, if it tells.
    pub fn once(mut tellings: Vec<Telling>) -> Option<Telling> {
        assert!(
            tellings.len() <= 1,
            "this event tells once, not {tellings:?}"
        );
        tellings.pop()
    }

    pub fn texts(tellings: Vec<Telling>) -> Vec<String> {
        tellings.into_iter().map(|telling| telling.text).collect()
    }

    pub fn when(at: &str) -> Option<DateTime<Utc>> {
        Some(at.parse().expect("the time parses"))
    }
}

#[cfg(test)]
mod tests {
    use super::testing::when;
    use super::*;

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

    /// An event no `bdi gates` has, reading `fields`.
    fn reading(fields: &'static [&'static str]) -> Event {
        Event {
            fields,
            happening: "did something",
            outcome: |_, _| Ok(None),
        }
    }

    #[test]
    fn the_events_ask_every_field_they_read_in_their_order() {
        let events = [
            reading(&["isDraft"]),
            reading(&[]),
            reading(&["mergeCommit{oid}", "reviewDecision"]),
        ];

        assert_eq!(fields(&events), "isDraft mergeCommit{oid} reviewDecision");
    }

    #[test]
    fn a_field_two_events_read_is_asked_once() {
        let events = [
            reading(&["headRefOid"]),
            reading(&["mergeable", "headRefOid"]),
        ];

        assert_eq!(fields(&events), "headRefOid mergeable");
    }
}
