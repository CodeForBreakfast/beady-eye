//! What a beads gate `bdi gates` settles waits on: a pull request for a
//! `gh:pr` gate, an issue for a `gh:issue` gate, a repository's next release
//! for a `gh:release` gate, a workflow run for a `gh:run` gate, its deadline
//! for a `timer` gate, and a bead closing for a `bead` gate.

use chrono::{DateTime, TimeDelta, Utc};

use crate::model::types::Bead;

/// The `await_type` bd gives a gate that waits on a pull request.
pub const PULL_REQUEST: &str = "gh:pr";

/// The `await_type` of a gate that waits on an issue closing.
pub const ISSUE: &str = "gh:issue";

/// The `await_type` of a gate that waits on a repository publishing a
/// release after the gate was made.
pub const RELEASE: &str = "gh:release";

/// The `await_type` of a gate that waits on a workflow run finishing.
pub const RUN: &str = "gh:run";

/// The `await_type` of a gate that waits for its timeout to run out after
/// the gate was made.
pub const TIMER: &str = "timer";

/// The `await_type` of a gate that waits on the bead its await id names
/// closing, in its own tracker or another.
pub const BEAD: &str = "bead";

/// The metadata key naming what a gh:pr gate waits for its pull request to
/// do, where that is anything but its merge.
pub const AWAITS: &str = "awaits";

/// What a gh:pr gate waits for its pull request to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Until {
    /// Merge, which is what a gate naming nothing waits for.
    Merged,
    /// Leave draft or merge, whichever comes first. A gate asks for it with
    /// `awaits=ready_for_review`, GitHub's name for the webhook action that
    /// takes a pull request out of draft.
    ReadyForReview,
    /// Be approved or merge, whichever comes first. A gate asks for it with
    /// `awaits=approved`. Approved is GitHub's review decision on the pull
    /// request, which a repository that requires no reviews never gives.
    Approved,
}

/// Why a gate cannot name the pull request it waits on, or what it waits
/// for that pull request to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    /// The gate's metadata holds no `repo`.
    NoRepo,
    /// The gate holds no await id.
    NoAwaitId,
    /// The gate's await id is not the number of the pull request or issue
    /// it waits on.
    AwaitIdNotANumber(String),
    /// A gh:run gate's await id is not a run's id. bd takes it for the name
    /// of a workflow whose newest run the gate waits on, and `bdi gates`
    /// does not look that run up.
    WorkflowName(String),
    /// The gate's [`AWAITS`] metadata names nothing `bdi gates` waits for.
    UnknownAwaits(String),
    /// The gate does not say when it was made, so no release can be told to
    /// have come after it and no timeout can be counted from it.
    Undated,
    /// A timer gate holds no timeout.
    NoTimeout,
    /// No configured project can be told to hold the bead a bead gate
    /// awaits: its prefix is not the gate's own, and not exactly one project
    /// states it.
    NoProjectHolds(String),
    /// The project a bead gate's awaited bead belongs in holds no such bead.
    NoSuchBead { project: String, bead: String },
}

/// Whether `gate` waits on a pull request, whichever one that is.
pub fn awaits_a_pull_request(gate: &Bead) -> bool {
    gate.value("await_type").as_deref() == Some(PULL_REQUEST)
}

/// When timer `gate` runs out: its timeout after it was made, as `bd gate
/// check` counts it. bd writes the timeout in nanoseconds and leaves out one
/// of none.
pub fn deadline(gate: &Bead) -> Result<DateTime<Utc>, Vec<Fault>> {
    let timeout = gate
        .value("timeout")
        .and_then(|nanoseconds| nanoseconds.parse().ok())
        .map(TimeDelta::nanoseconds);
    match (gate.created_at, timeout) {
        (Some(made), Some(timeout)) => Ok(made + timeout),
        (made, timeout) => Err(made
            .is_none()
            .then_some(Fault::Undated)
            .into_iter()
            .chain(timeout.is_none().then_some(Fault::NoTimeout))
            .collect()),
    }
}

/// The repository `gate` names, where it names one.
pub fn repo(gate: &Bead) -> Option<&str> {
    gate.metadata
        .get("repo")
        .map(String::as_str)
        .filter(|repo| !repo.is_empty())
}

/// A repository a gate names, taken apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Repository<'a> {
    /// The host, where the gate names one.
    pub host: Option<&'a str>,
    pub owner: &'a str,
    pub name: &'a str,
}

/// `repo` taken apart, where it is `OWNER/REPO` or `HOST/OWNER/REPO`.
pub fn repository(repo: &str) -> Option<Repository<'_>> {
    match repo.split('/').collect::<Vec<_>>()[..] {
        [owner, name] => Some(Repository {
            host: None,
            owner,
            name,
        }),
        [host, owner, name] => Some(Repository {
            host: Some(host),
            owner,
            name,
        }),
        _ => None,
    }
}

/// The account holding `repo`, where it is `OWNER/REPO` or
/// `HOST/OWNER/REPO`.
pub fn owner(repo: &str) -> Option<&str> {
    repository(repo).map(|repository| repository.owner)
}

/// The host holding `repo`, where it is `HOST/OWNER/REPO`. One that names
/// none is on whichever host `gh` picks.
pub fn host(repo: &str) -> Option<&str> {
    repository(repo).and_then(|repository| repository.host)
}

/// The number of the pull request or issue `gate` waits on, or the id of its
/// workflow run.
pub fn number(gate: &Bead) -> Result<u64, Fault> {
    let id = gate.value("await_id").ok_or(Fault::NoAwaitId)?;
    id.parse()
        .map_err(|_| Fault::AwaitIdNotANumber(id.to_string()))
}

/// What `gate` waits for its pull request to do.
pub fn until(gate: &Bead) -> Result<Until, Fault> {
    match gate.metadata.get(AWAITS).map(String::as_str) {
        None => Ok(Until::Merged),
        Some("ready_for_review") => Ok(Until::ReadyForReview),
        Some("approved") => Ok(Until::Approved),
        Some(other) => Err(Fault::UnknownAwaits(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::bd::parse_beads;
    #[test]
    fn a_repo_is_owned_by_the_account_before_its_name_whether_or_not_it_names_a_host() {
        assert_eq!(owner("dunwich/arkham"), Some("dunwich"));
        assert_eq!(owner("forge.invalid/dunwich/arkham"), Some("dunwich"));
    }

    #[test]
    fn only_a_repo_naming_its_host_has_one() {
        assert_eq!(host("forge.invalid/dunwich/arkham"), Some("forge.invalid"));
        for repo in ["dunwich/arkham", "arkham", "a/b/c/d"] {
            assert_eq!(host(repo), None, "{repo:?}");
        }
    }

    fn gate(metadata: serde_json::Value) -> Bead {
        let row = serde_json::json!([{
            "id": "ark-g1",
            "title": "Gate: gh:pr",
            "status": "open",
            "issue_type": "gate",
            "await_type": PULL_REQUEST,
            "await_id": "42",
            "metadata": metadata,
        }]);
        parse_beads(&row.to_string())
            .expect("the row parses")
            .remove(0)
    }

    #[test]
    fn a_gate_naming_nothing_it_awaits_waits_for_the_merge() {
        let gate = gate(serde_json::json!({ "repo": "dunwich/arkham" }));
        assert_eq!(until(&gate), Ok(Until::Merged));
    }

    #[test]
    fn a_gate_awaiting_ready_for_review_waits_for_its_pull_request_to_leave_draft() {
        let gate = gate(serde_json::json!({ "awaits": "ready_for_review" }));
        assert_eq!(until(&gate), Ok(Until::ReadyForReview));
    }

    #[test]
    fn a_gate_awaiting_approved_waits_for_its_pull_request_to_be_approved() {
        let gate = gate(serde_json::json!({ "awaits": "approved" }));
        assert_eq!(until(&gate), Ok(Until::Approved));
    }

    #[test]
    fn a_gate_awaiting_anything_else_is_faulty_rather_than_waiting_for_the_merge() {
        for awaits in ["merged", "Ready_For_Review", "Approved", ""] {
            let gate = gate(serde_json::json!({ "awaits": awaits }));
            assert_eq!(
                until(&gate),
                Err(Fault::UnknownAwaits(awaits.to_string())),
                "{awaits:?}"
            );
        }
    }

    fn timer(row: serde_json::Value) -> Bead {
        parse_beads(&serde_json::json!([row]).to_string())
            .expect("the row parses")
            .remove(0)
    }

    #[test]
    fn a_timer_gate_runs_out_its_timeout_after_it_was_made() {
        let captured = parse_beads(include_str!(
            "../../tests/fixtures/bd_1.3.0_gate_list_timer.json"
        ))
        .expect("the capture parses");

        assert_eq!(
            deadline(&captured[0]),
            Ok("2026-10-12T09:32:28Z".parse().expect("a time"))
        );
    }

    #[test]
    fn a_timer_gate_without_a_timeout_or_a_time_it_was_made_has_no_deadline() {
        let untimed = timer(serde_json::json!({
            "id": "ark-t1",
            "title": "the ninth gate opens",
            "status": "open",
            "issue_type": "gate",
            "created_at": "2026-10-12T07:32:28Z",
            "await_type": TIMER,
        }));
        let undated = timer(serde_json::json!({
            "id": "ark-t2",
            "title": "the ninth gate opens",
            "status": "open",
            "issue_type": "gate",
            "await_type": TIMER,
            "timeout": 7_200_000_000_000_u64,
        }));

        assert_eq!(deadline(&untimed), Err(vec![Fault::NoTimeout]));
        assert_eq!(deadline(&undated), Err(vec![Fault::Undated]));
    }

    #[test]
    fn a_repo_of_any_other_shape_has_no_owner() {
        for repo in ["arkham", "a/b/c/d"] {
            assert_eq!(owner(repo), None, "{repo:?}");
        }
    }
}
