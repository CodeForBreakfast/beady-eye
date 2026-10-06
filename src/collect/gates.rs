//! The gh:pr gates each configured project's tracker holds open: the pull
//! request each one waits on, and the beads it holds back.
//!
//! Whoever opens a pull request makes its gate with `bd gate create
//! --type=gh:pr --await-id=<number>` and writes the repository into the
//! gate's `repo` metadata. This reads those gates and acts on none of them.

use crate::collect::bd::Cli;
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
}
