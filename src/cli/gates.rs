//! `bdi gates`: every configured project's gh:pr gates, settled on a poll
//! and, where it listens, on each of GitHub's deliveries, with what each
//! settling found said on stdout.

use std::net::SocketAddr;
use std::path::Path;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use anyhow::Context;
use chrono::{DateTime, Utc};

use crate::app::gates::{delivered, look, Found, Read};
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

/// How long GitHub is left alone after a rate limit it does not say the end
/// of, which is the least GitHub asks of a client refused for a secondary
/// limit.
const UNSAID_WAIT: Duration = Duration::from_secs(60);

/// Look at the gates, report, and look again `[gates] poll_seconds` later,
/// until a signal ends the process. Listening on `listen`, settle the pull
/// request each delivery names in between. Nothing a settling finds stops the
/// next one, though a rate limit puts it off: GitHub is asked nothing until
/// the limit resets, and a delivery in the meantime is left to the look after.
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
    let from_environment = std::env::var(SECRET_VARIABLE).ok();
    std::env::remove_var(SECRET_VARIABLE);
    let secret = listen
        .map(|_| secret(secret_file, from_environment))
        .transpose()?;
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
    let (to_settle, delivered_for) = mpsc::sync_channel(webhook::MOST_WAITING);
    let reading_github = Arc::new(AtomicBool::new(true));
    let listening = match (listen, secret) {
        (Some(address), Some(secret)) => Some(webhook::listen(
            address,
            secret,
            to_settle.clone(),
            Arc::clone(&reading_github),
            |heard| {
                if let Some(said) = refused(heard) {
                    println!("{said}");
                }
            },
        )?),
        _ => None,
    };
    println!("{}", started(&cfg.gates, projects.len(), listening));
    let mut next_look = Instant::now();
    // ponytail: one wait for every host. Wait per host if one `bdi gates`
    // ever settles for logins on two hosts at once.
    let mut rate_limited_until = Instant::now();
    loop {
        let now = Instant::now();
        let settling = if now >= next_look {
            let settling = look(&trackers, &RealRunner, &projects, &cfg.gates);
            next_look = Instant::now() + cfg.gates.poll();
            settling
        } else {
            match delivered_for.recv_timeout(next_look - now) {
                Ok(pull_request) if Instant::now() < rate_limited_until => {
                    println!(
                        "{}",
                        spelled_out(&format!(
                            "{pull_request}: a delivery came while GitHub's rate limit is waited \
                             out, so the next look settles it"
                        ))
                    );
                    continue;
                }
                Ok(pull_request) => {
                    delivered(&trackers, &RealRunner, &projects, &cfg.gates, pull_request)
                }
                Err(_) => continue,
            }
        };
        match settling.github {
            Read::Answered => reading_github.store(true, Ordering::SeqCst),
            Read::Refused => reading_github.store(false, Ordering::SeqCst),
            Read::Unasked => {}
        }
        for found in settling.found {
            if let Found::RateLimited { resets, .. } = found {
                rate_limited_until = waited_out(resets);
                next_look = next_look.max(rate_limited_until);
            }
            if let Some(said) = reported(&found) {
                println!("{said}");
            }
        }
    }
}

/// When GitHub may be asked again after a rate limit that `resets` then, or
/// that GitHub did not say the end of.
fn waited_out(resets: Option<DateTime<Utc>>) -> Instant {
    let wait = match resets {
        Some(resets) => (resets - Utc::now()).to_std().unwrap_or(Duration::ZERO),
        None => UNSAID_WAIT,
    };
    Instant::now() + wait
}

/// The secret, from `file` where one is named and from what
/// [`SECRET_VARIABLE`] held otherwise.
///
/// The caller takes the variable out of the environment before any thread
/// starts, which is when changing the environment is sound, and whether or
/// not it is used, so no `bd` or `gh` this run starts is handed it.
fn secret(file: Option<&Path>, from_environment: Option<String>) -> anyhow::Result<Secret> {
    let text = match (file, from_environment) {
        (Some(file), _) => std::fs::read_to_string(file)
            .with_context(|| format!("reading the webhook secret from {}", file.display()))?,
        (None, Some(text)) => text,
        (None, None) => anyhow::bail!(
            "--listen needs the secret GitHub signs deliveries with, from \
             --webhook-secret-file or {SECRET_VARIABLE}"
        ),
    };
    Secret::new(&text).context("the webhook secret is empty")
}

fn started(gates: &Gates, projects: usize, listening: Option<SocketAddr>) -> String {
    let mut owners = if gates.owners.is_empty() {
        "every owner's repositories".to_string()
    } else {
        format!("repositories owned by {}", gates.owners.join(", "))
    };
    if !gates.excluded_owners.is_empty() {
        owners += &format!(
            " except those owned by {}",
            gates.excluded_owners.join(", ")
        );
    }
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
        Found::RateLimited {
            pull_request,
            resets: Some(resets),
        } => format!(
            "{pull_request}: GitHub refused it for the rate limit of the login gh runs as, so \
             GitHub is asked nothing more until the limit resets at {}",
            resets.format("%Y-%m-%d %H:%M:%S UTC")
        ),
        Found::RateLimited {
            pull_request,
            resets: None,
        } => format!(
            "{pull_request}: GitHub refused it for a rate limit without saying when it resets, \
             so GitHub is asked nothing more for at least {}s",
            UNSAID_WAIT.as_secs()
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
    fn a_rate_limit_github_does_not_say_the_end_of_is_reported_with_the_least_wait() {
        assert_eq!(
            reported(&Found::RateLimited {
                pull_request: PullRequest {
                    repo: "example/ark".to_string(),
                    number: 7,
                },
                resets: None,
            })
            .as_deref(),
            Some(
                "example/ark#7: GitHub refused it for a rate limit without saying when it \
                 resets, so GitHub is asked nothing more for at least 60s"
            )
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
    fn a_delivery_passed_over_or_refused_says_why_and_one_taken_says_nothing() {
        assert_eq!(
            refused(&Heard::NamesNoPullRequest),
            Some("a signed pull_request delivery was passed over: it names no pull request")
        );
        assert_eq!(
            refused(&Heard::Unmeasured),
            Some("a delivery was refused: it does not give its Content-Length up front")
        );
        assert_eq!(
            refused(&Heard::Settle(PullRequest {
                repo: "example/ark".to_string(),
                number: 7,
            })),
            None
        );
        assert_eq!(refused(&Heard::Ignored), None);
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
                    ..Gates::default()
                },
                2,
                None
            ),
            "bdi gates: settling the gh:pr gates of 2 projects, for repositories owned by \
             example, miskatonic, every 300s"
        );
        assert_eq!(
            started(
                &Gates {
                    excluded_owners: vec!["miskatonic".to_string()],
                    ..Gates::default()
                },
                1,
                None
            ),
            "bdi gates: settling the gh:pr gates of 1 project, for every owner's repositories \
             except those owned by miskatonic, every 60s"
        );
    }
}
