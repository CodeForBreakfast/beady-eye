//! The pull request a beads `gh:pr` gate waits on.

use crate::model::types::Bead;

/// The `await_type` bd gives a gate that waits on a pull request.
pub const PULL_REQUEST: &str = "gh:pr";

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
    /// The gate's await id is not a pull request's number.
    AwaitIdNotANumber(String),
    /// The gate's [`AWAITS`] metadata names nothing `bdi gates` waits for.
    UnknownAwaits(String),
}

/// Whether `gate` waits on a pull request, whichever one that is.
pub fn awaits_a_pull_request(gate: &Bead) -> bool {
    gate.value("await_type").as_deref() == Some(PULL_REQUEST)
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

/// The number of the pull request `gate` waits on.
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

    #[test]
    fn a_repo_of_any_other_shape_has_no_owner() {
        for repo in ["arkham", "a/b/c/d"] {
            assert_eq!(owner(repo), None, "{repo:?}");
        }
    }
}
