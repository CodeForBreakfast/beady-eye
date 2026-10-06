//! The pull request a beads `gh:pr` gate waits on.
//!
//! `gh:pr` is beads' own await type rather than a setup's convention, so the
//! gate's pull request is read here without a `[[badges]]` entry naming it.

use serde::Serialize;

use crate::model::types::Bead;

/// The pull request a `gh:pr` gate names: the gate's `repo` metadata and its
/// await id, as written, and where both are usable, the page they point at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PullRequest {
    /// `OWNER/REPO` or `HOST/OWNER/REPO`, as `bd gate create` writes it.
    pub repo: Option<String>,
    /// The pull request's number, where the gate wrote a number here.
    pub await_id: String,
    pub url: Option<String>,
}

/// Why a gate's pull request has no address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unlinked {
    AwaitIdNotANumber,
    NoRepo,
    RepoNotAnAddress,
}

/// The pull request this bead waits on, where it is a `gh:pr` gate.
pub fn pull_request(bead: &Bead) -> Option<PullRequest> {
    if bead.issue_type != "gate" || bead.value("await_type") != Some("gh:pr") {
        return None;
    }
    let mut awaited = PullRequest {
        repo: bead.value("metadata.repo").map(str::to_string),
        await_id: bead.value("await_id").unwrap_or_default().to_string(),
        url: None,
    };
    awaited.url = awaited.address().ok();
    Some(awaited)
}

impl PullRequest {
    /// The number the gate waits on, where its await id is one.
    pub fn number(&self) -> Option<&str> {
        let id = self.await_id.as_str();
        (!id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())).then_some(id)
    }

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
        let number = self.number().ok_or(Unlinked::AwaitIdNotANumber)?;
        let repo = self.repo.as_deref().ok_or(Unlinked::NoRepo)?;
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
                await_id: "12".to_string(),
                url: Some("https://github.com/dunwich/arkham/pull/12".to_string()),
            })
        );
    }

    #[test]
    fn a_gate_without_a_repo_names_its_pull_request_and_has_no_link() {
        let awaited = pull_request(&gate("30")).expect("a gh:pr gate");

        assert_eq!(
            (awaited.number(), awaited.url.as_deref()),
            (Some("30"), None)
        );
        assert_eq!(awaited.unlinked(), Some(Unlinked::NoRepo));
    }

    #[test]
    fn a_gate_awaiting_no_number_has_no_link() {
        let awaited = pull_request(&gate("the-wire")).expect("a gh:pr gate");

        assert_eq!((awaited.number(), awaited.url.as_deref()), (None, None));
        assert_eq!(awaited.unlinked(), Some(Unlinked::AwaitIdNotANumber));
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
        bead.values
            .insert("metadata.repo".to_string(), repo.to_string());
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
    fn an_await_id_with_a_sign_is_not_a_number() {
        let mut bead = gate("12");
        bead.values
            .insert("await_id".to_string(), "+12".to_string());

        assert_eq!(
            pull_request(&bead).unwrap().unlinked(),
            Some(Unlinked::AwaitIdNotANumber)
        );
    }
}
