use std::io::{ErrorKind, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;
use chrono::Utc;
use clap::Parser;

use beady_eye::collect::discovery;
use beady_eye::collect::run::{RealRunner, Runner};
use beady_eye::config::Config;
use beady_eye::model::snapshot::Filter;

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

    /// Emit the snapshot as JSON.
    #[arg(long)]
    json: bool,

    /// Draw every tree, including those with no live agent.
    #[arg(long)]
    all: bool,
}

fn main() -> anyhow::Result<ExitCode> {
    let cli = Cli::parse();

    let home = std::env::var_os("HOME").map(PathBuf::from);
    let cfg = match &cli.config {
        Some(named) => read_config(&RealRunner, &expand_tilde(named, home)),
        None => config_for_wherever_bdi_was_run(&RealRunner, &expand_tilde(DEFAULT_CONFIG, home)),
    }?;
    let cfg = cfg.with_roots_named_on_the_command_line(&cli.beads)?;

    let filter = if cli.all {
        Filter::All
    } else {
        Filter::LiveAgents
    };
    if cli.json {
        let snapshot = beady_eye::app::run(&cfg, &RealRunner, filter, Utc::now());
        println!("{}", serde_json::to_string_pretty(&snapshot)?);
        return Ok(ExitCode::SUCCESS);
    }

    if !std::io::stdout().is_terminal() {
        eprintln!("bdi's view needs a terminal; re-run with --json");
        return Ok(ExitCode::from(NO_TERMINAL));
    }

    let refresh = cfg.tui.refresh();
    // RealRunner is a unit struct, so the collection builds its own rather
    // than borrowing one across the thread it runs on.
    let projects = cfg
        .projects
        .iter()
        .map(|project| project.name.clone())
        .collect();
    let mut collection = beady_eye::app::Collection::default();
    beady_eye::tui::run(
        refresh,
        projects,
        Box::new(move |wanted| collection.collect(&cfg, &RealRunner, wanted, filter, Utc::now())),
    )?;

    Ok(ExitCode::SUCCESS)
}

/// A path the user named is read as written: a config that is not there is an
/// error, never a reason to look somewhere else.
fn read_config(runner: &dyn Runner, path: &Path) -> anyhow::Result<Config> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading the config at {}", path.display()))?;
    config_and_its_working_trees(&text, runner)
}

/// A config file's text, as a config whose projects know their working trees.
/// The file says where a project is; git says where else the same project is,
/// because a seat working in a linked worktree is working in the project.
fn config_and_its_working_trees(text: &str, runner: &dyn Runner) -> anyhow::Result<Config> {
    Ok(discovery::with_the_working_trees_git_lists(
        Config::from_toml(text)?,
        runner,
    ))
}

/// The config, or — where there is no config file at all — the repository the
/// current directory sits in. Only an absent file falls back; one that is
/// there and will not open is still an error.
fn config_for_wherever_bdi_was_run(runner: &dyn Runner, path: &Path) -> anyhow::Result<Config> {
    match std::fs::read_to_string(path) {
        Ok(text) => config_and_its_working_trees(&text, runner),
        Err(absent) if absent.kind() == ErrorKind::NotFound => {
            let cwd = std::env::current_dir().context("finding the current directory")?;
            let named = std::env::var(PROJECT_IN_THE_ENVIRONMENT).ok();
            discovery::from_the_current_directory(runner, &cwd, named.as_deref()).with_context(
                || {
                    format!(
                        "there is no config at {}, so bdi read the current directory",
                        path.display()
                    )
                },
            )
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
    use beady_eye::collect::run::{Env, RunFailure};
    use clap::CommandFactory;

    /// A config file naming one project, and git's answer for where that
    /// project's repository is worked in.
    const ONE_PROJECT: &str = r#"
[[projects]]
name = "orbital"
path = "/srv/work/orbital"
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
        let path = std::env::temp_dir().join(format!("bdi-{named}-{}.toml", std::process::id()));
        std::fs::write(&path, ONE_PROJECT).expect("the file is ours to write");
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

        let cfg =
            read_config(&ARepositoryWorkedInTwoPlaces, &path).expect("the config is ours to read");

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

        let cfg = config_for_wherever_bdi_was_run(&ARepositoryWorkedInTwoPlaces, &path)
            .expect("the config is ours to read");

        assert!(
            cfg.projects[0]
                .holds(Path::new("/tmp/seat-a/wt/src"))
                .is_some(),
            "a seat in a linked worktree is working in the project"
        );

        std::fs::remove_file(&path).expect("the file is ours to remove");
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
