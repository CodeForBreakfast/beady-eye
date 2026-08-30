use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;
use chrono::Utc;
use clap::Parser;

use beady_eye::collect::run::{RealRunner, Runner};
use beady_eye::config::Config;
use beady_eye::model::snapshot::Filter;

/// Where the config lives when nothing says otherwise.
const DEFAULT_CONFIG: &str = "~/.config/beady-eye/config.toml";

/// The variable that names the project when no config file names one. commy
/// resolves a project the same way, so a name set once reaches both.
const PROJECT_IN_THE_ENVIRONMENT: &str = "COMMY_PROJECT";

/// The interactive view is a plan of its own, so until it lands `--json` is
/// the only thing `bdi` can draw.
const NO_VIEW_YET: u8 = 2;

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
        Some(named) => read_config(&expand_tilde(named, home)),
        None => config_for_wherever_bdi_was_run(&RealRunner, &expand_tilde(DEFAULT_CONFIG, home)),
    }?;
    let cfg = cfg.with_roots_named_on_the_command_line(&cli.beads)?;

    let filter = if cli.all {
        Filter::All
    } else {
        Filter::LiveAgents
    };
    let snapshot = beady_eye::app::run(&cfg, &RealRunner, filter, Utc::now());

    if !cli.json {
        eprintln!("bdi has no interactive view yet; re-run with --json");
        return Ok(ExitCode::from(NO_VIEW_YET));
    }

    println!("{}", serde_json::to_string_pretty(&snapshot)?);
    Ok(ExitCode::SUCCESS)
}

/// A path the user named is read as written: a config that is not there is an
/// error, never a reason to look somewhere else.
fn read_config(path: &Path) -> anyhow::Result<Config> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading the config at {}", path.display()))?;
    Config::from_toml(&text)
}

/// The config, or — where there is no config file at all — the repository the
/// current directory sits in. Only an absent file falls back; one that is
/// there and will not open is still an error.
fn config_for_wherever_bdi_was_run(runner: &dyn Runner, path: &Path) -> anyhow::Result<Config> {
    match std::fs::read_to_string(path) {
        Ok(text) => Config::from_toml(&text),
        Err(absent) if absent.kind() == ErrorKind::NotFound => {
            let cwd = std::env::current_dir().context("finding the current directory")?;
            let named = std::env::var(PROJECT_IN_THE_ENVIRONMENT).ok();
            Config::from_the_current_directory(runner, &cwd, named.as_deref()).with_context(|| {
                format!(
                    "there is no config at {}, so bdi read the current directory",
                    path.display()
                )
            })
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
    use clap::CommandFactory;

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
