//! One look at every configured project's gh:pr, gh:issue, gh:release, gh:run
//! and timer gates, which `bdi gates` takes on its poll: each pull request,
//! issue, workflow run or repository's releases they wait on is asked of
//! GitHub once, where its
//! repository's owner is one this `bdi gates` settles for. A tracker is read
//! for them only where it may have changed since it was last read. A
//! delivery from GitHub settles the one pull request it names, by the same
//! rule, and a timer gate kept from the last read closes at its deadline.

use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};

use crate::app::cover::Cover;
use crate::app::tracker::{open_failure, tracker_failure};
use crate::collect::bd::Cli;
use crate::collect::changes::Heard;
use crate::collect::gates::{
    self, Awaited, Done, OpenGates, PullRequest, Settled, Told, Unseen, Waiting,
};
use crate::collect::github;
use crate::collect::pr_events::EVENTS;
use crate::collect::run::{FailureKind, RunFailure, Runner};
use crate::collect::tracker::OpenFailure;
use crate::collect::upstream::{Answer, Upstream};
use crate::collect::webhook::Commit;
use crate::config::{Gates, Poll, Project};
use crate::model::gate::{self, Fault};
use crate::model::snapshot::TrackerFailure;

/// Something a look at the gates found to report.
#[derive(Debug, PartialEq, Eq)]
pub enum Found {
    /// A project's tracker did not say which gates it holds open.
    TrackerUnread {
        project: String,
        failure: TrackerFailure,
    },
    /// A gate cannot name what it waits on, or what it waits for that to do,
    /// so nothing settles it.
    Unsettleable {
        project: String,
        gate: String,
        faults: Vec<Fault>,
    },
    /// GitHub did not say where a pull request stands, so no gate waiting on
    /// it was touched.
    GitHubUnread {
        pull_request: PullRequest,
        failure: RunFailure,
    },
    /// GitHub said where a pull request stands, but not what an event reads
    /// of it, so that event did nothing and the others settled it.
    Unseen {
        pull_request: PullRequest,
        happening: &'static str,
        why: String,
    },
    /// GitHub did not say which open pull requests a commit heads, so a
    /// delivery about its checks settled none.
    CommitUnread { commit: Commit, failure: RunFailure },
    /// GitHub refused to say where a pull request stands for the rate limit
    /// of the login gh runs as, so nothing more is asked of it until
    /// `resets`, or for a while where GitHub does not say when that is.
    RateLimited {
        pull_request: PullRequest,
        resets: Option<DateTime<Utc>>,
    },
    /// GitHub did not say whether what a gh:issue, gh:release or gh:run gate
    /// waits on has happened, so the gate was left open.
    UpstreamUnread {
        project: String,
        gate: String,
        upstream: Upstream,
        failure: RunFailure,
    },
    /// What a gh:issue, gh:release or gh:run gate waits on `happened`, or a
    /// timer gate's deadline passed, and what came of closing the gate.
    Ended {
        project: String,
        gate: String,
        happened: String,
        done: Result<(), TrackerFailure>,
    },
    /// What a gh:run gate waits on `happened` and failed its wait, so the
    /// gate was left open, as `bd gate check` leaves it.
    Failed {
        project: String,
        gate: String,
        happened: String,
    },
    /// The producer covering a project has said nothing of it for
    /// `silent_for`, so its gates are settled as last read until it speaks.
    Lapsed {
        project: String,
        silent_for: Duration,
    },
    /// The producer covering a project spoke again after it lapsed.
    Vouched { project: String },
    /// A write settling a pull request asked of one bead, for what the pull
    /// request did, and what came of it.
    Settling {
        pull_request: PullRequest,
        happening: &'static str,
        project: String,
        bead: String,
        done: Result<Done, TrackerFailure>,
    },
}

/// What GitHub made of the reads one settling asked of it, taken together:
/// one it answered outweighs any it refused.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Read {
    /// GitHub was asked nothing.
    #[default]
    Unasked,
    /// GitHub refused every read it was asked, whether for its rate limit or
    /// for any other reason.
    Refused,
    /// GitHub answered at least one read.
    Answered,
}

/// What one settling, a look or a delivery, came to.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Settling {
    pub found: Vec<Found>,
    pub github: Read,
}

/// Each configured project's open gates as `bdi gates` last read them, and
/// what says when its tracker is read again.
///
/// A project's tracker is read on every look while its `poll` keeps it on
/// the clock, as the view polls it. Off the clock, it is read only when a
/// producer says it changed, when its last read failed, or when settling
/// wrote to it. A producer covering a project replaces the read on the clock
/// rather than backing it up, so a producer that falls silent is said and
/// the clock does not take over.
pub struct Kept {
    projects: Vec<Project>,
    kept: Vec<KeptGates>,
}

struct KeptGates {
    read: Result<OpenGates, OpenFailure>,
    /// Whether `read` may no longer be what the tracker holds.
    stale: bool,
    /// Whether the project's `poll` reads it on every look at all.
    polls: bool,
    cover: Cover,
    /// When a producer last said anything of the project, or nothing where
    /// none ever has.
    vouched: Option<Instant>,
    lapsed: bool,
}

impl KeptGates {
    fn due(&self) -> bool {
        self.stale || self.on_the_clock()
    }

    fn on_the_clock(&self) -> bool {
        self.polls && self.cover.keeps_the_poll()
    }

    fn read(&mut self, cli: &Cli, project: &Project, settles: &Gates) {
        self.read = cli.gates(project, |repo| settles.settles(repo.and_then(gate::owner)));
        self.stale = self.read.is_err();
    }

    /// The deadline of each timer gate kept that has one.
    fn deadlines(&self) -> impl Iterator<Item = DateTime<Utc>> + '_ {
        self.read
            .iter()
            .flat_map(|open| &open.timers)
            .filter_map(|timer| timer.deadline.as_ref().ok().copied())
    }

    /// Take out of what is kept each timer gate whose deadline is `at` or
    /// before, to be closed.
    fn run_out(&mut self, at: DateTime<Utc>) -> Vec<(String, DateTime<Utc>)> {
        let Ok(open) = &mut self.read else {
            return Vec::new();
        };
        let mut run_out = Vec::new();
        open.timers.retain(|timer| match timer.deadline {
            Ok(deadline) if deadline <= at => {
                run_out.push((timer.id.clone(), deadline));
                false
            }
            _ => true,
        });
        run_out
    }

    /// What there is to say about the producer covering `project` falling
    /// silent for longer than `covered_for`, or speaking again after it did.
    fn lapse(&mut self, project: &str, now: Instant, covered_for: Duration) -> Option<Found> {
        if self.on_the_clock() {
            return None;
        }
        let lapsed = now.duration_since(self.vouched?) > covered_for;
        if lapsed == std::mem::replace(&mut self.lapsed, lapsed) {
            return None;
        }
        let project = project.to_string();
        Some(if lapsed {
            Found::Lapsed {
                project,
                silent_for: covered_for,
            }
        } else {
            Found::Vouched { project }
        })
    }
}

impl Kept {
    /// Every one of `projects`, none of them read yet.
    pub fn new(projects: Vec<Project>) -> Self {
        let kept = projects
            .iter()
            .map(|project| KeptGates {
                read: Ok(OpenGates::default()),
                stale: true,
                polls: project.poll != Poll::Never,
                cover: Cover::of(project.poll),
                vouched: None,
                lapsed: false,
            })
            .collect();
        Self { projects, kept }
    }

    pub fn projects(&self) -> &[Project] {
        &self.projects
    }

    /// Take what a producer said, `now`.
    pub fn heard(&mut self, heard: &Heard, now: Instant) {
        let (Heard::Changed(project) | Heard::Covered(project)) = heard;
        let Some(at) = self.projects.iter().position(|each| each.name == *project) else {
            return;
        };
        let kept = &mut self.kept[at];
        kept.vouched = Some(now);
        match heard {
            Heard::Changed(_) => kept.stale = true,
            Heard::Covered(_) => {
                kept.cover.covered();
            }
        }
    }

    /// Mark every project `settling` wrote to, to be read again on the next
    /// look.
    pub fn settled(&mut self, settling: &Settling) {
        for found in &settling.found {
            let (Found::Ended { project, .. } | Found::Settling { project, .. }) = found else {
                continue;
            };
            for (each, kept) in self.projects.iter().zip(&mut self.kept) {
                kept.stale |= each.name == *project;
            }
        }
    }

    /// Read again each tracker that is due, then settle every gate kept, as
    /// [`look`] does, and close each timer gate that has run out by `at`. A
    /// gate whose repository `settles` leaves to another `bdi gates` is
    /// passed over without a word, and a timer gate names none. `now` and
    /// `covered_for` decide whether a producer has fallen silent. What the
    /// look wrote is for [`Kept::settled`] to hear, as what a delivery wrote
    /// is.
    #[allow(clippy::too_many_arguments)]
    pub fn look(
        &mut self,
        cli: &Cli,
        gh: &dyn Runner,
        settles: &Gates,
        told: &Told,
        now: Instant,
        at: DateTime<Utc>,
        covered_for: Duration,
    ) -> Settling {
        let mut lapses = Vec::new();
        for (project, kept) in self.projects.iter().zip(&mut self.kept) {
            if kept.due() {
                kept.read(cli, project, settles);
            }
            lapses.extend(kept.lapse(&project.name, now, covered_for));
        }
        let ran_out = self.close_run_out(cli, at);
        let mut settling = look(cli, gh, &self.projects, self.reads(), told);
        settling
            .found
            .splice(0..0, lapses.into_iter().chain(ran_out));
        settling
    }

    /// When the first timer gate kept runs out.
    pub fn next_deadline(&self) -> Option<DateTime<Utc>> {
        self.kept.iter().flat_map(KeptGates::deadlines).min()
    }

    /// Close each timer gate kept that has run out by `at`, asking GitHub
    /// nothing. A tracker holding one is read again first where it may have
    /// changed since, so a gate someone closed meanwhile is not closed again.
    /// A gate that would not close is left to the next look, which reads its
    /// tracker again for the write.
    pub fn timers_run_out(&mut self, cli: &Cli, settles: &Gates, at: DateTime<Utc>) -> Settling {
        let mut found = Vec::new();
        for (project, kept) in self.projects.iter().zip(&mut self.kept) {
            if kept.due() && kept.deadlines().any(|deadline| deadline <= at) {
                kept.read(cli, project, settles);
                if let Err(failure) = &kept.read {
                    found.push(Found::TrackerUnread {
                        project: project.name.clone(),
                        failure: open_failure(failure),
                    });
                }
            }
        }
        found.extend(self.close_run_out(cli, at));
        Settling {
            found,
            github: Read::Unasked,
        }
    }

    fn close_run_out(&mut self, cli: &Cli, at: DateTime<Utc>) -> Vec<Found> {
        let mut found = Vec::new();
        for (project, kept) in self.projects.iter().zip(&mut self.kept) {
            for (gate, deadline) in kept.run_out(at) {
                let happened = format!(
                    "Its timer ran out at {}",
                    deadline.format("%Y-%m-%d %H:%M:%S UTC")
                );
                found.push(closed(cli, project, gate, happened));
            }
        }
        found
    }

    fn reads(&self) -> impl Iterator<Item = (&Project, &Result<OpenGates, OpenFailure>)> {
        self.projects
            .iter()
            .zip(self.kept.iter().map(|kept| &kept.read))
    }
}

/// Settle each pull request one of the gates `read` waits on, once however
/// many gates wait on it, asking GitHub about each repository's together,
/// and then each gh:issue, gh:release and gh:run gate, until GitHub refuses one for
/// its rate limit. No tracker is asked about a pull request that has done
/// nothing new to the gates waiting on it, which `told` says of the beads
/// they hold back.
fn look<'a>(
    cli: &Cli,
    gh: &dyn Runner,
    projects: &[Project],
    read: impl Iterator<Item = (&'a Project, &'a Result<OpenGates, OpenFailure>)>,
    told: &Told,
) -> Settling {
    let mut found = Vec::new();
    let mut awaited: Vec<Vec<Awaited>> = Vec::new();
    let mut upstream = Vec::new();
    for (project, read) in read {
        let name = &project.name;
        let open = match read {
            Ok(open) => open,
            Err(failure) => {
                found.push(Found::TrackerUnread {
                    project: name.clone(),
                    failure: open_failure(failure),
                });
                continue;
            }
        };
        for gate in &open.pull_requests {
            match &gate.awaits {
                Err(faults) => found.push(Found::Unsettleable {
                    project: name.clone(),
                    gate: gate.id.clone(),
                    faults: faults.clone(),
                }),
                Ok(wait) => awaiting(
                    &mut awaited,
                    wait.pull_request.clone(),
                    Waiting {
                        project: name.clone(),
                        until: wait.until,
                        blocks: gate.blocks.clone(),
                        made: gate.made,
                    },
                ),
            }
        }
        for gate in &open.upstream {
            match &gate.awaits {
                Err(faults) => found.push(Found::Unsettleable {
                    project: name.clone(),
                    gate: gate.id.clone(),
                    faults: faults.clone(),
                }),
                Ok(waits) => upstream.push((project, gate.id.clone(), waits.clone())),
            }
        }
        for gate in &open.timers {
            if let Err(faults) = &gate.deadline {
                found.push(Found::Unsettleable {
                    project: name.clone(),
                    gate: gate.id.clone(),
                    faults: faults.clone(),
                });
            }
        }
    }
    let mut github = Read::Unasked;
    for repository in &awaited {
        let settled = gates::settle_together(cli, gh, projects, &EVENTS, repository, told);
        for (each, settled) in repository.iter().zip(settled) {
            let each = findings(gh, each.pull_request.clone(), settled);
            found.extend(each.found);
            github = github.max(each.github);
            if matches!(found.last(), Some(Found::RateLimited { .. })) {
                return Settling { found, github };
            }
        }
    }
    let ended = ended(cli, gh, &upstream);
    found.extend(ended.found);
    Settling {
        found,
        github: github.max(ended.github),
    }
}

/// Ask GitHub once about each thing the gates `waiting` wait on, and close
/// each gate whose wait has ended, until GitHub refuses one for its rate
/// limit. A gate whose wait failed is reported on every look it stays open,
/// as `bd gate check` reports it on every check.
// ponytail: a rate limit stops this look but sets no wait of its own, so the
// next look asks once more. Say it as `Found::RateLimited` if upstream gates
// ever outnumber the pull requests that already set that wait.
fn ended(cli: &Cli, gh: &dyn Runner, waiting: &[(&Project, String, Upstream)]) -> Settling {
    let mut answers: Vec<(&Upstream, Result<Answer, RunFailure>)> = Vec::new();
    let mut settling = Settling::default();
    for (project, gate, upstream) in waiting {
        let answer = match answers.iter().find(|(asked, _)| asked.asks_as(upstream)) {
            Some((_, answer)) => answer.clone(),
            None => {
                let answer = upstream.ask(gh);
                answers.push((upstream, answer.clone()));
                answer
            }
        };
        let answer = match answer {
            Ok(answer) => answer,
            Err(failure) => {
                settling.github = settling.github.max(Read::Refused);
                let rate_limited = failure.kind == FailureKind::RateLimited;
                settling.found.push(Found::UpstreamUnread {
                    project: project.name.clone(),
                    gate: gate.clone(),
                    upstream: upstream.clone(),
                    failure,
                });
                if rate_limited {
                    break;
                }
                continue;
            }
        };
        settling.github = Read::Answered;
        if let Some(happened) = upstream.ended(&answer) {
            settling
                .found
                .push(closed(cli, project, gate.clone(), happened));
        } else if let Some(happened) = upstream.failed(&answer) {
            settling.found.push(Found::Failed {
                project: project.name.clone(),
                gate: gate.clone(),
                happened,
            });
        }
    }
    settling
}

/// Close `gate` in `project`'s tracker for what `happened`.
fn closed(cli: &Cli, project: &Project, gate: String, happened: String) -> Found {
    let done = cli
        .settling(project)
        .map_err(|failure| open_failure(&failure))
        .and_then(|tracker| {
            tracker
                .resolve(&gate, &format!("{happened}."))
                .map_err(|failure| tracker_failure(&failure))
        });
    Found::Ended {
        project: project.name.clone(),
        gate,
        happened,
        done,
    }
}

/// Add `gate` to those waiting on `pull_request`, grouped by repository.
/// GitHub reads a repository's name in any case, so neither comparison heeds
/// it.
fn awaiting(awaited: &mut Vec<Vec<Awaited>>, pull_request: PullRequest, gate: Waiting) {
    let repository = match awaited.iter().position(|repository| {
        repository[0]
            .pull_request
            .repo
            .eq_ignore_ascii_case(&pull_request.repo)
    }) {
        Some(at) => &mut awaited[at],
        None => {
            awaited.push(Vec::new());
            awaited.last_mut().expect("one was just pushed")
        }
    };
    match repository
        .iter_mut()
        .find(|seen| seen.pull_request.is(&pull_request))
    {
        Some(seen) => seen.waiting.push(gate),
        None => repository.push(Awaited {
            pull_request,
            waiting: vec![gate],
        }),
    }
}

/// Settle the one pull request a delivery named, as a look settles each one
/// it finds. A pull request whose repository `settles` leaves to another
/// `bdi gates` is passed over without a word.
pub fn delivered(
    cli: &Cli,
    gh: &dyn Runner,
    projects: &[Project],
    settles: &Gates,
    pull_request: PullRequest,
    told: &Told,
) -> Settling {
    if !settles.settles(gate::owner(&pull_request.repo)) {
        return Settling::default();
    }
    let settled = gates::settle(cli, gh, projects, &EVENTS, &pull_request, told);
    findings(gh, pull_request, settled)
}

/// Settle the open pull requests whose head is the commit a delivery about
/// its checks named, each as [`delivered`] settles one. A lookup the rate
/// limit refuses is reported like any other and does not put off the asks
/// after it, which the next one finds refused again.
pub fn delivered_commit(
    cli: &Cli,
    gh: &dyn Runner,
    projects: &[Project],
    settles: &Gates,
    commit: Commit,
    told: &Told,
) -> Settling {
    let Some(repository) = gate::repository(&commit.repo) else {
        return Settling::default();
    };
    if !settles.settles(Some(repository.owner)) {
        return Settling::default();
    }
    let numbers = match github::open_pull_requests_headed_by(gh, &repository, &commit.sha) {
        Ok(numbers) => numbers,
        Err(failure) => {
            return Settling {
                found: vec![Found::CommitUnread { commit, failure }],
                github: Read::Refused,
            }
        }
    };
    let mut settling = Settling {
        found: Vec::new(),
        github: Read::Answered,
    };
    for number in numbers {
        let pull_request = PullRequest {
            repo: commit.repo.clone(),
            number,
        };
        let each = delivered(cli, gh, projects, settles, pull_request, told);
        settling.found.extend(each.found);
        settling.github = settling.github.max(each.github);
        if matches!(settling.found.last(), Some(Found::RateLimited { .. })) {
            break;
        }
    }
    settling
}

/// What settling `pull_request` found to report, and whether GitHub answered.
fn findings(gh: &dyn Runner, pull_request: PullRequest, settled: Settled) -> Settling {
    let (projects, unseen) = match settled {
        Settled::NothingNew { unseen } => (Vec::new(), unseen),
        Settled::Acted { projects, unseen } => (projects, unseen),
        Settled::Unread(failure) => return unread(gh, pull_request, failure),
    };
    let unseen = unseen
        .into_iter()
        .map(|Unseen { happening, why }| Found::Unseen {
            pull_request: pull_request.clone(),
            happening,
            why,
        });
    let acted = projects.into_iter().flat_map(|settled| match settled.acts {
        Err(failure) => vec![Found::TrackerUnread {
            project: settled.project,
            failure: open_failure(&failure),
        }],
        Ok(acts) => acts
            .into_iter()
            .map(|act| Found::Settling {
                pull_request: pull_request.clone(),
                happening: act.happening,
                project: settled.project.clone(),
                bead: act.bead,
                done: act.done.map_err(|failure| tracker_failure(&failure)),
            })
            .collect(),
    });
    Settling {
        found: unseen.chain(acted).collect(),
        github: Read::Answered,
    }
}

/// What GitHub refusing to say where `pull_request` stands found to report.
fn unread(gh: &dyn Runner, pull_request: PullRequest, failure: RunFailure) -> Settling {
    let found = if failure.kind == FailureKind::RateLimited {
        let resets = github::spent_until(gh, gate::host(&pull_request.repo));
        Found::RateLimited {
            pull_request,
            resets: resets.ok().flatten(),
        }
    } else {
        Found::GitHubUnread {
            pull_request,
            failure,
        }
    };
    Settling {
        found: vec![found],
        github: Read::Refused,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::run::testing::FakeRunner;
    use std::path::PathBuf;

    const COVERED_FOR: Duration = Duration::from_secs(60);

    const GATE_LIST: &str = include_str!("../../tests/fixtures/bd_1.3.0_gate_list.json");
    const MERGED: &str = r#"{"state":"MERGED","isDraft":false,"mergeCommit":{"oid":"5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff"}}"#;
    /// #7 open and #42 merged, as the one query a look asks reads them.
    const QUERIED: &str =
        include_str!("../../tests/fixtures/gh_2.102.0_api_graphql_ark_42_merged.json");

    /// Each gh:pr gate in `GATE_LIST`, beside what `bd dep list` answers for
    /// the beads it holds back. ark-0i5 waits on example/ark#42, ark-eb1 on
    /// example/ark#7, ark-6pp names no repo and ark-tg0 no number.
    const HELD_BACK: [(&str, &str); 4] = [
        (
            "ark-0i5",
            include_str!("../../tests/fixtures/bd_1.3.0_dep_list_up_ark-0i5.json"),
        ),
        (
            "ark-eb1",
            include_str!("../../tests/fixtures/bd_1.3.0_dep_list_up_ark-eb1.json"),
        ),
        (
            "ark-6pp",
            include_str!("../../tests/fixtures/bd_1.3.0_dep_list_up_ark-6pp.json"),
        ),
        (
            "ark-tg0",
            include_str!("../../tests/fixtures/bd_1.3.0_dep_list_up_ark-tg0.json"),
        ),
    ];

    fn project(name: &str) -> Project {
        Project {
            name: name.to_string(),
            path: PathBuf::from(format!("/nowhere/{name}")),
            database: None,
            user: None,
            server: None,
            environment_command: None,
            credential_command: None,
            prefix: None,
            poll: Default::default(),
            events_journal: false,
            badges: Vec::new(),
            worktrees: Vec::new(),
        }
    }

    fn read(project: &str, subcommand: &str) -> String {
        format!("bd -C /nowhere/{project} --readonly {subcommand}")
    }

    fn gate_list(project: &str) -> String {
        read(project, "gate list --limit 0 --json")
    }

    /// A runner answering for `project` with the captured tracker.
    fn captured(runner: FakeRunner, project: &str) -> FakeRunner {
        captured_in(runner, project, "ark")
    }

    /// A runner answering for `project` with the captured tracker, its gates
    /// naming the repository `example/<name>`.
    fn captured_in(runner: FakeRunner, project: &str, name: &str) -> FakeRunner {
        let gates = GATE_LIST.replace("\"example/ark\"", &format!("\"example/{name}\""));
        HELD_BACK.iter().fold(
            runner.with(&gate_list(project), &gates),
            |runner, (gate, held)| {
                runner.with(
                    &read(
                        project,
                        &format!("dep list {gate} --direction=up --type blocks --json"),
                    ),
                    held,
                )
            },
        )
    }

    /// The query a settling of #`number` alone asks.
    fn viewed(number: u64) -> String {
        format!(
            "gh api graphql -f owner=example -f name=ark -f query=query($owner:String!,\
             $name:String!){{repository(owner:$owner,name:$name){{pr{number}:pullRequest\
             (number:{number}){{state isDraft mergeCommit{{oid}} reviewDecision commits(last:1){{nodes{{commit{{oid statusCheckRollup{{state contexts(last:100){{nodes{{...on CheckRun{{conclusion completedAt}} ...on StatusContext{{state createdAt}}}}}}}}}}}}}} reviews(last:5){{nodes{{url state submittedAt author{{login}}}}}} mergeable headRefOid headRef{{target{{...on Commit{{committedDate}}}}}} baseRef{{target{{...on Commit{{committedDate}}}}}} comments(last:5){{nodes{{url createdAt author{{login}}}}}}}}}}}}"
        )
    }

    /// GitHub's answer to [`viewed`]: #`number` with `fields`.
    fn answer(number: u64, fields: &str) -> String {
        format!(r#"{{"data":{{"repository":{{"pr{number}":{fields}}}}}}}"#)
    }

    /// The one query a look asks about #7 and #42 in `example/<name>`.
    fn queried(name: &str) -> String {
        format!(
            "gh api graphql -f owner=example -f name={name} -f query=query($owner:String!,\
             $name:String!){{repository(owner:$owner,name:$name){{pr7:pullRequest(number:7)\
             {{state isDraft mergeCommit{{oid}} reviewDecision commits(last:1){{nodes{{commit{{oid statusCheckRollup{{state contexts(last:100){{nodes{{...on CheckRun{{conclusion completedAt}} ...on StatusContext{{state createdAt}}}}}}}}}}}}}} reviews(last:5){{nodes{{url state submittedAt author{{login}}}}}} mergeable headRefOid headRef{{target{{...on Commit{{committedDate}}}}}} baseRef{{target{{...on Commit{{committedDate}}}}}} comments(last:5){{nodes{{url createdAt author{{login}}}}}}}} pr42:pullRequest(number:42){{state isDraft mergeCommit{{oid}} reviewDecision commits(last:1){{nodes{{commit{{oid statusCheckRollup{{state contexts(last:100){{nodes{{...on CheckRun{{conclusion completedAt}} ...on StatusContext{{state createdAt}}}}}}}}}}}}}} reviews(last:5){{nodes{{url state submittedAt author{{login}}}}}} mergeable headRefOid headRef{{target{{...on Commit{{committedDate}}}}}} baseRef{{target{{...on Commit{{committedDate}}}}}} comments(last:5){{nodes{{url createdAt author{{login}}}}}}}}}}}}"
        )
    }

    fn resolving_42(project: &str) -> String {
        format!(
            "bd -C /nowhere/{project} gate resolve ark-0i5 --reason Pull request example/ark#42 \
             merged as 5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff."
        )
    }

    fn pr(number: u64) -> PullRequest {
        PullRequest {
            repo: "example/ark".to_string(),
            number,
        }
    }

    fn settling(project: &str) -> Found {
        Found::Settling {
            pull_request: pr(42),
            happening: "merged",
            project: project.to_string(),
            bead: "ark-0i5".to_string(),
            done: Ok(Done::Resolved),
        }
    }

    fn unsettleable(project: &str, gate: &str, fault: Fault) -> Found {
        Found::Unsettleable {
            project: project.to_string(),
            gate: gate.to_string(),
            faults: vec![fault],
        }
    }

    fn no_number(project: &str) -> Found {
        unsettleable(
            project,
            "ark-tg0",
            Fault::AwaitIdNotANumber("the-ninth".to_string()),
        )
    }

    fn owners(owners: &[&str]) -> Gates {
        Gates {
            owners: owners.iter().map(|owner| owner.to_string()).collect(),
            ..Gates::default()
        }
    }

    fn excluding(owners: &[&str]) -> Gates {
        Gates {
            excluded_owners: owners.iter().map(|owner| owner.to_string()).collect(),
            ..Gates::default()
        }
    }

    fn looked(runner: &FakeRunner, projects: &[Project], settles: &Gates) -> Settling {
        first_look(runner, projects, settles, &Told::default())
    }

    /// The look a `bdi gates` takes as it starts, which reads every tracker.
    fn first_look(
        runner: &FakeRunner,
        projects: &[Project],
        settles: &Gates,
        told: &Told,
    ) -> Settling {
        Kept::new(projects.to_vec()).look(
            &Cli::new(runner),
            runner,
            settles,
            told,
            Instant::now(),
            Utc::now(),
            COVERED_FOR,
        )
    }

    fn asked_github(runner: &FakeRunner) -> Vec<String> {
        runner
            .calls()
            .into_iter()
            .map(|call| call.argv)
            .filter(|argv| argv.starts_with("gh "))
            .collect()
    }

    #[test]
    fn a_pull_request_gates_in_two_projects_wait_on_is_asked_of_github_once_and_settled_in_both() {
        let runner = captured(captured(FakeRunner::default(), "arkham"), "dunwich")
            .with(&queried("ark"), QUERIED)
            .with(&resolving_42("arkham"), "")
            .with(&resolving_42("dunwich"), "");

        let found = looked(
            &runner,
            &[project("arkham"), project("dunwich")],
            &Gates::default(),
        )
        .found;

        assert_eq!(asked_github(&runner), [queried("ark")]);
        assert_eq!(
            found,
            [
                unsettleable("arkham", "ark-6pp", Fault::NoRepo),
                no_number("arkham"),
                unsettleable("dunwich", "ark-6pp", Fault::NoRepo),
                no_number("dunwich"),
                settling("arkham"),
                settling("dunwich"),
            ]
        );
    }

    /// gh prints GitHub's answer, then exits 1 for its refusing the checks on
    /// #7, so GitHub answered the look and the merge of #42 is settled.
    #[test]
    fn a_look_github_refuses_one_field_of_says_which_event_it_cannot_see_and_settles_the_rest() {
        let refused = r#"{"data":{"repository":{
            "pr7":{"state":"OPEN","isDraft":false,"mergeCommit":null,"commits":{"nodes":[{"commit":{"oid":"a1b2c3","statusCheckRollup":null}}]}},
            "pr42":{"state":"MERGED","isDraft":false,"mergeCommit":{"oid":"5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff"}}}},
            "errors":[{"type":"FORBIDDEN","path":["repository","pr7","commits","nodes",0,"commit","statusCheckRollup"],"message":"Resource not accessible by personal access token"}]}"#;
        let runner = captured(FakeRunner::default(), "arkham")
            .failing_having_printed(&queried("ark"), refused, unavailable("gh"))
            .with(&resolving_42("arkham"), "");

        let Settling { found, github } = looked(&runner, &[project("arkham")], &Gates::default());

        assert_eq!(
            found,
            [
                unsettleable("arkham", "ark-6pp", Fault::NoRepo),
                no_number("arkham"),
                Found::Unseen {
                    pull_request: pr(7),
                    happening: "has failing checks",
                    why: "GitHub would not let gh read commits: Resource not accessible by \
                          personal access token"
                        .to_string(),
                },
                settling("arkham"),
            ]
        );
        assert_eq!(github, Read::Answered);
    }

    fn gate_lists_read(runner: &FakeRunner) -> usize {
        runner
            .calls()
            .into_iter()
            .filter(|call| call.argv == gate_list("arkham"))
            .count()
    }

    /// #7 is open and ready for review, and no gate waits for that.
    #[test]
    fn a_look_reads_the_gates_again_only_for_the_pull_request_that_merged() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&queried("ark"), QUERIED)
            .with(&resolving_42("arkham"), "");

        looked(&runner, &[project("arkham")], &Gates::default());

        assert_eq!(gate_lists_read(&runner), 2);
    }

    /// #7 was reviewed a second before ark-eb1, the gate waiting on it, was
    /// made, and #42 is a draft.
    #[test]
    fn a_look_reads_the_gates_again_for_nothing_older_than_the_gate_waiting() {
        let reviewed_before_the_gate = r#"{"data":{"repository":{
            "pr7":{"state":"OPEN","isDraft":false,"mergeCommit":null,"reviewDecision":null,"reviews":{"nodes":[{"url":"https://forge.invalid/example/ark/pull/7#pullrequestreview-11","state":"COMMENTED","submittedAt":"2026-10-06T08:24:48Z","author":{"login":"alice"}}]}},
            "pr42":{"state":"OPEN","isDraft":true,"mergeCommit":null,"reviewDecision":null}}}}"#;
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&queried("ark"), reviewed_before_the_gate);

        looked(&runner, &[project("arkham")], &Gates::default());

        assert_eq!(gate_lists_read(&runner), 1);
    }

    #[test]
    fn a_look_closes_a_gate_waiting_for_its_pull_request_to_leave_draft() {
        let mut rows: Vec<serde_json::Value> =
            serde_json::from_str(GATE_LIST).expect("the capture parses");
        for row in rows.iter_mut().filter(|row| row["id"] == "ark-eb1") {
            row["metadata"][gate::AWAITS] = "ready_for_review".into();
        }
        let gates = serde_json::to_string(&rows).expect("the rows print");
        let resolving_7 = "bd -C /nowhere/arkham gate resolve ark-eb1 --reason Pull request \
                           example/ark#7 is ready for review.";
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&gate_list("arkham"), &gates)
            .with(&queried("ark"), QUERIED)
            .with(resolving_7, "")
            .with(&resolving_42("arkham"), "");

        let found = looked(&runner, &[project("arkham")], &Gates::default()).found;

        assert_eq!(
            found,
            [
                unsettleable("arkham", "ark-6pp", Fault::NoRepo),
                no_number("arkham"),
                Found::Settling {
                    pull_request: pr(7),
                    happening: "is ready for review",
                    project: "arkham".to_string(),
                    bead: "ark-eb1".to_string(),
                    done: Ok(Done::Resolved),
                },
                settling("arkham"),
            ]
        );
    }

    #[test]
    fn a_look_closes_a_gate_waiting_for_its_pull_request_to_be_approved() {
        let mut rows: Vec<serde_json::Value> =
            serde_json::from_str(GATE_LIST).expect("the capture parses");
        for row in rows.iter_mut().filter(|row| row["id"] == "ark-eb1") {
            row["metadata"][gate::AWAITS] = "approved".into();
        }
        let gates = serde_json::to_string(&rows).expect("the rows print");
        let approved_7 = include_str!("../../tests/fixtures/gh_2.102.0_api_graphql_ark_42_merged.json")
            .replace(
                r#""pr7":{"state":"OPEN","isDraft":false,"mergeCommit":null"#,
                r#""pr7":{"state":"OPEN","isDraft":false,"mergeCommit":null,"reviewDecision":"APPROVED""#,
            );
        let resolving_7 = "bd -C /nowhere/arkham gate resolve ark-eb1 --reason Pull request \
                           example/ark#7 is approved.";
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&gate_list("arkham"), &gates)
            .with(&queried("ark"), &approved_7)
            .with(resolving_7, "")
            .with(&resolving_42("arkham"), "");

        let found = looked(&runner, &[project("arkham")], &Gates::default()).found;

        assert_eq!(
            found,
            [
                unsettleable("arkham", "ark-6pp", Fault::NoRepo),
                no_number("arkham"),
                Found::Settling {
                    pull_request: pr(7),
                    happening: "is approved",
                    project: "arkham".to_string(),
                    bead: "ark-eb1".to_string(),
                    done: Ok(Done::Resolved),
                },
                settling("arkham"),
            ]
        );
    }

    /// The runner panics on any call it was not given, so a gate whose held
    /// back beads were asked after, or that was settled, would fail the test.
    #[test]
    fn a_gate_whose_owner_this_bdi_gates_does_not_settle_for_is_passed_over_without_a_word() {
        let runner = FakeRunner::default().with(&gate_list("arkham"), GATE_LIST);

        let Settling { found, github } =
            looked(&runner, &[project("arkham")], &owners(&["miskatonic"]));

        assert_eq!(found, []);
        assert_eq!(asked_github(&runner), Vec::<String>::new());
        assert_eq!(github, Read::Unasked);
    }

    #[test]
    fn an_owner_is_matched_in_any_case() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&queried("ark"), QUERIED)
            .with(&resolving_42("arkham"), "");

        let found = looked(&runner, &[project("arkham")], &owners(&["Example"])).found;

        assert_eq!(
            found,
            [no_number("arkham"), settling("arkham")],
            "ark-6pp names no repo, so no owner, and is left to a bdi gates settling every owner's"
        );
    }

    #[test]
    fn a_bdi_gates_leaving_out_one_owner_settles_the_gates_of_an_owner_it_was_never_told_about() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&queried("ark"), QUERIED)
            .with(&resolving_42("arkham"), "");

        let found = looked(&runner, &[project("arkham")], &excluding(&["miskatonic"])).found;

        assert_eq!(
            found,
            [
                unsettleable("arkham", "ark-6pp", Fault::NoRepo),
                no_number("arkham"),
                settling("arkham"),
            ],
            "ark-6pp names no owner, so none is left out and it is settled here"
        );
    }

    #[test]
    fn a_gate_whose_owner_is_left_out_is_passed_over_without_a_word() {
        let runner = captured(FakeRunner::default(), "arkham");

        let Settling { found, github } =
            looked(&runner, &[project("arkham")], &excluding(&["Example"]));

        assert_eq!(
            found,
            [unsettleable("arkham", "ark-6pp", Fault::NoRepo)],
            "ark-6pp names no owner, so none is left out and it is reported here"
        );
        assert_eq!(asked_github(&runner), Vec::<String>::new());
        assert_eq!(github, Read::Unasked);
    }

    fn delivered_here(runner: &FakeRunner, settles: &Gates, number: u64) -> Settling {
        delivered(
            &Cli::new(runner),
            runner,
            &[project("arkham")],
            settles,
            pr(number),
            &Told::default(),
        )
    }

    const SHA: &str = "5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff";
    const HEADED_BY_SHA: &str =
        "gh api repos/example/ark/commits/5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff/pulls?per_page=100";

    fn commit() -> Commit {
        Commit {
            repo: "example/ark".to_string(),
            sha: SHA.to_string(),
        }
    }

    fn delivered_commit_here(runner: &FakeRunner, settles: &Gates) -> Settling {
        delivered_commit(
            &Cli::new(runner),
            runner,
            &[project("arkham")],
            settles,
            commit(),
            &Told::default(),
        )
    }

    /// #42 open with the commit as head, #43 open without it, #44 closed
    /// with it.
    const PULLS_OF_SHA: &str = r#"[
        {"number":42,"state":"open","head":{"sha":"5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff"}},
        {"number":43,"state":"open","head":{"sha":"0badc0de0badc0de0badc0de0badc0de0badc0de"}},
        {"number":44,"state":"closed","head":{"sha":"5eaf00d1c0ffee5eaf00d1c0ffee5eaf00d1c0ff"}}
    ]"#;

    #[test]
    fn a_delivery_about_a_commit_settles_each_open_pull_request_it_heads_and_no_other() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(HEADED_BY_SHA, PULLS_OF_SHA)
            .with(&viewed(42), &answer(42, MERGED))
            .with(&resolving_42("arkham"), "");

        let Settling { found, github } = delivered_commit_here(&runner, &owners(&["example"]));

        assert_eq!(found, [settling("arkham")]);
        assert_eq!(
            asked_github(&runner),
            [HEADED_BY_SHA.to_string(), viewed(42)]
        );
        assert_eq!(github, Read::Answered);
    }

    #[test]
    fn a_delivery_about_a_commit_no_open_pull_request_heads_asks_nothing_more() {
        let runner = FakeRunner::default().with(HEADED_BY_SHA, "[]");

        let Settling { found, github } = delivered_commit_here(&runner, &Gates::default());

        assert_eq!(found, []);
        assert_eq!(asked_github(&runner), [HEADED_BY_SHA.to_string()]);
        assert_eq!(github, Read::Answered);
    }

    #[test]
    fn a_commit_github_cannot_list_the_pull_requests_of_is_reported_and_settles_nothing() {
        let runner = FakeRunner::default().failing(HEADED_BY_SHA, unavailable("gh"));

        let Settling { found, github } = delivered_commit_here(&runner, &Gates::default());

        assert_eq!(
            found,
            [Found::CommitUnread {
                commit: commit(),
                failure: unavailable("gh"),
            }]
        );
        assert_eq!(github, Read::Refused);
    }

    #[test]
    fn a_delivery_about_a_commit_in_a_repository_this_bdi_gates_does_not_settle_for_asks_nothing() {
        let runner = FakeRunner::default();

        let Settling { found, github } = delivered_commit_here(&runner, &owners(&["miskatonic"]));

        assert_eq!(found, []);
        assert_eq!(runner.calls(), []);
        assert_eq!(github, Read::Unasked);
    }

    #[test]
    fn a_delivery_settles_the_pull_request_it_names_and_no_other() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&viewed(42), &answer(42, MERGED))
            .with(&resolving_42("arkham"), "");

        let found = delivered_here(&runner, &owners(&["example"]), 42).found;

        assert_eq!(found, [settling("arkham")]);
        assert_eq!(asked_github(&runner), [viewed(42)]);
    }

    /// GitHub is asked about it, as the settling of any pull request asks
    /// first, and the tracker is asked which gates wait on it. The runner
    /// panics on any call it was not given, so a write would fail the test.
    #[test]
    fn a_delivery_for_a_merged_pull_request_no_gate_waits_on_writes_nothing() {
        let runner = captured(FakeRunner::default(), "arkham").with(&viewed(9), &answer(9, MERGED));

        let Settling { found, github } = delivered_here(&runner, &Gates::default(), 9);

        assert_eq!(found, []);
        assert_eq!(asked_github(&runner), [viewed(9)]);
        assert_eq!(github, Read::Answered);
    }

    #[test]
    fn a_delivery_whose_owner_this_bdi_gates_does_not_settle_for_is_passed_over_without_a_word() {
        let runner = FakeRunner::default();

        let Settling { found, github } = delivered_here(&runner, &owners(&["miskatonic"]), 42);

        assert_eq!(found, []);
        assert_eq!(runner.calls(), []);
        assert_eq!(github, Read::Unasked);
    }

    #[test]
    fn a_delivery_whose_owner_is_left_out_is_passed_over_without_a_word() {
        let runner = FakeRunner::default();

        let Settling { found, github } = delivered_here(&runner, &excluding(&["example"]), 42);

        assert_eq!(found, []);
        assert_eq!(runner.calls(), []);
        assert_eq!(github, Read::Unasked);
    }

    fn unavailable(program: &str) -> RunFailure {
        RunFailure {
            kind: FailureKind::Unavailable,
            program: program.to_string(),
            detail: format!("{program} exited 1 for a reason bdi cannot place"),
            unreadable: None,
        }
    }

    #[test]
    fn each_repository_is_asked_of_github_once_and_each_one_settled() {
        let runner = captured_in(
            captured(FakeRunner::default(), "arkham"),
            "dunwich",
            "vault",
        )
        .with(&queried("ark"), QUERIED)
        .with(
            &queried("vault"),
            &QUERIED.replace("\"MERGED\"", "\"OPEN\""),
        )
        .with(&resolving_42("arkham"), "");

        let found = looked(
            &runner,
            &[project("arkham"), project("dunwich")],
            &owners(&["example"]),
        )
        .found;

        assert_eq!(asked_github(&runner), [queried("ark"), queried("vault")]);
        assert_eq!(
            found,
            [
                no_number("arkham"),
                no_number("dunwich"),
                settling("arkham")
            ]
        );
    }

    /// A failure that is not about a missing pull request would come back the
    /// same for each one asked alone, so none is asked again.
    #[test]
    fn a_repository_github_does_not_answer_for_is_reported_for_each_of_its_pull_requests() {
        let runner =
            captured(FakeRunner::default(), "arkham").failing(&queried("ark"), unavailable("gh"));

        let Settling { found, github } =
            looked(&runner, &[project("arkham")], &owners(&["example"]));

        let unread = |number| Found::GitHubUnread {
            pull_request: pr(number),
            failure: unavailable("gh"),
        };
        assert_eq!(found, [no_number("arkham"), unread(7), unread(42)]);
        assert_eq!(asked_github(&runner), [queried("ark")]);
        assert_eq!(github, Read::Refused);
    }

    fn gone() -> RunFailure {
        RunFailure {
            kind: FailureKind::Gone,
            program: "gh".to_string(),
            detail: "gh found no such repository, pull request or issue on GitHub".to_string(),
            unreadable: None,
        }
    }

    #[test]
    fn a_pull_request_github_does_not_have_is_reported_and_the_rest_of_its_repository_settled() {
        let runner = captured(FakeRunner::default(), "arkham")
            .failing(&queried("ark"), gone())
            .failing(&viewed(7), gone())
            .with(&viewed(42), &answer(42, MERGED))
            .with(&resolving_42("arkham"), "");

        let Settling { found, github } =
            looked(&runner, &[project("arkham")], &owners(&["example"]));

        assert_eq!(
            found,
            [
                no_number("arkham"),
                Found::GitHubUnread {
                    pull_request: pr(7),
                    failure: gone(),
                },
                settling("arkham"),
            ]
        );
        assert_eq!(
            asked_github(&runner),
            [queried("ark"), viewed(7), viewed(42)]
        );
        assert_eq!(
            github,
            Read::Answered,
            "one read answered outweighs one refused"
        );
    }

    fn rate_limited() -> RunFailure {
        RunFailure {
            kind: FailureKind::RateLimited,
            program: "gh".to_string(),
            detail: "gh was refused for GitHub's rate limit".to_string(),
            unreadable: None,
        }
    }

    const RATE_LIMIT: &str = "gh api rate_limit";
    const GRAPHQL_SPENT: &str =
        include_str!("../../tests/fixtures/gh_2.102.0_api_rate_limit_graphql_spent.json");

    /// The runner panics on any call it was not given, so asking GitHub about
    /// example/vault after example/ark was refused would fail the test.
    #[test]
    fn a_look_refused_for_a_rate_limit_asks_github_nothing_more_and_says_when_it_resets() {
        let runner = captured_in(
            captured(FakeRunner::default(), "arkham"),
            "dunwich",
            "vault",
        )
        .failing(&queried("ark"), rate_limited())
        .with(RATE_LIMIT, GRAPHQL_SPENT);

        let Settling { found, github } = looked(
            &runner,
            &[project("arkham"), project("dunwich")],
            &owners(&["example"]),
        );

        assert_eq!(
            found,
            [
                no_number("arkham"),
                no_number("dunwich"),
                Found::RateLimited {
                    pull_request: pr(7),
                    resets: DateTime::from_timestamp(1767227400, 0),
                },
            ]
        );
        assert_eq!(
            asked_github(&runner),
            [queried("ark"), RATE_LIMIT.to_string()]
        );
        assert_eq!(github, Read::Refused);
    }

    #[test]
    fn a_rate_limit_github_does_not_say_the_end_of_is_reported_without_one() {
        let runner = captured(FakeRunner::default(), "arkham")
            .failing(&viewed(42), rate_limited())
            .failing(RATE_LIMIT, unavailable("gh"));

        let found = delivered_here(&runner, &owners(&["example"]), 42).found;

        assert_eq!(
            found,
            [Found::RateLimited {
                pull_request: pr(42),
                resets: None,
            }]
        );
    }

    #[test]
    fn a_tracker_that_does_not_say_which_gates_it_holds_is_reported_and_the_rest_are_settled() {
        let runner = captured(FakeRunner::default(), "dunwich")
            .failing(&gate_list("arkham"), unavailable("bd"))
            .with(&queried("ark"), QUERIED)
            .with(&resolving_42("dunwich"), "");

        let found = looked(
            &runner,
            &[project("arkham"), project("dunwich")],
            &owners(&["example"]),
        )
        .found;

        let arkham_unread = || Found::TrackerUnread {
            project: "arkham".to_string(),
            failure: TrackerFailure::Unavailable,
        };
        assert_eq!(
            found,
            [
                arkham_unread(),
                no_number("dunwich"),
                arkham_unread(),
                settling("dunwich"),
            ],
            "settling #42 asks arkham again, and arkham is unread again"
        );
    }

    /// #7 closed unmerged and #42 open, as the one query a look asks reads
    /// them.
    fn queried_7_closed() -> String {
        include_str!("../../tests/fixtures/gh_2.102.0_api_graphql_ark_open.json")
            .replace(r#""pr7":{"state":"OPEN""#, r#""pr7":{"state":"CLOSED""#)
    }

    fn telling_7(bead: &str) -> String {
        format!(
            "bd -C /nowhere/arkham comments add {bead} Pull request example/ark#7 closed without \
             being merged, so the gh:pr gate waiting on it stays open."
        )
    }

    fn told_7(bead: &str) -> Found {
        Found::Settling {
            pull_request: pr(7),
            happening: "closed unmerged",
            project: "arkham".to_string(),
            bead: bead.to_string(),
            done: Ok(Done::Commented),
        }
    }

    /// The runner panics on any call it was not given, and is given no
    /// comments to read the second time, so a second look that asked any
    /// bead for its comments would fail the test.
    #[test]
    fn a_second_look_at_a_pull_request_closed_unmerged_asks_no_bead_for_its_comments() {
        let no_comments = include_str!("../../tests/fixtures/bd_1.3.0_comments_none.json");
        let first = captured(FakeRunner::default(), "arkham")
            .with(&queried("ark"), &queried_7_closed())
            .with(&read("arkham", "comments ark-2ud --json"), no_comments)
            .with(&read("arkham", "comments ark-45c --json"), no_comments)
            .with(&telling_7("ark-2ud"), "")
            .with(&telling_7("ark-45c"), "");
        let second =
            captured(FakeRunner::default(), "arkham").with(&queried("ark"), &queried_7_closed());
        let told = Told::default();
        let look_with = |runner: &FakeRunner| {
            first_look(runner, &[project("arkham")], &owners(&["example"]), &told).found
        };

        let found_first = look_with(&first);
        let found_second = look_with(&second);

        assert_eq!(
            found_first,
            [no_number("arkham"), told_7("ark-2ud"), told_7("ark-45c")]
        );
        assert_eq!(found_second, [no_number("arkham")]);
        assert_eq!(gate_lists_read(&second), 1);
    }

    /// ark-zl0 waits on example/ark#31 closing, ark-8dq on example/ark
    /// publishing a release after 2026-10-12T07:32:28Z, and ark-6dq names no
    /// repo.
    const UPSTREAM: &str = include_str!("../../tests/fixtures/bd_1.3.0_gate_list_upstream.json");

    const ISSUE_31: &str = "gh api graphql -f owner=example -f name=ark -f \
                            query=query($owner:String!,$name:String!){repository(owner:$owner,\
                            name:$name){issue(number:31){state}}}";

    const RELEASES: &str = "gh api graphql -f owner=example -f name=ark -f \
                            query=query($owner:String!,$name:String!){repository(owner:$owner,\
                            name:$name){releases(first:10,orderBy:{field:CREATED_AT,direction:\
                            DESC}){nodes{tagName isDraft publishedAt}}}}";

    fn issue(state: &str) -> String {
        format!(r#"{{"data":{{"repository":{{"issue":{{"state":"{state}"}}}}}}}}"#)
    }

    /// Releases published at each time given, the newest made first.
    fn published(releases: &[(&str, &str)]) -> String {
        let nodes: Vec<String> = releases
            .iter()
            .map(|(tag, at)| {
                format!(r#"{{"tagName":"{tag}","isDraft":false,"publishedAt":"{at}"}}"#)
            })
            .collect();
        format!(
            r#"{{"data":{{"repository":{{"releases":{{"nodes":[{}]}}}}}}}}"#,
            nodes.join(",")
        )
    }

    /// Nothing published since the gates in [`UPSTREAM`] were made.
    fn published_before() -> String {
        published(&[("v1.1.0", "2026-10-11T09:00:00Z")])
    }

    fn upstream_in(project: &str) -> FakeRunner {
        FakeRunner::default().with(&gate_list(project), UPSTREAM)
    }

    fn closing(project: &str, gate: &str, reason: &str) -> String {
        format!("bd -C /nowhere/{project} gate resolve {gate} --reason {reason}")
    }

    fn ended(project: &str, gate: &str, happened: &str) -> Found {
        Found::Ended {
            project: project.to_string(),
            gate: gate.to_string(),
            happened: happened.to_string(),
            done: Ok(()),
        }
    }

    #[test]
    fn a_look_closes_a_gate_whose_upstream_issue_has_closed() {
        let runner = upstream_in("arkham")
            .with(ISSUE_31, &issue("CLOSED"))
            .with(RELEASES, &published_before())
            .with(&closing("arkham", "ark-zl0", "example/ark#31 closed."), "");

        let Settling { found, github } = looked(&runner, &[project("arkham")], &Gates::default());

        assert_eq!(
            found,
            [
                unsettleable("arkham", "ark-6dq", Fault::NoRepo),
                ended("arkham", "ark-zl0", "example/ark#31 closed"),
            ]
        );
        assert_eq!(github, Read::Answered);
    }

    #[test]
    fn a_look_closes_a_gate_on_the_first_release_published_after_the_gate_was_made() {
        let runner = upstream_in("arkham")
            .with(ISSUE_31, &issue("OPEN"))
            .with(
                RELEASES,
                &published(&[
                    ("v1.3.0", "2026-10-14T09:00:00Z"),
                    ("v1.2.0", "2026-10-13T09:00:00Z"),
                    ("v1.1.0", "2026-10-11T09:00:00Z"),
                ]),
            )
            .with(
                &closing("arkham", "ark-8dq", "example/ark published v1.2.0."),
                "",
            );

        let found = looked(&runner, &[project("arkham")], &Gates::default()).found;

        assert_eq!(
            found,
            [
                unsettleable("arkham", "ark-6dq", Fault::NoRepo),
                ended("arkham", "ark-8dq", "example/ark published v1.2.0"),
            ]
        );
    }

    /// The runner panics on any call it was not given, so closing either gate
    /// would fail the test.
    #[test]
    fn a_look_leaves_open_a_gate_whose_issue_is_open_and_one_no_release_has_come_after() {
        let runner = upstream_in("arkham")
            .with(ISSUE_31, &issue("OPEN"))
            .with(RELEASES, &published_before());

        let Settling { found, github } = looked(&runner, &[project("arkham")], &Gates::default());

        assert_eq!(found, [unsettleable("arkham", "ark-6dq", Fault::NoRepo)]);
        assert_eq!(github, Read::Answered);
    }

    #[test]
    fn the_gh_pr_gates_and_the_upstream_gates_of_a_tracker_are_settled_on_the_one_look() {
        let runner = captured(FakeRunner::default(), "arkham")
            .merging(&gate_list("arkham"), UPSTREAM)
            .with(&queried("ark"), QUERIED)
            .with(&resolving_42("arkham"), "")
            .with(ISSUE_31, &issue("CLOSED"))
            .with(RELEASES, &published_before())
            .with(&closing("arkham", "ark-zl0", "example/ark#31 closed."), "");

        let found = looked(&runner, &[project("arkham")], &Gates::default()).found;

        assert_eq!(
            found,
            [
                unsettleable("arkham", "ark-6pp", Fault::NoRepo),
                no_number("arkham"),
                unsettleable("arkham", "ark-6dq", Fault::NoRepo),
                settling("arkham"),
                ended("arkham", "ark-zl0", "example/ark#31 closed"),
            ]
        );
        assert_eq!(
            gate_lists_read(&runner),
            2,
            "one to find the gates and one to settle #42, as before"
        );
    }

    #[test]
    fn an_issue_gates_in_two_projects_wait_on_is_asked_of_github_once_and_settled_in_both() {
        let runner = upstream_in("arkham")
            .with(&gate_list("dunwich"), UPSTREAM)
            .with(ISSUE_31, &issue("CLOSED"))
            .with(RELEASES, &published_before())
            .with(&closing("arkham", "ark-zl0", "example/ark#31 closed."), "")
            .with(&closing("dunwich", "ark-zl0", "example/ark#31 closed."), "");

        let found = looked(
            &runner,
            &[project("arkham"), project("dunwich")],
            &owners(&["example"]),
        )
        .found;

        assert_eq!(
            found,
            [
                ended("arkham", "ark-zl0", "example/ark#31 closed"),
                ended("dunwich", "ark-zl0", "example/ark#31 closed"),
            ]
        );
        assert_eq!(asked_github(&runner), [RELEASES, ISSUE_31]);
    }

    #[test]
    fn an_upstream_gate_github_does_not_answer_for_is_reported_and_left_open() {
        let runner = upstream_in("arkham")
            .failing(ISSUE_31, unavailable("gh"))
            .failing(RELEASES, unavailable("gh"));

        let Settling { found, github } =
            looked(&runner, &[project("arkham")], &owners(&["example"]));

        assert_eq!(
            found,
            [
                Found::UpstreamUnread {
                    project: "arkham".to_string(),
                    gate: "ark-8dq".to_string(),
                    upstream: Upstream::Release {
                        repo: "example/ark".to_string(),
                        after: "2026-10-12T07:32:28Z".parse().unwrap(),
                    },
                    failure: unavailable("gh"),
                },
                Found::UpstreamUnread {
                    project: "arkham".to_string(),
                    gate: "ark-zl0".to_string(),
                    upstream: Upstream::Issue {
                        repo: "example/ark".to_string(),
                        number: 31,
                    },
                    failure: unavailable("gh"),
                },
            ]
        );
        assert_eq!(github, Read::Refused);
    }

    const RUNS: &str = include_str!("../../tests/fixtures/bd_1.3.0_gate_list_run.json");

    const RUN_4242: &str = "gh api repos/example/ark/actions/runs/4242";

    fn ran(status: &str, conclusion: &str) -> String {
        format!(r#"{{"id":4242,"name":"CI","status":"{status}","conclusion":{conclusion}}}"#)
    }

    fn runs_in(project: &str) -> FakeRunner {
        FakeRunner::default().with(&gate_list(project), RUNS)
    }

    fn workflow_named(project: &str) -> Found {
        unsettleable(
            project,
            "ark-68j",
            Fault::WorkflowName("release.yml".to_string()),
        )
    }

    #[test]
    fn a_look_closes_a_gate_whose_run_has_succeeded() {
        let runner = runs_in("arkham")
            .with(RUN_4242, &ran("completed", r#""success""#))
            .with(
                &closing(
                    "arkham",
                    "ark-tft",
                    "example/ark's run 4242 of CI succeeded.",
                ),
                "",
            );

        let Settling { found, github } = looked(&runner, &[project("arkham")], &Gates::default());

        assert_eq!(
            found,
            [
                unsettleable("arkham", "ark-9zz", Fault::NoRepo),
                workflow_named("arkham"),
                ended(
                    "arkham",
                    "ark-tft",
                    "example/ark's run 4242 of CI succeeded"
                ),
            ]
        );
        assert_eq!(github, Read::Answered);
    }

    /// The runner panics on any call it was not given, so closing the gate
    /// would fail the test.
    #[test]
    fn a_look_leaves_open_a_gate_whose_run_failed_and_says_so() {
        let runner = runs_in("arkham").with(RUN_4242, &ran("completed", r#""failure""#));

        let found = looked(&runner, &[project("arkham")], &Gates::default()).found;

        assert_eq!(
            found.last(),
            Some(&Found::Failed {
                project: "arkham".to_string(),
                gate: "ark-tft".to_string(),
                happened: "example/ark's run 4242 of CI failed".to_string(),
            })
        );
    }

    /// The runner panics on any call it was not given.
    #[test]
    fn a_look_leaves_open_without_a_word_a_gate_whose_run_is_still_going() {
        let runner = runs_in("arkham").with(RUN_4242, &ran("in_progress", "null"));

        let found = looked(&runner, &[project("arkham")], &Gates::default()).found;

        assert_eq!(
            found,
            [
                unsettleable("arkham", "ark-9zz", Fault::NoRepo),
                workflow_named("arkham"),
            ]
        );
    }

    /// The runner panics on any call it was not given, so asking after the
    /// issue would fail the test.
    #[test]
    fn a_look_refused_for_a_rate_limit_asks_after_no_more_upstream_gates() {
        let runner = upstream_in("arkham").failing(RELEASES, rate_limited());

        let found = looked(&runner, &[project("arkham")], &owners(&["example"])).found;

        assert_eq!(asked_github(&runner), [RELEASES]);
        assert!(
            matches!(found[..], [Found::UpstreamUnread { ref failure, .. }] if failure.kind == FailureKind::RateLimited),
            "{found:?}"
        );
    }

    /// The runner panics on any call it was not given.
    #[test]
    fn an_upstream_gate_whose_owner_this_bdi_gates_does_not_settle_for_is_passed_over() {
        let runner = upstream_in("arkham");

        let Settling { found, github } =
            looked(&runner, &[project("arkham")], &owners(&["miskatonic"]));

        assert_eq!(found, []);
        assert_eq!(github, Read::Unasked);
    }

    /// #7 and #42 both open, as the one query a look asks reads them.
    const QUERIED_OPEN: &str =
        include_str!("../../tests/fixtures/gh_2.102.0_api_graphql_ark_open.json");

    /// arkham's captured tracker, behind a GitHub where nothing has happened.
    fn quiet() -> FakeRunner {
        captured(FakeRunner::default(), "arkham").with(&queried("ark"), QUERIED_OPEN)
    }

    fn arkham() -> Kept {
        Kept::new(vec![project("arkham")])
    }

    fn covered() -> Heard {
        Heard::Covered("arkham".to_string())
    }

    fn look_at(kept: &mut Kept, runner: &FakeRunner, now: Instant) -> Vec<Found> {
        kept.look(
            &Cli::new(runner),
            runner,
            &owners(&["example"]),
            &Told::default(),
            now,
            Utc::now(),
            COVERED_FOR,
        )
        .found
    }

    #[test]
    fn a_tracker_no_producer_names_is_read_on_every_look() {
        let runner = quiet();
        let mut kept = arkham();
        let now = Instant::now();

        look_at(&mut kept, &runner, now);
        look_at(&mut kept, &runner, now);

        assert_eq!(gate_lists_read(&runner), 2);
    }

    #[test]
    fn a_covered_tracker_is_read_again_only_once_its_producer_says_it_changed() {
        let runner = quiet();
        let mut kept = arkham();
        let now = Instant::now();
        kept.heard(&covered(), now);

        look_at(&mut kept, &runner, now);
        look_at(&mut kept, &runner, now);
        look_at(&mut kept, &runner, now);
        assert_eq!(gate_lists_read(&runner), 1);
        assert_eq!(asked_github(&runner).len(), 3, "each look asks GitHub");

        kept.heard(&Heard::Changed("arkham".to_string()), now);
        look_at(&mut kept, &runner, now);
        look_at(&mut kept, &runner, now);
        assert_eq!(gate_lists_read(&runner), 2);
    }

    #[test]
    fn a_covered_tracker_settling_wrote_to_is_read_again_and_one_it_only_found_is_not() {
        let runner = quiet();
        let mut kept = arkham();
        let now = Instant::now();
        kept.heard(&covered(), now);
        look_at(&mut kept, &runner, now);

        kept.settled(&Settling {
            found: vec![no_number("arkham")],
            github: Read::Answered,
        });
        look_at(&mut kept, &runner, now);
        assert_eq!(gate_lists_read(&runner), 1);

        kept.settled(&Settling {
            found: vec![settling("arkham")],
            github: Read::Answered,
        });
        look_at(&mut kept, &runner, now);
        assert_eq!(gate_lists_read(&runner), 2);
    }

    #[test]
    fn a_covered_tracker_whose_read_failed_is_read_again_on_the_next_look() {
        let runner = FakeRunner::default().failing(&gate_list("arkham"), unavailable("bd"));
        let mut kept = arkham();
        let now = Instant::now();
        kept.heard(&covered(), now);

        look_at(&mut kept, &runner, now);
        look_at(&mut kept, &runner, now);

        assert_eq!(gate_lists_read(&runner), 2);
    }

    /// The lapse is said, and the tracker is still not read on the clock.
    #[test]
    fn a_producer_falling_silent_is_said_once_and_speaking_again_is_said() {
        let runner = quiet();
        let mut kept = arkham();
        let start = Instant::now();
        kept.heard(&covered(), start);
        let silent = start + COVERED_FOR + Duration::from_secs(1);
        let lapsed = Found::Lapsed {
            project: "arkham".to_string(),
            silent_for: COVERED_FOR,
        };
        let vouched = Found::Vouched {
            project: "arkham".to_string(),
        };

        assert!(!look_at(&mut kept, &runner, start).contains(&lapsed));
        assert!(look_at(&mut kept, &runner, silent).contains(&lapsed));
        assert!(!look_at(&mut kept, &runner, silent).contains(&lapsed));
        kept.heard(&covered(), silent);
        assert!(look_at(&mut kept, &runner, silent).contains(&vouched));
        assert!(!look_at(&mut kept, &runner, silent).contains(&vouched));
        assert_eq!(gate_lists_read(&runner), 1);
    }

    fn arkham_polling(poll: Poll) -> Kept {
        Kept::new(vec![Project {
            poll,
            ..project("arkham")
        }])
    }

    /// A bd wrapper or a git hook sends bare names without promising to
    /// report every change, so they never stand the clock down.
    #[test]
    fn a_tracker_only_bare_names_are_heard_for_is_read_on_every_look() {
        let runner = quiet();
        let mut kept = arkham();
        let start = Instant::now();
        kept.heard(&Heard::Changed("arkham".to_string()), start);

        look_at(&mut kept, &runner, start);
        look_at(&mut kept, &runner, start);
        let silent = start + COVERED_FOR + Duration::from_secs(1);
        let found = look_at(&mut kept, &runner, silent);

        assert_eq!(gate_lists_read(&runner), 3);
        assert!(
            !found
                .iter()
                .any(|found| matches!(found, Found::Lapsed { .. })),
            "a tracker on the clock does not lapse: {found:?}"
        );
    }

    /// `poll = true` is the reader's choice of the clock, and a producer
    /// covering the tracker does not take it away.
    #[test]
    fn a_covered_tracker_whose_poll_is_true_is_read_on_every_look() {
        let runner = quiet();
        let mut kept = arkham_polling(Poll::Always);
        let start = Instant::now();
        kept.heard(&covered(), start);

        look_at(&mut kept, &runner, start);
        look_at(&mut kept, &runner, start);
        let silent = start + COVERED_FOR + Duration::from_secs(1);
        let found = look_at(&mut kept, &runner, silent);

        assert_eq!(gate_lists_read(&runner), 3);
        assert!(
            !found
                .iter()
                .any(|found| matches!(found, Found::Lapsed { .. })),
            "a tracker on the clock does not lapse: {found:?}"
        );
    }

    /// `poll = false` reads the tracker once to begin with and then only when
    /// something says it changed.
    #[test]
    fn a_tracker_whose_poll_is_false_is_never_read_on_the_clock() {
        let runner = quiet();
        let mut kept = arkham_polling(Poll::Never);
        let now = Instant::now();

        look_at(&mut kept, &runner, now);
        look_at(&mut kept, &runner, now);
        assert_eq!(gate_lists_read(&runner), 1);

        kept.heard(&Heard::Changed("arkham".to_string()), now);
        look_at(&mut kept, &runner, now);
        look_at(&mut kept, &runner, now);
        assert_eq!(gate_lists_read(&runner), 2);
    }

    /// With nothing on the clock, a producer falling silent is all there is
    /// to say about the tracker going stale.
    #[test]
    fn a_tracker_whose_poll_is_false_lapses_when_its_producer_falls_silent() {
        let runner = quiet();
        let mut kept = arkham_polling(Poll::Never);
        let start = Instant::now();
        kept.heard(&Heard::Changed("arkham".to_string()), start);
        let silent = start + COVERED_FOR + Duration::from_secs(1);

        assert!(
            look_at(&mut kept, &runner, silent).contains(&Found::Lapsed {
                project: "arkham".to_string(),
                silent_for: COVERED_FOR,
            })
        );
    }

    /// ark-06g, a timer gate of two hours made on 2026-10-12 at 07:32:28.
    const GATE_LIST_TIMER: &str =
        include_str!("../../tests/fixtures/bd_1.3.0_gate_list_timer.json");

    const RAN_OUT: &str = "Its timer ran out at 2026-10-12 09:32:28 UTC";

    fn at(when: &str) -> DateTime<Utc> {
        when.parse().expect("a time")
    }

    fn timed(runner: FakeRunner) -> FakeRunner {
        runner.with(&gate_list("arkham"), GATE_LIST_TIMER)
    }

    fn closing_06g() -> String {
        format!("bd -C /nowhere/arkham gate resolve ark-06g --reason {RAN_OUT}.")
    }

    fn ran_out(done: Result<(), TrackerFailure>) -> Found {
        Found::Ended {
            project: "arkham".to_string(),
            gate: "ark-06g".to_string(),
            happened: RAN_OUT.to_string(),
            done,
        }
    }

    fn timer_look(kept: &mut Kept, runner: &FakeRunner, when: &str) -> Settling {
        kept.look(
            &Cli::new(runner),
            runner,
            &Gates::default(),
            &Told::default(),
            Instant::now(),
            at(when),
            COVERED_FOR,
        )
    }

    fn deadline_passed(kept: &mut Kept, runner: &FakeRunner, when: &str) -> Settling {
        kept.timers_run_out(&Cli::new(runner), &Gates::default(), at(when))
    }

    /// Such as one that ran out while `bdi gates` was down.
    #[test]
    fn a_timer_gate_that_ran_out_before_a_look_is_closed_on_it() {
        let runner = timed(FakeRunner::default()).with(&closing_06g(), "");

        let settling = timer_look(&mut arkham(), &runner, "2026-10-13T00:00:00Z");

        assert_eq!(
            settling,
            Settling {
                found: vec![ran_out(Ok(()))],
                github: Read::Unasked,
            }
        );
    }

    /// The runner panics on any call it was not given, a close included.
    #[test]
    fn a_timer_gate_yet_to_run_out_is_kept_until_its_deadline() {
        let runner = timed(FakeRunner::default());
        let mut kept = arkham();

        let settling = timer_look(&mut kept, &runner, "2026-10-12T08:00:00Z");

        assert_eq!(settling.found, []);
        assert_eq!(kept.next_deadline(), Some(at("2026-10-12T09:32:28Z")));
    }

    #[test]
    fn a_covered_tracker_s_timer_gate_closes_at_its_deadline_from_the_gates_kept() {
        let runner = timed(FakeRunner::default()).with(&closing_06g(), "");
        let mut kept = arkham();
        kept.heard(&covered(), Instant::now());
        timer_look(&mut kept, &runner, "2026-10-12T08:00:00Z");

        let early = deadline_passed(&mut kept, &runner, "2026-10-12T09:32:27Z");
        let settling = deadline_passed(&mut kept, &runner, "2026-10-12T09:32:28Z");

        assert_eq!(early, Settling::default());
        assert_eq!(
            settling,
            Settling {
                found: vec![ran_out(Ok(()))],
                github: Read::Unasked,
            }
        );
        assert_eq!(gate_lists_read(&runner), 1);
        assert_eq!(kept.next_deadline(), None, "a gate is closed once");
    }

    #[test]
    fn a_timer_gate_whose_producer_said_its_tracker_changed_is_read_again_before_it_closes() {
        let runner = timed(FakeRunner::default()).with(&closing_06g(), "");
        let mut kept = arkham();
        kept.heard(&covered(), Instant::now());
        timer_look(&mut kept, &runner, "2026-10-12T08:00:00Z");
        kept.heard(&Heard::Changed("arkham".to_string()), Instant::now());

        deadline_passed(&mut kept, &runner, "2026-10-12T09:32:28Z");

        let calls: Vec<String> = runner.calls().into_iter().map(|call| call.argv).collect();
        assert_eq!(calls[1..], [gate_list("arkham"), closing_06g()]);
    }

    /// The next look reads the tracker again, as after any write, and tries
    /// once more if the gate is still open.
    #[test]
    fn a_timer_gate_that_would_not_close_is_said_and_left_to_the_next_look() {
        let runner = timed(FakeRunner::default()).failing(&closing_06g(), unavailable("bd"));
        let mut kept = arkham();
        timer_look(&mut kept, &runner, "2026-10-12T08:00:00Z");

        let settling = deadline_passed(&mut kept, &runner, "2026-10-12T09:32:28Z");

        assert!(
            matches!(settling.found[..], [Found::Ended { done: Err(_), .. }]),
            "{settling:?}"
        );
        assert_eq!(kept.next_deadline(), None);
    }

    #[test]
    fn a_timer_gate_without_a_timeout_cannot_be_settled() {
        let untimed = GATE_LIST_TIMER.replace(",\n    \"timeout\": 7200000000000", "");
        assert_ne!(untimed, GATE_LIST_TIMER);
        let runner = FakeRunner::default().with(&gate_list("arkham"), &untimed);
        let mut kept = arkham();

        let settling = timer_look(&mut kept, &runner, "2026-10-13T00:00:00Z");

        assert_eq!(
            settling.found,
            [unsettleable("arkham", "ark-06g", Fault::NoTimeout)]
        );
        assert_eq!(kept.next_deadline(), None);
    }
}
