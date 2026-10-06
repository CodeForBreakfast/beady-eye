//! `bdi gates`: every configured project's gh:pr gates, settled on a poll,
//! with what each look found said on stdout.

use std::path::Path;
use std::process::ExitCode;

use anyhow::Context;

use crate::app::gates::{look, Found};
use crate::collect::bd;
use crate::collect::environment::EnvironmentCache;
use crate::collect::gates::Done;
use crate::collect::run::RealRunner;
use crate::config::{Gates, Project};
use crate::model::gate::Fault;
use crate::view::phrase;

use super::{read_config, Launch, Reading};

/// Look at the gates, report, wait `[gates] poll_seconds`, and look again,
/// until a signal ends the process. Nothing a look finds stops the next one.
///
/// The config is read once. Replacing what it settles is a restart.
pub(super) fn settle_on_a_poll(config: &Path) -> anyhow::Result<ExitCode> {
    let cwd = std::env::current_dir().context("finding the current directory")?;
    let cfg = read_config(
        &RealRunner,
        config,
        &Launch {
            cwd: &cwd,
            reading: Reading::EveryProject,
            roots: &[],
        },
    )?
    .config;
    let projects: Vec<Project> = cfg.read().cloned().collect();
    let trackers = bd::Cli::new(&RealRunner).caching_environments(EnvironmentCache::here());
    println!("{}", started(&cfg.gates, projects.len()));
    loop {
        for found in look(&trackers, &RealRunner, &projects, &cfg.gates) {
            if let Some(said) = reported(&found) {
                println!("{said}");
            }
        }
        std::thread::sleep(cfg.gates.poll());
    }
}

fn started(gates: &Gates, projects: usize) -> String {
    let owners = if gates.owners.is_empty() {
        "every owner's repositories".to_string()
    } else {
        format!("repositories owned by {}", gates.owners.join(", "))
    };
    format!(
        "bdi gates: settling the gh:pr gates of {projects} projects, for {owners}, every {}s",
        gates.poll_seconds
    )
}

/// What a look found, as one line, or nothing for a bead told on an earlier
/// look. A pull request closed unmerged leaves its gate open, so it is
/// settled again on every look and would otherwise say so every time.
fn reported(found: &Found) -> Option<String> {
    Some(match found {
        Found::TrackerUnread { project, failure } => format!(
            "{project}: its gh:pr gates could not be read: {}",
            phrase::tracker_failure(failure)
        ),
        Found::NoPullRequest {
            project,
            gate,
            faults,
        } => format!(
            "{project}: gate {gate} names no pull request to settle: {}",
            faults.iter().map(fault).collect::<Vec<_>>().join(", and ")
        ),
        Found::GitHubUnread {
            pull_request,
            failure,
        } => format!(
            "{pull_request}: GitHub did not say where it stands, so no gate waiting on it was \
             touched: {failure}"
        ),
        Found::Settling {
            pull_request,
            project,
            bead,
            done,
        } => match done {
            Ok(Done::Resolved) => format!("{pull_request} merged: {project} closed gate {bead}"),
            Ok(Done::Commented) => {
                format!("{pull_request} closed unmerged: {project} told {bead}")
            }
            Ok(Done::AlreadyCommented) => return None,
            Err(failure) => format!(
                "{pull_request}: {project} could not settle {bead}: {}",
                phrase::tracker_failure(failure)
            ),
        },
    })
}

fn fault(fault: &Fault) -> String {
    match fault {
        Fault::NoRepo => "it names no repo".to_string(),
        Fault::NoAwaitId => "it has no await id".to_string(),
        Fault::AwaitIdNotANumber(id) => format!("its await id “{id}” is not a number"),
    }
}
