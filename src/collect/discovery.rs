//! The parts of a configuration `bdi` measures rather than being told: the
//! project the current directory sits in, where no config names one, and the
//! working trees git lists for each project a config does name.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::collect::run::{Env, FailureKind, Runner};
use crate::config::{Config, Environment, Project, Scope};

/// The one project a run with no config file draws, and whether naming it
/// took a guess nothing else would admit to.
#[derive(Debug)]
pub struct Discovered {
    pub config: Config,
    /// git could not be run at all, so the project is named after the
    /// directory its tracker sits at the top of, and nothing else was going
    /// to name it: `BDI_PROJECT` is unset, and a config file would have kept
    /// discovery from running at all.
    ///
    /// Clear for the three git outcomes a reader can see for themselves, and
    /// clear where the environment named the project, because neither is a
    /// guess.
    pub named_without_git: bool,
}

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
) -> anyhow::Result<Discovered> {
    let tracker = match runner.run("bd", &["where", "--json"], Some(cwd), &Env::new()) {
        Ok(said) => said,
        Err(failure) => {
            // bd that never ran has said nothing about this directory.
            if matches!(
                failure.kind,
                FailureKind::NotInstalled
                    | FailureKind::Unstartable
                    | FailureKind::InstalledUnstartable
            ) {
                return Err(failure.into());
            }
            anyhow::bail!("{} is not in anything beads tracks", cwd.display());
        }
    };

    // Read off the question git is always asked, because it is the one every
    // run reaches: a name in the environment means the remote is never asked
    // for, and a git that never ran would then go unnoticed.
    let toplevel = git(runner, cwd, &["rev-parse", "--show-toplevel"]);
    let git_never_ran = matches!(toplevel, Git::NeverRan);
    let repository = toplevel.line().map(PathBuf::from);
    // A directory in no repository has no worktrees to list, and asking
    // git for them is only a second way to hear that.
    let worktrees = match &repository {
        Some(_) => worktrees_of(runner, cwd),
        None => Vec::new(),
    };
    let root = repository
        .or_else(|| the_tracked_tree_around(&tracker, cwd))
        .unwrap_or_else(|| cwd.to_path_buf());
    let named_in_the_environment = name_from_the_environment.filter(|name| !name.is_empty());
    let name = named_in_the_environment
        .map(str::to_string)
        .or_else(|| {
            git(runner, cwd, &["remote", "get-url", "origin"])
                .line()
                .map(|url| repository_name(&url))
        })
        .unwrap_or_else(|| directory_name(&root));

    Ok(Discovered {
        config: Config::naming(vec![Project {
            name,
            path: root,
            environment: Environment::Ambient,
            credential_command: None,
            poll: true,
            worktrees,
        }]),
        named_without_git: git_never_ran && named_in_the_environment.is_none(),
    })
}

/// The config scoped to the project holding `cwd` — the directory `bdi` was
/// started in — or left whole where none does.
///
/// `Project::holds` is asked first, against the paths the config names. It
/// misses one case: a linked worktree placed outside the project's tree,
/// because the scope is settled before any project has been asked where it
/// is worked. So where nothing holds the directory, git is asked once, from
/// the directory itself, for the working trees of whatever repository it
/// sits in, and the directory's counterpart in each of them is tried instead
/// — each, because a config may name a project by its place in a linked
/// worktree rather than the main one. Nothing runs in any project's
/// directory.
pub fn scoped_to_the_directory(config: Config, runner: &dyn Runner, cwd: &Path) -> Config {
    let config = config.scoped_to_the_project_holding(cwd);
    if matches!(config.scope, Scope::Directory { .. }) {
        return config;
    }
    let counterparts = the_same_place_in_each(&worktrees_of(runner, cwd), cwd);
    config.scoped_to_the_project_holding_any_of(&counterparts)
}

/// The config a file spelled out, with each project's working trees filled
/// in from git — so a project a file names holds what the same project would
/// hold had discovery found it.
///
/// A project's path is wherever its config said, which may be a directory
/// inside the repository rather than the checkout itself. A linked worktree
/// is a second copy of the whole repository, so that directory has a
/// counterpart at the same place in each of them, and those counterparts are
/// the project — never the whole repository a subdirectory happens to sit in.
pub fn with_the_working_trees_git_lists(mut config: Config, runner: &dyn Runner) -> Config {
    let scope = config.scope.clone();
    for project in config.projects.iter_mut().filter(|p| scope.reads(&p.name)) {
        let listed = worktrees_of(runner, &project.path);
        project.worktrees = the_same_place_in_each(&listed, &project.path);
    }
    config
}

/// Where `path` sits again in each of the working trees listed, given one of
/// them holds it. Nothing, where none does: git listed no working trees, or
/// listed the ones of some other repository, and the project is left holding
/// the path it was configured with.
///
/// A worktree added inside the checkout is held by both, so the one the path
/// is really in is the deepest — the tie `Project::holds` breaks the same
/// way — which is the one leaving the shortest remainder.
fn the_same_place_in_each(working_trees: &[PathBuf], path: &Path) -> Vec<PathBuf> {
    let Some(within) = working_trees
        .iter()
        .filter_map(|tree| path.strip_prefix(tree).ok())
        .min_by_key(|within| within.components().count())
    else {
        return Vec::new();
    };
    working_trees.iter().map(|tree| tree.join(within)).collect()
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
    let Some(listed) = git(runner, cwd, &["worktree", "list", "--porcelain"]).line() else {
        return Vec::new();
    };
    listed
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .map(PathBuf::from)
        .collect()
}

/// Where bd keeps the beads it answered about, as `bd where` reports it.
#[derive(Deserialize)]
struct Workspace {
    path: PathBuf,
}

/// What beads calls its workspace when it makes one inside the work it
/// tracks. A workspace under any other name was named outright rather than
/// found, so the tree it belongs to is not above it.
const WORKSPACE: &str = ".beads";

/// The tree the workspace bd found sits at the top of, where the reader is
/// working somewhere inside it.
///
/// This is what a project is rooted at when git cannot say where the
/// repository starts. The directory `bdi` was started in is not the project
/// — it is wherever the reader happened to stand — and a project rooted
/// there is named after that directory and holds no seat working outside it.
///
/// The tracker is what a run with no config draws, so the project is the
/// tree that tracker belongs to — including where a redirect put it above
/// the reader, since those are still the beads being drawn and the panes
/// working on them are still working on them. Rooting narrower than the
/// tracker is the defect this fixes: beads on the screen and no seat against
/// any of them.
///
/// Two things have to hold, because `BEADS_DIR` and bd's redirects reach a
/// tracker from anywhere and bd's answer says where the beads are rather
/// than where the work is. The workspace has to be one beads made inside a
/// tree, which is what its name says: a tracker named outright is not inside
/// the work it tracks, so the directory above it is one the two happen to
/// share — `/srv/shared-beads` read from `/srv/project` would otherwise make
/// the project `/srv`, and every sibling of the reader's own would be in it.
/// And the tree has to be above the reader, since a project rooted off to
/// one side would hold nothing they are working in at all.
///
/// Neither holding leaves the directory they were started in, which is what
/// they had before any of this.
fn the_tracked_tree_around(said: &str, cwd: &Path) -> Option<PathBuf> {
    let workspace: Workspace = serde_json::from_str(said).ok()?;
    if workspace.path.file_name() != Some(OsStr::new(WORKSPACE)) {
        return None;
    }
    let tree = workspace.path.parent()?;
    cwd.starts_with(tree).then(|| tree.to_path_buf())
}

/// One line of git's answer, or which kind of nothing git had to give.
///
/// Three of the four outcomes are one state here, because they are one state
/// to a reader: a directory in no repository, a repository with no `origin`,
/// and a git that ran and failed are each something they can see for
/// themselves. The fourth is not, so it is the one kept apart — the same
/// split `from_the_current_directory` makes for bd above, on the same three
/// kinds of failure.
enum Git {
    /// git ran and answered with one line.
    Said(String),
    /// git ran and had nothing to say.
    SaidNothing,
    /// git could not be run at all: nothing is installed under that name, or
    /// what is could not be started.
    NeverRan,
}

impl Git {
    /// The line, where there is one — for a caller with nothing different to
    /// do about a git that never ran.
    fn line(self) -> Option<String> {
        match self {
            Git::Said(line) => Some(line),
            Git::SaidNothing | Git::NeverRan => None,
        }
    }
}

/// What git said to one question, asked where bdi was run.
fn git(runner: &dyn Runner, cwd: &Path, args: &[&str]) -> Git {
    let said = match runner.run("git", args, Some(cwd), &Env::new()) {
        Ok(said) => said,
        Err(failure)
            if matches!(
                failure.kind,
                FailureKind::NotInstalled
                    | FailureKind::Unstartable
                    | FailureKind::InstalledUnstartable
            ) =>
        {
            return Git::NeverRan
        }
        Err(_) => return Git::SaidNothing,
    };
    match said.trim() {
        "" => Git::SaidNothing,
        line => Git::Said(line.to_string()),
    }
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
    use crate::collect::worktree::testing::{a_scratch_directory, git_in};
    use crate::config::{Roots, Tui};

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
        .expect("the repository is a project")
        .config;

        assert_eq!(
            cfg.projects,
            vec![Project {
                name: "ground-station".to_string(),
                path: PathBuf::from("/srv/work/orbital"),
                environment: Environment::Ambient,
                credential_command: None,
                poll: true,
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
            .expect("the repository is a project")
            .config;

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
            .expect("the repository is a project")
            .config;

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
            .expect("the repository is a project")
            .config;

        assert!(
            cfg.projects[0]
                .worktrees
                .iter()
                .all(|w| w.starts_with("/srv") || w.starts_with("/tmp")),
            "a HEAD or a branch was read as a directory: {:?}",
            cfg.projects[0].worktrees
        );
    }

    /// A config file naming one project, in the repository the fake answers
    /// for.
    const ONE_CONFIGURED_PROJECT: &str = r#"
[[projects]]
name = "orbital"
path = "/srv/work/orbital"
"#;

    /// A config naming a directory inside the repository rather than the
    /// checkout itself.
    const A_PROJECT_IN_A_SUBDIRECTORY: &str = r#"
[[projects]]
name = "dish"
path = "/srv/work/orbital/crates/dish"
"#;

    /// Two, each in its own repository, so a test can see which directory
    /// each question was asked in.
    const TWO_CONFIGURED_PROJECTS: &str = r#"
[[projects]]
name = "orbital"
path = "/srv/work/orbital"
credential_command = "secret-tool lookup tracker orbital"

[[projects]]
name = "harbour"
path = "/srv/work/harbour"
credential_command = "secret-tool lookup tracker harbour"
"#;

    /// A project configured as a directory inside its repository, and `bdi`
    /// started at that place in a linked worktree cut somewhere else: the
    /// config's path holds nothing of it, and the counterpart in the main
    /// working tree is what the project holds.
    #[test]
    fn a_linked_worktree_of_a_project_inside_the_repository_is_that_project() {
        let runner =
            FakeRunner::default().with("git worktree list --porcelain", A_WORKTREE_PER_SEAT);

        let cfg = scoped_to_the_directory(
            Config::from_toml(A_PROJECT_IN_A_SUBDIRECTORY).expect("the config parses"),
            &runner,
            Path::new("/tmp/seat-a/wt/crates/dish/src"),
        );

        assert_eq!(
            cfg.scope,
            Scope::Directory {
                project: "dish".to_string(),
                widened: Vec::new(),
            }
        );
        assert_eq!(
            runner.call("git worktree list --porcelain").cwd,
            Some(PathBuf::from("/tmp/seat-a/wt/crates/dish/src")),
            "git is asked once, from the directory bdi was started in"
        );
    }

    /// A directory a configured project holds outright needs no git: the
    /// config alone answers, and the one call is spent only where nothing
    /// holds the directory. A runner with nothing staged fails the test on
    /// any call at all.
    #[test]
    fn a_directory_a_project_holds_is_read_without_asking_git() {
        let runner = FakeRunner::default();

        let cfg = scoped_to_the_directory(
            Config::from_toml(TWO_CONFIGURED_PROJECTS).expect("the config parses"),
            &runner,
            Path::new("/srv/work/harbour/src"),
        );

        assert_eq!(
            cfg.scope,
            Scope::Directory {
                project: "harbour".to_string(),
                widened: Vec::new(),
            }
        );
        assert!(runner.calls().is_empty());
    }

    /// A config may name a project by its place in a linked worktree rather
    /// than in the main one, and `bdi` may be started at that place in a
    /// third. Every working tree git lists holds the same place, so the
    /// counterpart in each is tried, not only the main tree's.
    #[test]
    fn a_project_configured_in_one_linked_worktree_holds_the_same_place_in_another() {
        let runner =
            FakeRunner::default().with("git worktree list --porcelain", A_WORKTREE_PER_SEAT);

        let cfg = scoped_to_the_directory(
            Config::from_toml(
                r#"
[[projects]]
name = "dish"
path = "/tmp/seat-b/wt/crates/dish"
"#,
            )
            .expect("the config parses"),
            &runner,
            Path::new("/tmp/seat-a/wt/crates/dish/src"),
        );

        assert_eq!(
            cfg.scope,
            Scope::Directory {
                project: "dish".to_string(),
                widened: Vec::new(),
            }
        );
    }

    /// A directory in a repository no configured project is in — a linked
    /// worktree of something else, or a repository nobody configured —
    /// leaves the config whole, and the sibling directory at the same place
    /// in the listing is not taken for a project either.
    #[test]
    fn a_working_tree_of_no_configured_project_leaves_every_project_read() {
        let runner =
            FakeRunner::default().with("git worktree list --porcelain", A_WORKTREE_PER_SEAT);

        let cfg = scoped_to_the_directory(
            Config::from_toml(TWO_CONFIGURED_PROJECTS).expect("the config parses"),
            &runner,
            Path::new("/home/elsewhere/notes"),
        );

        assert_eq!(cfg.scope, Scope::Everything);
    }

    /// The bug: a project a config file names holds only the path the file
    /// spelled out, so a seat working in a linked worktree sits under nothing
    /// bdi knows about — exactly what discovery was fixed for.
    #[test]
    fn a_configured_project_holds_every_working_tree_git_lists() {
        let runner =
            FakeRunner::default().with("git worktree list --porcelain", A_WORKTREE_PER_SEAT);

        let cfg = with_the_working_trees_git_lists(
            Config::from_toml(ONE_CONFIGURED_PROJECT).expect("the config parses"),
            &runner,
        );

        assert_eq!(
            cfg.projects[0].worktrees,
            vec![
                PathBuf::from("/srv/work/orbital"),
                PathBuf::from("/tmp/seat-a/wt"),
                PathBuf::from("/tmp/seat-b/wt"),
            ]
        );
        assert!(
            cfg.projects[0]
                .holds(Path::new("/tmp/seat-a/wt/src"))
                .is_some(),
            "a seat in a linked worktree is working in the project"
        );
    }

    /// git answers for the directory it runs in, so each project is asked
    /// where it sits rather than wherever bdi was started.
    #[test]
    fn each_configured_project_is_asked_from_its_own_path() {
        let runner = FakeRunner::default().with("git worktree list --porcelain", ONE_CHECKOUT);

        with_the_working_trees_git_lists(
            Config::from_toml(TWO_CONFIGURED_PROJECTS).expect("the config parses"),
            &runner,
        );

        let asked: Vec<Option<PathBuf>> = runner
            .calls()
            .into_iter()
            .filter(|call| call.argv == "git worktree list --porcelain")
            .map(|call| call.cwd)
            .collect();
        assert_eq!(
            asked,
            vec![
                Some(PathBuf::from("/srv/work/orbital")),
                Some(PathBuf::from("/srv/work/harbour")),
            ]
        );
    }

    /// A config naming a directory inside the repository means that
    /// directory, not the repository around it: the seat working on it in a
    /// sibling worktree is in the project, and the sibling directory nobody
    /// configured is not.
    #[test]
    fn a_project_inside_the_repository_is_that_place_in_each_working_tree() {
        let runner =
            FakeRunner::default().with("git worktree list --porcelain", A_WORKTREE_PER_SEAT);

        let cfg = with_the_working_trees_git_lists(
            Config::from_toml(A_PROJECT_IN_A_SUBDIRECTORY).expect("the config parses"),
            &runner,
        );

        assert_eq!(
            cfg.projects[0].worktrees,
            vec![
                PathBuf::from("/srv/work/orbital/crates/dish"),
                PathBuf::from("/tmp/seat-a/wt/crates/dish"),
                PathBuf::from("/tmp/seat-b/wt/crates/dish"),
            ]
        );
        assert!(
            cfg.projects[0]
                .holds(Path::new("/srv/work/orbital/docs"))
                .is_none(),
            "a project configured as one directory annexed the repository around it"
        );
    }

    /// Degrade, never disappear: a configured path git will not answer for
    /// still holds itself, and says nothing alarming about it.
    #[test]
    fn a_configured_path_that_is_no_repository_still_holds_itself() {
        let runner =
            FakeRunner::default().failing("git worktree list --porcelain", no_such_repository());

        let cfg = with_the_working_trees_git_lists(
            Config::from_toml(ONE_CONFIGURED_PROJECT).expect("the config parses"),
            &runner,
        );

        assert!(cfg.projects[0].worktrees.is_empty());
        assert!(
            cfg.projects[0]
                .holds(Path::new("/srv/work/orbital/src"))
                .is_some(),
            "a project in no repository holds nothing at all"
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

    /// A repository with a directory inside it and a linked worktree cut
    /// from before that directory existed, so the same place in the two
    /// working trees is a directory in one and nothing at all in the other.
    fn a_repository_with_a_subdirectory_and_a_linked_worktree(
        named: &str,
    ) -> (PathBuf, PathBuf, PathBuf) {
        let scratch = a_scratch_directory(named);
        let checkout = scratch.join("checkout");
        std::fs::create_dir_all(&checkout).expect("the directory is ours to make");
        git_in(&checkout, &["init"]);
        git_in(&checkout, &["commit", "--allow-empty", "-m", "root"]);
        let root = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&checkout)
            .output()
            .expect("git runs");
        let root = String::from_utf8_lossy(&root.stdout).trim().to_string();

        let inside = checkout.join("crates/dish");
        std::fs::create_dir_all(&inside).expect("the directory is ours to make");
        std::fs::write(inside.join("Cargo.toml"), "").expect("the file is ours to write");
        git_in(&checkout, &["add", "."]);
        git_in(&checkout, &["commit", "-m", "a crate"]);

        let linked = scratch.join("seat/wt");
        git_in(
            &checkout,
            &[
                "worktree",
                "add",
                "--detach",
                &linked.display().to_string(),
                &root,
            ],
        );

        (scratch, checkout, linked)
    }

    /// The re-rooting is a claim about what `git worktree list --porcelain`
    /// really emits, so this one measures it against git rather than against
    /// another test's idea of git.
    #[test]
    fn a_configured_subdirectory_is_re_rooted_onto_gits_own_listing() {
        let (scratch, checkout, linked) =
            a_repository_with_a_subdirectory_and_a_linked_worktree("configured-subdirectory");

        let cfg = with_the_working_trees_git_lists(
            Config::from_toml(&format!(
                "[[projects]]\nname = \"dish\"\npath = \"{}\"\n",
                checkout.join("crates/dish").display()
            ))
            .expect("the config parses"),
            &RealRunner,
        );

        assert_eq!(
            cfg.projects[0].worktrees,
            vec![checkout.join("crates/dish"), linked.join("crates/dish")]
        );

        std::fs::remove_dir_all(&scratch).expect("the directory is ours to remove");
    }

    /// A linked worktree has its own branch out, so the project's directory
    /// may not be there at all. It stays in the set: a project's territory is
    /// the repository's layout, not what somebody has checked out this
    /// minute, and a path no pane is under costs nothing to carry.
    #[test]
    fn a_place_that_is_not_there_on_this_branch_is_still_the_projects() {
        let (scratch, checkout, linked) =
            a_repository_with_a_subdirectory_and_a_linked_worktree("place-not-there");
        assert!(
            !linked.join("crates/dish").exists(),
            "the worktree was cut from before the directory, so it should not be there"
        );

        let cfg = with_the_working_trees_git_lists(
            Config::from_toml(&format!(
                "[[projects]]\nname = \"dish\"\npath = \"{}\"\n",
                checkout.join("crates/dish").display()
            ))
            .expect("the config parses"),
            &RealRunner,
        );

        assert!(
            cfg.projects[0]
                .holds(&linked.join("crates/dish/src"))
                .is_some(),
            "a seat working there was dropped because the directory is not checked out"
        );

        std::fs::remove_dir_all(&scratch).expect("the directory is ours to remove");
    }

    /// git answers with the real directory, so a config naming the same
    /// checkout through a symlink is in none of the working trees git listed.
    /// Nothing of git's answer is taken for it, and the project is left
    /// holding the path it was configured with — the seats working through
    /// that spelling still join, and no other repository's trees are annexed.
    #[test]
    fn a_path_in_none_of_the_working_trees_git_listed_holds_only_itself() {
        let scratch = a_scratch_directory("symlinked-path");
        let checkout = scratch.join("checkout");
        std::fs::create_dir_all(&checkout).expect("the directory is ours to make");
        git_in(&checkout, &["init"]);
        git_in(&checkout, &["commit", "--allow-empty", "-m", "root"]);
        let spelled = scratch.join("orbital");
        std::os::unix::fs::symlink(&checkout, &spelled).expect("the link is ours to make");

        let cfg = with_the_working_trees_git_lists(
            Config::from_toml(&format!(
                "[[projects]]\nname = \"orbital\"\npath = \"{}\"\n",
                spelled.display()
            ))
            .expect("the config parses"),
            &RealRunner,
        );

        assert!(
            cfg.projects[0].worktrees.is_empty(),
            "a listing holding none of the project's paths was taken for it: {:?}",
            cfg.projects[0].worktrees
        );
        assert!(
            cfg.projects[0].holds(&spelled.join("src")).is_some(),
            "the project stopped holding the path it was configured with"
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
        .expect("the repository is a project")
        .config;

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
        .expect("the repository is a project")
        .config;

        assert_eq!(cfg.projects[0].name, "atlas");
    }

    #[test]
    fn an_empty_name_in_the_environment_is_no_name_at_all() {
        let cfg = from_the_current_directory(
            &a_tracked_repository(),
            Path::new("/srv/work/orbital"),
            Some(""),
        )
        .expect("the repository is a project")
        .config;

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
            .expect("the repository is a project")
            .config;

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
                .expect("the repository is a project")
                .config;

            assert_eq!(cfg.projects[0].name, "ground-station", "from {url}");
        }
    }

    /// git that is not installed, which is how a machine that has never had
    /// one answers every question about a repository.
    fn no_git() -> RunFailure {
        RunFailure::not_installed("git", "No such file or directory (os error 2)")
    }

    /// What `bd where --json` says, as bd writes it: the workspace, the
    /// database inside it, and the schema that database is at.
    const THE_TRACKER_AT_ORBITAL: &str = r#"{
  "database_path": "/srv/work/orbital/.beads/dolt",
  "path": "/srv/work/orbital/.beads",
  "schema_version": 1
}"#;

    /// A tree beads tracks on a machine with no git, so nothing can say
    /// where the repository starts.
    fn a_tracked_tree_with_no_git() -> FakeRunner {
        FakeRunner::default()
            .with("bd where --json", THE_TRACKER_AT_ORBITAL)
            .failing("git rev-parse --show-toplevel", no_git())
            .failing("git remote get-url origin", no_git())
    }

    /// The directory the reader was standing in is not the project. bd was
    /// asked where the tracker is before anything else was asked at all, and
    /// a tracker sits at the root of the work it tracks.
    #[test]
    fn a_project_no_git_can_root_is_rooted_at_the_tracker_above_the_reader() {
        let cfg = from_the_current_directory(
            &a_tracked_tree_with_no_git(),
            Path::new("/srv/work/orbital/src"),
            None,
        )
        .expect("the tree bd tracks is a project")
        .config;

        assert_eq!(cfg.projects[0].path, PathBuf::from("/srv/work/orbital"));
    }

    /// What a root taken from the reader costs: `Project::holds` is what
    /// places a pane, so a project rooted where one reader happened to stand
    /// holds no seat working anywhere else in the same tree, and every one of
    /// them reads as unstaffed.
    #[test]
    fn a_project_rooted_at_its_tracker_holds_the_seats_working_elsewhere_in_it() {
        let cfg = from_the_current_directory(
            &a_tracked_tree_with_no_git(),
            Path::new("/srv/work/orbital/src"),
            None,
        )
        .expect("the tree bd tracks is a project")
        .config;

        assert!(
            cfg.projects[0]
                .holds(Path::new("/srv/work/orbital/docs"))
                .is_some(),
            "a seat working elsewhere in the tracked tree is in no project"
        );
    }

    /// The name is read off the root, so the root is what settles it: two
    /// readers of one tracker standing in different directories are looking
    /// at one project under one name, rather than at `orbital` and `src`.
    #[test]
    fn a_project_no_git_can_name_is_named_after_the_tracker_not_the_reader() {
        for standing_in in [
            "/srv/work/orbital",
            "/srv/work/orbital/src",
            "/srv/work/orbital/crates/dish",
        ] {
            let cfg = from_the_current_directory(
                &a_tracked_tree_with_no_git(),
                Path::new(standing_in),
                None,
            )
            .expect("the tree bd tracks is a project")
            .config;

            assert_eq!(cfg.projects[0].name, "orbital", "standing in {standing_in}");
        }
    }

    /// The half of that name a reader cannot see for themselves. It is the
    /// tree's rather than the repository's, and the only thing that knows so
    /// is the call that came back saying git never ran.
    #[test]
    fn a_machine_with_no_git_says_the_name_it_gave_the_project_is_a_guess() {
        let discovered = from_the_current_directory(
            &a_tracked_tree_with_no_git(),
            Path::new("/srv/work/orbital/src"),
            None,
        )
        .expect("the tree bd tracks is a project");

        assert!(discovered.named_without_git);
    }

    /// A repository with no `origin` is the documented answer and says
    /// nothing. git ran, and a reader can see for themselves that their
    /// repository has no remote — a warning here is the one they learn to
    /// ignore, and every warning beside it goes with it.
    #[test]
    fn a_repository_with_no_origin_is_named_after_its_directory_and_says_nothing() {
        let runner = FakeRunner::default()
            .with("bd where --json", THE_TRACKER_AT_ORBITAL)
            .with("git rev-parse --show-toplevel", "/srv/work/orbital\n")
            .with("git worktree list --porcelain", ONE_CHECKOUT)
            .failing("git remote get-url origin", no_such_repository());

        let discovered = from_the_current_directory(&runner, Path::new("/srv/work/orbital"), None)
            .expect("the repository is a project");

        assert_eq!(discovered.config.projects[0].name, "orbital");
        assert!(!discovered.named_without_git);
    }

    /// The other outcome git that ran can have: a directory in no repository
    /// at all. The reader knows where they are standing, so this is silent
    /// too, and the name is the directory's for a reason they can see.
    #[test]
    fn a_directory_in_no_repository_is_named_after_itself_and_says_nothing() {
        let runner = FakeRunner::default()
            .with("bd where --json", r#"{"path":"/srv/beads/.beads"}"#)
            .failing("git rev-parse --show-toplevel", no_such_repository())
            .failing("git remote get-url origin", no_such_repository());

        let discovered = from_the_current_directory(&runner, Path::new("/srv/loose"), None)
            .expect("the directory is a project");

        assert_eq!(discovered.config.projects[0].name, "loose");
        assert!(!discovered.named_without_git);
    }

    /// `BDI_PROJECT` is a name rather than a guess, so a machine with no git
    /// and a name in its environment has nothing to be told about.
    #[test]
    fn a_project_the_environment_names_is_no_guess_on_a_machine_with_no_git() {
        let discovered = from_the_current_directory(
            &a_tracked_tree_with_no_git(),
            Path::new("/srv/work/orbital/src"),
            Some("atlas"),
        )
        .expect("the tree bd tracks is a project");

        assert_eq!(discovered.config.projects[0].name, "atlas");
        assert!(!discovered.named_without_git);
    }

    /// An empty `BDI_PROJECT` is no name at all — here as everywhere else —
    /// so it leaves the name a guess rather than settling it. The name and
    /// what is said about it are read off the same test, because a
    /// fall-through reordered so they disagree is the whole failure.
    #[test]
    fn an_empty_name_in_the_environment_leaves_the_name_a_guess() {
        let discovered = from_the_current_directory(
            &a_tracked_tree_with_no_git(),
            Path::new("/srv/work/orbital/src"),
            Some(""),
        )
        .expect("the tree bd tracks is a project");

        assert_eq!(discovered.config.projects[0].name, "orbital");
        assert!(discovered.named_without_git);
    }

    /// A redirect is the reader saying which beads are theirs, and `bd where`
    /// reports the workspace in use rather than the one they redirected from.
    /// So a directory whose tracker redirects upwards is drawn as the tree
    /// that tracker belongs to — under that tree's name, holding the panes
    /// working anywhere in it.
    ///
    /// Rooting narrower than the tracker is the defect this fixes, in
    /// miniature: `bdi` would draw a tracker's beads and then attribute none
    /// of the panes working on them.
    #[test]
    fn a_tracker_the_reader_redirected_upwards_roots_them_at_the_tree_it_belongs_to() {
        let runner = FakeRunner::default()
            .with("bd where --json", r#"{"path":"/srv/project/.beads"}"#)
            .failing("git rev-parse --show-toplevel", no_git())
            .failing("git remote get-url origin", no_git());

        let cfg = from_the_current_directory(&runner, Path::new("/srv/project/sub"), None)
            .expect("the tree bd tracks is a project")
            .config;

        assert_eq!(cfg.projects[0].path, PathBuf::from("/srv/project"));
        assert_eq!(cfg.projects[0].name, "project");
        assert!(
            cfg.projects[0]
                .holds(Path::new("/srv/project/other"))
                .is_some(),
            "a seat working on the same tracker elsewhere in the tree is in no project"
        );
    }

    /// A tracker named outright is not inside the work it tracks, so the
    /// directory above it is one the two happen to share rather than a root.
    /// `BEADS_DIR` pointing at `/srv/shared-beads` from `/srv/project` would
    /// otherwise make the project `/srv`, taking in every sibling of the
    /// reader's own and every pane working in one.
    #[test]
    fn a_tracker_beside_the_reader_rather_than_above_them_is_no_root() {
        let runner = FakeRunner::default()
            .with("bd where --json", r#"{"path":"/srv/shared-beads"}"#)
            .failing("git rev-parse --show-toplevel", no_git())
            .failing("git remote get-url origin", no_git());

        let cfg = from_the_current_directory(&runner, Path::new("/srv/project"), None)
            .expect("the directory is a project")
            .config;

        assert_eq!(cfg.projects[0].path, PathBuf::from("/srv/project"));
        assert_eq!(cfg.projects[0].name, "project");
    }

    /// A reader who put a tracker in their home directory said their home
    /// directory is the project, and it is read as one. That is the answer
    /// git already gives for a repository opened there, and a rule refusing
    /// it on this half of the function alone is one the reader could not
    /// predict from the other half.
    #[test]
    fn a_tracker_far_above_the_reader_is_still_the_root() {
        let runner = FakeRunner::default()
            .with("bd where --json", r#"{"path":"/home/pilot/.beads"}"#)
            .failing("git rev-parse --show-toplevel", no_git())
            .failing("git remote get-url origin", no_git());

        let cfg = from_the_current_directory(&runner, Path::new("/home/pilot/work/orbital"), None)
            .expect("the tree bd tracks is a project")
            .config;

        assert_eq!(cfg.projects[0].path, PathBuf::from("/home/pilot"));
    }

    /// git answering is the whole answer: where it says where the repository
    /// starts, that is the root, whatever the tracker bd found sits beside.
    /// Nothing about a machine with git changes.
    #[test]
    fn a_repository_git_names_is_the_root_whatever_the_tracker_sits_beside() {
        let runner = a_tracked_repository().with(
            "bd where --json",
            r#"{"path":"/srv/work/orbital/crates/dish/.beads"}"#,
        );

        let cfg = from_the_current_directory(&runner, Path::new("/srv/work/orbital/src"), None)
            .expect("the repository is a project")
            .config;

        assert_eq!(cfg.projects[0].path, PathBuf::from("/srv/work/orbital"));
        assert_eq!(cfg.projects[0].name, "ground-station");
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
            .expect("the directory is a project")
            .config;

        assert_eq!(
            cfg.projects,
            vec![Project {
                name: "loose".to_string(),
                path: PathBuf::from("/srv/loose"),
                environment: Environment::Ambient,
                credential_command: None,
                poll: true,
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
            RunFailure::not_installed("bd", "No such file or directory (os error 2)"),
        );

        let err = from_the_current_directory(&runner, Path::new("/srv/loose"), None)
            .unwrap_err()
            .to_string();

        assert!(err.contains("bd is not installed"), "got: {err}");
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
