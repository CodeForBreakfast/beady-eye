//! One look at every configured project's gh:pr gates, which `bdi gates`
//! takes on its poll: each pull request they wait on is settled once, where
//! its repository's owner is one this `bdi gates` settles for. A delivery
//! from GitHub settles the one pull request it names, by the same rule.

use chrono::{DateTime, Utc};

use crate::app::tracker::{open_failure, tracker_failure};
use crate::collect::bd::Cli;
use crate::collect::gates::{self, Awaited, Done, PrGate, PullRequest, Settled, Told, Waiting};
use crate::collect::github;
use crate::collect::pr_events::EVENTS;
use crate::collect::run::{FailureKind, RunFailure, Runner};
use crate::collect::webhook::Commit;
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
    /// A gate cannot name the pull request it waits on, or what it waits for
    /// that pull request to do, so nothing settles it.
    Unsettleable {
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
    /// GitHub did not say which open pull requests a commit heads, so a
    /// delivery about its checks settled none.
    CommitUnread { commit: Commit, failure: RunFailure },
    /// GitHub refused to say where a pull request stands for the rate limit
    /// of the login gh runs as, so nothing more is asked of it until
    /// `resets`, or for a while where GitHub does not say when that is.
    RateLimited {
        pull_request: PullRequest,
        resets: Option<DateTime<Utc>>,
    },
    /// A write settling a pull request asked of one bead, for what the pull
    /// request did, and what came of it.
    Settling {
        pull_request: PullRequest,
        happening: &'static str,
        project: String,
        bead: String,
        done: Result<Done, TrackerFailure>,
    },
}

/// What GitHub made of the reads one settling asked of it, taken together:
/// one it answered outweighs any it refused.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Read {
    /// GitHub was asked nothing.
    #[default]
    Unasked,
    /// GitHub refused every read it was asked, whether for its rate limit or
    /// for any other reason.
    Refused,
    /// GitHub answered at least one read.
    Answered,
}

/// What one settling, a look or a delivery, came to.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Settling {
    pub found: Vec<Found>,
    pub github: Read,
}

/// Read every open gh:pr gate across `projects`, then settle each pull
/// request one of them waits on, once however many gates wait on it, asking
/// GitHub about each repository's together, until GitHub refuses one for its
/// rate limit. A gate whose repository `settles` leaves to another
/// `bdi gates` is passed over without a word. No tracker is asked about a
/// pull request that has done nothing new to the gates waiting on it, which
/// `told` says of the beads they hold back.
pub fn look(
    cli: &Cli,
    gh: &dyn Runner,
    projects: &[Project],
    settles: &Gates,
    told: &Told,
) -> Settling {
    let mut found = Vec::new();
    let mut awaited: Vec<Vec<Awaited>> = Vec::new();
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
                Err(faults) => found.push(Found::Unsettleable {
                    project: read.project.clone(),
                    gate: gate.id,
                    faults,
                }),
                Ok(wait) => awaiting(
                    &mut awaited,
                    wait.pull_request,
                    Waiting {
                        project: read.project.clone(),
                        until: wait.until,
                        blocks: gate.blocks,
                    },
                ),
            }
        }
    }
    let mut github = Read::Unasked;
    for repository in &awaited {
        let settled = gates::settle_together(cli, gh, projects, &EVENTS, repository, told);
        for (each, settled) in repository.iter().zip(settled) {
            let each = findings(gh, each.pull_request.clone(), settled);
            found.extend(each.found);
            github = github.max(each.github);
            if matches!(found.last(), Some(Found::RateLimited { .. })) {
                return Settling { found, github };
            }
        }
    }
    Settling { found, github }
}

/// Add `gate` to those waiting on `pull_request`, grouped by repository.
/// GitHub reads a repository's name in any case, so neither comparison heeds
/// it.
fn awaiting(awaited: &mut Vec<Vec<Awaited>>, pull_request: PullRequest, gate: Waiting) {
    let repository = match awaited.iter().position(|repository| {
        repository[0]
            .pull_request
            .repo
            .eq_ignore_ascii_case(&pull_request.repo)
    }) {
        Some(at) => &mut awaited[at],
        None => {
            awaited.push(Vec::new());
            awaited.last_mut().expect("one was just pushed")
        }
    };
    match repository
        .iter_mut()
        .find(|seen| seen.pull_request.is(&pull_request))
    {
        Some(seen) => seen.waiting.push(gate),
        None => repository.push(Awaited {
            pull_request,
            waiting: vec![gate],
        }),
    }
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
    told: &Told,
) -> Settling {
    if !settles.settles(gate::owner(&pull_request.repo)) {
        return Settling::default();
    }
    let settled = gates::settle(cli, gh, projects, &EVENTS, &pull_request, told);
    findings(gh, pull_request, settled)
}

/// Settle the open pull requests whose head is the commit a delivery about
/// its checks named, each as [`delivered`] settles one. A lookup the rate
/// limit refuses is reported like any other and does not put off the asks
/// after it, which the next one finds refused again.
pub fn delivered_commit(
    cli: &Cli,
    gh: &dyn Runner,
    projects: &[Project],
    settles: &Gates,
    commit: Commit,
    told: &Told,
) -> Settling {
    let Some(repository) = gate::repository(&commit.repo) else {
        return Settling::default();
    };
    if !settles.settles(Some(repository.owner)) {
        return Settling::default();
    }
    let numbers = match github::open_pull_requests_headed_by(gh, &repository, &commit.sha) {
        Ok(numbers) => numbers,
        Err(failure) => {
            return Settling {
                found: vec![Found::CommitUnread { commit, failure }],
                github: Read::Refused,
            }
        }
    };
    let mut settling = Settling {
        found: Vec::new(),
        github: Read::Answered,
    };
    for number in numbers {
        let pull_request = PullRequest {
            repo: commit.repo.clone(),
            number,
        };
        let each = delivered(cli, gh, projects, settles, pull_request, told);
        settling.found.extend(each.found);
        settling.github = settling.github.max(each.github);
        if matches!(settling.found.last(), Some(Found::RateLimited { .. })) {
            break;
        }
    }
    settling
}

/// What settling `pull_request` found to report, and whether GitHub answered.
fn findings(gh: &dyn Runner, pull_request: PullRequest, settled: Settled) -> Settling {
    let github = match settled {
        Settled::Unread(_) => Read::Refused,
        Settled::NothingNew | Settled::Acted(_) => Read::Answered,
    };
    let found = match settled {
        Settled::NothingNew => Vec::new(),
        Settled::Unread(failure) if failure.kind == FailureKind::RateLimited => {
            let resets = github::spent_until(gh, gate::host(&pull_request.repo));
            vec![Found::RateLimited {
                pull_request,
                resets: resets.ok().flatten(),
            }]
        }
        Settled::Unread(failure) => vec![Found::GitHubUnread {
            pull_request,
            failure,
        }],
        Settled::Acted(each) => each
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
                        happening: act.happening,
                        project: settled.project.clone(),
                        bead: act.bead,
                        done: act.done.map_err(|failure| tracker_failure(&failure)),
                    })
                    .collect(),
            })
            .collect(),
    };
    Settling { found, github }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::run::testing::FakeRunner;
    use std::path::PathBuf;

    const GATE_LIST: &str = include_str!("../../tests/fixtures/bd_1.3.0_gate_list.json");
    const MERGED: &str = r#"{"state":"MERGED","isDraft":false,"mergeCommit":{"oid":"5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff"}}"#;
    /// #7 open and #42 merged, as the one query a look asks reads them.
    const QUERIED: &str =
        include_str!("../../tests/fixtures/gh_2.102.0_api_graphql_ark_42_merged.json");

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
        captured_in(runner, project, "ark")
    }

    /// A runner answering for `project` with the captured tracker, its gates
    /// naming the repository `example/<name>`.
    fn captured_in(runner: FakeRunner, project: &str, name: &str) -> FakeRunner {
        let gates = GATE_LIST.replace("\"example/ark\"", &format!("\"example/{name}\""));
        HELD_BACK.iter().fold(
            runner.with(&gate_list(project), &gates),
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

    /// The query a settling of #`number` alone asks.
    fn viewed(number: u64) -> String {
        format!(
            "gh api graphql -f owner=example -f name=ark -f query=query($owner:String!,\
             $name:String!){{repository(owner:$owner,name:$name){{pr{number}:pullRequest\
             (number:{number}){{state isDraft mergeCommit{{oid}}}}}}}}"
        )
    }

    /// GitHub's answer to [`viewed`]: #`number` with `fields`.
    fn answer(number: u64, fields: &str) -> String {
        format!(r#"{{"data":{{"repository":{{"pr{number}":{fields}}}}}}}"#)
    }

    /// The one query a look asks about #7 and #42 in `example/<name>`.
    fn queried(name: &str) -> String {
        format!(
            "gh api graphql -f owner=example -f name={name} -f query=query($owner:String!,\
             $name:String!){{repository(owner:$owner,name:$name){{pr7:pullRequest(number:7)\
             {{state isDraft mergeCommit{{oid}}}} pr42:pullRequest(number:42){{state isDraft mergeCommit{{oid}}}}}}}}"
        )
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
            happening: "merged",
            project: project.to_string(),
            bead: "ark-0i5".to_string(),
            done: Ok(Done::Resolved),
        }
    }

    fn unsettleable(project: &str, gate: &str, fault: Fault) -> Found {
        Found::Unsettleable {
            project: project.to_string(),
            gate: gate.to_string(),
            faults: vec![fault],
        }
    }

    fn no_number(project: &str) -> Found {
        unsettleable(
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

    fn excluding(owners: &[&str]) -> Gates {
        Gates {
            excluded_owners: owners.iter().map(|owner| owner.to_string()).collect(),
            ..Gates::default()
        }
    }

    fn looked(runner: &FakeRunner, projects: &[Project], settles: &Gates) -> Settling {
        look(
            &Cli::new(runner),
            runner,
            projects,
            settles,
            &Told::default(),
        )
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
            .with(&queried("ark"), QUERIED)
            .with(&resolving_42("arkham"), "")
            .with(&resolving_42("dunwich"), "");

        let found = looked(
            &runner,
            &[project("arkham"), project("dunwich")],
            &Gates::default(),
        )
        .found;

        assert_eq!(asked_github(&runner), [queried("ark")]);
        assert_eq!(
            found,
            [
                unsettleable("arkham", "ark-6pp", Fault::NoRepo),
                no_number("arkham"),
                unsettleable("dunwich", "ark-6pp", Fault::NoRepo),
                no_number("dunwich"),
                settling("arkham"),
                settling("dunwich"),
            ]
        );
    }

    fn gate_lists_read(runner: &FakeRunner) -> usize {
        runner
            .calls()
            .into_iter()
            .filter(|call| call.argv == gate_list("arkham"))
            .count()
    }

    /// #7 is open and ready for review, and no gate waits for that.
    #[test]
    fn a_look_reads_the_gates_again_only_for_the_pull_request_that_merged() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&queried("ark"), QUERIED)
            .with(&resolving_42("arkham"), "");

        looked(&runner, &[project("arkham")], &Gates::default());

        assert_eq!(gate_lists_read(&runner), 2);
    }

    #[test]
    fn a_look_closes_a_gate_waiting_for_its_pull_request_to_leave_draft() {
        let mut rows: Vec<serde_json::Value> =
            serde_json::from_str(GATE_LIST).expect("the capture parses");
        for row in rows.iter_mut().filter(|row| row["id"] == "ark-eb1") {
            row["metadata"][gate::AWAITS] = "ready_for_review".into();
        }
        let gates = serde_json::to_string(&rows).expect("the rows print");
        let resolving_7 = "bd -C /nowhere/arkham gate resolve ark-eb1 --reason Pull request \
                           example/ark#7 is ready for review.";
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&gate_list("arkham"), &gates)
            .with(&queried("ark"), QUERIED)
            .with(resolving_7, "")
            .with(&resolving_42("arkham"), "");

        let found = looked(&runner, &[project("arkham")], &Gates::default()).found;

        assert_eq!(
            found,
            [
                unsettleable("arkham", "ark-6pp", Fault::NoRepo),
                no_number("arkham"),
                Found::Settling {
                    pull_request: pr(7),
                    happening: "is ready for review",
                    project: "arkham".to_string(),
                    bead: "ark-eb1".to_string(),
                    done: Ok(Done::Resolved),
                },
                settling("arkham"),
            ]
        );
    }

    /// The runner panics on any call it was not given, so a gate whose held
    /// back beads were asked after, or that was settled, would fail the test.
    #[test]
    fn a_gate_whose_owner_this_bdi_gates_does_not_settle_for_is_passed_over_without_a_word() {
        let runner = FakeRunner::default().with(&gate_list("arkham"), GATE_LIST);

        let Settling { found, github } =
            looked(&runner, &[project("arkham")], &owners(&["miskatonic"]));

        assert_eq!(found, []);
        assert_eq!(asked_github(&runner), Vec::<String>::new());
        assert_eq!(github, Read::Unasked);
    }

    #[test]
    fn an_owner_is_matched_in_any_case() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&queried("ark"), QUERIED)
            .with(&resolving_42("arkham"), "");

        let found = looked(&runner, &[project("arkham")], &owners(&["Example"])).found;

        assert_eq!(
            found,
            [no_number("arkham"), settling("arkham")],
            "ark-6pp names no repo, so no owner, and is left to a bdi gates settling every owner's"
        );
    }

    #[test]
    fn a_bdi_gates_leaving_out_one_owner_settles_the_gates_of_an_owner_it_was_never_told_about() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&queried("ark"), QUERIED)
            .with(&resolving_42("arkham"), "");

        let found = looked(&runner, &[project("arkham")], &excluding(&["miskatonic"])).found;

        assert_eq!(
            found,
            [
                unsettleable("arkham", "ark-6pp", Fault::NoRepo),
                no_number("arkham"),
                settling("arkham"),
            ],
            "ark-6pp names no owner, so none is left out and it is settled here"
        );
    }

    #[test]
    fn a_gate_whose_owner_is_left_out_is_passed_over_without_a_word() {
        let runner = captured(FakeRunner::default(), "arkham");

        let Settling { found, github } =
            looked(&runner, &[project("arkham")], &excluding(&["Example"]));

        assert_eq!(
            found,
            [unsettleable("arkham", "ark-6pp", Fault::NoRepo)],
            "ark-6pp names no owner, so none is left out and it is reported here"
        );
        assert_eq!(asked_github(&runner), Vec::<String>::new());
        assert_eq!(github, Read::Unasked);
    }

    fn delivered_here(runner: &FakeRunner, settles: &Gates, number: u64) -> Settling {
        delivered(
            &Cli::new(runner),
            runner,
            &[project("arkham")],
            settles,
            pr(number),
            &Told::default(),
        )
    }

    const SHA: &str = "5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff";
    const HEADED_BY_SHA: &str =
        "gh api repos/example/ark/commits/5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff/pulls?per_page=100";

    fn commit() -> Commit {
        Commit {
            repo: "example/ark".to_string(),
            sha: SHA.to_string(),
        }
    }

    fn delivered_commit_here(runner: &FakeRunner, settles: &Gates) -> Settling {
        delivered_commit(
            &Cli::new(runner),
            runner,
            &[project("arkham")],
            settles,
            commit(),
            &Told::default(),
        )
    }

    /// #42 open with the commit as head, #43 open without it, #44 closed
    /// with it.
    const PULLS_OF_SHA: &str = r#"[
        {"number":42,"state":"open","head":{"sha":"5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff"}},
        {"number":43,"state":"open","head":{"sha":"0badc0de0badc0de0badc0de0badc0de0badc0de"}},
        {"number":44,"state":"closed","head":{"sha":"5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff"}}
    ]"#;

    #[test]
    fn a_delivery_about_a_commit_settles_each_open_pull_request_it_heads_and_no_other() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(HEADED_BY_SHA, PULLS_OF_SHA)
            .with(&viewed(42), &answer(42, MERGED))
            .with(&resolving_42("arkham"), "");

        let Settling { found, github } = delivered_commit_here(&runner, &owners(&["example"]));

        assert_eq!(found, [settling("arkham")]);
        assert_eq!(
            asked_github(&runner),
            [HEADED_BY_SHA.to_string(), viewed(42)]
        );
        assert_eq!(github, Read::Answered);
    }

    #[test]
    fn a_delivery_about_a_commit_no_open_pull_request_heads_asks_nothing_more() {
        let runner = FakeRunner::default().with(HEADED_BY_SHA, "[]");

        let Settling { found, github } = delivered_commit_here(&runner, &Gates::default());

        assert_eq!(found, []);
        assert_eq!(asked_github(&runner), [HEADED_BY_SHA.to_string()]);
        assert_eq!(github, Read::Answered);
    }

    #[test]
    fn a_commit_github_cannot_list_the_pull_requests_of_is_reported_and_settles_nothing() {
        let runner = FakeRunner::default().failing(HEADED_BY_SHA, unavailable("gh"));

        let Settling { found, github } = delivered_commit_here(&runner, &Gates::default());

        assert_eq!(
            found,
            [Found::CommitUnread {
                commit: commit(),
                failure: unavailable("gh"),
            }]
        );
        assert_eq!(github, Read::Refused);
    }

    #[test]
    fn a_delivery_about_a_commit_in_a_repository_this_bdi_gates_does_not_settle_for_asks_nothing() {
        let runner = FakeRunner::default();

        let Settling { found, github } = delivered_commit_here(&runner, &owners(&["miskatonic"]));

        assert_eq!(found, []);
        assert_eq!(runner.calls(), []);
        assert_eq!(github, Read::Unasked);
    }

    #[test]
    fn a_delivery_settles_the_pull_request_it_names_and_no_other() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&viewed(42), &answer(42, MERGED))
            .with(&resolving_42("arkham"), "");

        let found = delivered_here(&runner, &owners(&["example"]), 42).found;

        assert_eq!(found, [settling("arkham")]);
        assert_eq!(asked_github(&runner), [viewed(42)]);
    }

    /// GitHub is asked about it, as the settling of any pull request asks
    /// first, and the tracker is asked which gates wait on it. The runner
    /// panics on any call it was not given, so a write would fail the test.
    #[test]
    fn a_delivery_for_a_merged_pull_request_no_gate_waits_on_writes_nothing() {
        let runner = captured(FakeRunner::default(), "arkham").with(&viewed(9), &answer(9, MERGED));

        let Settling { found, github } = delivered_here(&runner, &Gates::default(), 9);

        assert_eq!(found, []);
        assert_eq!(asked_github(&runner), [viewed(9)]);
        assert_eq!(github, Read::Answered);
    }

    #[test]
    fn a_delivery_whose_owner_this_bdi_gates_does_not_settle_for_is_passed_over_without_a_word() {
        let runner = FakeRunner::default();

        let Settling { found, github } = delivered_here(&runner, &owners(&["miskatonic"]), 42);

        assert_eq!(found, []);
        assert_eq!(runner.calls(), []);
        assert_eq!(github, Read::Unasked);
    }

    #[test]
    fn a_delivery_whose_owner_is_left_out_is_passed_over_without_a_word() {
        let runner = FakeRunner::default();

        let Settling { found, github } = delivered_here(&runner, &excluding(&["example"]), 42);

        assert_eq!(found, []);
        assert_eq!(runner.calls(), []);
        assert_eq!(github, Read::Unasked);
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
    fn each_repository_is_asked_of_github_once_and_each_one_settled() {
        let runner = captured_in(
            captured(FakeRunner::default(), "arkham"),
            "dunwich",
            "vault",
        )
        .with(&queried("ark"), QUERIED)
        .with(
            &queried("vault"),
            &QUERIED.replace("\"MERGED\"", "\"OPEN\""),
        )
        .with(&resolving_42("arkham"), "");

        let found = looked(
            &runner,
            &[project("arkham"), project("dunwich")],
            &owners(&["example"]),
        )
        .found;

        assert_eq!(asked_github(&runner), [queried("ark"), queried("vault")]);
        assert_eq!(
            found,
            [
                no_number("arkham"),
                no_number("dunwich"),
                settling("arkham")
            ]
        );
    }

    /// A failure that is not about a missing pull request would come back the
    /// same for each one asked alone, so none is asked again.
    #[test]
    fn a_repository_github_does_not_answer_for_is_reported_for_each_of_its_pull_requests() {
        let runner =
            captured(FakeRunner::default(), "arkham").failing(&queried("ark"), unavailable("gh"));

        let Settling { found, github } =
            looked(&runner, &[project("arkham")], &owners(&["example"]));

        let unread = |number| Found::GitHubUnread {
            pull_request: pr(number),
            failure: unavailable("gh"),
        };
        assert_eq!(found, [no_number("arkham"), unread(7), unread(42)]);
        assert_eq!(asked_github(&runner), [queried("ark")]);
        assert_eq!(github, Read::Refused);
    }

    fn gone() -> RunFailure {
        RunFailure {
            kind: FailureKind::Gone,
            program: "gh".to_string(),
            detail: "gh found no such repository or pull request on GitHub".to_string(),
            unreadable: None,
        }
    }

    #[test]
    fn a_pull_request_github_does_not_have_is_reported_and_the_rest_of_its_repository_settled() {
        let runner = captured(FakeRunner::default(), "arkham")
            .failing(&queried("ark"), gone())
            .failing(&viewed(7), gone())
            .with(&viewed(42), &answer(42, MERGED))
            .with(&resolving_42("arkham"), "");

        let Settling { found, github } =
            looked(&runner, &[project("arkham")], &owners(&["example"]));

        assert_eq!(
            found,
            [
                no_number("arkham"),
                Found::GitHubUnread {
                    pull_request: pr(7),
                    failure: gone(),
                },
                settling("arkham"),
            ]
        );
        assert_eq!(
            asked_github(&runner),
            [queried("ark"), viewed(7), viewed(42)]
        );
        assert_eq!(
            github,
            Read::Answered,
            "one read answered outweighs one refused"
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
    /// example/vault after example/ark was refused would fail the test.
    #[test]
    fn a_look_refused_for_a_rate_limit_asks_github_nothing_more_and_says_when_it_resets() {
        let runner = captured_in(
            captured(FakeRunner::default(), "arkham"),
            "dunwich",
            "vault",
        )
        .failing(&queried("ark"), rate_limited())
        .with(RATE_LIMIT, GRAPHQL_SPENT);

        let Settling { found, github } = looked(
            &runner,
            &[project("arkham"), project("dunwich")],
            &owners(&["example"]),
        );

        assert_eq!(
            found,
            [
                no_number("arkham"),
                no_number("dunwich"),
                Found::RateLimited {
                    pull_request: pr(7),
                    resets: DateTime::from_timestamp(1767227400, 0),
                },
            ]
        );
        assert_eq!(
            asked_github(&runner),
            [queried("ark"), RATE_LIMIT.to_string()]
        );
        assert_eq!(github, Read::Refused);
    }

    #[test]
    fn a_rate_limit_github_does_not_say_the_end_of_is_reported_without_one() {
        let runner = captured(FakeRunner::default(), "arkham")
            .failing(&viewed(42), rate_limited())
            .failing(RATE_LIMIT, unavailable("gh"));

        let found = delivered_here(&runner, &owners(&["example"]), 42).found;

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
            .with(&queried("ark"), QUERIED)
            .with(&resolving_42("dunwich"), "");

        let found = looked(
            &runner,
            &[project("arkham"), project("dunwich")],
            &owners(&["example"]),
        )
        .found;

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

    /// #7 closed unmerged and #42 open, as the one query a look asks reads
    /// them.
    fn queried_7_closed() -> String {
        include_str!("../../tests/fixtures/gh_2.102.0_api_graphql_ark_open.json")
            .replace(r#""pr7":{"state":"OPEN""#, r#""pr7":{"state":"CLOSED""#)
    }

    fn telling_7(bead: &str) -> String {
        format!(
            "bd -C /nowhere/arkham comments add {bead} Pull request example/ark#7 closed without \
             being merged, so the gh:pr gate waiting on it stays open."
        )
    }

    fn told_7(bead: &str) -> Found {
        Found::Settling {
            pull_request: pr(7),
            happening: "closed unmerged",
            project: "arkham".to_string(),
            bead: bead.to_string(),
            done: Ok(Done::Commented),
        }
    }

    /// The runner panics on any call it was not given, and is given no
    /// comments to read the second time, so a second look that asked any
    /// bead for its comments would fail the test.
    #[test]
    fn a_second_look_at_a_pull_request_closed_unmerged_asks_no_bead_for_its_comments() {
        let no_comments = include_str!("../../tests/fixtures/bd_1.3.0_comments_none.json");
        let first = captured(FakeRunner::default(), "arkham")
            .with(&queried("ark"), &queried_7_closed())
            .with(&read("arkham", "comments ark-2ud --json"), no_comments)
            .with(&read("arkham", "comments ark-45c --json"), no_comments)
            .with(&telling_7("ark-2ud"), "")
            .with(&telling_7("ark-45c"), "");
        let second =
            captured(FakeRunner::default(), "arkham").with(&queried("ark"), &queried_7_closed());
        let told = Told::default();
        let look_with = |runner: &FakeRunner| {
            look(
                &Cli::new(runner),
                runner,
                &[project("arkham")],
                &owners(&["example"]),
                &told,
            )
            .found
        };

        let found_first = look_with(&first);
        let found_second = look_with(&second);

        assert_eq!(
            found_first,
            [no_number("arkham"), told_7("ark-2ud"), told_7("ark-45c")]
        );
        assert_eq!(found_second, [no_number("arkham")]);
        assert_eq!(gate_lists_read(&second), 1);
    }
}
