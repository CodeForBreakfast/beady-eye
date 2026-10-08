//! The gh:pr gates each configured project's tracker holds open: the pull
//! request each one waits on, and the beads it holds back.
//!
//! Whoever opens a pull request makes its gate with `bd gate create
//! --type=gh:pr --await-id=<number>` and writes the repository into the
//! gate's `repo` metadata. This reads those gates, and settles the ones
//! waiting on a pull request GitHub says has done what they wait for.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fmt;

use chrono::{DateTime, Utc};

use crate::collect::bd::{Cli, Settling};
use crate::collect::github::{self, Observed};
use crate::collect::pr_events::{self, Event, Outcome};
use crate::collect::run::{FailureKind, RunFailure, Runner};
use crate::collect::tracker::OpenFailure;
use crate::config::Project;
use crate::model::gate::{self, Fault, Until};
use crate::model::types::Bead;

/// One open gh:pr gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrGate {
    pub id: String,
    /// The repository the gate names, whether or not it names a pull request
    /// in it.
    pub repo: Option<String>,
    /// The pull request the gate waits on and what it waits for it to do, or
    /// every reason the gate cannot say.
    pub awaits: Result<Wait, Vec<Fault>>,
    /// The beads the gate holds back.
    pub blocks: Vec<String>,
    /// When the gate was made, where the tracker says.
    pub made: Option<DateTime<Utc>>,
}

/// What a gate waits for, and of which pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wait {
    pub pull_request: PullRequest,
    pub until: Until,
}

/// A pull request, as a gate names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
    /// `OWNER/REPO`, or `HOST/OWNER/REPO`, as the gate's metadata holds it.
    pub repo: String,
    pub number: u64,
}

/// One configured project's open gh:pr gates, or why its tracker did not
/// give them.
#[derive(Debug)]
pub struct ProjectGates {
    pub project: String,
    pub gates: Result<Vec<PrGate>, OpenFailure>,
}

/// Every configured project's open gh:pr gates that are `wanted`. A project
/// whose tracker does not answer is reported with its failure, and the rest
/// are read.
pub fn across(
    cli: &Cli,
    projects: &[Project],
    wanted: impl Fn(&PrGate) -> bool,
) -> Vec<ProjectGates> {
    projects
        .iter()
        .map(|project| ProjectGates {
            project: project.name.clone(),
            gates: cli.pr_gates(project, &wanted),
        })
        .collect()
}

/// What settling the gates waiting on one pull request came to.
#[derive(Debug, PartialEq, Eq)]
pub enum Settled {
    /// GitHub did not say where the pull request stands, so no tracker was
    /// asked anything.
    Unread(RunFailure),
    /// Nothing the pull request has done that GitHub let `bdi gates` see is
    /// new to a gate waiting on it, so no tracker was asked anything.
    NothingNew { unseen: Vec<Unseen> },
    /// The pull request did something new to a gate waiting on it, and this
    /// is what each configured project did about it.
    Acted {
        projects: Vec<ProjectSettled>,
        unseen: Vec<Unseen>,
    },
}

/// An event GitHub's answer for a pull request did not let `bdi gates` see,
/// so nothing it would have done was done, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unseen {
    /// What the event would have reported the pull request did.
    pub happening: &'static str,
    pub why: String,
}

/// What one configured project did about a pull request, or why its tracker
/// did not say which gates wait on it.
#[derive(Debug, PartialEq, Eq)]
pub struct ProjectSettled {
    pub project: String,
    pub acts: Result<Vec<Act>, OpenFailure>,
}

/// One write settling asked for, on the bead it was asked of.
#[derive(Debug, PartialEq, Eq)]
pub struct Act {
    pub bead: String,
    /// What the pull request did that asked for the write, as the event
    /// says it.
    pub happening: &'static str,
    /// What was done, or the failed call that left the bead as it was.
    pub done: Result<Done, RunFailure>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Done {
    /// The gate was closed, so the beads it held back are free of it.
    Resolved,
    /// The waiting bead was told what the pull request did.
    Commented,
    /// The waiting bead had already been told, by an earlier settling.
    AlreadyCommented,
}

/// A pull request a look found gates waiting on, and those gates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Awaited {
    pub pull_request: PullRequest,
    pub waiting: Vec<Waiting>,
}

/// One gate a look found waiting on a pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiting {
    pub project: String,
    pub until: Until,
    /// The beads the gate holds back.
    pub blocks: Vec<String>,
    /// When the gate was made, where the tracker says.
    pub made: Option<DateTime<Utc>>,
}

/// Which bead has been told what, as far as this process knows. A bead it
/// has not told is asked for its comments before it is told, so a process
/// that starts afresh tells no bead twice, and one that keeps this between
/// settlings asks no tracker about a bead it has told.
// ponytail: nothing is forgotten, at one entry per bead told each thing.
// Forget the beads no look finds held back if a process ever lives long
// enough for that to matter.
#[derive(Debug, Default)]
pub struct Told(RefCell<BTreeSet<(String, String, String)>>);

impl Told {
    /// Whether `bead` in `project` is known to carry `text`. The comparison
    /// ignores case because `text` names the repository as a gate spelt it,
    /// which can differ from one settling to the next.
    fn knows(&self, project: &str, bead: &str, text: &str) -> bool {
        self.0.borrow().contains(&told(project, bead, text))
    }

    fn learn(&self, project: &str, bead: &str, text: &str) {
        self.0.borrow_mut().insert(told(project, bead, text));
    }
}

fn told(project: &str, bead: &str, text: &str) -> (String, String, String) {
    (
        project.to_string(),
        bead.to_string(),
        text.to_ascii_lowercase(),
    )
}

/// Re-read `pr` from GitHub, then settle it as [`settled`] does, acting on
/// everything `events` say it has done, since no look said which gates wait
/// on it.
pub fn settle(
    cli: &Cli,
    gh: &dyn Runner,
    projects: &[Project],
    events: &[Event],
    pr: &PullRequest,
    told: &Told,
) -> Settled {
    let observed = observe(gh, pr, &pr_events::fields(events));
    settled(cli, projects, events, pr, observed, None, told)
}

/// The most pull requests one query asks about. The query is one argument to
/// `gh`, and an argument has a length limit of its own on Linux.
const PULL_REQUESTS_PER_QUERY: usize = 100;

/// Re-read every one of `awaited`, which share a repository, from GitHub in
/// one query for each [`PULL_REQUESTS_PER_QUERY`] of them, then settle each
/// as [`settled`] does, in the order given, acting only on what is new to
/// the gates the look found waiting. A query that fails leaves the others'
/// pull requests settled.
///
/// Nothing is asked until it is wanted, so a caller that stops early asks
/// GitHub about none of the pull requests after where it stopped.
pub fn settle_together<'a>(
    cli: &'a Cli,
    gh: &'a dyn Runner,
    projects: &'a [Project],
    events: &'a [Event],
    awaited: &'a [Awaited],
    told: &'a Told,
) -> impl Iterator<Item = Settled> + 'a {
    awaited
        .chunks(PULL_REQUESTS_PER_QUERY)
        .flat_map(move |asked| settle_asked_together(cli, gh, projects, events, asked, told))
}

/// Re-read every one of `asked` in one query, then settle each.
///
/// A query naming one pull request or repository GitHub does not have fails
/// as a whole, so then each is read on its own and the rest are still
/// settled. A number past GraphQL's 32-bit `Int` is one GitHub does not
/// have: measured on gh 2.102.0, it says so rather than refusing the query.
/// Each pull request in a repository the query cannot name is read on its
/// own too.
fn settle_asked_together<'a>(
    cli: &'a Cli,
    gh: &'a dyn Runner,
    projects: &'a [Project],
    events: &'a [Event],
    asked: &'a [Awaited],
    told: &'a Told,
) -> impl Iterator<Item = Settled> + 'a {
    let fields = pr_events::fields(events);
    let numbers: Vec<u64> = asked.iter().map(|each| each.pull_request.number).collect();
    let together = asked
        .first()
        .and_then(|each| gate::repository(&each.pull_request.repo))
        .map(|repo| github::pull_requests(gh, &repo, &numbers, &fields));
    let observed: Box<dyn Iterator<Item = Result<Observed, RunFailure>> + 'a> = match together {
        Some(Ok(observed)) => Box::new(observed.into_iter()),
        Some(Err(failure)) if failure.kind != FailureKind::Gone || asked.len() == 1 => {
            Box::new(asked.iter().map(move |_| Err(failure.clone())))
        }
        _ => Box::new(
            asked
                .iter()
                .map(move |each| observe(gh, &each.pull_request, &fields)),
        ),
    };
    asked.iter().zip(observed).map(move |(each, observed)| {
        settled(
            cli,
            projects,
            events,
            &each.pull_request,
            observed,
            Some(&each.waiting),
            told,
        )
    })
}

/// `pr` as GitHub has it, asked `fields` beside its state.
fn observe(gh: &dyn Runner, pr: &PullRequest, fields: &str) -> Result<Observed, RunFailure> {
    let repo = gate::repository(&pr.repo).ok_or_else(|| RunFailure {
        kind: FailureKind::Gone,
        program: "gh".to_string(),
        detail: format!(
            "{} is not OWNER/REPO or HOST/OWNER/REPO, so GitHub has no such repository",
            pr.repo
        ),
        unreadable: None,
    })?;
    github::pull_requests(gh, &repo, &[pr.number], fields)?.remove(0)
}

/// Settle every open gh:pr gate waiting on `pr` in each of `projects`, doing
/// what each of `events` asks of what GitHub said. Where a look found the
/// gates `waiting` on `pr`, an event's outcome is acted on only where it is
/// new to them: a gate waiting for it, or a bead held back that has not been
/// told it.
///
/// Correct however many times it runs: a closed gate is no longer read, and
/// a bead already told is not told again. A pull request GitHub did not
/// answer for leaves every tracker untouched, and an event whose fields
/// GitHub did not answer is the only one passed over.
fn settled(
    cli: &Cli,
    projects: &[Project],
    events: &[Event],
    pr: &PullRequest,
    observed: Result<Observed, RunFailure>,
    waiting: Option<&[Waiting]>,
    told: &Told,
) -> Settled {
    let (outcomes, unseen) = match observed {
        Ok(observed) => outcomes(events, pr, &observed),
        Err(failure) => return Settled::Unread(failure),
    };
    let new: Vec<(&'static str, Outcome)> = outcomes
        .into_iter()
        .filter(|(_, outcome)| waiting.is_none_or(|waiting| is_new(outcome, waiting, told)))
        .collect();
    if new.is_empty() {
        return Settled::NothingNew { unseen };
    }
    Settled::Acted {
        projects: projects
            .iter()
            .map(|project| ProjectSettled {
                project: project.name.clone(),
                acts: cli.settling(project).and_then(|tracker| {
                    let gates = tracker.pr_gates_waiting(|wait| {
                        wait.pull_request.is(pr)
                            && new.iter().any(|(_, outcome)| outcome.concerns(wait.until))
                    })?;
                    Ok(acted(&tracker, &project.name, &gates, &new, told))
                }),
            })
            .collect(),
        unseen,
    }
}

/// What each of `events` makes of `pr`, beside what it did, as the event
/// says it, and each event GitHub's answer did not let it see.
fn outcomes(
    events: &[Event],
    pr: &PullRequest,
    observed: &Observed,
) -> (Vec<(&'static str, Outcome)>, Vec<Unseen>) {
    let mut seen = Vec::new();
    let mut unseen = Vec::new();
    for event in events {
        let outcome = match observed
            .refused
            .iter()
            .find(|refusal| event.reads(&refusal.field))
        {
            Some(refusal) => Err(format!(
                "GitHub would not let gh read {}: {}",
                refusal.field, refusal.why
            )),
            None => (event.outcome)(pr, observed)
                .map_err(|e| format!("GitHub answered its fields in a way bdi cannot read: {e}")),
        };
        match outcome {
            Ok(Some(outcome)) => seen.push((event.happening, outcome)),
            Ok(None) => {}
            Err(why) => unseen.push(Unseen {
                happening: event.happening,
                why,
            }),
        }
    }
    (seen, unseen)
}

/// Whether `outcome` asks anything not yet done of the gates `waiting`.
fn is_new(outcome: &Outcome, waiting: &[Waiting], told: &Told) -> bool {
    waiting.iter().any(|gate| match outcome {
        Outcome::Resolve { awaited, .. } => awaited(gate.until),
        Outcome::Tell(tellings) => gate.blocks.iter().any(|bead| {
            tellings.iter().any(|telling| {
                telling.is_news_to(gate.made) && !told.knows(&gate.project, bead, &telling.text)
            })
        }),
    })
}

/// Do each of `outcomes` to `gates`, read afresh from `project`'s tracker.
/// A gate one outcome closed is not closed again by the next.
fn acted(
    tracker: &Settling,
    project: &str,
    gates: &[PrGate],
    outcomes: &[(&'static str, Outcome)],
    told: &Told,
) -> Vec<Act> {
    let mut resolved = BTreeSet::new();
    let mut closed = BTreeSet::new();
    let mut acts = Vec::new();
    for (happening, outcome) in outcomes {
        let concerned = gates.iter().filter(|gate| {
            gate.awaits
                .as_ref()
                .is_ok_and(|wait| outcome.concerns(wait.until))
        });
        match outcome {
            Outcome::Resolve { reason, .. } => {
                for gate in concerned.filter(|gate| resolved.insert(gate.id.clone())) {
                    let done = tracker.resolve(&gate.id, reason).map(|()| Done::Resolved);
                    if done.is_ok() {
                        closed.insert(gate.id.clone());
                    }
                    acts.push(Act {
                        bead: gate.id.clone(),
                        happening,
                        done,
                    });
                }
            }
            Outcome::Tell(tellings) => {
                let open: Vec<&PrGate> = concerned
                    .filter(|gate| !closed.contains(&gate.id))
                    .collect();
                for telling in tellings {
                    let held_back: BTreeSet<&String> = open
                        .iter()
                        .filter(|gate| telling.is_news_to(gate.made))
                        .flat_map(|gate| &gate.blocks)
                        .collect();
                    for bead in held_back {
                        acts.push(Act {
                            bead: bead.clone(),
                            happening,
                            done: tell(tracker, project, bead, &telling.text, told),
                        });
                    }
                }
            }
        }
    }
    acts
}

/// Comment `text` on `bead` unless it already carries it, asking the
/// tracker only where `told` does not know.
fn tell(
    tracker: &Settling,
    project: &str,
    bead: &str,
    text: &str,
    told: &Told,
) -> Result<Done, RunFailure> {
    if told.knows(project, bead, text) {
        return Ok(Done::AlreadyCommented);
    }
    let carried = tracker
        .comments(bead)?
        .iter()
        .any(|comment| comment.eq_ignore_ascii_case(text));
    if !carried {
        tracker.comment(bead, text)?;
    }
    told.learn(project, bead, text);
    Ok(if carried {
        Done::AlreadyCommented
    } else {
        Done::Commented
    })
}

impl PullRequest {
    /// Whether this is `other`. GitHub reads a repository's name in any
    /// case, so a gate can spell it differently from GitHub and still mean it.
    pub fn is(&self, other: &PullRequest) -> bool {
        self.number == other.number && self.repo.eq_ignore_ascii_case(&other.repo)
    }
}

impl fmt::Display for PullRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.repo, self.number)
    }
}

impl PrGate {
    /// `gate` as the pull request it waits on, holding back `blocks`.
    pub(crate) fn of(gate: &Bead, blocks: Vec<String>) -> Self {
        let repo = gate::repo(gate);
        let number = gate::number(gate);
        let until = gate::until(gate);
        let awaits = match (repo, number, until) {
            (Some(repo), Ok(number), Ok(until)) => Ok(Wait {
                pull_request: PullRequest {
                    repo: repo.to_string(),
                    number,
                },
                until,
            }),
            (repo, number, until) => Err(repo
                .is_none()
                .then_some(Fault::NoRepo)
                .into_iter()
                .chain(number.err())
                .chain(until.err())
                .collect()),
        };
        PrGate {
            id: gate.id.clone(),
            repo: repo.map(str::to_string),
            awaits,
            blocks,
            made: gate.created_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::bd::parse_beads;
    use crate::collect::pr_events::EVENTS;
    use crate::collect::run::testing::FakeRunner;
    use crate::collect::run::{FailureKind, RunFailure};
    use std::ops::RangeInclusive;
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
        across(&Cli::new(runner), projects, |_| true)
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
                repo: Some("example/ark".to_string()),
                awaits: Ok(Wait {
                    pull_request: PullRequest {
                        repo: "example/ark".to_string(),
                        number: 42,
                    },
                    until: Until::Merged,
                }),
                blocks: vec!["ark-qca".to_string()],
                made: "2026-10-06T08:24:48Z".parse().ok(),
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
                repo: None,
                awaits: Err(vec![Fault::NoRepo]),
                blocks: vec!["ark-92q".to_string()],
                made: "2026-10-06T08:24:52Z".parse().ok(),
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
            "await_type": gate::PULL_REQUEST,
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

    const MERGED: &str = r#"{"state":"MERGED","isDraft":false,"mergeCommit":{"oid":"5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff"}}"#;
    const CLOSED: &str = r#"{"state":"CLOSED","isDraft":false,"mergeCommit":null}"#;
    const OPEN: &str = r#"{"state":"OPEN","isDraft":false,"mergeCommit":null}"#;
    const DRAFT: &str = r#"{"state":"OPEN","isDraft":true,"mergeCommit":null}"#;
    const APPROVED: &str =
        r#"{"state":"OPEN","isDraft":false,"mergeCommit":null,"reviewDecision":"APPROVED"}"#;
    const NO_COMMENTS: &str = include_str!("../../tests/fixtures/bd_1.3.0_comments_none.json");
    /// ark-2ud's comments once it has been told example/ark#7 closed
    /// unmerged, beside a comment of its own.
    const TOLD: &str = include_str!("../../tests/fixtures/bd_1.3.0_comments_told_ark-2ud.json");
    /// ark-2ud's comments before it is told, with a comment of its own.
    const OWN: &str = include_str!("../../tests/fixtures/bd_1.3.0_comments_own_ark-2ud.json");

    /// In the captured tracker, ark-0i5 waits on #42 and holds back ark-qca,
    /// and ark-eb1 waits on #7 and holds back ark-2ud and ark-45c.
    fn pr(number: u64) -> PullRequest {
        PullRequest {
            repo: "example/ark".to_string(),
            number,
        }
    }

    /// The query about #`number` in `repo`, asking what [`EVENTS`] read.
    fn viewed_in(repo: &str, number: u64) -> String {
        let (owner, name) = repo.split_once('/').expect("the repo names its owner");
        format!(
            "gh api graphql -f owner={owner} -f name={name} -f query=query($owner:String!,\
             $name:String!){{repository(owner:$owner,name:$name){{pr{number}:pullRequest\
             (number:{number}){{state isDraft mergeCommit{{oid}} reviewDecision commits(last:1){{nodes{{commit{{oid statusCheckRollup{{state contexts(last:100){{nodes{{...on CheckRun{{conclusion completedAt}} ...on StatusContext{{state createdAt}}}}}}}}}}}}}} reviews(last:5){{nodes{{url state submittedAt author{{login}}}}}} mergeable headRefOid headRef{{target{{...on Commit{{committedDate}}}}}} baseRef{{target{{...on Commit{{committedDate}}}}}} comments(last:5){{nodes{{url createdAt author{{login}}}}}}}}}}}}"
        )
    }

    fn viewed(number: u64) -> String {
        viewed_in("example/ark", number)
    }

    /// GitHub's answer to [`viewed`]: #`number` with `fields`.
    fn answer(number: u64, fields: &str) -> String {
        format!(r#"{{"data":{{"repository":{{"pr{number}":{fields}}}}}}}"#)
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
        settle(
            &Cli::new(runner),
            runner,
            projects,
            &EVENTS,
            pr,
            &Told::default(),
        )
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
            Settled::Acted {
                mut projects,
                unseen,
            } => {
                assert_eq!(unseen, [], "GitHub let every event see the pull request");
                assert_eq!(projects.len(), 1, "one project was configured");
                let project = projects.remove(0);
                assert_eq!(project.project, "arkham");
                project.acts.expect("arkham's tracker answered")
            }
            not_finished => panic!("settling came to {not_finished:?}"),
        }
    }

    fn act(bead: &str, happening: &'static str, done: Done) -> Act {
        Act {
            bead: bead.to_string(),
            happening,
            done: Ok(done),
        }
    }

    fn merged(bead: &str) -> Act {
        act(bead, "merged", Done::Resolved)
    }

    fn closed(bead: &str, done: Done) -> Act {
        act(bead, "closed unmerged", done)
    }

    #[test]
    fn a_merge_closes_the_gate_waiting_on_it_with_a_reason_naming_the_merge() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&viewed(42), &answer(42, MERGED))
            .with(&resolving_42("arkham"), "✓ Gate resolved: ark-0i5\n");

        let settled = settled(&runner, &[project("arkham")], &pr(42));

        assert_eq!(acts(settled), [merged("ark-0i5")]);
        assert_eq!(writes(&runner), [resolving_42("arkham")]);
    }

    /// A gate left open behind a pull request closed unmerged is settled
    /// again on every look, so a settling that asked after every gate's
    /// beads would cost each look the square of the gates.
    #[test]
    fn settling_asks_which_beads_are_held_back_only_of_the_gates_waiting_on_the_pull_request() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&viewed(42), &answer(42, MERGED))
            .with(&resolving_42("arkham"), "");

        settled(&runner, &[project("arkham")], &pr(42));

        assert_eq!(dep_lists_read(&runner), [held_back_by("arkham", "ark-0i5")]);
    }

    fn dep_lists_read(runner: &FakeRunner) -> Vec<String> {
        runner
            .calls()
            .into_iter()
            .map(|call| call.argv)
            .filter(|argv| argv.contains(" dep list "))
            .collect()
    }

    #[test]
    fn a_merge_settled_again_writes_nothing_because_its_gate_is_closed() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&gate_list("arkham"), &gate_list_without("ark-0i5"))
            .with(&viewed(42), &answer(42, MERGED));

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
            .with(
                &viewed(42),
                &answer(
                    42,
                    r#"{"state":"MERGED","isDraft":false,"mergeCommit":null}"#,
                ),
            )
            .with(&reason, "");

        let settled = settled(&runner, &[project("arkham")], &pr(42));

        assert_eq!(acts(settled), [merged("ark-0i5")]);
        assert_eq!(writes(&runner), [reason]);
    }

    #[test]
    fn a_close_without_a_merge_comments_on_each_held_back_bead_and_leaves_the_gate_open() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&viewed(7), &answer(7, CLOSED))
            .with(&comments_on("arkham", "ark-2ud"), OWN)
            .with(&comments_on("arkham", "ark-45c"), NO_COMMENTS)
            .with(&telling("arkham", "ark-2ud"), "Comment added to ark-2ud\n")
            .with(&telling("arkham", "ark-45c"), "Comment added to ark-45c\n");

        let settled = settled(&runner, &[project("arkham")], &pr(7));

        assert_eq!(
            acts(settled),
            [
                closed("ark-2ud", Done::Commented),
                closed("ark-45c", Done::Commented)
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
            .with(&viewed(7), &answer(7, CLOSED))
            .with(&comments_on("arkham", "ark-2ud"), TOLD)
            .with(&comments_on("arkham", "ark-45c"), NO_COMMENTS)
            .with(&telling("arkham", "ark-45c"), "Comment added to ark-45c\n");

        let settled = settled(&runner, &[project("arkham")], &pr(7));

        assert_eq!(
            acts(settled),
            [
                closed("ark-2ud", Done::AlreadyCommented),
                closed("ark-45c", Done::Commented)
            ]
        );
        assert_eq!(writes(&runner), [telling("arkham", "ark-45c")]);
    }

    /// An open pull request whose head commit `oid` failed its checks, with
    /// `decision` as GitHub's review decision, a JSON value.
    fn checks_failed_deciding(decision: &str, oid: &str) -> String {
        format!(
            r#"{{"state":"OPEN","isDraft":false,"mergeCommit":null,"reviewDecision":{decision},"commits":{{"nodes":[{{"commit":{{"oid":"{oid}","statusCheckRollup":{{"state":"FAILURE"}}}}}}]}}}}"#
        )
    }

    fn checks_failed_on(oid: &str) -> String {
        checks_failed_deciding("null", oid)
    }

    fn checks_failed_telling_on(number: u64, project: &str, bead: &str, oid: &str) -> String {
        written(
            project,
            &format!(
                "comments add {bead} The checks on {oid}, the head of pull request \
                 example/ark#{number}, failed, so the gh:pr gate waiting on it stays open."
            ),
        )
    }

    fn checks_failed_telling(project: &str, bead: &str, oid: &str) -> String {
        checks_failed_telling_on(7, project, bead, oid)
    }

    fn failing_checks(bead: &str, done: Done) -> Act {
        act(bead, "has failing checks", done)
    }

    #[test]
    fn failed_checks_comment_on_each_held_back_bead_and_leave_the_gate_open() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&viewed(7), &answer(7, &checks_failed_on("a1b2c3")))
            .with(&comments_on("arkham", "ark-2ud"), OWN)
            .with(&comments_on("arkham", "ark-45c"), NO_COMMENTS)
            .with(
                &checks_failed_telling("arkham", "ark-2ud", "a1b2c3"),
                "Comment added to ark-2ud\n",
            )
            .with(
                &checks_failed_telling("arkham", "ark-45c", "a1b2c3"),
                "Comment added to ark-45c\n",
            );

        let settled = settled(&runner, &[project("arkham")], &pr(7));

        assert_eq!(
            acts(settled),
            [
                failing_checks("ark-2ud", Done::Commented),
                failing_checks("ark-45c", Done::Commented)
            ]
        );
        assert_eq!(
            writes(&runner),
            [
                checks_failed_telling("arkham", "ark-2ud", "a1b2c3"),
                checks_failed_telling("arkham", "ark-45c", "a1b2c3")
            ]
        );
    }

    /// A fix that fails again is a new head commit, so it is a new telling,
    /// and the commit already told is not told twice.
    #[test]
    fn failed_checks_are_told_once_for_each_head_commit() {
        let told = Told::default();
        let settle_7 = |oid: &str| {
            let runner = captured(FakeRunner::default(), "arkham")
                .with(&viewed(7), &answer(7, &checks_failed_on(oid)))
                .with(&comments_on("arkham", "ark-2ud"), NO_COMMENTS)
                .with(&comments_on("arkham", "ark-45c"), NO_COMMENTS)
                .with(
                    &checks_failed_telling("arkham", "ark-2ud", oid),
                    "Comment added to ark-2ud\n",
                )
                .with(
                    &checks_failed_telling("arkham", "ark-45c", oid),
                    "Comment added to ark-45c\n",
                );
            let settled = settle(
                &Cli::new(&runner),
                &runner,
                &[project("arkham")],
                &EVENTS,
                &pr(7),
                &told,
            );
            (acts(settled), writes(&runner))
        };

        let (_, first) = settle_7("a1b2c3");
        let (same, same_writes) = settle_7("a1b2c3");
        let (_, fixed) = settle_7("d4e5f6");

        assert_eq!(first.len(), 2);
        assert_eq!(
            same,
            [
                failing_checks("ark-2ud", Done::AlreadyCommented),
                failing_checks("ark-45c", Done::AlreadyCommented)
            ]
        );
        assert_eq!(same_writes, Vec::<String>::new());
        assert_eq!(
            fixed,
            [
                checks_failed_telling("arkham", "ark-2ud", "d4e5f6"),
                checks_failed_telling("arkham", "ark-45c", "d4e5f6")
            ]
        );
    }

    /// The runner panics on any call it was not given, so a tracker asked
    /// anything at all fails the test.
    #[test]
    fn checks_that_have_not_failed_ask_no_tracker_anything() {
        for rollup in ["null", r#"{"state":"SUCCESS"}"#, r#"{"state":"PENDING"}"#] {
            let passing = format!(
                r#"{{"state":"OPEN","isDraft":false,"mergeCommit":null,"reviewDecision":null,"commits":{{"nodes":[{{"commit":{{"oid":"a1b2c3","statusCheckRollup":{rollup}}}}}]}}}}"#
            );
            let runner = FakeRunner::default().with(&queried(7..=7), &answer(7, &passing));

            assert_eq!(
                settled_together(&runner, &[awaited(7, Until::Merged, &["ark-2ud"])]),
                [Settled::NothingNew { unseen: vec![] }],
                "{rollup}"
            );
        }
    }

    /// An open pull request whose head commit `oid` GitHub says is `answer`.
    fn mergeable_as(answer: &str, oid: &str) -> String {
        format!(
            r#"{{"state":"OPEN","isDraft":false,"mergeCommit":null,"reviewDecision":null,"mergeable":"{answer}","headRefOid":"{oid}"}}"#
        )
    }

    fn conflict_telling(project: &str, bead: &str, oid: &str) -> String {
        written(
            project,
            &format!(
                "comments add {bead} The head of pull request example/ark#7, {oid}, conflicts \
                 with its base, so the gh:pr gate waiting on it stays open."
            ),
        )
    }

    fn conflicting_with_base(bead: &str, done: Done) -> Act {
        act(bead, "conflicts with its base", done)
    }

    #[test]
    fn a_conflict_comments_on_each_held_back_bead_once_for_each_head_commit() {
        let told = Told::default();
        let settle_7 = |oid: &str| {
            let runner = captured(FakeRunner::default(), "arkham")
                .with(&viewed(7), &answer(7, &mergeable_as("CONFLICTING", oid)))
                .with(&comments_on("arkham", "ark-2ud"), NO_COMMENTS)
                .with(&comments_on("arkham", "ark-45c"), NO_COMMENTS)
                .with(
                    &conflict_telling("arkham", "ark-2ud", oid),
                    "Comment added to ark-2ud\n",
                )
                .with(
                    &conflict_telling("arkham", "ark-45c", oid),
                    "Comment added to ark-45c\n",
                );
            let settled = settle(
                &Cli::new(&runner),
                &runner,
                &[project("arkham")],
                &EVENTS,
                &pr(7),
                &told,
            );
            (acts(settled), writes(&runner))
        };

        let (first, first_writes) = settle_7("a1b2c3");
        let (same, same_writes) = settle_7("a1b2c3");
        let (_, rebased) = settle_7("d4e5f6");

        assert_eq!(
            first,
            [
                conflicting_with_base("ark-2ud", Done::Commented),
                conflicting_with_base("ark-45c", Done::Commented)
            ]
        );
        assert_eq!(first_writes.len(), 2);
        assert_eq!(
            same,
            [
                conflicting_with_base("ark-2ud", Done::AlreadyCommented),
                conflicting_with_base("ark-45c", Done::AlreadyCommented)
            ]
        );
        assert_eq!(same_writes, Vec::<String>::new());
        assert_eq!(
            rebased,
            [
                conflict_telling("arkham", "ark-2ud", "d4e5f6"),
                conflict_telling("arkham", "ark-45c", "d4e5f6")
            ]
        );
    }

    /// The runner panics on any call it was not given, so a tracker asked
    /// anything at all fails the test.
    #[test]
    fn a_pull_request_that_merges_cleanly_or_is_not_yet_known_to_asks_no_tracker_anything() {
        for mergeability in ["MERGEABLE", "UNKNOWN"] {
            let runner = FakeRunner::default().with(
                &queried(7..=7),
                &answer(7, &mergeable_as(mergeability, "a1b2c3")),
            );

            assert_eq!(
                settled_together(&runner, &[awaited(7, Until::Merged, &["ark-2ud"])]),
                [Settled::NothingNew { unseen: vec![] }],
                "{mergeability}"
            );
        }
    }

    /// A gate the approval closed no longer holds anything back, so a bead
    /// only it held back is not told that it stays open.
    #[test]
    fn failed_checks_are_not_told_to_a_bead_held_back_by_a_gate_an_approval_just_closed() {
        let resolving = written(
            "arkham",
            &format!("gate resolve ark-eb1 --reason {APPROVED_REASON}"),
        );
        let runner = captured(FakeRunner::default(), "arkham")
            .with(
                &gate_list("arkham"),
                &gate_list_with_42_awaiting("approved"),
            )
            .with(
                &viewed(42),
                &answer(42, &checks_failed_deciding(r#""APPROVED""#, "a1b2c3")),
            )
            .with(&resolving, "✓ Gate resolved: ark-eb1\n")
            .with(&comments_on("arkham", "ark-qca"), NO_COMMENTS)
            .with(
                &checks_failed_telling_on(42, "arkham", "ark-qca", "a1b2c3"),
                "Comment added to ark-qca\n",
            );

        let settled = settled(&runner, &[project("arkham")], &pr(42));

        assert_eq!(
            acts(settled),
            [
                act("ark-eb1", "is approved", Done::Resolved),
                failing_checks("ark-qca", Done::Commented)
            ]
        );
    }

    /// A gate bd failed to close is still open, so what is said of it stays
    /// true.
    #[test]
    fn failed_checks_are_told_to_a_bead_held_back_by_a_gate_an_approval_failed_to_close() {
        let resolving = written(
            "arkham",
            &format!("gate resolve ark-eb1 --reason {APPROVED_REASON}"),
        );
        let runner = captured(FakeRunner::default(), "arkham")
            .with(
                &gate_list("arkham"),
                &gate_list_with_42_awaiting("approved"),
            )
            .with(
                &viewed(42),
                &answer(42, &checks_failed_deciding(r#""APPROVED""#, "a1b2c3")),
            )
            .failing(&resolving, unavailable("bd"))
            .with(&comments_on("arkham", "ark-qca"), NO_COMMENTS)
            .with(&comments_on("arkham", "ark-2ud"), NO_COMMENTS)
            .with(&comments_on("arkham", "ark-45c"), NO_COMMENTS)
            .with(
                &checks_failed_telling_on(42, "arkham", "ark-qca", "a1b2c3"),
                "Comment added to ark-qca\n",
            )
            .with(
                &checks_failed_telling_on(42, "arkham", "ark-2ud", "a1b2c3"),
                "Comment added to ark-2ud\n",
            )
            .with(
                &checks_failed_telling_on(42, "arkham", "ark-45c", "a1b2c3"),
                "Comment added to ark-45c\n",
            );

        let acts = acts(settled(&runner, &[project("arkham")], &pr(42)));

        let told: Vec<&str> = acts
            .iter()
            .filter(|act| act.happening == "has failing checks")
            .map(|act| act.bead.as_str())
            .collect();
        assert_eq!(told, ["ark-2ud", "ark-45c", "ark-qca"]);
    }

    /// An open pull request with each of `reviews` submitted, as `(reviewer,
    /// GitHub's state, review number)`.
    fn reviewed_by(reviews: &[(&str, &str, u32)]) -> String {
        let nodes: Vec<String> = reviews
            .iter()
            .map(|(login, state, id)| {
                format!(
                    r#"{{"url":"https://forge.invalid/example/ark/pull/7#pullrequestreview-{id}","state":"{state}","author":{{"login":"{login}"}}}}"#
                )
            })
            .collect();
        format!(
            r#"{{"state":"OPEN","isDraft":false,"mergeCommit":null,"reviewDecision":null,"reviews":{{"nodes":[{}]}}}}"#,
            nodes.join(",")
        )
    }

    fn review_telling(bead: &str, login: &str, said: &str, id: u32) -> String {
        written(
            "arkham",
            &format!(
                "comments add {bead} {login} reviewed pull request example/ark#7: {said}. \
                 https://forge.invalid/example/ark/pull/7#pullrequestreview-{id}"
            ),
        )
    }

    fn reviewed(bead: &str, done: Done) -> Act {
        act(bead, "was reviewed", done)
    }

    #[test]
    fn each_review_comments_once_on_each_held_back_bead_and_leaves_the_gate_open() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(
                &viewed(7),
                &answer(
                    7,
                    &reviewed_by(&[("alice", "APPROVED", 11), ("bob", "CHANGES_REQUESTED", 12)]),
                ),
            )
            .with(&comments_on("arkham", "ark-2ud"), OWN)
            .with(&comments_on("arkham", "ark-45c"), NO_COMMENTS)
            .with(
                &review_telling("ark-2ud", "alice", "approved", 11),
                "Comment added to ark-2ud\n",
            )
            .with(
                &review_telling("ark-45c", "alice", "approved", 11),
                "Comment added to ark-45c\n",
            )
            .with(
                &review_telling("ark-2ud", "bob", "changes requested", 12),
                "Comment added to ark-2ud\n",
            )
            .with(
                &review_telling("ark-45c", "bob", "changes requested", 12),
                "Comment added to ark-45c\n",
            );

        let settled = settled(&runner, &[project("arkham")], &pr(7));

        assert_eq!(
            acts(settled),
            [
                reviewed("ark-2ud", Done::Commented),
                reviewed("ark-45c", Done::Commented),
                reviewed("ark-2ud", Done::Commented),
                reviewed("ark-45c", Done::Commented),
            ]
        );
        assert_eq!(
            writes(&runner),
            [
                review_telling("ark-2ud", "alice", "approved", 11),
                review_telling("ark-45c", "alice", "approved", 11),
                review_telling("ark-2ud", "bob", "changes requested", 12),
                review_telling("ark-45c", "bob", "changes requested", 12),
            ]
        );
    }

    /// A review submitted since the last look is the only one told, and a
    /// look that finds nothing new writes nothing.
    #[test]
    fn a_review_is_told_once_and_a_repeat_writes_nothing() {
        let told = Told::default();
        let settle_7 = |reviews: &[(&str, &str, u32)]| {
            let mut runner = captured(FakeRunner::default(), "arkham")
                .with(&viewed(7), &answer(7, &reviewed_by(reviews)))
                .with(&comments_on("arkham", "ark-2ud"), NO_COMMENTS)
                .with(&comments_on("arkham", "ark-45c"), NO_COMMENTS);
            for (login, state, id) in reviews {
                let said = state.to_ascii_lowercase().replace('_', " ");
                for bead in ["ark-2ud", "ark-45c"] {
                    runner = runner.with(
                        &review_telling(bead, login, &said, *id),
                        &format!("Comment added to {bead}\n"),
                    );
                }
            }
            let settled = settle(
                &Cli::new(&runner),
                &runner,
                &[project("arkham")],
                &EVENTS,
                &pr(7),
                &told,
            );
            (acts(settled), writes(&runner))
        };
        let first_review = ("alice", "COMMENTED", 11);

        let (_, first) = settle_7(&[first_review]);
        let (same, same_writes) = settle_7(&[first_review]);
        let (_, later) = settle_7(&[first_review, ("alice", "APPROVED", 12)]);

        assert_eq!(first.len(), 2);
        assert_eq!(
            same,
            [
                reviewed("ark-2ud", Done::AlreadyCommented),
                reviewed("ark-45c", Done::AlreadyCommented)
            ]
        );
        assert_eq!(same_writes, Vec::<String>::new());
        assert_eq!(
            later,
            [
                review_telling("ark-2ud", "alice", "approved", 12),
                review_telling("ark-45c", "alice", "approved", 12)
            ]
        );
    }

    /// The sweep already knows what it told, so a pull request whose reviews
    /// every held-back bead has been told asks no tracker anything.
    #[test]
    fn reviews_every_held_back_bead_has_been_told_ask_no_tracker_anything() {
        let told = Told::default();
        let runner = captured(FakeRunner::default(), "arkham")
            .with(
                &queried(7..=7),
                &answer(7, &reviewed_by(&[("alice", "COMMENTED", 11)])),
            )
            .with(&comments_on("arkham", "ark-2ud"), NO_COMMENTS)
            .with(&comments_on("arkham", "ark-45c"), NO_COMMENTS)
            .with(
                &review_telling("ark-2ud", "alice", "commented", 11),
                "Comment added to ark-2ud\n",
            )
            .with(
                &review_telling("ark-45c", "alice", "commented", 11),
                "Comment added to ark-45c\n",
            );
        let awaiting = [awaited(7, Until::Merged, &["ark-2ud", "ark-45c"])];
        let sweep = || {
            settle_together(
                &Cli::new(&runner),
                &runner,
                &[project("arkham")],
                &EVENTS,
                &awaiting,
                &told,
            )
            .collect::<Vec<_>>()
        };

        let first = sweep();
        let again = sweep();

        assert!(matches!(first[0], Settled::Acted { .. }));
        assert_eq!(again, [Settled::NothingNew { unseen: vec![] }]);
    }

    /// An open pull request with a conversation comment from each of
    /// `comments`, as `(author, comment number)`.
    fn commented_by(comments: &[(&str, u32)]) -> String {
        let nodes: Vec<String> = comments
            .iter()
            .map(|(login, id)| {
                format!(
                    r#"{{"url":"https://forge.invalid/example/ark/pull/7#issuecomment-{id}","author":{{"login":"{login}"}}}}"#
                )
            })
            .collect();
        format!(
            r#"{{"state":"OPEN","isDraft":false,"mergeCommit":null,"reviewDecision":null,"comments":{{"nodes":[{}]}}}}"#,
            nodes.join(",")
        )
    }

    fn comment_telling(bead: &str, login: &str, id: u32) -> String {
        written(
            "arkham",
            &format!(
                "comments add {bead} {login} commented on pull request example/ark#7. \
                 https://forge.invalid/example/ark/pull/7#issuecomment-{id}"
            ),
        )
    }

    fn commented(bead: &str, done: Done) -> Act {
        act(bead, "was commented on", done)
    }

    #[test]
    fn each_comment_comments_once_on_each_held_back_bead_and_leaves_the_gate_open() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(
                &viewed(7),
                &answer(7, &commented_by(&[("alice", 11), ("bob", 12)])),
            )
            .with(&comments_on("arkham", "ark-2ud"), OWN)
            .with(&comments_on("arkham", "ark-45c"), NO_COMMENTS)
            .with(
                &comment_telling("ark-2ud", "alice", 11),
                "Comment added to ark-2ud\n",
            )
            .with(
                &comment_telling("ark-45c", "alice", 11),
                "Comment added to ark-45c\n",
            )
            .with(
                &comment_telling("ark-2ud", "bob", 12),
                "Comment added to ark-2ud\n",
            )
            .with(
                &comment_telling("ark-45c", "bob", 12),
                "Comment added to ark-45c\n",
            );

        let settled = settled(&runner, &[project("arkham")], &pr(7));

        assert_eq!(
            acts(settled),
            [
                commented("ark-2ud", Done::Commented),
                commented("ark-45c", Done::Commented),
                commented("ark-2ud", Done::Commented),
                commented("ark-45c", Done::Commented),
            ]
        );
        assert_eq!(
            writes(&runner),
            [
                comment_telling("ark-2ud", "alice", 11),
                comment_telling("ark-45c", "alice", 11),
                comment_telling("ark-2ud", "bob", 12),
                comment_telling("ark-45c", "bob", 12),
            ]
        );
    }

    /// A comment made since the last look is the only one told, and a look
    /// that finds nothing new writes nothing.
    #[test]
    fn a_comment_is_told_once_and_a_repeat_writes_nothing() {
        let told = Told::default();
        let settle_7 = |comments: &[(&str, u32)]| {
            let mut runner = captured(FakeRunner::default(), "arkham")
                .with(&viewed(7), &answer(7, &commented_by(comments)))
                .with(&comments_on("arkham", "ark-2ud"), NO_COMMENTS)
                .with(&comments_on("arkham", "ark-45c"), NO_COMMENTS);
            for (login, id) in comments {
                for bead in ["ark-2ud", "ark-45c"] {
                    runner = runner.with(
                        &comment_telling(bead, login, *id),
                        &format!("Comment added to {bead}\n"),
                    );
                }
            }
            let settled = settle(
                &Cli::new(&runner),
                &runner,
                &[project("arkham")],
                &EVENTS,
                &pr(7),
                &told,
            );
            (acts(settled), writes(&runner))
        };
        let first_comment = ("alice", 11);

        let (_, first) = settle_7(&[first_comment]);
        let (same, same_writes) = settle_7(&[first_comment]);
        let (_, later) = settle_7(&[first_comment, ("alice", 12)]);

        assert_eq!(first.len(), 2);
        assert_eq!(
            same,
            [
                commented("ark-2ud", Done::AlreadyCommented),
                commented("ark-45c", Done::AlreadyCommented)
            ]
        );
        assert_eq!(same_writes, Vec::<String>::new());
        assert_eq!(
            later,
            [
                comment_telling("ark-2ud", "alice", 12),
                comment_telling("ark-45c", "alice", 12)
            ]
        );
    }

    /// The sweep already knows what it told, so a pull request whose comments
    /// every held-back bead has been told asks no tracker anything.
    #[test]
    fn comments_every_held_back_bead_has_been_told_ask_no_tracker_anything() {
        let told = Told::default();
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&queried(7..=7), &answer(7, &commented_by(&[("alice", 11)])))
            .with(&comments_on("arkham", "ark-2ud"), NO_COMMENTS)
            .with(&comments_on("arkham", "ark-45c"), NO_COMMENTS)
            .with(
                &comment_telling("ark-2ud", "alice", 11),
                "Comment added to ark-2ud\n",
            )
            .with(
                &comment_telling("ark-45c", "alice", 11),
                "Comment added to ark-45c\n",
            );
        let awaiting = [awaited(7, Until::Merged, &["ark-2ud", "ark-45c"])];
        let sweep = || {
            settle_together(
                &Cli::new(&runner),
                &runner,
                &[project("arkham")],
                &EVENTS,
                &awaiting,
                &told,
            )
            .collect::<Vec<_>>()
        };

        let first = sweep();
        let again = sweep();

        assert!(matches!(first[0], Settled::Acted { .. }));
        assert_eq!(again, [Settled::NothingNew { unseen: vec![] }]);
    }

    /// When the captured tracker made ark-eb1, the gate waiting on #7.
    const EB1_MADE: &str = "2026-10-06T08:24:49Z";

    fn made_at(mut awaited: Awaited, made: &str) -> Awaited {
        for gate in &mut awaited.waiting {
            gate.made = made.parse().ok();
        }
        awaited
    }

    /// #7 open with failing checks, a review, a conflict and a comment, each
    /// of them `at`.
    fn everything_at(at: &str) -> String {
        format!(
            r#"{{"state":"OPEN","isDraft":false,"mergeCommit":null,"reviewDecision":null,
            "commits":{{"nodes":[{{"commit":{{"oid":"a1b2c3","statusCheckRollup":{{"state":"FAILURE","contexts":{{"nodes":[{{"conclusion":"FAILURE","completedAt":"{at}"}}]}}}}}}}}]}},
            "reviews":{{"nodes":[{{"url":"https://forge.invalid/example/ark/pull/7#pullrequestreview-11","state":"COMMENTED","submittedAt":"{at}","author":{{"login":"alice"}}}}]}},
            "mergeable":"CONFLICTING","headRefOid":"a1b2c3","headRef":{{"target":{{"committedDate":"{at}"}}}},"baseRef":{{"target":{{"committedDate":"{at}"}}}},
            "comments":{{"nodes":[{{"url":"https://forge.invalid/example/ark/pull/7#issuecomment-12","createdAt":"{at}","author":{{"login":"bob"}}}}]}}}}"#
        )
    }

    /// Whoever made a gate could already see what its pull request had done,
    /// so a bdi that newly tells it would wake every seat for old news.
    #[test]
    fn a_gate_made_after_failing_checks_a_review_a_conflict_and_a_comment_tells_none_of_them() {
        let runner = captured(FakeRunner::default(), "arkham").with(
            &viewed(7),
            &answer(7, &everything_at("2026-10-06T08:24:48Z")),
        );
        let looked = made_at(awaited(7, Until::Merged, &["ark-2ud", "ark-45c"]), EB1_MADE);

        let swept = settled_together(&runner, &[looked]);
        assert_eq!(swept, [Settled::NothingNew { unseen: vec![] }]);
        assert_eq!(runner.calls().len(), 1, "only GitHub was asked");

        let delivered = settled(&runner, &[project("arkham")], &pr(7));
        assert_eq!(acts(delivered), []);
        assert_eq!(writes(&runner), Vec::<String>::new());
    }

    #[test]
    fn a_gate_made_before_failing_checks_a_review_a_conflict_and_a_comment_tells_each_once() {
        let tellings = |bead| {
            [
                checks_failed_telling("arkham", bead, "a1b2c3"),
                review_telling(bead, "alice", "commented", 11),
                conflict_telling("arkham", bead, "a1b2c3"),
                comment_telling(bead, "bob", 12),
            ]
        };
        let mut runner = captured(FakeRunner::default(), "arkham")
            .with(&viewed(7), &answer(7, &everything_at(EB1_MADE)));
        for bead in ["ark-2ud", "ark-45c"] {
            runner = runner.with(&comments_on("arkham", bead), NO_COMMENTS);
            for telling in tellings(bead) {
                runner = runner.with(&telling, &format!("Comment added to {bead}\n"));
            }
        }
        let told = Told::default();
        let looked = [made_at(
            awaited(7, Until::Merged, &["ark-2ud", "ark-45c"]),
            EB1_MADE,
        )];
        let sweep = || {
            settle_together(
                &Cli::new(&runner),
                &runner,
                &[project("arkham")],
                &EVENTS,
                &looked,
                &told,
            )
            .collect::<Vec<_>>()
        };

        sweep();
        let again = sweep();

        let [a, b, c, d] = tellings("ark-2ud");
        let [e, f, g, h] = tellings("ark-45c");
        assert_eq!(writes(&runner), [a, e, b, f, c, g, d, h]);
        assert_eq!(again, [Settled::NothingNew { unseen: vec![] }]);
    }

    fn comments_read(runner: &FakeRunner) -> usize {
        runner
            .calls()
            .into_iter()
            .filter(|call| call.argv.contains(" comments ") && call.argv.contains(" --readonly "))
            .count()
    }

    /// The second settling reads the gates afresh, as every settling that
    /// acts does, and asks no bead for its comments.
    #[test]
    fn a_bead_this_process_told_is_not_asked_for_its_comments_again() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&viewed(7), &answer(7, CLOSED))
            .with(&comments_on("arkham", "ark-2ud"), NO_COMMENTS)
            .with(&comments_on("arkham", "ark-45c"), TOLD)
            .with(&telling("arkham", "ark-2ud"), "");
        let told = Told::default();
        let settle_7 = || {
            settle(
                &Cli::new(&runner),
                &runner,
                &[project("arkham")],
                &EVENTS,
                &pr(7),
                &told,
            )
        };

        settle_7();
        let again = settle_7();

        assert_eq!(
            acts(again),
            [
                closed("ark-2ud", Done::AlreadyCommented),
                closed("ark-45c", Done::AlreadyCommented)
            ]
        );
        assert_eq!(comments_read(&runner), 2);
        assert_eq!(writes(&runner), [telling("arkham", "ark-2ud")]);
    }

    /// A comment bd failed to add is one nobody was told, so the next
    /// settling looks again.
    #[test]
    fn a_bead_whose_comment_failed_is_asked_for_its_comments_again() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&viewed(7), &answer(7, CLOSED))
            .with(&comments_on("arkham", "ark-2ud"), NO_COMMENTS)
            .with(&comments_on("arkham", "ark-45c"), TOLD)
            .failing(&telling("arkham", "ark-2ud"), unavailable("bd"));
        let told = Told::default();

        for _ in 0..2 {
            settle(
                &Cli::new(&runner),
                &runner,
                &[project("arkham")],
                &EVENTS,
                &pr(7),
                &told,
            );
        }

        let asked_2ud = runner
            .calls()
            .into_iter()
            .filter(|call| call.argv == comments_on("arkham", "ark-2ud"))
            .count();
        assert_eq!(asked_2ud, 2);
    }

    /// The captured gate list with ark-eb1 waiting on #42 to leave draft,
    /// beside ark-0i5 waiting on it to merge.
    fn gate_list_with_42_awaited_for_review() -> String {
        gate_list_with_42_awaiting("ready_for_review")
    }

    /// The captured gate list with ark-eb1 waiting on #42 for `awaits`.
    fn gate_list_with_42_awaiting(awaits: &str) -> String {
        let mut rows: Vec<serde_json::Value> =
            serde_json::from_str(GATE_LIST).expect("the capture parses");
        for row in rows.iter_mut().filter(|row| row["id"] == "ark-eb1") {
            row["await_id"] = "42".into();
            row["metadata"][gate::AWAITS] = awaits.into();
        }
        serde_json::to_string(&rows).expect("the rows print")
    }

    const READY_REASON: &str = "Pull request example/ark#42 is ready for review.";

    #[test]
    fn leaving_draft_closes_each_gate_waiting_for_that_and_leaves_the_merge_gates_open() {
        let resolving = written(
            "arkham",
            &format!("gate resolve ark-eb1 --reason {READY_REASON}"),
        );
        let runner = captured(FakeRunner::default(), "arkham")
            .with(
                &gate_list("arkham"),
                &gate_list_with_42_awaited_for_review(),
            )
            .with(&viewed(42), &answer(42, OPEN))
            .with(&resolving, "✓ Gate resolved: ark-eb1\n");

        let settled = settled(&runner, &[project("arkham")], &pr(42));

        assert_eq!(
            acts(settled),
            [act("ark-eb1", "is ready for review", Done::Resolved)]
        );
        assert_eq!(writes(&runner), [resolving]);
        assert_eq!(
            dep_lists_read(&runner),
            [held_back_by("arkham", "ark-eb1")],
            "the gate waiting for the merge is not asked what it holds back"
        );
    }

    #[test]
    fn a_merge_closes_a_gate_waiting_for_review_beside_the_one_waiting_for_the_merge() {
        let resolving_eb1 = written(
            "arkham",
            &format!("gate resolve ark-eb1 --reason {MERGED_REASON}"),
        );
        let runner = captured(FakeRunner::default(), "arkham")
            .with(
                &gate_list("arkham"),
                &gate_list_with_42_awaited_for_review(),
            )
            .with(&viewed(42), &answer(42, MERGED))
            .with(&resolving_eb1, "")
            .with(&resolving_42("arkham"), "");

        let settled = settled(&runner, &[project("arkham")], &pr(42));

        assert_eq!(acts(settled), [merged("ark-eb1"), merged("ark-0i5")]);
        assert_eq!(writes(&runner), [resolving_eb1, resolving_42("arkham")]);
    }

    const APPROVED_REASON: &str = "Pull request example/ark#42 is approved.";

    #[test]
    fn an_approval_closes_each_gate_waiting_for_that_and_leaves_the_merge_gates_open() {
        let resolving = written(
            "arkham",
            &format!("gate resolve ark-eb1 --reason {APPROVED_REASON}"),
        );
        let runner = captured(FakeRunner::default(), "arkham")
            .with(
                &gate_list("arkham"),
                &gate_list_with_42_awaiting("approved"),
            )
            .with(&viewed(42), &answer(42, APPROVED))
            .with(&resolving, "✓ Gate resolved: ark-eb1\n");

        let settled = settled(&runner, &[project("arkham")], &pr(42));

        assert_eq!(
            acts(settled),
            [act("ark-eb1", "is approved", Done::Resolved)]
        );
        assert_eq!(writes(&runner), [resolving]);
    }

    /// GitHub gives no review decision where reviews are not required, and
    /// none until a review that counts has been given where they are.
    #[test]
    fn a_pull_request_without_an_approval_leaves_a_gate_waiting_for_one_open() {
        for decision in ["null", r#""REVIEW_REQUIRED""#, r#""CHANGES_REQUESTED""#] {
            let answered = format!(
                r#"{{"state":"OPEN","isDraft":false,"mergeCommit":null,"reviewDecision":{decision}}}"#
            );
            let runner = FakeRunner::default().with(&queried(42..=42), &answer(42, &answered));

            assert_eq!(
                settled_together(&runner, &[awaited(42, Until::Approved, &["ark-qca"])]),
                [Settled::NothingNew { unseen: vec![] }],
                "{decision}"
            );
        }
    }

    /// The runner panics on any call it was not given, so a tracker asked
    /// anything at all fails the test.
    #[test]
    fn an_approval_nothing_waits_for_asks_no_tracker_anything() {
        let runner = FakeRunner::default().with(&queried(42..=42), &answer(42, APPROVED));

        assert_eq!(
            settled_together(&runner, &[awaited(42, Until::Merged, &["ark-qca"])]),
            [Settled::NothingNew { unseen: vec![] }]
        );
    }

    #[test]
    fn a_merge_closes_a_gate_waiting_for_approval_beside_the_one_waiting_for_the_merge() {
        let resolving_eb1 = written(
            "arkham",
            &format!("gate resolve ark-eb1 --reason {MERGED_REASON}"),
        );
        let runner = captured(FakeRunner::default(), "arkham")
            .with(
                &gate_list("arkham"),
                &gate_list_with_42_awaiting("approved"),
            )
            .with(&viewed(42), &answer(42, MERGED))
            .with(&resolving_eb1, "")
            .with(&resolving_42("arkham"), "");

        let settled = settled(&runner, &[project("arkham")], &pr(42));

        assert_eq!(acts(settled), [merged("ark-eb1"), merged("ark-0i5")]);
        assert_eq!(writes(&runner), [resolving_eb1, resolving_42("arkham")]);
    }

    /// The runner panics on any call it was not given, so a tracker asked
    /// anything at all fails the test.
    #[test]
    fn a_draft_asks_no_tracker_anything_however_often_it_is_settled() {
        let runner = FakeRunner::default().with(&viewed(42), &answer(42, DRAFT));

        for _ in 0..2 {
            assert_eq!(
                settled(&runner, &[project("arkham")], &pr(42)),
                Settled::NothingNew { unseen: vec![] }
            );
        }
    }

    /// An open pull request is settled on every look until it merges, so a
    /// look that asked the trackers about each one would cost a read of every
    /// tracker for each. The runner panics on any call it was not given.
    #[test]
    fn a_look_asks_no_tracker_about_an_open_pull_request_no_gate_waits_on_for_review() {
        let runner = FakeRunner::default().with(&queried(42..=42), &answer(42, OPEN));

        assert_eq!(
            settled_together(&runner, &[awaited(42, Until::Merged, &["ark-qca"])]),
            [Settled::NothingNew { unseen: vec![] }]
        );
    }

    #[test]
    fn an_open_pull_request_settled_on_its_own_asks_the_trackers_whether_a_gate_waits_for_review() {
        let runner = captured(FakeRunner::default(), "arkham").with(&viewed(42), &answer(42, OPEN));

        let settled = settled(&runner, &[project("arkham")], &pr(42));

        assert_eq!(acts(settled), []);
        assert_eq!(writes(&runner), Vec::<String>::new());
    }

    #[test]
    fn a_gate_awaiting_what_bdi_gates_does_not_know_is_reported_with_it() {
        let mut gate = gate(Some("42"), Some("example/ark"));
        gate.metadata
            .insert(gate::AWAITS.to_string(), "merged".to_string());

        assert_eq!(
            PrGate::of(&gate, Vec::new()).awaits,
            Err(vec![Fault::UnknownAwaits("merged".to_string())])
        );
    }

    #[test]
    fn a_gate_naming_its_repository_in_another_case_still_waits_on_the_pull_request() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&viewed_in("Example/Ark", 42), &answer(42, MERGED))
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

        assert_eq!(acts(settled), [merged("ark-0i5")]);
    }

    #[test]
    fn a_close_settled_again_under_another_case_of_its_repository_adds_no_second_comment() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&viewed_in("Example/Ark", 7), &answer(7, CLOSED))
            .with(&comments_on("arkham", "ark-2ud"), TOLD)
            .with(&comments_on("arkham", "ark-45c"), TOLD);

        let settled = settled(
            &runner,
            &[project("arkham")],
            &PullRequest {
                repo: "Example/Ark".to_string(),
                number: 7,
            },
        );

        assert_eq!(
            acts(settled),
            [
                closed("ark-2ud", Done::AlreadyCommented),
                closed("ark-45c", Done::AlreadyCommented)
            ]
        );
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

    /// What settling came to in arkham alone, and the events GitHub hid.
    fn acted_and_unseen(settled: Settled) -> (Vec<Act>, Vec<Unseen>) {
        match settled {
            Settled::Acted {
                mut projects,
                unseen,
            } => {
                assert_eq!(projects.len(), 1, "one project was configured");
                let project = projects.remove(0);
                (project.acts.expect("arkham's tracker answered"), unseen)
            }
            not_acted => panic!("settling came to {not_acted:?}"),
        }
    }

    fn happenings(unseen: &[Unseen]) -> Vec<&'static str> {
        unseen.iter().map(|unseen| unseen.happening).collect()
    }

    /// A field GitHub answers null without saying why, where the event
    /// reading it cannot take a null.
    #[test]
    fn a_pull_request_with_a_field_one_event_cannot_read_is_settled_by_the_others() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(
                &viewed(42),
                &answer(
                    42,
                    r#"{"state":"MERGED","isDraft":false,"mergeCommit":{"oid":"5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff"},"commits":null}"#,
                ),
            )
            .with(&resolving_42("arkham"), "");

        let (acts, unseen) = acted_and_unseen(settled(&runner, &[project("arkham")], &pr(42)));

        assert_eq!(acts, [merged("ark-0i5")]);
        assert_eq!(happenings(&unseen), ["has failing checks"]);
        assert!(unseen[0].why.contains("cannot read"), "{}", unseen[0].why);
    }

    /// Each of `pulls`, a number and its fields, as GitHub answers a token
    /// refused the checks on their heads: the data with `statusCheckRollup`
    /// null, and an error naming it for each.
    fn checks_refused(pulls: &[(u64, &str)]) -> String {
        let data: Vec<String> = pulls
            .iter()
            .map(|(number, fields)| {
                format!(
                    r#""pr{number}":{{{fields},"commits":{{"nodes":[{{"commit":{{"oid":"a1b2c3","statusCheckRollup":null}}}}]}}}}"#
                )
            })
            .collect();
        let errors: Vec<String> = pulls
            .iter()
            .map(|(number, _)| {
                format!(
                    r#"{{"type":"FORBIDDEN","path":["repository","pr{number}","commits","nodes",0,"commit","statusCheckRollup"],"message":"Resource not accessible by personal access token"}}"#
                )
            })
            .collect();
        format!(
            r#"{{"data":{{"repository":{{{}}}}},"errors":[{}]}}"#,
            data.join(","),
            errors.join(",")
        )
    }

    /// #42 merged, as [`MERGED`] has it, without its braces.
    const MERGED_FIELDS: &str = r#""state":"MERGED","isDraft":false,"mergeCommit":{"oid":"5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff"}"#;

    fn refused_the_checks() -> Unseen {
        Unseen {
            happening: "has failing checks",
            why: "GitHub would not let gh read commits: Resource not accessible by personal \
                  access token"
                .to_string(),
        }
    }

    /// gh prints GitHub's whole answer, then exits 1 for the error in it. The
    /// refusal is what says the checks are unseen: the null it leaves reads
    /// as a head with no checks at all.
    #[test]
    fn a_merge_settles_while_github_refuses_the_token_the_checks() {
        let runner = captured(FakeRunner::default(), "arkham")
            .failing_having_printed(
                &viewed(42),
                &checks_refused(&[(42, MERGED_FIELDS)]),
                unavailable("gh"),
            )
            .with(&resolving_42("arkham"), "");

        let settled = settled(&runner, &[project("arkham")], &pr(42));

        assert_eq!(
            acted_and_unseen(settled),
            (vec![merged("ark-0i5")], vec![refused_the_checks()])
        );
    }

    #[test]
    fn an_approval_settles_while_github_refuses_the_token_the_checks() {
        let resolving = written(
            "arkham",
            &format!("gate resolve ark-eb1 --reason {APPROVED_REASON}"),
        );
        let runner = captured(FakeRunner::default(), "arkham")
            .with(
                &gate_list("arkham"),
                &gate_list_with_42_awaiting("approved"),
            )
            .failing_having_printed(
                &viewed(42),
                &checks_refused(&[(
                    42,
                    r#""state":"OPEN","isDraft":false,"mergeCommit":null,"reviewDecision":"APPROVED""#,
                )]),
                unavailable("gh"),
            )
            .with(&resolving, "");

        let settled = settled(&runner, &[project("arkham")], &pr(42));

        assert_eq!(
            acted_and_unseen(settled),
            (
                vec![act("ark-eb1", "is approved", Done::Resolved)],
                vec![refused_the_checks()]
            )
        );
    }

    /// The runner panics on any call it was not given, so falling back to a
    /// query for each pull request would fail the test.
    #[test]
    fn a_look_github_refuses_the_checks_of_settles_every_pull_request_in_its_one_query() {
        let both = checks_refused(&[
            (41, r#""state":"OPEN","isDraft":true,"mergeCommit":null"#),
            (42, MERGED_FIELDS),
        ]);
        let runner = captured(FakeRunner::default(), "arkham")
            .failing_having_printed(&queried(41..=42), &both, unavailable("gh"))
            .with(&resolving_42("arkham"), "");

        let settled = settled_together(
            &runner,
            &[
                awaited(41, Until::Merged, &["ark-2ud"]),
                awaited(42, Until::Merged, &["ark-45c"]),
            ],
        );

        let [first, second]: [Settled; 2] = settled.try_into().expect("two were settled");
        assert_eq!(
            first,
            Settled::NothingNew {
                unseen: vec![refused_the_checks()]
            }
        );
        assert_eq!(
            acted_and_unseen(second),
            (vec![merged("ark-0i5")], vec![refused_the_checks()])
        );
    }

    #[test]
    fn a_gate_bd_fails_to_close_is_reported() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&viewed(42), &answer(42, MERGED))
            .failing(&resolving_42("arkham"), unavailable("bd"));

        let settled = settled(&runner, &[project("arkham")], &pr(42));

        assert_eq!(
            acts(settled),
            [Act {
                bead: "ark-0i5".to_string(),
                happening: "merged",
                done: Err(unavailable("bd")),
            }]
        );
    }

    #[test]
    fn a_bead_whose_comments_bd_cannot_read_is_reported_and_not_commented_on() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&viewed(7), &answer(7, CLOSED))
            .failing(&comments_on("arkham", "ark-2ud"), unavailable("bd"))
            .with(&comments_on("arkham", "ark-45c"), NO_COMMENTS)
            .with(&telling("arkham", "ark-45c"), "");

        let settled = settled(&runner, &[project("arkham")], &pr(7));

        assert_eq!(
            acts(settled),
            [
                Act {
                    bead: "ark-2ud".to_string(),
                    happening: "closed unmerged",
                    done: Err(unavailable("bd")),
                },
                closed("ark-45c", Done::Commented)
            ]
        );
        assert_eq!(writes(&runner), [telling("arkham", "ark-45c")]);
    }

    #[test]
    fn a_comment_bd_fails_to_add_is_reported() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&viewed(7), &answer(7, CLOSED))
            .with(&comments_on("arkham", "ark-2ud"), NO_COMMENTS)
            .with(&comments_on("arkham", "ark-45c"), TOLD)
            .failing(&telling("arkham", "ark-2ud"), unavailable("bd"));

        let settled = settled(&runner, &[project("arkham")], &pr(7));

        assert_eq!(
            acts(settled),
            [
                Act {
                    bead: "ark-2ud".to_string(),
                    happening: "closed unmerged",
                    done: Err(unavailable("bd")),
                },
                closed("ark-45c", Done::AlreadyCommented)
            ]
        );
    }

    #[test]
    fn a_tracker_that_does_not_say_which_gates_wait_is_reported_and_the_others_are_settled() {
        let runner = captured(FakeRunner::default(), "dunwich")
            .failing(&gate_list("arkham"), unavailable("bd"))
            .with(&viewed(42), &answer(42, MERGED))
            .with(&resolving_42("dunwich"), "");

        let settled = settled(&runner, &[project("arkham"), project("dunwich")], &pr(42));

        assert_eq!(
            settled,
            Settled::Acted {
                projects: vec![
                    ProjectSettled {
                        project: "arkham".to_string(),
                        acts: Err(OpenFailure::Refused(unavailable("bd"))),
                    },
                    ProjectSettled {
                        project: "dunwich".to_string(),
                        acts: Ok(vec![merged("ark-0i5")]),
                    },
                ],
                unseen: vec![],
            }
        );
        assert_eq!(writes(&runner), [resolving_42("dunwich")]);
    }

    /// #`number` in example/ark, as a look found a gate in arkham waiting on
    /// it for `until` and holding back `blocks`.
    fn awaited(number: u64, until: Until, blocks: &[&str]) -> Awaited {
        Awaited {
            pull_request: pr(number),
            waiting: vec![Waiting {
                project: "arkham".to_string(),
                until,
                blocks: blocks.iter().map(|bead| bead.to_string()).collect(),
                made: None,
            }],
        }
    }

    fn settled_together(runner: &FakeRunner, awaited: &[Awaited]) -> Vec<Settled> {
        settle_together(
            &Cli::new(runner),
            runner,
            &[project("arkham")],
            &EVENTS,
            awaited,
            &Told::default(),
        )
        .collect()
    }

    /// The runner panics on any call it was not given, so asking again would
    /// fail the test.
    #[test]
    fn a_lone_pull_request_github_does_not_have_is_asked_about_once() {
        let gone = RunFailure {
            kind: FailureKind::Gone,
            program: "gh".to_string(),
            detail: "gh found no such repository or pull request on GitHub".to_string(),
            unreadable: None,
        };
        let runner = FakeRunner::default().failing(&queried(7..=7), gone.clone());

        assert_eq!(
            settled_together(&runner, &[awaited(7, Until::Merged, &["ark-2ud"])]),
            [Settled::Unread(gone)]
        );
    }

    /// The runner panics on any call it was not given, so asking GitHub
    /// would fail the test.
    #[test]
    fn a_pull_request_in_a_repository_no_query_can_name_is_unread_without_asking_github() {
        let unnamed = |number| Awaited {
            pull_request: PullRequest {
                repo: "ark".to_string(),
                number,
            },
            waiting: Vec::new(),
        };
        let runner = FakeRunner::default();

        let settled = settled_together(&runner, &[unnamed(7), unnamed(42)]);

        assert_eq!(settled.len(), 2);
        for each in settled {
            match each {
                Settled::Unread(failure) => {
                    assert_eq!(failure.kind, FailureKind::Gone);
                    assert!(failure.detail.contains("ark"), "{}", failure.detail);
                }
                settled => panic!("settling came to {settled:?}"),
            }
        }
    }

    /// The query about each of `numbers` in example/ark.
    fn queried(numbers: RangeInclusive<u64>) -> String {
        let asked: Vec<String> = numbers
            .map(|n| format!("pr{n}:pullRequest(number:{n}){{state isDraft mergeCommit{{oid}} reviewDecision commits(last:1){{nodes{{commit{{oid statusCheckRollup{{state contexts(last:100){{nodes{{...on CheckRun{{conclusion completedAt}} ...on StatusContext{{state createdAt}}}}}}}}}}}}}} reviews(last:5){{nodes{{url state submittedAt author{{login}}}}}} mergeable headRefOid headRef{{target{{...on Commit{{committedDate}}}}}} baseRef{{target{{...on Commit{{committedDate}}}}}} comments(last:5){{nodes{{url createdAt author{{login}}}}}}}}"))
            .collect();
        format!(
            "gh api graphql -f owner=example -f name=ark -f query=query($owner:String!,\
             $name:String!){{repository(owner:$owner,name:$name){{{}}}}}",
            asked.join(" ")
        )
    }

    #[test]
    fn a_repository_with_more_pull_requests_than_one_query_asks_about_is_asked_in_several() {
        let unavailable = RunFailure {
            kind: FailureKind::Unavailable,
            program: "gh".to_string(),
            detail: "gh exited 1 for a reason bdi cannot place".to_string(),
            unreadable: None,
        };
        let runner = FakeRunner::default()
            .failing(&queried(1..=100), unavailable.clone())
            .with(&queried(101..=101), &answer(101, OPEN));
        let awaited: Vec<Awaited> = (1..=101)
            .map(|number| awaited(number, Until::Merged, &[]))
            .collect();

        let mut expected: Vec<Settled> = (1..=100)
            .map(|_| Settled::Unread(unavailable.clone()))
            .collect();
        expected.push(Settled::NothingNew { unseen: vec![] });
        assert_eq!(settled_together(&runner, &awaited), expected);
        assert_eq!(runner.calls().len(), 2);
    }

    /// The runner panics on any call it was not given, so asking about #101
    /// would fail the test.
    #[test]
    fn a_caller_that_stops_early_asks_about_no_more_pull_requests() {
        let runner = FakeRunner::default().with(&queried(1..=100), r#"{"data":{"repository":{}}}"#);
        let awaited: Vec<Awaited> = (1..=101)
            .map(|number| awaited(number, Until::Merged, &[]))
            .collect();

        let first = settle_together(
            &Cli::new(&runner),
            &runner,
            &[project("arkham")],
            &EVENTS,
            &awaited,
            &Told::default(),
        )
        .next();

        assert!(matches!(first, Some(Settled::Unread(_))), "{first:?}");
        assert_eq!(runner.calls().len(), 1);
    }

    /// An event no `bdi gates` has, which tells each bead held back by a
    /// gate on an open pull request its head commit, as an event for
    /// failing checks would.
    const HEAD: Event = Event {
        fields: &["headRefOid"],
        happening: "has a new head",
        outcome: |pr, observed| {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Fields {
                head_ref_oid: String,
            }
            let Fields { head_ref_oid } = serde::Deserialize::deserialize(&observed.fields)?;
            Ok((observed.state == github::State::Open).then(|| {
                Outcome::Tell(vec![pr_events::Telling {
                    text: format!("Pull request {pr} is at {head_ref_oid}."),
                    happened: None,
                }])
            }))
        },
    };

    fn head_queried() -> String {
        "gh api graphql -f owner=example -f name=ark -f query=query($owner:String!,\
         $name:String!){repository(owner:$owner,name:$name){pr7:pullRequest(number:7)\
         {state headRefOid}}}"
            .to_string()
    }

    fn at(head: &str) -> String {
        answer(7, &format!(r#"{{"state":"OPEN","headRefOid":"{head}"}}"#))
    }

    fn telling_head(bead: &str, head: &str) -> String {
        written(
            "arkham",
            &format!("comments add {bead} Pull request example/ark#7 is at {head}."),
        )
    }

    /// A runner answering for arkham with the captured tracker, GitHub
    /// saying #7 is at `head`, and every bead #7's gate holds back without
    /// a comment and taking one about `head`.
    fn head_runner(head: &str) -> FakeRunner {
        captured(FakeRunner::default(), "arkham")
            .with(&head_queried(), &at(head))
            .with(&comments_on("arkham", "ark-2ud"), NO_COMMENTS)
            .with(&comments_on("arkham", "ark-45c"), NO_COMMENTS)
            .with(&telling_head("ark-2ud", head), "")
            .with(&telling_head("ark-45c", head), "")
    }

    fn looked_at_head(runner: &FakeRunner, told: &Told) -> Vec<Settled> {
        settle_together(
            &Cli::new(runner),
            runner,
            &[project("arkham")],
            &[HEAD],
            &[awaited(7, Until::Merged, &["ark-2ud", "ark-45c"])],
            told,
        )
        .collect()
    }

    #[test]
    fn an_event_asks_github_for_its_own_fields_and_tells_each_held_back_bead() {
        let runner = head_runner("c0ffee");

        let settled = looked_at_head(&runner, &Told::default());

        assert_eq!(settled.len(), 1);
        let told: Vec<Act> = acts(settled.into_iter().next().expect("one was settled"));
        assert_eq!(
            told,
            [
                act("ark-2ud", "has a new head", Done::Commented),
                act("ark-45c", "has a new head", Done::Commented)
            ]
        );
    }

    /// The runner panics on any call it was not given, so a look that asked
    /// any tracker anything the second time would fail the test.
    #[test]
    fn a_look_with_nothing_new_to_tell_asks_no_tracker_anything() {
        let told = Told::default();
        let first = head_runner("c0ffee");
        looked_at_head(&first, &told);
        let second = FakeRunner::default().with(&head_queried(), &at("c0ffee"));

        assert_eq!(
            looked_at_head(&second, &told),
            [Settled::NothingNew { unseen: vec![] }]
        );
    }

    #[test]
    fn a_new_key_for_the_same_event_is_told_again() {
        let told = Told::default();
        looked_at_head(&head_runner("c0ffee"), &told);
        let moved = head_runner("decade");

        let settled = looked_at_head(&moved, &told);

        assert_eq!(
            writes(&moved),
            [
                telling_head("ark-2ud", "decade"),
                telling_head("ark-45c", "decade")
            ]
        );
        assert_eq!(settled.len(), 1);
    }

    /// An event no `bdi gates` has, which closes every gate on a merge, as
    /// the merge does.
    const ALSO_ON_MERGE: Event = Event {
        fields: &[],
        happening: "merged again",
        outcome: |_, observed| {
            Ok(
                (observed.state == github::State::Merged).then(|| Outcome::Resolve {
                    awaited: |_| true,
                    reason: "Merged again.".to_string(),
                }),
            )
        },
    };

    /// The runner panics on any call it was not given, so a second close of
    /// ark-0i5 would fail the test.
    #[test]
    fn a_gate_two_events_would_close_is_closed_once() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&viewed(42), &answer(42, MERGED))
            .with(&resolving_42("arkham"), "");
        let events = [EVENTS.as_slice(), &[ALSO_ON_MERGE]].concat();

        let settled = settle(
            &Cli::new(&runner),
            &runner,
            &[project("arkham")],
            &events,
            &pr(42),
            &Told::default(),
        );

        assert_eq!(acts(settled), [merged("ark-0i5")]);
    }

    /// A bead the look found held back that this process has not told is
    /// enough to act, even where another bead behind the same gate was told.
    #[test]
    fn a_look_tells_a_bead_newly_held_back_behind_a_gate_already_told() {
        let told = Told::default();
        told.learn(
            "arkham",
            "ark-2ud",
            "Pull request example/ark#7 is at c0ffee.",
        );
        let runner = head_runner("c0ffee");

        looked_at_head(&runner, &told);

        assert_eq!(writes(&runner), [telling_head("ark-45c", "c0ffee")]);
    }
}
