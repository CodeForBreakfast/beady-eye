//! One look at every configured project's gh:pr gates, which `bdi gates`
//! takes on its poll: each pull request they wait on is settled once, where
//! its repository's owner is one this `bdi gates` settles for. A delivery
//! from GitHub settles the one pull request it names, by the same rule.

use chrono::{DateTime, Utc};

use crate::app::tracker::{open_failure, tracker_failure};
use crate::collect::bd::Cli;
use crate::collect::gates::{self, Done, PrGate, PullRequest, Settled};
use crate::collect::github;
use crate::collect::run::{FailureKind, RunFailure, Runner};
use crate::config::{Gates, Project};
use crate::model::gate::{self, Fault};
use crate::model::snapshot::TrackerFailure;

/// Something a look at the gates found to report.
#[derive(Debug, PartialEq, Eq)]
pub enum Found {
    /// A project's tracker did not say which gh:pr gates it holds open.
    TrackerUnread {
        project: String,
        failure: TrackerFailure,
    },
    /// A gate cannot name the pull request it waits on, so nothing settles it.
    NoPullRequest {
        project: String,
        gate: String,
        faults: Vec<Fault>,
    },
    /// GitHub did not say where a pull request stands, so no gate waiting on
    /// it was touched.
    GitHubUnread {
        pull_request: PullRequest,
        failure: RunFailure,
    },
    /// GitHub refused to say where a pull request stands for the rate limit
    /// of the login gh runs as, so nothing more is asked of it until
    /// `resets`, or for a while where GitHub does not say when that is.
    RateLimited {
        pull_request: PullRequest,
        resets: Option<DateTime<Utc>>,
    },
    /// A write settling a finished pull request asked of one bead, and what
    /// came of it.
    Settling {
        pull_request: PullRequest,
        project: String,
        bead: String,
        done: Result<Done, TrackerFailure>,
    },
}

/// Read every open gh:pr gate across `projects`, then settle each pull
/// request one of them waits on, once however many gates wait on it, until
/// GitHub refuses one for its rate limit. A gate whose repository `settles`
/// leaves to another `bdi gates` is passed over without a word.
pub fn look(cli: &Cli, gh: &dyn Runner, projects: &[Project], settles: &Gates) -> Vec<Found> {
    let mut found = Vec::new();
    let mut awaited: Vec<PullRequest> = Vec::new();
    let settled_here = |gate: &PrGate| settles.settles(gate.repo.as_deref().and_then(gate::owner));
    for read in gates::across(cli, projects, settled_here) {
        let open = match read.gates {
            Ok(open) => open,
            Err(failure) => {
                found.push(Found::TrackerUnread {
                    project: read.project,
                    failure: open_failure(&failure),
                });
                continue;
            }
        };
        for gate in open {
            match gate.awaits {
                Err(faults) => found.push(Found::NoPullRequest {
                    project: read.project.clone(),
                    gate: gate.id,
                    faults,
                }),
                Ok(pull_request) => {
                    if !awaited.iter().any(|seen| seen.is(&pull_request)) {
                        awaited.push(pull_request);
                    }
                }
            }
        }
    }
    for pull_request in awaited {
        found.extend(settled(cli, gh, projects, pull_request));
        if matches!(found.last(), Some(Found::RateLimited { .. })) {
            break;
        }
    }
    found
}

/// Settle the one pull request a delivery named, as a look settles each one
/// it finds. A pull request whose repository `settles` leaves to another
/// `bdi gates` is passed over without a word.
pub fn delivered(
    cli: &Cli,
    gh: &dyn Runner,
    projects: &[Project],
    settles: &Gates,
    pull_request: PullRequest,
) -> Vec<Found> {
    if !settles.settles(gate::owner(&pull_request.repo)) {
        return Vec::new();
    }
    settled(cli, gh, projects, pull_request)
}

/// What settling `pull_request` across `projects` found to report.
fn settled(
    cli: &Cli,
    gh: &dyn Runner,
    projects: &[Project],
    pull_request: PullRequest,
) -> Vec<Found> {
    match gates::settle(cli, gh, projects, &pull_request) {
        Settled::Open => Vec::new(),
        Settled::Unread(failure) if failure.kind == FailureKind::RateLimited => {
            vec![Found::RateLimited {
                pull_request,
                resets: github::spent_until(gh).ok().flatten(),
            }]
        }
        Settled::Unread(failure) => vec![Found::GitHubUnread {
            pull_request,
            failure,
        }],
        Settled::Finished(each) => each
            .into_iter()
            .flat_map(|settled| match settled.acts {
                Err(failure) => vec![Found::TrackerUnread {
                    project: settled.project,
                    failure: open_failure(&failure),
                }],
                Ok(acts) => acts
                    .into_iter()
                    .map(|act| Found::Settling {
                        pull_request: pull_request.clone(),
                        project: settled.project.clone(),
                        bead: act.bead,
                        done: act.done.map_err(|failure| tracker_failure(&failure)),
                    })
                    .collect(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::run::testing::FakeRunner;
    use std::path::PathBuf;

    const GATE_LIST: &str = include_str!("../../tests/fixtures/bd_1.3.0_gate_list.json");
    const MERGED: &str = include_str!("../../tests/fixtures/gh_2.102.0_pr_view_merged.json");
    const OPEN: &str = include_str!("../../tests/fixtures/gh_2.102.0_pr_view_open.json");

    /// Each gh:pr gate in `GATE_LIST`, beside what `bd dep list` answers for
    /// the beads it holds back. ark-0i5 waits on example/ark#42, ark-eb1 on
    /// example/ark#7, ark-6pp names no repo and ark-tg0 no number.
    const HELD_BACK: [(&str, &str); 4] = [
        (
            "ark-0i5",
            include_str!("../../tests/fixtures/bd_1.3.0_dep_list_up_ark-0i5.json"),
        ),
        (
            "ark-eb1",
            include_str!("../../tests/fixtures/bd_1.3.0_dep_list_up_ark-eb1.json"),
        ),
        (
            "ark-6pp",
            include_str!("../../tests/fixtures/bd_1.3.0_dep_list_up_ark-6pp.json"),
        ),
        (
            "ark-tg0",
            include_str!("../../tests/fixtures/bd_1.3.0_dep_list_up_ark-tg0.json"),
        ),
    ];

    fn project(name: &str) -> Project {
        Project {
            name: name.to_string(),
            path: PathBuf::from(format!("/nowhere/{name}")),
            environment_command: None,
            credential_command: None,
            prefix: None,
            poll: true,
            events_journal: false,
            badges: Vec::new(),
            worktrees: Vec::new(),
        }
    }

    fn read(project: &str, subcommand: &str) -> String {
        format!("bd -C /nowhere/{project} --readonly {subcommand}")
    }

    fn gate_list(project: &str) -> String {
        read(project, "gate list --limit 0 --json")
    }

    /// A runner answering for `project` with the captured tracker.
    fn captured(runner: FakeRunner, project: &str) -> FakeRunner {
        HELD_BACK.iter().fold(
            runner.with(&gate_list(project), GATE_LIST),
            |runner, (gate, held)| {
                runner.with(
                    &read(
                        project,
                        &format!("dep list {gate} --direction=up --type blocks --json"),
                    ),
                    held,
                )
            },
        )
    }

    fn viewed(number: u64) -> String {
        format!("gh pr view {number} --repo example/ark --json state,mergeCommit")
    }

    fn resolving_42(project: &str) -> String {
        format!(
            "bd -C /nowhere/{project} gate resolve ark-0i5 --reason Pull request example/ark#42 \
             merged as 5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff."
        )
    }

    fn pr(number: u64) -> PullRequest {
        PullRequest {
            repo: "example/ark".to_string(),
            number,
        }
    }

    fn settling(project: &str) -> Found {
        Found::Settling {
            pull_request: pr(42),
            project: project.to_string(),
            bead: "ark-0i5".to_string(),
            done: Ok(Done::Resolved),
        }
    }

    fn no_pull_request(project: &str, gate: &str, fault: Fault) -> Found {
        Found::NoPullRequest {
            project: project.to_string(),
            gate: gate.to_string(),
            faults: vec![fault],
        }
    }

    fn no_number(project: &str) -> Found {
        no_pull_request(
            project,
            "ark-tg0",
            Fault::AwaitIdNotANumber("the-ninth".to_string()),
        )
    }

    fn owners(owners: &[&str]) -> Gates {
        Gates {
            owners: owners.iter().map(|owner| owner.to_string()).collect(),
            ..Gates::default()
        }
    }

    fn looked(runner: &FakeRunner, projects: &[Project], settles: &Gates) -> Vec<Found> {
        look(&Cli::new(runner), runner, projects, settles)
    }

    fn asked_github(runner: &FakeRunner) -> Vec<String> {
        runner
            .calls()
            .into_iter()
            .map(|call| call.argv)
            .filter(|argv| argv.starts_with("gh "))
            .collect()
    }

    #[test]
    fn a_pull_request_gates_in_two_projects_wait_on_is_asked_of_github_once_and_settled_in_both() {
        let runner = captured(captured(FakeRunner::default(), "arkham"), "dunwich")
            .with(&viewed(42), MERGED)
            .with(&viewed(7), OPEN)
            .with(&resolving_42("arkham"), "")
            .with(&resolving_42("dunwich"), "");

        let found = looked(
            &runner,
            &[project("arkham"), project("dunwich")],
            &Gates::default(),
        );

        assert_eq!(asked_github(&runner), [viewed(7), viewed(42)]);
        assert_eq!(
            found,
            [
                no_pull_request("arkham", "ark-6pp", Fault::NoRepo),
                no_number("arkham"),
                no_pull_request("dunwich", "ark-6pp", Fault::NoRepo),
                no_number("dunwich"),
                settling("arkham"),
                settling("dunwich"),
            ]
        );
    }

    /// The runner panics on any call it was not given, so a gate whose held
    /// back beads were asked after, or that was settled, would fail the test.
    #[test]
    fn a_gate_whose_owner_this_bdi_gates_does_not_settle_for_is_passed_over_without_a_word() {
        let runner = FakeRunner::default().with(&gate_list("arkham"), GATE_LIST);

        let found = looked(&runner, &[project("arkham")], &owners(&["miskatonic"]));

        assert_eq!(found, []);
        assert_eq!(asked_github(&runner), Vec::<String>::new());
    }

    #[test]
    fn an_owner_is_matched_in_any_case() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&viewed(42), MERGED)
            .with(&viewed(7), OPEN)
            .with(&resolving_42("arkham"), "");

        let found = looked(&runner, &[project("arkham")], &owners(&["Example"]));

        assert_eq!(
            found,
            [no_number("arkham"), settling("arkham")],
            "ark-6pp names no repo, so no owner, and is left to a bdi gates settling every owner's"
        );
    }

    fn delivered_here(runner: &FakeRunner, settles: &Gates, number: u64) -> Vec<Found> {
        delivered(
            &Cli::new(runner),
            runner,
            &[project("arkham")],
            settles,
            pr(number),
        )
    }

    #[test]
    fn a_delivery_settles_the_pull_request_it_names_and_no_other() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&viewed(42), MERGED)
            .with(&resolving_42("arkham"), "");

        let found = delivered_here(&runner, &owners(&["example"]), 42);

        assert_eq!(found, [settling("arkham")]);
        assert_eq!(asked_github(&runner), [viewed(42)]);
    }

    /// GitHub is asked about it, as the settling of any pull request asks
    /// first, and the tracker is asked which gates wait on it. The runner
    /// panics on any call it was not given, so a write would fail the test.
    #[test]
    fn a_delivery_for_a_merged_pull_request_no_gate_waits_on_writes_nothing() {
        let runner = captured(FakeRunner::default(), "arkham").with(&viewed(9), MERGED);

        let found = delivered_here(&runner, &Gates::default(), 9);

        assert_eq!(found, []);
        assert_eq!(asked_github(&runner), [viewed(9)]);
    }

    #[test]
    fn a_delivery_whose_owner_this_bdi_gates_does_not_settle_for_is_passed_over_without_a_word() {
        let runner = FakeRunner::default();

        let found = delivered_here(&runner, &owners(&["miskatonic"]), 42);

        assert_eq!(found, []);
        assert_eq!(runner.calls(), []);
    }

    fn unavailable(program: &str) -> RunFailure {
        RunFailure {
            kind: FailureKind::Unavailable,
            program: program.to_string(),
            detail: format!("{program} exited 1 for a reason bdi cannot place"),
            unreadable: None,
        }
    }

    #[test]
    fn a_pull_request_github_does_not_answer_for_is_reported_and_the_rest_are_settled() {
        let runner = captured(FakeRunner::default(), "arkham")
            .failing(&viewed(7), unavailable("gh"))
            .with(&viewed(42), MERGED)
            .with(&resolving_42("arkham"), "");

        let found = looked(&runner, &[project("arkham")], &owners(&["example"]));

        assert_eq!(
            found,
            [
                no_number("arkham"),
                Found::GitHubUnread {
                    pull_request: pr(7),
                    failure: unavailable("gh"),
                },
                settling("arkham"),
            ]
        );
    }

    fn rate_limited() -> RunFailure {
        RunFailure {
            kind: FailureKind::RateLimited,
            program: "gh".to_string(),
            detail: "gh was refused for GitHub's rate limit".to_string(),
            unreadable: None,
        }
    }

    const RATE_LIMIT: &str = "gh api rate_limit";
    const GRAPHQL_SPENT: &str =
        include_str!("../../tests/fixtures/gh_2.102.0_api_rate_limit_graphql_spent.json");

    /// The runner panics on any call it was not given, so asking GitHub about
    /// #42 after #7 was refused would fail the test.
    #[test]
    fn a_look_refused_for_a_rate_limit_asks_github_nothing_more_and_says_when_it_resets() {
        let runner = captured(FakeRunner::default(), "arkham")
            .failing(&viewed(7), rate_limited())
            .with(RATE_LIMIT, GRAPHQL_SPENT);

        let found = looked(&runner, &[project("arkham")], &owners(&["example"]));

        assert_eq!(
            found,
            [
                no_number("arkham"),
                Found::RateLimited {
                    pull_request: pr(7),
                    resets: DateTime::from_timestamp(1767227400, 0),
                },
            ]
        );
        assert_eq!(asked_github(&runner), [viewed(7), RATE_LIMIT.to_string()]);
    }

    #[test]
    fn a_rate_limit_github_does_not_say_the_end_of_is_reported_without_one() {
        let runner = captured(FakeRunner::default(), "arkham")
            .failing(&viewed(42), rate_limited())
            .failing(RATE_LIMIT, unavailable("gh"));

        let found = delivered_here(&runner, &owners(&["example"]), 42);

        assert_eq!(
            found,
            [Found::RateLimited {
                pull_request: pr(42),
                resets: None,
            }]
        );
    }

    #[test]
    fn a_tracker_that_does_not_say_which_gates_it_holds_is_reported_and_the_rest_are_settled() {
        let runner = captured(FakeRunner::default(), "dunwich")
            .failing(&gate_list("arkham"), unavailable("bd"))
            .with(&viewed(42), MERGED)
            .with(&viewed(7), OPEN)
            .with(&resolving_42("dunwich"), "");

        let found = looked(
            &runner,
            &[project("arkham"), project("dunwich")],
            &owners(&["example"]),
        );

        let arkham_unread = || Found::TrackerUnread {
            project: "arkham".to_string(),
            failure: TrackerFailure::Unavailable,
        };
        assert_eq!(
            found,
            [
                arkham_unread(),
                no_number("dunwich"),
                arkham_unread(),
                settling("dunwich"),
            ],
            "settling #42 asks arkham again, and arkham is unread again"
        );
    }
}
