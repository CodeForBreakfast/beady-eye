//! What `bdi` does when it is run: the arguments it takes, the config those
//! arguments resolve against, and whether the snapshot is drawn or printed.

use std::io::{ErrorKind, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;
use chrono::Utc;
use clap::Parser;

use crate::collect::discovery;
use crate::collect::run::{RealRunner, Runner};
use crate::config::Config;
use crate::model::snapshot::Filter;

/// Where the config lives when nothing says otherwise.
const DEFAULT_CONFIG: &str = "~/.config/beady-eye/config.toml";

/// The variable that names the project when no config file names one. commy
/// resolves a project the same way, so a name set once reaches both.
const PROJECT_IN_THE_ENVIRONMENT: &str = "COMMY_PROJECT";

/// The view is drawn on the alternate screen, so a `bdi` whose output is a
/// pipe has nowhere to draw and `--json` is the only thing it can give.
const NO_TERMINAL: u8 = 2;

#[derive(Parser)]
#[command(name = "bdi", version, about = "A tree of work in flight")]
struct Cli {
    /// Draw the tree this bead roots, alongside the trees bdi discovers.
    /// Write it as <project>:<bead-id> where the config names more than one
    /// project; a bare id means the only project there is.
    #[arg(value_name = "BEAD-ID")]
    beads: Vec<String>,

    /// Read the configuration from this file, rather than
    /// ~/.config/beady-eye/config.toml.
    #[arg(long)]
    config: Option<String>,

    /// Draw only the projects named, repeating the option for each. The ones
    /// left out are not read at all, rather than read and hidden.
    #[arg(long = "project", value_name = "NAME")]
    projects: Vec<String>,

    /// Emit the snapshot as JSON.
    #[arg(long)]
    json: bool,

    /// Draw every tree, including those with no live agent.
    #[arg(long)]
    all: bool,
}

pub fn run() -> anyhow::Result<ExitCode> {
    let cli = Cli::parse();

    let home = std::env::var_os("HOME").map(PathBuf::from);
    let scope = &cli.projects;
    let cfg = match &cli.config {
        Some(named) => read_config(&RealRunner, &expand_tilde(named, home), scope),
        None => {
            config_for_wherever_bdi_was_run(&RealRunner, &expand_tilde(DEFAULT_CONFIG, home), scope)
        }
    }?;
    let cfg = cfg.with_roots_named_on_the_command_line(&cli.beads)?;

    let filter = if cli.all {
        Filter::All
    } else {
        Filter::LiveAgents
    };
    if cli.json {
        let snapshot = crate::app::run(&cfg, &RealRunner, filter, Utc::now());
        println!("{}", serde_json::to_string_pretty(&snapshot)?);
        return Ok(ExitCode::SUCCESS);
    }

    if !std::io::stdout().is_terminal() {
        eprintln!("bdi's view needs a terminal; re-run with --json");
        return Ok(ExitCode::from(NO_TERMINAL));
    }

    let refresh = cfg.tui.refresh();
    let patience = cfg.tui.unanswered_after();
    // RealRunner is a unit struct, so the collection builds its own rather
    // than borrowing one across the thread it runs on.
    let projects = cfg
        .projects
        .iter()
        .map(|project| project.name.clone())
        .collect();
    let mut collection = crate::app::Collection::default();
    crate::tui::run(
        refresh,
        patience,
        filter,
        projects,
        Box::new(move |wanted| collection.collect(&cfg, &RealRunner, wanted, filter, Utc::now())),
    )?;

    Ok(ExitCode::SUCCESS)
}

/// A path the user named is read as written: a config that is not there is an
/// error, never a reason to look somewhere else.
fn read_config(runner: &dyn Runner, path: &Path, scope: &[String]) -> anyhow::Result<Config> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading the config at {}", path.display()))?;
    config_and_its_working_trees(&text, runner, scope)
}

/// A config file's text, as a config whose projects know their working trees.
/// The file says where a project is; git says where else the same project is,
/// because a seat working in a linked worktree is working in the project.
///
/// Scoped before git is asked, because asking is a `git worktree list` in
/// each project's own directory — a project the reader excluded would
/// otherwise still be gone to, which is the gathering scoping exists to
/// avoid. The whole config is parsed first either way: what `[roots.explicit]`
/// is checked against is the config as written, not the part of it this run
/// wants.
fn config_and_its_working_trees(
    text: &str,
    runner: &dyn Runner,
    scope: &[String],
) -> anyhow::Result<Config> {
    Ok(discovery::with_the_working_trees_git_lists(
        Config::from_toml(text)?.scoped_to(scope)?,
        runner,
    ))
}

/// The config, or — where there is no config file at all — the repository the
/// current directory sits in. Only an absent file falls back; one that is
/// there and will not open is still an error.
fn config_for_wherever_bdi_was_run(
    runner: &dyn Runner,
    path: &Path,
    scope: &[String],
) -> anyhow::Result<Config> {
    match std::fs::read_to_string(path) {
        Ok(text) => config_and_its_working_trees(&text, runner, scope),
        Err(absent) if absent.kind() == ErrorKind::NotFound => {
            let cwd = std::env::current_dir().context("finding the current directory")?;
            let named = std::env::var(PROJECT_IN_THE_ENVIRONMENT).ok();
            discovery::from_the_current_directory(runner, &cwd, named.as_deref())
                .with_context(|| {
                    format!(
                        "there is no config at {}, so bdi read the current directory",
                        path.display()
                    )
                })?
                .scoped_to(scope)
        }
        Err(unreadable) => {
            Err(unreadable).with_context(|| format!("reading the config at {}", path.display()))
        }
    }
}

/// `~` belongs to the shell, so a config path written with one is expanded
/// here rather than handed to the filesystem verbatim.
fn expand_tilde(path: &str, home: Option<PathBuf>) -> PathBuf {
    match (path.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::run::testing::FakeRunner;
    use crate::collect::run::{Env, RunFailure};
    use clap::CommandFactory;

    /// A config file naming one project, and git's answer for where that
    /// project's repository is worked in.
    const ONE_PROJECT: &str = r#"
[[projects]]
name = "orbital"
path = "/srv/work/orbital"
"#;

    const TWO_PROJECTS: &str = r#"
[[projects]]
name = "orbital"
path = "/srv/work/orbital"

[[projects]]
name = "ferry"
path = "/srv/work/ferry"
"#;

    const A_WORKTREE_PER_SEAT: &str = "\
worktree /srv/work/orbital
HEAD 4d3c1f0e9b8a7c6d5e4f3a2b1c0d9e8f7a6b5c4d
branch refs/heads/main

worktree /tmp/seat-a/wt
HEAD 1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b
detached
";

    /// A config file of our own, written where the test can hand its path to
    /// the thing that reads it.
    fn a_config_file(named: &str) -> PathBuf {
        a_config_file_holding(named, ONE_PROJECT)
    }

    fn a_config_file_holding(named: &str, text: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("bdi-{named}-{}.toml", std::process::id()));
        std::fs::write(&path, text).expect("the file is ours to write");
        path
    }

    /// git, as far as reading a config needs it: the one listing, whatever
    /// is asked and wherever it is asked from.
    struct ARepositoryWorkedInTwoPlaces;

    impl Runner for ARepositoryWorkedInTwoPlaces {
        fn run(
            &self,
            _program: &str,
            _args: &[&str],
            _cwd: Option<&Path>,
            _env: &Env,
        ) -> Result<String, RunFailure> {
            Ok(A_WORKTREE_PER_SEAT.to_string())
        }
    }

    /// A config file is where a project is written down, and nothing written
    /// down can say where the seats are. Both ways of reading one ask git.
    #[test]
    fn a_config_the_command_line_names_learns_its_working_trees() {
        let path = a_config_file("named-config");

        let cfg = read_config(&ARepositoryWorkedInTwoPlaces, &path, &[])
            .expect("the config is ours to read");

        assert_eq!(
            cfg.projects[0].worktrees,
            vec![
                PathBuf::from("/srv/work/orbital"),
                PathBuf::from("/tmp/seat-a/wt"),
            ]
        );

        std::fs::remove_file(&path).expect("the file is ours to remove");
    }

    #[test]
    fn the_config_found_where_bdi_looks_learns_its_working_trees() {
        let path = a_config_file("default-config");

        let cfg = config_for_wherever_bdi_was_run(&ARepositoryWorkedInTwoPlaces, &path, &[])
            .expect("the config is ours to read");

        assert!(
            cfg.projects[0]
                .holds(Path::new("/tmp/seat-a/wt/src"))
                .is_some(),
            "a seat in a linked worktree is working in the project"
        );

        std::fs::remove_file(&path).expect("the file is ours to remove");
    }

    /// Only an absent config falls back. One that is there and will not open
    /// is an error, because reading the current directory instead would answer
    /// a question the user did not ask and look like it had answered theirs.
    #[test]
    fn a_config_that_will_not_open_is_an_error_rather_than_a_fallback() {
        let a_directory = std::env::temp_dir();

        let refused =
            config_for_wherever_bdi_was_run(&ARepositoryWorkedInTwoPlaces, &a_directory, &[])
                .expect_err("a directory is not a config file");

        assert!(
            format!("{refused:#}")
                .contains(&format!("reading the config at {}", a_directory.display())),
            "a config that will not open should be reported as itself, not as \
             a missing one; said: {refused:#}"
        );
    }

    /// Asking git where a project is worked is a subprocess in that project's
    /// own directory, so it is gathering like any other and a scope has to
    /// come first. It did not: `git worktree list` ran in every configured
    /// project's path before the scope was applied, which the collection
    /// tests could not see because they are handed a config that has already
    /// been through this.
    #[test]
    fn a_project_the_scope_left_out_is_not_even_asked_where_it_is_worked() {
        let path = a_config_file_holding("scoped-worktrees", TWO_PROJECTS);
        let runner =
            FakeRunner::default().with("git worktree list --porcelain", A_WORKTREE_PER_SEAT);

        read_config(&runner, &path, &["orbital".to_string()]).expect("the config is ours to read");

        // The directory is what says which project a call was for: the argv
        // is the same line whichever project it is asked about.
        let asked: Vec<Option<PathBuf>> = runner.calls().into_iter().map(|c| c.cwd).collect();
        assert!(
            !asked.contains(&Some(PathBuf::from("/srv/work/ferry"))),
            "ferry was scoped out, so nothing should have gone to its directory; asked {asked:?}"
        );
        assert!(
            asked.contains(&Some(PathBuf::from("/srv/work/orbital"))),
            "orbital was scoped in, so git was asked where it is worked; asked {asked:?}"
        );

        std::fs::remove_file(&path).expect("the file is ours to remove");
    }

    /// A run with no config file at all still has a scope to honour. The
    /// project it discovers is one project, so a scope naming a different one
    /// selects nothing and is refused — rather than starting on the project
    /// the reader did not ask for, which is what a scope the fallback ignored
    /// would do.
    #[test]
    fn a_scope_naming_no_discovered_project_is_refused_with_no_config_file() {
        let absent = std::env::temp_dir().join(format!("bdi-absent-{}.toml", std::process::id()));
        let runner = FakeRunner::default()
            .with("bd where --json", "{}")
            .with("git rev-parse --show-toplevel", "/srv/work/orbital")
            .with("git worktree list --porcelain", A_WORKTREE_PER_SEAT)
            .with("git remote get-url origin", "git@host:owner/orbital.git");

        let refused =
            config_for_wherever_bdi_was_run(&runner, &absent, &["nothing-of-the-sort".to_string()])
                .expect_err("the scope names no project the current directory is in");

        assert!(
            format!("{refused:#}").contains("nothing-of-the-sort"),
            "got: {refused:#}"
        );
    }

    #[test]
    fn the_command_line_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn no_config_is_named_unless_one_is_asked_for() {
        assert_eq!(Cli::parse_from(["bdi"]).config, None);
    }

    #[test]
    fn a_config_named_on_the_command_line_is_taken_as_written() {
        let cli = Cli::parse_from(["bdi", "--config", "/etc/beady-eye.toml"]);

        assert_eq!(cli.config.as_deref(), Some("/etc/beady-eye.toml"));
    }

    #[test]
    fn bead_ids_are_taken_as_arguments() {
        let cli = Cli::parse_from(["bdi", "orb-7", "ferry:fer-9", "--json"]);

        assert_eq!(cli.beads, ["orb-7", "ferry:fer-9"]);
        assert!(cli.json);
    }

    #[test]
    fn the_projects_to_draw_are_taken_as_a_repeatable_option() {
        let cli = Cli::parse_from(["bdi", "--project", "orbital", "--project", "ferry"]);

        assert_eq!(cli.projects, ["orbital", "ferry"]);
    }

    #[test]
    fn no_project_is_named_unless_one_is_asked_for() {
        assert!(Cli::parse_from(["bdi"]).projects.is_empty());
    }

    #[test]
    fn a_leading_tilde_becomes_the_home_directory() {
        let path = expand_tilde(DEFAULT_CONFIG, Some(PathBuf::from("/home/pilot")));

        assert_eq!(
            path,
            PathBuf::from("/home/pilot/.config/beady-eye/config.toml")
        );
    }

    #[test]
    fn a_path_that_names_no_home_is_left_as_written() {
        for path in ["/etc/beady-eye.toml", "~elsewhere/config.toml"] {
            assert_eq!(
                expand_tilde(path, Some(PathBuf::from("/home/pilot"))),
                PathBuf::from(path)
            );
        }
    }

    /// Without a `HOME` there is nothing to expand to, and a path we cannot
    /// resolve is better reported by the open that fails than guessed at.
    #[test]
    fn without_a_home_the_path_is_left_as_written() {
        assert_eq!(
            expand_tilde(DEFAULT_CONFIG, None),
            PathBuf::from(DEFAULT_CONFIG)
        );
    }
}
