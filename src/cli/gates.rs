//! `bdi gates`: every configured project's gh:pr gates, settled on a poll
//! and, where it listens, on each of GitHub's deliveries, with what each
//! settling found said on stdout.

use std::net::SocketAddr;
use std::path::Path;
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::Instant;

use anyhow::Context;

use crate::app::gates::{delivered, look, Found};
use crate::collect::bd;
use crate::collect::environment::EnvironmentCache;
use crate::collect::gates::Done;
use crate::collect::run::RealRunner;
use crate::collect::webhook::{self, Heard, Secret};
use crate::config::{Gates, Project};
use crate::model::gate::Fault;
use crate::view::phrase;

use super::{read_config, Launch, Reading};

/// Where the secret GitHub signs deliveries with is read from, where no file
/// is named for it.
const SECRET_VARIABLE: &str = "BDI_GATES_WEBHOOK_SECRET";

/// Look at the gates, report, and look again `[gates] poll_seconds` later,
/// until a signal ends the process. Listening on `listen`, settle the pull
/// request each delivery names in between. Nothing a settling finds stops the
/// next one.
///
/// Every settling runs on this one thread, one after another, which is what
/// keeps a delivery and a look arriving together from closing a gate twice
/// or telling a bead twice.
///
/// The config is read once. Replacing what it settles is a restart.
pub(super) fn settle(
    config: &Path,
    listen: Option<&str>,
    secret_file: Option<&Path>,
) -> anyhow::Result<ExitCode> {
    let secret = listen.map(|_| secret(secret_file)).transpose()?;
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
    let (hearing, heard) = mpsc::channel();
    let listening = match (listen, secret) {
        (Some(address), Some(secret)) => Some(webhook::listen(address, secret, hearing.clone())?),
        _ => None,
    };
    println!("{}", started(&cfg.gates, projects.len(), listening));
    let mut next_look = Instant::now();
    loop {
        let now = Instant::now();
        let found = if now >= next_look {
            let found = look(&trackers, &RealRunner, &projects, &cfg.gates);
            next_look = Instant::now() + cfg.gates.poll();
            found
        } else {
            match heard.recv_timeout(next_look - now) {
                Ok(Heard::Settle(pull_request)) => {
                    delivered(&trackers, &RealRunner, &projects, &cfg.gates, pull_request)
                }
                Ok(other) => {
                    if let Some(said) = refused(&other) {
                        println!("{said}");
                    }
                    continue;
                }
                Err(_) => continue,
            }
        };
        for found in found {
            if let Some(said) = reported(&found) {
                println!("{said}");
            }
        }
    }
}

/// The secret, from `file` where one is named and from [`SECRET_VARIABLE`]
/// otherwise. The variable is taken out of the environment once read, so no
/// `bd` or `gh` this run starts is handed it. That is done before any thread
/// starts, which is when changing the environment is sound.
fn secret(file: Option<&Path>) -> anyhow::Result<Secret> {
    let text = match file {
        Some(file) => std::fs::read_to_string(file)
            .with_context(|| format!("reading the webhook secret from {}", file.display()))?,
        None => {
            let text = std::env::var(SECRET_VARIABLE).with_context(|| {
                format!(
                    "--listen needs the secret GitHub signs deliveries with, from \
                     --webhook-secret-file or {SECRET_VARIABLE}"
                )
            })?;
            std::env::remove_var(SECRET_VARIABLE);
            text
        }
    };
    Secret::new(&text).context("the webhook secret is empty")
}

fn started(gates: &Gates, projects: usize, listening: Option<SocketAddr>) -> String {
    let owners = if gates.owners.is_empty() {
        "every owner's repositories".to_string()
    } else {
        format!("repositories owned by {}", gates.owners.join(", "))
    };
    let plural = if projects == 1 { "" } else { "s" };
    let deliveries = match listening {
        Some(address) => format!(", and on GitHub's deliveries to {address}"),
        None => String::new(),
    };
    format!(
        "bdi gates: settling the gh:pr gates of {projects} project{plural}, for {owners}, every \
         {}s{deliveries}",
        gates.poll_seconds
    )
}

/// A delivery refused or passed over, as one line, or nothing for one there
/// is nothing to say about.
fn refused(heard: &Heard) -> Option<&'static str> {
    match heard {
        Heard::Unsigned => Some("a delivery was refused: it carries no X-Hub-Signature-256"),
        Heard::Forged => {
            Some("a delivery was refused: its X-Hub-Signature-256 is not the secret's")
        }
        Heard::NamesNoPullRequest => {
            Some("a signed pull_request delivery was passed over: it names no pull request")
        }
        Heard::TooLarge => {
            Some("a delivery was refused: it is larger than any pull_request delivery")
        }
        Heard::Unmeasured => {
            Some("a delivery was refused: it does not give its Content-Length up front")
        }
        Heard::Healthy | Heard::Settle(_) | Heard::Ignored | Heard::Unknown => None,
    }
}

/// What a look found, as one line, or nothing for a bead told on an earlier
/// look. A pull request closed unmerged leaves its gate open, so it is
/// settled again on every look and would otherwise say so every time.
fn reported(found: &Found) -> Option<String> {
    let line = match found {
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
    };
    Some(spelled_out(&line))
}

/// `line` with each control character written as its escape. Ids, repos and
/// await ids are whatever a tracker's writers put there, and printed raw they
/// could move the terminal or start a report line of their own.
fn spelled_out(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for glyph in line.chars() {
        if glyph.is_control() {
            out.extend(glyph.escape_default());
        } else {
            out.push(glyph);
        }
    }
    out
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
    fn a_control_character_the_tracker_wrote_is_spelled_out_on_the_line() {
        assert_eq!(
            reported(&Found::NoPullRequest {
                project: "arkham".to_string(),
                gate: "ark-tg0".to_string(),
                faults: vec![Fault::AwaitIdNotANumber("\u{1b}[2J\nforged".to_string())],
            })
            .as_deref(),
            Some(
                "arkham: gate ark-tg0 names no pull request to settle: its await id \
                 “\\u{1b}[2J\\nforged” is not a number"
            )
        );
    }

    #[test]
    fn starting_says_how_many_projects_whose_repositories_and_how_often() {
        assert_eq!(
            started(&Gates::default(), 1, None),
            "bdi gates: settling the gh:pr gates of 1 project, for every owner's repositories, \
             every 60s"
        );
        assert_eq!(
            started(
                &Gates {
                    poll_seconds: 300,
                    owners: vec!["example".to_string(), "miskatonic".to_string()],
                },
                2,
                None
            ),
            "bdi gates: settling the gh:pr gates of 2 projects, for repositories owned by \
             example, miskatonic, every 300s"
        );
    }
}
