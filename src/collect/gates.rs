//! The gh:pr gates each configured project's tracker holds open: the pull
//! request each one waits on, and the beads it holds back.
//!
//! Whoever opens a pull request makes its gate with `bd gate create
//! --type=gh:pr --await-id=<number>` and writes the repository into the
//! gate's `repo` metadata. This reads those gates, and settles the ones
//! waiting on a pull request GitHub says has finished.

use std::collections::BTreeSet;
use std::fmt;

use crate::collect::bd::{Cli, Settling};
use crate::collect::github::{self, State};
use crate::collect::run::{RunFailure, Runner};
use crate::collect::tracker::OpenFailure;
use crate::config::Project;
use crate::model::types::Bead;

/// The `await_type` bd gives a gate that waits on a pull request.
const PULL_REQUEST: &str = "gh:pr";

/// One open gh:pr gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrGate {
    pub id: String,
    /// The pull request the gate waits on, or every reason the gate cannot
    /// name one.
    pub awaits: Result<PullRequest, Vec<Fault>>,
    /// The beads the gate holds back.
    pub blocks: Vec<String>,
}

/// A pull request, as a gate names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
    /// `OWNER/REPO`, or `HOST/OWNER/REPO`, as the gate's metadata holds it.
    pub repo: String,
    pub number: u64,
}

/// Why a gate cannot name the pull request it waits on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    /// The gate's metadata holds no `repo`.
    NoRepo,
    /// The gate holds no await id.
    NoAwaitId,
    /// The gate's await id is not a pull request's number.
    AwaitIdNotANumber(String),
}

/// One configured project's open gh:pr gates, or why its tracker did not
/// give them.
///
/// Expected dead for the reason `across` is.
#[derive(Debug)]
#[cfg_attr(not(feature = "testing"), expect(dead_code))]
pub struct ProjectGates {
    pub project: String,
    pub gates: Result<Vec<PrGate>, OpenFailure>,
}

/// Every configured project's open gh:pr gates. A project whose tracker
/// does not answer is reported with its failure, and the rest are read.
///
/// Nothing `bdi` runs reads the gates yet. The expectation stands in for the
/// first caller and fails the build on the change that adds one.
#[cfg_attr(not(feature = "testing"), expect(dead_code))]
pub fn across(cli: &Cli, projects: &[Project]) -> Vec<ProjectGates> {
    projects
        .iter()
        .map(|project| ProjectGates {
            project: project.name.clone(),
            gates: cli.pr_gates(project),
        })
        .collect()
}

/// What settling the gates waiting on one pull request came to.
#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(not(feature = "testing"), expect(dead_code))]
pub enum Settled {
    /// GitHub did not say where the pull request stands, so no tracker was
    /// asked anything.
    Unread(RunFailure),
    /// The pull request is still open, so no tracker was asked anything.
    Open,
    /// The pull request merged or closed, and this is what each configured
    /// project did about it.
    Finished(Vec<ProjectSettled>),
}

/// What one configured project did about a finished pull request, or why
/// its tracker did not say which gates wait on it.
#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(not(feature = "testing"), expect(dead_code))]
pub struct ProjectSettled {
    pub project: String,
    pub acts: Result<Vec<Act>, OpenFailure>,
}

/// One write settling asked for, on the bead it was asked of.
#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(not(feature = "testing"), expect(dead_code))]
pub struct Act {
    pub bead: String,
    /// What was done, or the failed call that left the bead as it was.
    pub done: Result<Done, RunFailure>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Done {
    /// The gate was closed, so the beads it held back are free of it.
    Resolved,
    /// The waiting bead was told the pull request closed unmerged.
    Commented,
    /// The waiting bead had already been told, by an earlier settling.
    AlreadyCommented,
}

/// Re-read `pr` from GitHub, then settle every open gh:pr gate waiting on it
/// in each of `projects`. A merge closes each gate, and a close without one
/// leaves each open and comments once on every bead it holds back.
///
/// Correct however many times it runs: a closed gate is no longer read, and
/// a bead already told is not told again. GitHub is read before any tracker,
/// so a pull request GitHub cannot answer for leaves every tracker untouched.
///
/// Expected dead for the reason `across` is.
#[cfg_attr(not(feature = "testing"), expect(dead_code))]
pub fn settle(cli: &Cli, gh: &dyn Runner, projects: &[Project], pr: &PullRequest) -> Settled {
    match github::state(gh, pr) {
        Err(failure) => Settled::Unread(failure),
        Ok(State::Open) => Settled::Open,
        Ok(State::Merged { commit }) => {
            let reason = match commit {
                Some(commit) => format!("Pull request {pr} merged as {commit}."),
                None => format!("Pull request {pr} merged."),
            };
            each_project(cli, projects, pr, |tracker, waiting| {
                waiting
                    .iter()
                    .map(|gate| Act {
                        bead: gate.id.clone(),
                        done: tracker.resolve(&gate.id, &reason).map(|()| Done::Resolved),
                    })
                    .collect()
            })
        }
        Ok(State::Closed) => {
            let told = format!(
                "Pull request {pr} closed without being merged, so the gh:pr gate waiting on \
                 it stays open."
            );
            each_project(cli, projects, pr, |tracker, waiting| {
                let held_back: BTreeSet<&String> =
                    waiting.iter().flat_map(|gate| &gate.blocks).collect();
                held_back
                    .into_iter()
                    .map(|bead| Act {
                        bead: bead.clone(),
                        done: tell(tracker, bead, &told),
                    })
                    .collect()
            })
        }
    }
}

/// What each of `projects` did with the open gates waiting on `pr`, read
/// afresh from its tracker.
fn each_project(
    cli: &Cli,
    projects: &[Project],
    pr: &PullRequest,
    act: impl Fn(&Settling, &[PrGate]) -> Vec<Act>,
) -> Settled {
    Settled::Finished(
        projects
            .iter()
            .map(|project| ProjectSettled {
                project: project.name.clone(),
                acts: cli.settling(project).and_then(|tracker| {
                    let waiting: Vec<PrGate> = tracker
                        .pr_gates()?
                        .into_iter()
                        .filter(|gate| gate.awaits.as_ref().is_ok_and(|awaits| awaits.is(pr)))
                        .collect();
                    Ok(act(&tracker, &waiting))
                }),
            })
            .collect(),
    )
}

/// Comment `told` on `bead` unless it already carries it.
fn tell(tracker: &Settling, bead: &str, told: &str) -> Result<Done, RunFailure> {
    if tracker
        .comments(bead)?
        .iter()
        .any(|comment| comment == told)
    {
        return Ok(Done::AlreadyCommented);
    }
    tracker.comment(bead, told).map(|()| Done::Commented)
}

impl PullRequest {
    /// Whether this is `other`. GitHub reads a repository's name in any
    /// case, so a gate can spell it differently from GitHub and still mean it.
    fn is(&self, other: &PullRequest) -> bool {
        self.number == other.number && self.repo.eq_ignore_ascii_case(&other.repo)
    }
}

impl fmt::Display for PullRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.repo, self.number)
    }
}

/// Whether `gate` waits on a pull request, whichever one that is.
pub(crate) fn awaits_a_pull_request(gate: &Bead) -> bool {
    gate.value("await_type") == Some(PULL_REQUEST)
}

impl PrGate {
    /// `gate` as the pull request it waits on, holding back `blocks`.
    pub(crate) fn of(gate: &Bead, blocks: Vec<String>) -> Self {
        let repo = gate.metadata.get("repo").filter(|repo| !repo.is_empty());
        let number = match gate.value("await_id") {
            None => Err(Fault::NoAwaitId),
            Some(id) => id
                .parse::<u64>()
                .map_err(|_| Fault::AwaitIdNotANumber(id.to_string())),
        };
        let awaits = match (repo, number) {
            (Some(repo), Ok(number)) => Ok(PullRequest {
                repo: repo.clone(),
                number,
            }),
            (repo, number) => Err(repo
                .is_none()
                .then_some(Fault::NoRepo)
                .into_iter()
                .chain(number.err())
                .collect()),
        };
        PrGate {
            id: gate.id.clone(),
            awaits,
            blocks,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::bd::parse_beads;
    use crate::collect::run::testing::FakeRunner;
    use crate::collect::run::{FailureKind, RunFailure};
    use std::path::PathBuf;

    const GATE_LIST: &str = include_str!("../../tests/fixtures/bd_1.3.0_gate_list.json");

    /// Each gh:pr gate in `GATE_LIST`, beside what `bd dep list` answers for
    /// the beads it holds back.
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

    /// A bd call against `project`'s tracker, as the runner spells it.
    fn spelled(project: &str, subcommand: &str) -> String {
        format!("bd -C /nowhere/{project} --readonly {subcommand}")
    }

    fn gate_list(project: &str) -> String {
        spelled(project, "gate list --limit 0 --json")
    }

    fn held_back_by(project: &str, gate: &str) -> String {
        spelled(
            project,
            &format!("dep list {gate} --direction=up --type blocks --json"),
        )
    }

    /// A runner answering for `project` with the captured tracker.
    fn captured(runner: FakeRunner, project: &str) -> FakeRunner {
        HELD_BACK.iter().fold(
            runner.with(&gate_list(project), GATE_LIST),
            |runner, (gate, held)| runner.with(&held_back_by(project, gate), held),
        )
    }

    fn read(runner: &FakeRunner, projects: &[Project]) -> Vec<ProjectGates> {
        across(&Cli::new(runner), projects)
    }

    fn the_gate<'g>(gates: &'g [PrGate], id: &str) -> &'g PrGate {
        gates
            .iter()
            .find(|gate| gate.id == id)
            .unwrap_or_else(|| panic!("{id} was read"))
    }

    #[test]
    fn a_captured_tracker_gives_each_open_pull_request_gate_and_the_beads_it_holds_back() {
        let runner = captured(FakeRunner::default(), "arkham");

        let read = read(&runner, &[project("arkham")]);

        let gates = read[0].gates.as_ref().expect("the tracker answered");
        let ids: Vec<&str> = gates.iter().map(|gate| gate.id.as_str()).collect();
        assert_eq!(ids, ["ark-6pp", "ark-tg0", "ark-eb1", "ark-0i5"]);
        assert_eq!(
            the_gate(gates, "ark-0i5"),
            &PrGate {
                id: "ark-0i5".to_string(),
                awaits: Ok(PullRequest {
                    repo: "example/ark".to_string(),
                    number: 42,
                }),
                blocks: vec!["ark-qca".to_string()],
            }
        );
        assert_eq!(
            the_gate(gates, "ark-eb1").blocks,
            ["ark-2ud", "ark-45c"],
            "one gate can hold back more than one bead"
        );
    }

    /// The capture holds a human gate as well, and nothing is asked about
    /// the beads it holds back.
    #[test]
    fn a_gate_waiting_on_anything_but_a_pull_request_is_not_read() {
        let runner = captured(FakeRunner::default(), "arkham");

        let read = read(&runner, &[project("arkham")]);

        let gates = read[0].gates.as_ref().expect("the tracker answered");
        assert!(gates.iter().all(|gate| gate.id != "ark-77f"));
        assert!(runner
            .calls()
            .iter()
            .all(|call| !call.argv.contains("ark-77f")));
    }

    #[test]
    fn a_gate_with_no_repo_is_reported_as_having_none() {
        let runner = captured(FakeRunner::default(), "arkham");

        let read = read(&runner, &[project("arkham")]);

        let gates = read[0].gates.as_ref().expect("the tracker answered");
        assert_eq!(
            the_gate(gates, "ark-6pp"),
            &PrGate {
                id: "ark-6pp".to_string(),
                awaits: Err(vec![Fault::NoRepo]),
                blocks: vec!["ark-92q".to_string()],
            }
        );
    }

    #[test]
    fn a_gate_whose_await_id_is_not_a_number_is_reported_with_it() {
        let runner = captured(FakeRunner::default(), "arkham");

        let read = read(&runner, &[project("arkham")]);

        let gates = read[0].gates.as_ref().expect("the tracker answered");
        assert_eq!(
            the_gate(gates, "ark-tg0").awaits,
            Err(vec![Fault::AwaitIdNotANumber("the-ninth".to_string())])
        );
    }

    /// A gh:pr gate as bd writes its row, holding `await_id` and `repo` where
    /// they are given.
    fn gate(await_id: Option<&str>, repo: Option<&str>) -> Bead {
        let mut row = serde_json::json!({
            "id": "ark-g1",
            "title": "Gate: gh:pr",
            "status": "open",
            "issue_type": "gate",
            "await_type": PULL_REQUEST,
        });
        if let Some(id) = await_id {
            row["await_id"] = id.into();
        }
        if let Some(repo) = repo {
            row["metadata"] = serde_json::json!({ "repo": repo });
        }
        parse_beads(&serde_json::json!([row]).to_string())
            .expect("the row parses")
            .remove(0)
    }

    /// bd makes a gh:pr gate without an await id when it is given none.
    #[test]
    fn a_gate_with_no_await_id_is_reported_as_having_none() {
        assert_eq!(
            PrGate::of(&gate(None, Some("example/ark")), Vec::new()).awaits,
            Err(vec![Fault::NoAwaitId])
        );
    }

    #[test]
    fn a_gate_wrong_both_ways_is_reported_both_ways() {
        assert_eq!(
            PrGate::of(&gate(Some("the-ninth"), None), Vec::new()).awaits,
            Err(vec![
                Fault::NoRepo,
                Fault::AwaitIdNotANumber("the-ninth".to_string())
            ])
        );
    }

    #[test]
    fn an_empty_repo_is_no_repo() {
        assert_eq!(
            PrGate::of(&gate(Some("42"), Some("")), Vec::new()).awaits,
            Err(vec![Fault::NoRepo])
        );
    }

    #[test]
    fn an_unreachable_tracker_is_reported_and_the_other_projects_are_still_read() {
        let runner = captured(FakeRunner::default(), "dunwich").failing(
            &gate_list("arkham"),
            RunFailure {
                kind: FailureKind::Unavailable,
                program: "bd".to_string(),
                detail: "bd could not reach the tracker".to_string(),
                unreadable: None,
            },
        );

        let read = read(&runner, &[project("arkham"), project("dunwich")]);

        assert_eq!(read[0].project, "arkham");
        match &read[0].gates {
            Err(OpenFailure::Refused(failure)) => {
                assert_eq!(failure.kind, FailureKind::Unavailable)
            }
            answered => panic!("arkham's tracker answered {answered:?}"),
        }
        assert_eq!(read[1].project, "dunwich");
        assert_eq!(read[1].gates.as_ref().expect("dunwich answered").len(), 4);
    }

    #[test]
    fn a_tracker_that_cannot_say_which_beads_a_gate_holds_back_is_reported() {
        let runner = captured(FakeRunner::default(), "arkham").failing(
            &held_back_by("arkham", "ark-eb1"),
            RunFailure::parse("bd", "not JSON"),
        );

        let read = read(&runner, &[project("arkham")]);

        match &read[0].gates {
            Err(OpenFailure::Refused(failure)) => assert_eq!(failure.kind, FailureKind::Parse),
            answered => panic!("arkham's tracker answered {answered:?}"),
        }
    }

    const MERGED: &str = include_str!("../../tests/fixtures/gh_2.102.0_pr_view_merged.json");
    const CLOSED: &str = include_str!("../../tests/fixtures/gh_2.102.0_pr_view_closed.json");
    const OPEN: &str = include_str!("../../tests/fixtures/gh_2.102.0_pr_view_open.json");
    const NO_COMMENTS: &str = include_str!("../../tests/fixtures/bd_1.3.0_comments_none.json");
    /// ark-2ud's comments once it has been told example/ark#7 closed
    /// unmerged, beside a comment of its own.
    const TOLD: &str = include_str!("../../tests/fixtures/bd_1.3.0_comments_told_ark-2ud.json");

    /// In the captured tracker, ark-0i5 waits on #42 and holds back ark-qca,
    /// and ark-eb1 waits on #7 and holds back ark-2ud and ark-45c.
    fn pr(number: u64) -> PullRequest {
        PullRequest {
            repo: "example/ark".to_string(),
            number,
        }
    }

    fn viewed(number: u64) -> String {
        format!("gh pr view {number} --repo example/ark --json state,mergeCommit")
    }

    /// A write to `project`'s tracker, as the runner spells it.
    fn written(project: &str, subcommand: &str) -> String {
        format!("bd -C /nowhere/{project} {subcommand}")
    }

    fn comments_on(project: &str, bead: &str) -> String {
        spelled(project, &format!("comments {bead} --json"))
    }

    const MERGED_REASON: &str =
        "Pull request example/ark#42 merged as 5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff.";

    fn resolving_42(project: &str) -> String {
        written(
            project,
            &format!("gate resolve ark-0i5 --reason {MERGED_REASON}"),
        )
    }

    const CLOSED_TOLD: &str = "Pull request example/ark#7 closed without being merged, so the \
                               gh:pr gate waiting on it stays open.";

    fn telling(project: &str, bead: &str) -> String {
        written(project, &format!("comments add {bead} {CLOSED_TOLD}"))
    }

    /// The captured gate list as bd prints it once `gate` is closed: `bd
    /// gate list` reads open gates alone.
    fn gate_list_without(gate: &str) -> String {
        let mut rows: Vec<serde_json::Value> =
            serde_json::from_str(GATE_LIST).expect("the capture parses");
        rows.retain(|row| row["id"] != gate);
        serde_json::to_string(&rows).expect("the rows print")
    }

    fn settled(runner: &FakeRunner, projects: &[Project], pr: &PullRequest) -> Settled {
        settle(&Cli::new(runner), runner, projects, pr)
    }

    /// Every bd call that was not a read.
    fn writes(runner: &FakeRunner) -> Vec<String> {
        runner
            .calls()
            .into_iter()
            .map(|call| call.argv)
            .filter(|argv| argv.starts_with("bd ") && !argv.contains(" --readonly "))
            .collect()
    }

    fn acts(settled: Settled) -> Vec<Act> {
        match settled {
            Settled::Finished(mut projects) => {
                assert_eq!(projects.len(), 1, "one project was configured");
                let project = projects.remove(0);
                assert_eq!(project.project, "arkham");
                project.acts.expect("arkham's tracker answered")
            }
            not_finished => panic!("settling came to {not_finished:?}"),
        }
    }

    fn act(bead: &str, done: Done) -> Act {
        Act {
            bead: bead.to_string(),
            done: Ok(done),
        }
    }

    #[test]
    fn a_merge_closes_the_gate_waiting_on_it_with_a_reason_naming_the_merge() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&viewed(42), MERGED)
            .with(&resolving_42("arkham"), "✓ Gate resolved: ark-0i5\n");

        let settled = settled(&runner, &[project("arkham")], &pr(42));

        assert_eq!(acts(settled), [act("ark-0i5", Done::Resolved)]);
        assert_eq!(writes(&runner), [resolving_42("arkham")]);
    }

    #[test]
    fn a_merge_settled_again_writes_nothing_because_its_gate_is_closed() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&gate_list("arkham"), &gate_list_without("ark-0i5"))
            .with(&viewed(42), MERGED);

        let settled = settled(&runner, &[project("arkham")], &pr(42));

        assert_eq!(acts(settled), []);
        assert_eq!(writes(&runner), Vec::<String>::new());
    }

    #[test]
    fn a_merge_github_names_no_commit_for_still_closes_the_gate() {
        let reason = written(
            "arkham",
            "gate resolve ark-0i5 --reason Pull request example/ark#42 merged.",
        );
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&viewed(42), r#"{"mergeCommit":null,"state":"MERGED"}"#)
            .with(&reason, "");

        let settled = settled(&runner, &[project("arkham")], &pr(42));

        assert_eq!(acts(settled), [act("ark-0i5", Done::Resolved)]);
        assert_eq!(writes(&runner), [reason]);
    }

    #[test]
    fn a_close_without_a_merge_comments_on_each_held_back_bead_and_leaves_the_gate_open() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&viewed(7), CLOSED)
            .with(&comments_on("arkham", "ark-2ud"), NO_COMMENTS)
            .with(&comments_on("arkham", "ark-45c"), NO_COMMENTS)
            .with(&telling("arkham", "ark-2ud"), "Comment added to ark-2ud\n")
            .with(&telling("arkham", "ark-45c"), "Comment added to ark-45c\n");

        let settled = settled(&runner, &[project("arkham")], &pr(7));

        assert_eq!(
            acts(settled),
            [
                act("ark-2ud", Done::Commented),
                act("ark-45c", Done::Commented)
            ]
        );
        assert_eq!(
            writes(&runner),
            [telling("arkham", "ark-2ud"), telling("arkham", "ark-45c")]
        );
    }

    #[test]
    fn a_close_settled_again_adds_no_second_comment() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&viewed(7), CLOSED)
            .with(&comments_on("arkham", "ark-2ud"), TOLD)
            .with(&comments_on("arkham", "ark-45c"), NO_COMMENTS)
            .with(&telling("arkham", "ark-45c"), "Comment added to ark-45c\n");

        let settled = settled(&runner, &[project("arkham")], &pr(7));

        assert_eq!(
            acts(settled),
            [
                act("ark-2ud", Done::AlreadyCommented),
                act("ark-45c", Done::Commented)
            ]
        );
        assert_eq!(writes(&runner), [telling("arkham", "ark-45c")]);
    }

    /// The runner panics on any call it was not given, so a tracker asked
    /// anything at all fails the test.
    #[test]
    fn an_open_pull_request_asks_no_tracker_anything_however_often_it_is_settled() {
        let runner = FakeRunner::default().with(&viewed(42), OPEN);

        for _ in 0..2 {
            assert_eq!(
                settled(&runner, &[project("arkham")], &pr(42)),
                Settled::Open
            );
        }
    }

    #[test]
    fn a_gate_naming_its_repository_in_another_case_still_waits_on_the_pull_request() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(
                "gh pr view 42 --repo Example/Ark --json state,mergeCommit",
                MERGED,
            )
            .with(
                &written(
                    "arkham",
                    "gate resolve ark-0i5 --reason Pull request Example/Ark#42 merged as \
                     5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff.",
                ),
                "",
            );

        let settled = settled(
            &runner,
            &[project("arkham")],
            &PullRequest {
                repo: "Example/Ark".to_string(),
                number: 42,
            },
        );

        assert_eq!(acts(settled), [act("ark-0i5", Done::Resolved)]);
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
    fn a_pull_request_github_does_not_answer_for_is_reported_and_no_tracker_is_asked() {
        let runner = FakeRunner::default().failing(&viewed(42), unavailable("gh"));

        assert_eq!(
            settled(&runner, &[project("arkham")], &pr(42)),
            Settled::Unread(unavailable("gh"))
        );
    }

    #[test]
    fn a_gate_bd_fails_to_close_is_reported() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&viewed(42), MERGED)
            .failing(&resolving_42("arkham"), unavailable("bd"));

        let settled = settled(&runner, &[project("arkham")], &pr(42));

        assert_eq!(
            acts(settled),
            [Act {
                bead: "ark-0i5".to_string(),
                done: Err(unavailable("bd")),
            }]
        );
    }

    #[test]
    fn a_bead_whose_comments_bd_cannot_read_is_reported_and_not_commented_on() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&viewed(7), CLOSED)
            .failing(&comments_on("arkham", "ark-2ud"), unavailable("bd"))
            .with(&comments_on("arkham", "ark-45c"), NO_COMMENTS)
            .with(&telling("arkham", "ark-45c"), "");

        let settled = settled(&runner, &[project("arkham")], &pr(7));

        assert_eq!(
            acts(settled),
            [
                Act {
                    bead: "ark-2ud".to_string(),
                    done: Err(unavailable("bd")),
                },
                act("ark-45c", Done::Commented)
            ]
        );
        assert_eq!(writes(&runner), [telling("arkham", "ark-45c")]);
    }

    #[test]
    fn a_comment_bd_fails_to_add_is_reported() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&viewed(7), CLOSED)
            .with(&comments_on("arkham", "ark-2ud"), NO_COMMENTS)
            .with(&comments_on("arkham", "ark-45c"), TOLD)
            .failing(&telling("arkham", "ark-2ud"), unavailable("bd"));

        let settled = settled(&runner, &[project("arkham")], &pr(7));

        assert_eq!(
            acts(settled),
            [
                Act {
                    bead: "ark-2ud".to_string(),
                    done: Err(unavailable("bd")),
                },
                act("ark-45c", Done::AlreadyCommented)
            ]
        );
    }

    #[test]
    fn a_tracker_that_does_not_say_which_gates_wait_is_reported_and_the_others_are_settled() {
        let runner = captured(FakeRunner::default(), "dunwich")
            .failing(&gate_list("arkham"), unavailable("bd"))
            .with(&viewed(42), MERGED)
            .with(&resolving_42("dunwich"), "");

        let settled = settled(&runner, &[project("arkham"), project("dunwich")], &pr(42));

        assert_eq!(
            settled,
            Settled::Finished(vec![
                ProjectSettled {
                    project: "arkham".to_string(),
                    acts: Err(OpenFailure::Refused(unavailable("bd"))),
                },
                ProjectSettled {
                    project: "dunwich".to_string(),
                    acts: Ok(vec![act("ark-0i5", Done::Resolved)]),
                },
            ])
        );
        assert_eq!(writes(&runner), [resolving_42("dunwich")]);
    }
}
