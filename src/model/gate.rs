//! The pull request a beads `gh:pr` gate waits on.

use crate::model::types::Bead;

/// The `await_type` bd gives a gate that waits on a pull request.
pub const PULL_REQUEST: &str = "gh:pr";

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

/// Whether `gate` waits on a pull request, whichever one that is.
pub fn awaits_a_pull_request(gate: &Bead) -> bool {
    gate.value("await_type") == Some(PULL_REQUEST)
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

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn a_repo_of_any_other_shape_has_no_owner() {
        for repo in ["arkham", "a/b/c/d"] {
            assert_eq!(owner(repo), None, "{repo:?}");
        }
    }
}
