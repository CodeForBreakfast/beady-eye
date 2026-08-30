use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;
use chrono::Utc;
use clap::Parser;

use beady_eye::collect::run::RealRunner;
use beady_eye::config::Config;
use beady_eye::model::snapshot::Filter;

/// Where the config lives when nothing says otherwise.
const DEFAULT_CONFIG: &str = "~/.config/beady-eye/config.toml";

/// The interactive view is a plan of its own, so until it lands `--json` is
/// the only thing `bdi` can draw.
const NO_VIEW_YET: u8 = 2;

#[derive(Parser)]
#[command(name = "bdi", version, about = "A tree of work in flight")]
struct Cli {
    /// Read the configuration from this file.
    #[arg(long, default_value = DEFAULT_CONFIG)]
    config: String,

    /// Emit the snapshot as JSON.
    #[arg(long)]
    json: bool,

    /// Draw every tree, including those with no live agent.
    #[arg(long)]
    all: bool,
}

fn main() -> anyhow::Result<ExitCode> {
    let cli = Cli::parse();

    let path = expand_tilde(&cli.config, std::env::var_os("HOME").map(PathBuf::from));
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("reading the config at {}", path.display()))?;
    let cfg = Config::from_toml(&text)?;

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
