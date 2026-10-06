//! The pull request a beads `gh:pr` gate waits on.
//!
//! `gh:pr` is beads' own await type rather than a setup's convention, so the
//! gate's pull request is read here without a `[[badges]]` entry naming it.

use serde::Serialize;

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

/// The account holding `repo`, where it is `OWNER/REPO` or
/// `HOST/OWNER/REPO`.
pub fn owner(repo: &str) -> Option<&str> {
    match repo.split('/').collect::<Vec<_>>()[..] {
        [owner, _] | [_, owner, _] => Some(owner),
        _ => None,
    }
}

/// The number of the pull request `gate` waits on.
pub fn number(gate: &Bead) -> Result<u64, Fault> {
    let id = gate.value("await_id").ok_or(Fault::NoAwaitId)?;
    id.parse()
        .map_err(|_| Fault::AwaitIdNotANumber(id.to_string()))
}

/// The pull request a `gh:pr` gate names, as the tree draws it: the gate's
/// `repo` metadata and its await id, as written, and where both are usable,
/// the page they point at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PullRequest {
    /// `OWNER/REPO` or `HOST/OWNER/REPO`, as `bd gate create` writes it.
    pub repo: Option<String>,
    /// The await id as the gate wrote it, which should be the pull request's
    /// number.
    pub await_id: Option<String>,
    pub url: Option<String>,
    #[serde(skip)]
    pub number: Result<u64, Fault>,
}

/// Why a gate's pull request has no address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unlinked {
    Fault(Fault),
    RepoNotAnAddress,
}

/// The pull request this bead waits on, where it is a `gh:pr` gate.
pub fn pull_request(bead: &Bead) -> Option<PullRequest> {
    if bead.issue_type != "gate" || !awaits_a_pull_request(bead) {
        return None;
    }
    let mut awaited = PullRequest {
        repo: repo(bead).map(str::to_string),
        await_id: bead.value("await_id").map(str::to_string),
        url: None,
        number: number(bead),
    };
    awaited.url = awaited.address().ok();
    Some(awaited)
}

impl PullRequest {
    /// The repository's own name, without its owner or host.
    pub fn name(&self) -> Option<&str> {
        self.repo.as_deref()?.rsplit('/').next()
    }

    /// What stands between this gate and a link to its pull request.
    pub fn unlinked(&self) -> Option<Unlinked> {
        self.address().err()
    }

    /// The pull request's page. A repo with no host is on GitHub, as `gh`
    /// takes one.
    fn address(&self) -> Result<String, Unlinked> {
        let number = self.number.clone().map_err(Unlinked::Fault)?;
        let repo = self.repo.as_deref().ok_or(Unlinked::Fault(Fault::NoRepo))?;
        let parts: Vec<&str> = repo.split('/').collect();
        if !parts.iter().all(|part| is_a_repo_part(part)) {
            return Err(Unlinked::RepoNotAnAddress);
        }
        match parts[..] {
            [_, _] => Ok(format!("https://github.com/{repo}/pull/{number}")),
            [_, _, _] => Ok(format!("https://{repo}/pull/{number}")),
            _ => Err(Unlinked::RepoNotAnAddress),
        }
    }
}

/// The characters `bd gate create` allows in each part of a repo.
fn is_a_repo_part(part: &str) -> bool {
    !part.is_empty()
        && part
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::bd::parse_beads;

    /// Three `gh:pr` gates and the beads they block, as bd 1.3.0's `bd list
    /// --all --include-gates` wrote them on a throwaway tracker. One bead
    /// carried a `repo`, and `bd gate create` copied it onto its gate; the
    /// other gates have none, and one waits on an await id that is no number.
    const GATES: &str = include_str!("../../tests/fixtures/bd_1.3.0_gh_pr_gates.json");

    fn gate(await_id: &str) -> Bead {
        parse_beads(GATES)
            .expect("the capture parses")
            .into_iter()
            .find(|bead| bead.issue_type == "gate" && bead.value("await_id") == Some(await_id))
            .expect("the capture holds the gate")
    }

    #[test]
    fn a_gate_with_a_repo_links_to_its_pull_request_on_github() {
        assert_eq!(
            pull_request(&gate("12")),
            Some(PullRequest {
                repo: Some("dunwich/arkham".to_string()),
                await_id: Some("12".to_string()),
                url: Some("https://github.com/dunwich/arkham/pull/12".to_string()),
                number: Ok(12),
            })
        );
    }

    #[test]
    fn a_gate_without_a_repo_names_its_pull_request_and_has_no_link() {
        let awaited = pull_request(&gate("30")).expect("a gh:pr gate");

        assert_eq!((&awaited.number, awaited.url.as_deref()), (&Ok(30), None));
        assert_eq!(awaited.unlinked(), Some(Unlinked::Fault(Fault::NoRepo)));
    }

    #[test]
    fn a_gate_awaiting_no_number_has_no_link() {
        let awaited = pull_request(&gate("the-wire")).expect("a gh:pr gate");

        assert_eq!(awaited.url, None);
        assert_eq!(
            awaited.unlinked(),
            Some(Unlinked::Fault(Fault::AwaitIdNotANumber(
                "the-wire".to_string()
            )))
        );
    }

    #[test]
    fn a_gate_awaiting_nothing_has_no_link() {
        let mut bead = gate("12");
        bead.values.remove("await_id");

        let awaited = pull_request(&bead).expect("a gh:pr gate");

        assert_eq!((awaited.await_id, awaited.url), (None, None));
        assert_eq!(awaited.number, Err(Fault::NoAwaitId));
    }

    #[test]
    fn a_bead_that_is_no_gh_pr_gate_waits_on_no_pull_request() {
        let beads = parse_beads(GATES).expect("the capture parses");
        let waiting = beads
            .iter()
            .find(|bead| bead.issue_type == "task")
            .expect("the capture holds a bead the gate blocks");
        let mut human = gate("12");
        human
            .values
            .insert("await_type".to_string(), "human".to_string());

        assert_eq!(pull_request(waiting), None);
        assert_eq!(pull_request(&human), None);
    }

    fn with_repo(repo: &str) -> PullRequest {
        let mut bead = gate("12");
        bead.metadata.insert("repo".to_string(), repo.to_string());
        pull_request(&bead).expect("a gh:pr gate")
    }

    #[test]
    fn a_repo_naming_its_host_links_to_that_host() {
        assert_eq!(
            with_repo("forge.invalid/dunwich/arkham").url.as_deref(),
            Some("https://forge.invalid/dunwich/arkham/pull/12")
        );
    }

    #[test]
    fn a_repo_that_is_no_address_has_no_link() {
        for repo in [
            "arkham",
            "dunwich/",
            "a/b/c/d",
            "dunwich/ark ham",
            "dunwich/arkham?x=1",
        ] {
            let awaited = with_repo(repo);
            assert_eq!(awaited.url, None, "{repo:?}");
            assert_eq!(
                awaited.unlinked(),
                Some(Unlinked::RepoNotAnAddress),
                "{repo:?}"
            );
        }
    }

    #[test]
    fn an_empty_repo_is_no_repo() {
        let awaited = with_repo("");

        assert_eq!(awaited.repo, None);
        assert_eq!(awaited.unlinked(), Some(Unlinked::Fault(Fault::NoRepo)));
    }

    #[test]
    fn a_repo_is_owned_by_the_account_before_its_name_whether_or_not_it_names_a_host() {
        assert_eq!(owner("dunwich/arkham"), Some("dunwich"));
        assert_eq!(owner("forge.invalid/dunwich/arkham"), Some("dunwich"));
    }

    #[test]
    fn a_repo_of_any_other_shape_has_no_owner() {
        for repo in ["arkham", "a/b/c/d"] {
            assert_eq!(owner(repo), None, "{repo:?}");
        }
    }
}
