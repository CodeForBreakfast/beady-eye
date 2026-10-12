//! The gh:issue and gh:release gates each configured project's tracker holds
//! open, and what GitHub says has become of what each waits on.
//!
//! Whoever waits makes the gate with `bd gate create --type=gh:issue
//! --await-id=<number>`, or `--type=gh:release` with no await id, and writes
//! the repository into the gate's `repo` metadata. A gh:issue gate settles
//! once its issue closes, whatever the reason. A gh:release gate settles once
//! its repository publishes a release after the gate was made.

use std::fmt;

use chrono::{DateTime, Utc};

use crate::collect::github::{self, Release};
use crate::collect::run::{RunFailure, Runner};
use crate::model::gate::{self, Fault};
use crate::model::types::Bead;

/// One open gh:issue or gh:release gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamGate {
    pub id: String,
    /// The repository the gate names, whether or not it can be settled.
    pub repo: Option<String>,
    /// What the gate waits on, or every reason the gate cannot say.
    pub awaits: Result<Upstream, Vec<Fault>>,
}

/// What a gh:issue or gh:release gate waits on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Upstream {
    /// An issue closing.
    Issue { repo: String, number: u64 },
    /// A release published after `after`, when the gate was made.
    Release { repo: String, after: DateTime<Utc> },
}

/// What GitHub said of the thing an [`Upstream`] asks after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    IssueClosed(bool),
    Published(Vec<Release>),
}

impl UpstreamGate {
    /// `gate` as what it waits on, where it is a gh:issue or gh:release gate.
    pub(crate) fn of(gate: &Bead) -> Option<Self> {
        let repo = gate::repo(gate);
        let awaits = match gate.value("await_type").as_deref() {
            Some(gate::ISSUE) => match (repo, gate::number(gate)) {
                (Some(repo), Ok(number)) => Ok(Upstream::Issue {
                    repo: repo.to_string(),
                    number,
                }),
                (repo, number) => Err(faults(repo, number.err())),
            },
            Some(gate::RELEASE) => match (repo, gate.created_at) {
                (Some(repo), Some(after)) => Ok(Upstream::Release {
                    repo: repo.to_string(),
                    after,
                }),
                (repo, after) => Err(faults(repo, after.is_none().then_some(Fault::Undated))),
            },
            _ => return None,
        };
        Some(UpstreamGate {
            id: gate.id.clone(),
            repo: repo.map(str::to_string),
            awaits,
        })
    }
}

fn faults(repo: Option<&str>, other: Option<Fault>) -> Vec<Fault> {
    repo.is_none()
        .then_some(Fault::NoRepo)
        .into_iter()
        .chain(other)
        .collect()
}

impl Upstream {
    pub fn repo(&self) -> &str {
        match self {
            Upstream::Issue { repo, .. } | Upstream::Release { repo, .. } => repo,
        }
    }

    /// Whether this asks GitHub the same as `other`, so one answer does for
    /// both. GitHub reads a repository's name in any case.
    pub fn asks_as(&self, other: &Upstream) -> bool {
        let same_number = match (self, other) {
            (Upstream::Issue { number: a, .. }, Upstream::Issue { number: b, .. }) => a == b,
            (Upstream::Release { .. }, Upstream::Release { .. }) => true,
            _ => false,
        };
        same_number && self.repo().eq_ignore_ascii_case(other.repo())
    }

    /// What GitHub says of what this waits on.
    pub fn ask(&self, gh: &dyn Runner) -> Result<Answer, RunFailure> {
        let repo = github::repository(self.repo())?;
        match self {
            Upstream::Issue { number, .. } => {
                github::issue_closed(gh, &repo, *number).map(Answer::IssueClosed)
            }
            Upstream::Release { .. } => {
                github::published_releases(gh, &repo).map(Answer::Published)
            }
        }
    }

    /// What happened that ends the wait, as `answer` says, or nothing while
    /// the wait goes on. Of several releases published since the gate was
    /// made, the first ends it.
    pub fn ended(&self, answer: &Answer) -> Option<String> {
        match (self, answer) {
            (Upstream::Issue { .. }, Answer::IssueClosed(true)) => Some(format!("{self} closed")),
            (Upstream::Release { repo, after }, Answer::Published(releases)) => releases
                .iter()
                .filter(|release| release.published > *after)
                .min_by_key(|release| release.published)
                .map(|release| format!("{repo} published {}", release.tag)),
            _ => None,
        }
    }
}

impl fmt::Display for Upstream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Upstream::Issue { repo, number } => write!(f, "{repo}#{number}"),
            Upstream::Release { repo, .. } => write!(f, "{repo}'s releases"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::bd::parse_beads;

    const GATES: &str = include_str!("../../tests/fixtures/bd_1.3.0_gate_list_upstream.json");

    fn gates() -> Vec<Option<UpstreamGate>> {
        parse_beads(GATES)
            .expect("the capture parses")
            .iter()
            .map(UpstreamGate::of)
            .collect()
    }

    fn at(when: &str) -> DateTime<Utc> {
        when.parse().expect("a time")
    }

    #[test]
    fn each_gh_issue_and_gh_release_gate_says_what_it_waits_on_or_why_it_cannot() {
        assert_eq!(
            gates(),
            [
                Some(UpstreamGate {
                    id: "ark-6dq".to_string(),
                    repo: None,
                    awaits: Err(vec![Fault::NoRepo]),
                }),
                Some(UpstreamGate {
                    id: "ark-8dq".to_string(),
                    repo: Some("example/ark".to_string()),
                    awaits: Ok(Upstream::Release {
                        repo: "example/ark".to_string(),
                        after: at("2026-10-12T07:32:28Z"),
                    }),
                }),
                Some(UpstreamGate {
                    id: "ark-zl0".to_string(),
                    repo: Some("example/ark".to_string()),
                    awaits: Ok(Upstream::Issue {
                        repo: "example/ark".to_string(),
                        number: 31,
                    }),
                }),
            ]
        );
    }

    #[test]
    fn a_gh_pr_gate_is_no_upstream_gate() {
        let gh_pr = include_str!("../../tests/fixtures/bd_1.3.0_gate_list.json");
        let none: Vec<Option<UpstreamGate>> = parse_beads(gh_pr)
            .expect("the capture parses")
            .iter()
            .map(UpstreamGate::of)
            .collect();

        assert!(none.iter().all(Option::is_none), "{none:?}");
    }

    fn release(tag: &str, published: &str) -> Release {
        Release {
            tag: tag.to_string(),
            published: at(published),
        }
    }

    fn waiting_since(after: &str) -> Upstream {
        Upstream::Release {
            repo: "example/ark".to_string(),
            after: at(after),
        }
    }

    #[test]
    fn the_first_release_published_after_the_gate_was_made_ends_its_wait() {
        let published = Answer::Published(vec![
            release("v1.3.0", "2026-10-14T09:00:00Z"),
            release("v1.2.0", "2026-10-13T09:00:00Z"),
            release("v1.1.0", "2026-10-11T09:00:00Z"),
        ]);

        assert_eq!(
            waiting_since("2026-10-12T07:32:28Z").ended(&published),
            Some("example/ark published v1.2.0".to_string())
        );
    }

    #[test]
    fn a_release_published_before_the_gate_was_made_does_not_end_its_wait() {
        let published = Answer::Published(vec![release("v1.1.0", "2026-10-11T09:00:00Z")]);

        assert_eq!(
            waiting_since("2026-10-12T07:32:28Z").ended(&published),
            None
        );
    }

    #[test]
    fn an_issue_ends_its_wait_once_it_has_closed() {
        let issue = Upstream::Issue {
            repo: "example/ark".to_string(),
            number: 31,
        };

        assert_eq!(
            issue.ended(&Answer::IssueClosed(true)),
            Some("example/ark#31 closed".to_string())
        );
        assert_eq!(issue.ended(&Answer::IssueClosed(false)), None);
    }

    #[test]
    fn gates_ask_github_the_same_only_of_the_same_issue_or_the_same_repositorys_releases() {
        let issue = |repo: &str, number| Upstream::Issue {
            repo: repo.to_string(),
            number,
        };
        let releases = |repo: &str, after| Upstream::Release {
            repo: repo.to_string(),
            after: at(after),
        };

        assert!(issue("example/ark", 31).asks_as(&issue("Example/Ark", 31)));
        assert!(!issue("example/ark", 31).asks_as(&issue("example/ark", 32)));
        assert!(!issue("example/ark", 31).asks_as(&releases("example/ark", "2026-10-12T07:32:28Z")));
        assert!(releases("example/ark", "2026-10-12T07:32:28Z")
            .asks_as(&releases("example/ark", "2026-10-01T00:00:00Z")));
        assert!(!releases("example/ark", "2026-10-12T07:32:28Z")
            .asks_as(&releases("example/arc", "2026-10-12T07:32:28Z")));
    }
}
