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
    let plural = if projects == 1 { "" } else { "s" };
    format!(
        "bdi gates: settling the gh:pr gates of {projects} project{plural}, for {owners}, every \
         {}s",
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::gates::PullRequest;
    use crate::model::snapshot::TrackerFailure;

    fn told(done: Result<Done, TrackerFailure>) -> Option<String> {
        reported(&Found::Settling {
            pull_request: PullRequest {
                repo: "example/ark".to_string(),
                number: 7,
            },
            project: "arkham".to_string(),
            bead: "ark-2ud".to_string(),
            done,
        })
    }

    #[test]
    fn a_bead_told_its_pull_request_closed_unmerged_is_reported_once() {
        assert_eq!(
            told(Ok(Done::Commented)).as_deref(),
            Some("example/ark#7 closed unmerged: arkham told ark-2ud")
        );
        assert_eq!(told(Ok(Done::AlreadyCommented)), None);
    }

    #[test]
    fn a_write_that_failed_is_reported_against_its_bead() {
        assert_eq!(
            told(Err(TrackerFailure::Unavailable)).as_deref(),
            Some("example/ark#7: arkham could not settle ark-2ud: the tracker did not answer")
        );
    }

    #[test]
    fn a_gate_naming_no_pull_request_is_reported_with_every_reason() {
        assert_eq!(
            reported(&Found::NoPullRequest {
                project: "arkham".to_string(),
                gate: "ark-tg0".to_string(),
                faults: vec![
                    Fault::NoRepo,
                    Fault::AwaitIdNotANumber("the-ninth".to_string())
                ],
            })
            .as_deref(),
            Some(
                "arkham: gate ark-tg0 names no pull request to settle: it names no repo, and its \
                 await id “the-ninth” is not a number"
            )
        );
        assert_eq!(
            reported(&Found::NoPullRequest {
                project: "arkham".to_string(),
                gate: "ark-g1".to_string(),
                faults: vec![Fault::NoAwaitId],
            })
            .as_deref(),
            Some("arkham: gate ark-g1 names no pull request to settle: it has no await id")
        );
    }

    #[test]
    fn starting_says_how_many_projects_whose_repositories_and_how_often() {
        assert_eq!(
            started(&Gates::default(), 1),
            "bdi gates: settling the gh:pr gates of 1 project, for every owner's repositories, \
             every 60s"
        );
        assert_eq!(
            started(
                &Gates {
                    poll_seconds: 300,
                    owners: vec!["example".to_string(), "miskatonic".to_string()],
                },
                2
            ),
            "bdi gates: settling the gh:pr gates of 2 projects, for repositories owned by \
             example, miskatonic, every 300s"
        );
    }
}
