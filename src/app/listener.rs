//! What `bdi listen` holds of every project, and the seam what it holds comes
//! across.
//!
//! A change source finds out what each tracker holds, by whatever means it
//! has, and says so one project at a time. The listener keeps what it is told
//! and never asks which source told it, so a source can be replaced without
//! anything that watches or delivers changing.

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::model::snapshot::TrackerFailure;
use crate::model::types::Printed;

/// One bead as the listener holds it: its tracker's row, and the readiness
/// `bdi --beads` gives it.
///
/// The readiness sits beside the row rather than in it, so nothing `bdi` adds
/// can be taken for a field of bd's or collide with one a later bd adds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    /// The row as bd printed it, so a field `bdi` never reads still reaches a
    /// consumer. Nothing where the tracker was read without keeping its rows.
    pub row: Option<Arc<Printed>>,
    pub ready: bool,
    /// Every bead blocking this one, in its own project or another.
    pub blocked_by: Vec<String>,
}

/// What a change source says of one project.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    pub project: String,
    pub said: Said,
    /// bd's event records since the source last answered for this project,
    /// each as bd printed it. None from a source that reads no journal.
    pub events: Vec<serde_json::Value>,
}

/// How current a project is, and the beads it holds where they were read.
#[derive(Debug, Clone, PartialEq)]
pub enum Said {
    /// Every bead the tracker holds, wisps among them, keyed by id, as of
    /// `at`.
    Read {
        at: DateTime<Utc>,
        beads: BTreeMap<String, Held>,
    },
    /// Nothing has moved as of `at`, so the beads held already stand.
    Vouched { at: DateTime<Utc> },
    /// The last attempt to reach the tracker failed. The beads held already
    /// stand as the last known, and as old as when they were last vouched
    /// for.
    Unreachable(TrackerFailure),
}

/// Where the listener's answers come from.
pub trait ChangeSource: Send {
    /// Block until the source says something of a project, and hand it over.
    /// Nothing once the source has stopped.
    fn next(&mut self) -> Option<Answer>;
}

/// One project as the listener holds it.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Standing {
    /// Nothing until the source has read the project, which is not the same
    /// as a project holding no beads.
    pub beads: Option<BTreeMap<String, Held>>,
    /// The instant the source last vouched for the beads, by reading them or
    /// by saying nothing had moved.
    pub as_of: Option<DateTime<Utc>>,
    /// Why the last attempt to reach the tracker failed, where it did.
    pub unreachable: Option<TrackerFailure>,
}

/// Every project's beads as last read, and how current each is.
#[derive(Debug, Default)]
pub struct Hold {
    projects: BTreeMap<String, Standing>,
}

impl Hold {
    /// Take what a source said of one project.
    pub fn take(&mut self, answer: Answer) {
        let standing = self.projects.entry(answer.project).or_default();
        match answer.said {
            Said::Read { at, beads } => {
                standing.beads = Some(beads);
                standing.as_of = Some(at);
                standing.unreachable = None;
            }
            Said::Vouched { at } => standing.as_of = Some(at),
            Said::Unreachable(failure) => standing.unreachable = Some(failure),
        }
    }

    /// What the listener holds of `project`, where a source has said anything
    /// of it.
    #[cfg(test)]
    pub fn of(&self, project: &str) -> Option<&Standing> {
        self.projects.get(project)
    }
}

/// Hold everything `source` says until it stops.
pub fn hold(source: &mut dyn ChangeSource, into: &std::sync::Mutex<Hold>) {
    while let Some(answer) = source.next() {
        into.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take(answer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::fixtures::now;

    fn a_bead() -> Held {
        Held {
            row: None,
            ready: true,
            blocked_by: Vec::new(),
        }
    }

    fn read(project: &str, at: DateTime<Utc>, ids: &[&str]) -> Answer {
        said(
            project,
            Said::Read {
                at,
                beads: ids.iter().map(|id| (id.to_string(), a_bead())).collect(),
            },
        )
    }

    fn said(project: &str, said: Said) -> Answer {
        Answer {
            project: project.to_string(),
            said,
            events: Vec::new(),
        }
    }

    fn later(seconds: i64) -> DateTime<Utc> {
        now() + chrono::TimeDelta::seconds(seconds)
    }

    fn ids(standing: &Standing) -> Option<Vec<&str>> {
        standing
            .beads
            .as_ref()
            .map(|beads| beads.keys().map(String::as_str).collect())
    }

    #[test]
    fn a_project_nothing_has_been_said_of_is_not_held() {
        assert_eq!(Hold::default().of("dunwich"), None);
    }

    /// A read replaces every bead, so a bead the tracker no longer holds is
    /// no longer held either.
    #[test]
    fn a_read_replaces_what_was_held() {
        let mut hold = Hold::default();
        hold.take(read("dunwich", now(), &["dun-1", "dun-2"]));

        hold.take(read("dunwich", later(30), &["dun-1"]));

        let standing = hold.of("dunwich").expect("dunwich was read");
        assert_eq!(ids(standing), Some(vec!["dun-1"]));
        assert_eq!(standing.as_of, Some(later(30)));
    }

    #[test]
    fn a_project_vouched_for_keeps_its_beads_and_is_current_as_of_then() {
        let mut hold = Hold::default();
        hold.take(read("dunwich", now(), &["dun-1"]));

        hold.take(said("dunwich", Said::Vouched { at: later(10) }));

        let standing = hold.of("dunwich").expect("dunwich was read");
        assert_eq!(ids(standing), Some(vec!["dun-1"]));
        assert_eq!(standing.as_of, Some(later(10)));
    }

    /// The beads stand as the last known, and how old they are is still the
    /// age of the read that brought them.
    #[test]
    fn a_tracker_that_cannot_be_reached_leaves_the_last_known_beads_and_their_age() {
        let mut hold = Hold::default();
        hold.take(read("dunwich", now(), &["dun-1"]));

        hold.take(said(
            "dunwich",
            Said::Unreachable(TrackerFailure::Unavailable),
        ));

        let standing = hold.of("dunwich").expect("dunwich was read");
        assert_eq!(ids(standing), Some(vec!["dun-1"]));
        assert_eq!(standing.as_of, Some(now()));
        assert_eq!(standing.unreachable, Some(TrackerFailure::Unavailable));
    }

    #[test]
    fn a_read_after_a_failure_says_the_tracker_answers_again() {
        let mut hold = Hold::default();
        hold.take(said(
            "dunwich",
            Said::Unreachable(TrackerFailure::Unavailable),
        ));

        hold.take(read("dunwich", later(30), &[]));

        let standing = hold.of("dunwich").expect("dunwich was read");
        assert_eq!(standing.unreachable, None);
        assert_eq!(ids(standing), Some(Vec::new()));
    }

    /// Unread is not the same as empty, so a watch can wait for the first
    /// read rather than be told there is nothing.
    #[test]
    fn a_project_only_vouched_for_has_no_beads_yet() {
        let mut hold = Hold::default();

        hold.take(said("dunwich", Said::Vouched { at: now() }));

        assert_eq!(hold.of("dunwich").and_then(ids), None);
    }

    #[test]
    fn what_is_said_of_one_project_leaves_another_alone() {
        let mut hold = Hold::default();
        hold.take(read("dunwich", now(), &["dun-1"]));

        hold.take(read("ferry", later(5), &["fer-1"]));

        let dunwich = hold.of("dunwich").expect("dunwich was read");
        assert_eq!(ids(dunwich), Some(vec!["dun-1"]));
        assert_eq!(dunwich.as_of, Some(now()));
    }
}
