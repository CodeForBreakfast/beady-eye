use std::path::{Path, PathBuf};

use crate::collect::run::{Env, FailureKind, Runner};
use crate::config::{Anomalies, Config, Join, Project, Roots, Tui};

/// The single project `bdi` reads when no config file names one: the
/// repository the current directory sits in, on the ambient credential.
///
/// bd and git are asked where their own things are rather than walked for
/// here, so `BEADS_DIR`, a redirect or a worktree resolves the way it does
/// for any other command run in the same place.
pub fn from_the_current_directory(
    runner: &dyn Runner,
    cwd: &Path,
    name_from_the_environment: Option<&str>,
) -> anyhow::Result<Config> {
    if let Err(failure) = runner.run("bd", &["where", "--json"], Some(cwd), &Env::new()) {
        // bd that never ran has said nothing about this directory.
        if failure.kind == FailureKind::Exec {
            return Err(failure.into());
        }
        anyhow::bail!("{} is not in anything beads tracks", cwd.display());
    }

    let repository = git(runner, cwd, &["rev-parse", "--show-toplevel"]).map(PathBuf::from);
    // A directory in no repository has no worktrees to list, and asking
    // git for them is only a second way to hear that.
    let worktrees = match &repository {
        Some(_) => worktrees_of(runner, cwd),
        None => Vec::new(),
    };
    let root = repository.unwrap_or_else(|| cwd.to_path_buf());
    let name = name_from_the_environment
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .or_else(|| {
            git(runner, cwd, &["remote", "get-url", "origin"]).map(|url| repository_name(&url))
        })
        .unwrap_or_else(|| directory_name(&root));

    Ok(Config {
        projects: vec![Project {
            name,
            path: root,
            credential_command: None,
            worktrees,
        }],
        roots: Roots::default(),
        badges: Vec::new(),
        anomalies: Anomalies::default(),
        join: Join::default(),
        tui: Tui::default(),
    })
}

/// Every working tree of the repository the directory sits in, as
/// `git worktree list` reports them, and empty where git reports none.
///
/// A seat that works in its own worktree leaves its siblings' panes in
/// directories under neither each other nor the checkout, so a project's
/// territory is all of its working trees rather than only the one bdi was
/// run from. Each porcelain record opens with the directory and continues
/// with the commit and the branch, which name no directory at all.
fn worktrees_of(runner: &dyn Runner, cwd: &Path) -> Vec<PathBuf> {
    let Some(listed) = git(runner, cwd, &["worktree", "list", "--porcelain"]) else {
        return Vec::new();
    };
    listed
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .map(PathBuf::from)
        .collect()
}

/// One line of git's answer, or nothing where git has none to give: no
/// repository, no remote, or no git at all.
fn git(runner: &dyn Runner, cwd: &Path, args: &[&str]) -> Option<String> {
    let said = runner.run("git", args, Some(cwd), &Env::new()).ok()?;
    let line = said.trim();
    (!line.is_empty()).then(|| line.to_string())
}

/// The repository a remote URL names, in any of the spellings git accepts:
/// `git@host:owner/name.git`, `https://host/owner/name`, `/srv/git/name.git`.
fn repository_name(url: &str) -> String {
    let named = url.trim_end_matches('/');
    let named = named.rsplit(['/', ':']).next().unwrap_or(named);
    named.trim_end_matches(".git").to_string()
}

/// What a directory is called, or its whole path where it is called nothing.
fn directory_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::run::testing::FakeRunner;
    use crate::collect::run::{RealRunner, RunFailure};

    /// A repository beads tracks, as bd and git answer for it. The remote and
    /// the directory disagree deliberately, so a test can tell which was read.
    fn a_tracked_repository() -> FakeRunner {
        FakeRunner::default()
            .with("bd where --json", r#"{"path":"/srv/work/orbital/.beads"}"#)
            .with("git rev-parse --show-toplevel", "/srv/work/orbital\n")
            .with(
                "git remote get-url origin",
                "git@github.com:pilot/ground-station.git\n",
            )
            .with("git worktree list --porcelain", ONE_CHECKOUT)
    }

    /// `git worktree list --porcelain` for a repository nobody has added a
    /// worktree to: the checkout itself, and nothing else.
    const ONE_CHECKOUT: &str = "\
worktree /srv/work/orbital
HEAD 4d3c1f0e9b8a7c6d5e4f3a2b1c0d9e8f7a6b5c4d
branch refs/heads/main
";

    /// The same, for a repository worked in the way this one is: a checkout
    /// and a worktree per seat, each somewhere else entirely.
    const A_WORKTREE_PER_SEAT: &str = "\
worktree /srv/work/orbital
HEAD 4d3c1f0e9b8a7c6d5e4f3a2b1c0d9e8f7a6b5c4d
branch refs/heads/main

worktree /tmp/seat-a/wt
HEAD 1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b
detached

worktree /tmp/seat-b/wt
HEAD 9f8e7d6c5b4a39281706f5e4d3c2b1a09f8e7d6c
detached
";

    fn no_such_repository() -> RunFailure {
        RunFailure {
            kind: FailureKind::Unavailable,
            program: "git".to_string(),
            detail: "git exited 128 for a reason bdi cannot place".to_string(),
        }
    }

    #[test]
    fn the_repository_the_directory_sits_in_becomes_the_one_project() {
        let cfg = from_the_current_directory(
            &a_tracked_repository(),
            Path::new("/srv/work/orbital/src"),
            None,
        )
        .expect("the repository is a project");

        assert_eq!(
            cfg.projects,
            vec![Project {
                name: "ground-station".to_string(),
                path: PathBuf::from("/srv/work/orbital"),
                credential_command: None,
                worktrees: vec![PathBuf::from("/srv/work/orbital")],
            }]
        );
    }

    /// The bug this fixes: a seat works in its own worktree, its siblings
    /// work in theirs, and the panes are in none of the directories bdi was
    /// run from. All of them are the project.
    #[test]
    fn every_worktree_of_the_repository_belongs_to_the_project() {
        let runner =
            a_tracked_repository().with("git worktree list --porcelain", A_WORKTREE_PER_SEAT);

        let cfg = from_the_current_directory(&runner, Path::new("/tmp/seat-a/wt"), None)
            .expect("the repository is a project");

        assert_eq!(
            cfg.projects[0].worktrees,
            vec![
                PathBuf::from("/srv/work/orbital"),
                PathBuf::from("/tmp/seat-a/wt"),
                PathBuf::from("/tmp/seat-b/wt"),
            ]
        );
    }

    /// Degrade, never disappear: git that answers nothing leaves the project
    /// holding the one directory it was found in.
    #[test]
    fn a_repository_git_lists_no_worktrees_for_still_holds_its_own_path() {
        let runner = FakeRunner::default()
            .with("bd where --json", r#"{"path":"/srv/work/orbital/.beads"}"#)
            .with("git rev-parse --show-toplevel", "/srv/work/orbital\n")
            .with(
                "git remote get-url origin",
                "git@github.com:pilot/ground-station.git\n",
            )
            .failing("git worktree list --porcelain", no_such_repository());

        let cfg = from_the_current_directory(&runner, Path::new("/srv/work/orbital"), None)
            .expect("the repository is a project");

        assert!(cfg.projects[0].worktrees.is_empty());
        assert!(
            cfg.projects[0]
                .holds(Path::new("/srv/work/orbital/src"))
                .is_some(),
            "a project git listed no worktrees for holds nothing at all"
        );
    }

    /// A porcelain record is more than its first line, and only the first
    /// line names a directory.
    #[test]
    fn only_the_worktree_lines_of_the_listing_are_directories() {
        let runner =
            a_tracked_repository().with("git worktree list --porcelain", A_WORKTREE_PER_SEAT);

        let cfg = from_the_current_directory(&runner, Path::new("/tmp/seat-a/wt"), None)
            .expect("the repository is a project");

        assert!(
            cfg.projects[0]
                .worktrees
                .iter()
                .all(|w| w.starts_with("/srv") || w.starts_with("/tmp")),
            "a HEAD or a branch was read as a directory: {:?}",
            cfg.projects[0].worktrees
        );
    }

    /// A directory of our own to build a repository in, outside anything
    /// this checkout tracks.
    fn a_scratch_directory(named: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("bdi-{named}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("the directory is ours to make");
        std::fs::canonicalize(&path).expect("the directory we just made resolves")
    }

    /// git run with an identity and a default branch of our own, so the test
    /// says the same thing on a machine whose git is configured differently
    /// and on one where it is not configured at all.
    fn git_in(cwd: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=bdi tests",
                "-c",
                "user.email=tests@beady-eye.invalid",
                "-c",
                "init.defaultBranch=main",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?} in {}: {}",
            cwd.display(),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// The parse is worth only what git actually prints, so this one builds a
    /// repository and a linked worktree and reads git's own answer through
    /// the runner that really runs it.
    ///
    /// It reads from the *linked* worktree, because that is the seat that saw
    /// nothing: a worktree that cannot name the checkout it came from staffs
    /// no row in it.
    #[test]
    fn a_worktree_lists_the_checkout_it_was_added_from_and_itself() {
        let scratch = a_scratch_directory("worktree-listing");
        let checkout = scratch.join("checkout");
        std::fs::create_dir_all(&checkout).expect("the directory is ours to make");
        git_in(&checkout, &["init"]);
        git_in(&checkout, &["commit", "--allow-empty", "-m", "root"]);

        let linked = scratch.join("seat/wt");
        git_in(
            &checkout,
            &["worktree", "add", "--detach", &linked.display().to_string()],
        );

        assert_eq!(
            worktrees_of(&RealRunner, &linked),
            vec![checkout, linked],
            "git did not list both working trees the way the parse expects"
        );

        std::fs::remove_dir_all(&scratch).expect("the directory is ours to remove");
    }

    /// The listing is asked for where bdi was run, because a worktree only
    /// knows its siblings from inside the repository.
    #[test]
    fn the_worktrees_are_listed_from_where_bdi_was_run() {
        let runner = a_tracked_repository();

        from_the_current_directory(&runner, Path::new("/srv/work/orbital/src"), None)
            .expect("the repository is a project");

        assert_eq!(
            runner.call("git worktree list --porcelain").cwd,
            Some(PathBuf::from("/srv/work/orbital/src"))
        );
    }

    #[test]
    fn a_synthesised_project_gets_every_other_default() {
        let cfg = from_the_current_directory(
            &a_tracked_repository(),
            Path::new("/srv/work/orbital"),
            None,
        )
        .expect("the repository is a project");

        assert_eq!(cfg.roots, Roots::default());
        assert!(cfg.badges.is_empty());
        assert_eq!(cfg.anomalies.stale_claim_days, 30);
        assert_eq!(cfg.join.pane_key, "agent_pane");
        assert_eq!(cfg.tui, Tui::default());
    }

    #[test]
    fn the_environment_names_the_project_ahead_of_git() {
        let cfg = from_the_current_directory(
            &a_tracked_repository(),
            Path::new("/srv/work/orbital"),
            Some("atlas"),
        )
        .expect("the repository is a project");

        assert_eq!(cfg.projects[0].name, "atlas");
    }

    #[test]
    fn an_empty_name_in_the_environment_is_no_name_at_all() {
        let cfg = from_the_current_directory(
            &a_tracked_repository(),
            Path::new("/srv/work/orbital"),
            Some(""),
        )
        .expect("the repository is a project");

        assert_eq!(cfg.projects[0].name, "ground-station");
    }

    #[test]
    fn a_repository_with_no_remote_is_named_by_its_directory() {
        let runner = FakeRunner::default()
            .with("bd where --json", r#"{"path":"/srv/work/orbital/.beads"}"#)
            .with("git rev-parse --show-toplevel", "/srv/work/orbital\n")
            .with("git worktree list --porcelain", ONE_CHECKOUT)
            .failing("git remote get-url origin", no_such_repository());

        let cfg = from_the_current_directory(&runner, Path::new("/srv/work/orbital"), None)
            .expect("the repository is a project");

        assert_eq!(cfg.projects[0].name, "orbital");
    }

    #[test]
    fn every_spelling_of_a_remote_names_the_same_project() {
        for url in [
            "git@github.com:pilot/ground-station.git",
            "https://github.com/pilot/ground-station.git",
            "https://github.com/pilot/ground-station",
            "ssh://git@host/~pilot/ground-station.git/",
            "/srv/git/ground-station.git",
        ] {
            let runner = FakeRunner::default()
                .with("bd where --json", r#"{"path":"/srv/work/orbital/.beads"}"#)
                .with("git rev-parse --show-toplevel", "/srv/work/orbital\n")
                .with("git worktree list --porcelain", ONE_CHECKOUT)
                .with("git remote get-url origin", &format!("{url}\n"));

            let cfg = from_the_current_directory(&runner, Path::new("/srv/work/orbital"), None)
                .expect("the repository is a project");

            assert_eq!(cfg.projects[0].name, "ground-station", "from {url}");
        }
    }

    /// `BEADS_DIR` reaches a tracker from anywhere, so a directory in no
    /// repository is still worth reading; it is just its own project.
    #[test]
    fn a_tracker_outside_any_repository_is_read_from_where_bdi_was_run() {
        let runner = FakeRunner::default()
            .with("bd where --json", r#"{"path":"/srv/beads/.beads"}"#)
            .failing("git rev-parse --show-toplevel", no_such_repository())
            .failing("git remote get-url origin", no_such_repository());

        let cfg = from_the_current_directory(&runner, Path::new("/srv/loose"), None)
            .expect("the directory is a project");

        assert_eq!(
            cfg.projects,
            vec![Project {
                name: "loose".to_string(),
                path: PathBuf::from("/srv/loose"),
                credential_command: None,
                worktrees: Vec::new(),
            }]
        );
    }

    #[test]
    fn a_directory_beads_does_not_track_is_reported() {
        let runner = FakeRunner::default().failing(
            "bd where --json",
            RunFailure {
                kind: FailureKind::Unavailable,
                program: "bd".to_string(),
                detail: "bd exited 1 for a reason bdi cannot place".to_string(),
            },
        );

        let err = from_the_current_directory(&runner, Path::new("/srv/loose"), None)
            .unwrap_err()
            .to_string();

        assert!(err.contains("/srv/loose"), "got: {err}");
        assert!(err.contains("beads"), "got: {err}");
    }

    /// bd that never ran has said nothing about this directory, and telling
    /// someone to move is the wrong answer to a missing binary.
    #[test]
    fn a_bd_that_cannot_run_says_so_rather_than_blaming_the_directory() {
        let runner = FakeRunner::default().failing(
            "bd where --json",
            RunFailure::exec("bd", "No such file or directory (os error 2)"),
        );

        let err = from_the_current_directory(&runner, Path::new("/srv/loose"), None)
            .unwrap_err()
            .to_string();

        assert!(err.contains("bd could not be run"), "got: {err}");
        assert!(!err.contains("/srv/loose"), "got: {err}");
    }

    #[test]
    fn the_tracker_is_probed_where_bdi_was_run() {
        let runner = a_tracked_repository();

        from_the_current_directory(&runner, Path::new("/srv/work/orbital/src"), None)
            .expect("the repository is a project");

        assert_eq!(
            runner.call("bd where --json").cwd,
            Some(PathBuf::from("/srv/work/orbital/src"))
        );
    }
}
