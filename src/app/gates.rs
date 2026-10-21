//! One look at every configured project's gh:pr, gh:issue, gh:release, gh:run,
//! timer and bead gates, which `bdi gates` takes on its poll: each pull request,
//! issue, workflow run or repository's releases they wait on is asked of
//! GitHub once, where its
//! repository's owner is one this `bdi gates` settles for. A tracker is read
//! for them only where it may have changed since it was last read. A
//! delivery from GitHub settles the one pull request it names against the
//! gates kept from the last read, reading only a tracker that is stale, and a
//! timer gate kept from the last read closes at its deadline. A bead gate's
//! bead is read as the gate first appears and again only when the tracker
//! holding it is read again, so a producer saying that tracker changed is what
//! closes the gate.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};

use crate::app::cover::Cover;
use crate::app::tracker::{open_failure, tracker_failure};
use crate::collect::bd::Cli;
use crate::collect::changes::Heard;
use crate::collect::gates::{
    self, Awaited, BeadGate, Done, OpenGates, PrGate, PullRequest, Settled, Told, Unseen, Wait,
    Waiting,
};
use crate::collect::github;
use crate::collect::pr_events::EVENTS;
use crate::collect::run::{FailureKind, RunFailure, Runner};
use crate::collect::tracker::OpenFailure;
use crate::collect::upstream::{Answer, Upstream};
use crate::collect::webhook::{Commit, Said};
use crate::config::{Gates, Poll, Project};
use crate::model::gate::{self, Fault};
use crate::model::snapshot::TrackerFailure;
use crate::model::tree::{prefix_of, states_the_prefix_of};
use crate::model::types::Status;

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
    /// The tracker `holder` did not say whether `bead`, which bead gate
    /// `gate` in `project` awaits, has closed, so the gate was left open.
    AwaitedUnread {
        project: String,
        gate: String,
        holder: String,
        bead: String,
        failure: TrackerFailure,
    },
    /// What a gh:issue, gh:release or gh:run gate waits on `happened`, a timer
    /// gate's deadline passed, or a bead gate's bead closed, and what came of
    /// closing the gate.
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
    /// Each bead gate in `read`, with the bead it awaits, whose bead has been
    /// read since the gate came to await it. Each is read again only when its
    /// bead's tracker is.
    asked: BTreeSet<(String, String)>,
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
                asked: BTreeSet::new(),
            })
            .collect();
        Self { projects, kept }
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
    /// look. A bead gate that would not close has its bead asked after again
    /// then too.
    pub fn settled(&mut self, settling: &Settling) {
        for found in &settling.found {
            let (Found::Ended { project, .. } | Found::Settling { project, .. }) = found else {
                continue;
            };
            for (each, kept) in self.projects.iter().zip(&mut self.kept) {
                if each.name != *project {
                    continue;
                }
                kept.stale = true;
                if let Found::Ended {
                    gate, done: Err(_), ..
                } = found
                {
                    kept.asked.retain(|(asked, _)| asked != gate);
                }
            }
        }
    }

    /// Read again each tracker that is due, then settle every gate kept, as
    /// [`look`] does, close each timer gate that has run out by `at`, and
    /// close each bead gate whose bead has closed. A gate whose repository
    /// `settles` leaves to another `bdi gates` is passed over without a word,
    /// and a timer gate and a bead gate name none. `now` and
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
        let mut read_again = Vec::new();
        for (project, kept) in self.projects.iter().zip(&mut self.kept) {
            let due = kept.due();
            if due {
                kept.read(cli, project, settles);
            }
            read_again.push(due);
            lapses.extend(kept.lapse(&project.name, now, covered_for));
        }
        let ran_out = self.close_run_out(cli, at);
        let beads = self.close_awaited_closed(cli, &read_again);
        let mut settling = look(cli, gh, &self.projects, self.reads(), told);
        settling.found.splice(
            0..0,
            lapses
                .into_iter()
                .chain(ran_out)
                .chain(self.unsettleable_bead_gates())
                .chain(beads),
        );
        settling
    }

    /// Read again each tracker a producer said changed, and close each bead
    /// gate whose bead has closed, asking GitHub nothing. A tracker the clock
    /// reads waits for the next look, unless a producer named it too.
    pub fn changed(&mut self, cli: &Cli, settles: &Gates) -> Settling {
        let (mut found, read_again) = self.read_where(cli, settles, |kept| kept.stale);
        found.extend(self.close_awaited_closed(cli, &read_again));
        Settling {
            found,
            github: Read::Unasked,
        }
    }

    /// Read again each tracker `wanted` picks, saying each that does not
    /// answer, along with which trackers were read.
    fn read_where(
        &mut self,
        cli: &Cli,
        settles: &Gates,
        wanted: impl Fn(&KeptGates) -> bool,
    ) -> (Vec<Found>, Vec<bool>) {
        let mut found = Vec::new();
        let mut read_again = Vec::new();
        for (project, kept) in self.projects.iter().zip(&mut self.kept) {
            let wanted = wanted(kept);
            if wanted {
                kept.read(cli, project, settles);
                if let Err(failure) = &kept.read {
                    found.push(Found::TrackerUnread {
                        project: project.name.clone(),
                        failure: open_failure(failure),
                    });
                }
            }
            read_again.push(wanted);
        }
        (found, read_again)
    }

    /// Which project holds the bead that bead gate `gate`, kept for the
    /// project at `at`, awaits: its own where the two ids share a prefix, and
    /// otherwise the one project whose config states the bead's prefix.
    fn holder<'g>(&self, at: usize, gate: &'g BeadGate) -> Result<(usize, &'g str), Vec<Fault>> {
        let bead = gate.awaits.as_ref().map_err(Clone::clone)?;
        if prefix_of(&gate.id) == prefix_of(bead) {
            return Ok((at, bead));
        }
        let mut stating = self.projects.iter().enumerate().filter(|(_, project)| {
            project
                .prefix
                .as_deref()
                .is_some_and(|stated| states_the_prefix_of(stated, bead))
        });
        match (stating.next(), stating.next()) {
            (Some((holder, _)), None) => Ok((holder, bead)),
            _ => Err(vec![Fault::NoProjectHolds(bead.clone())]),
        }
    }

    /// Each bead gate kept that cannot name a project holding its bead.
    fn unsettleable_bead_gates(&self) -> Vec<Found> {
        let mut found = Vec::new();
        for (at, kept) in self.kept.iter().enumerate() {
            for gate in kept.read.iter().flat_map(|open| &open.beads) {
                if let Err(faults) = self.holder(at, gate) {
                    found.push(Found::Unsettleable {
                        project: self.projects[at].name.clone(),
                        gate: gate.id.clone(),
                        faults,
                    });
                }
            }
        }
        found
    }

    /// Close each bead gate kept whose bead has closed, reading the beads of
    /// those that appeared since their tracker was last read and of those
    /// whose bead's tracker `read_again` says was just read, one listing per
    /// tracker. A tracker that does not answer is read again on the next look.
    fn close_awaited_closed(&mut self, cli: &Cli, read_again: &[bool]) -> Vec<Found> {
        let mut asking: BTreeMap<usize, Vec<(usize, String, String)>> = BTreeMap::new();
        for at in 0..self.kept.len() {
            let mut kept_gates = BTreeSet::new();
            for gate in self.kept[at].read.iter().flat_map(|open| &open.beads) {
                let Ok((holder, bead)) = self.holder(at, gate) else {
                    continue;
                };
                let awaiting = (gate.id.clone(), bead.to_string());
                if read_again[holder] || !self.kept[at].asked.contains(&awaiting) {
                    asking
                        .entry(holder)
                        .or_default()
                        .push((at, gate.id.clone(), bead.to_string()));
                }
                kept_gates.insert(awaiting);
            }
            self.kept[at].asked = kept_gates;
        }
        let mut found = Vec::new();
        for (holder, waiting) in asking {
            let mut beads: Vec<String> = Vec::new();
            for (_, _, bead) in &waiting {
                if !beads.contains(bead) {
                    beads.push(bead.clone());
                }
            }
            let holder_name = self.projects[holder].name.clone();
            let listed = match cli.awaited(&self.projects[holder], &beads) {
                Ok(listed) => listed,
                Err(failure) => {
                    self.kept[holder].stale = true;
                    let failure = open_failure(&failure);
                    found.extend(waiting.into_iter().map(|(at, gate, bead)| {
                        Found::AwaitedUnread {
                            project: self.projects[at].name.clone(),
                            gate,
                            holder: holder_name.clone(),
                            bead,
                            failure: failure.clone(),
                        }
                    }));
                    continue;
                }
            };
            for (at, gate, bead) in waiting {
                let project = &self.projects[at];
                match listed.iter().find(|listed| listed.id == bead) {
                    None => found.push(Found::Unsettleable {
                        project: project.name.clone(),
                        gate,
                        faults: vec![Fault::NoSuchBead {
                            project: holder_name.clone(),
                            bead,
                        }],
                    }),
                    Some(listed) if listed.status == Status::Closed => {
                        let happened = format!("Bead {bead} in {holder_name} closed");
                        found.push(closed(cli, project, gate, happened));
                    }
                    Some(_) => {}
                }
            }
        }
        found
    }

    /// When the first timer gate kept runs out.
    pub fn next_deadline(&self) -> Option<DateTime<Utc>> {
        self.kept.iter().flat_map(KeptGates::deadlines).min()
    }

    /// Close each timer gate kept that has run out by `at`, asking GitHub
    /// nothing. A tracker holding one is read again first where it may have
    /// changed since, so a gate someone closed meanwhile is not closed again.
    /// A gate that would not close is left to the next look, which reads its
    /// tracker again for the write. A bead gate whose bead is in a tracker
    /// read again is settled too.
    pub fn timers_run_out(&mut self, cli: &Cli, settles: &Gates, at: DateTime<Utc>) -> Settling {
        let (mut found, read_again) = self.read_where(cli, settles, |kept| {
            kept.due() && kept.deadlines().any(|deadline| deadline <= at)
        });
        found.extend(self.close_run_out(cli, at));
        found.extend(self.close_awaited_closed(cli, &read_again));
        Settling {
            found,
            github: Read::Unasked,
        }
    }

    /// Settle the one pull request a delivery named against the gates kept,
    /// as a look settles each one it finds. Only a stale tracker is read
    /// again first, so a tracker the clock reads waits for the next look. A
    /// pull request no gate kept waits on asks nothing of GitHub, and one
    /// whose repository `settles` leaves to another `bdi gates` is passed
    /// over without a word. A bead gate whose bead is in a tracker read again
    /// is settled too. What the delivery `said` happened is told to each bead
    /// a gate still open on the pull request holds back.
    pub fn delivered(
        &mut self,
        cli: &Cli,
        gh: &dyn Runner,
        settles: &Gates,
        pull_request: PullRequest,
        said: Option<&Said>,
        told: &Told,
    ) -> Settling {
        if !settles.settles(gate::owner(&pull_request.repo)) {
            return Settling::default();
        }
        let (mut found, read_again) = self.read_where(cli, settles, |kept| kept.stale);
        found.extend(self.close_awaited_closed(cli, &read_again));
        let Some(awaited) = self.awaited(pull_request) else {
            return Settling {
                found,
                github: Read::Unasked,
            };
        };
        let settled =
            gates::settle_delivered(cli, gh, &self.projects, EVENTS, &awaited, said, told);
        let settled = findings(gh, awaited.pull_request, settled);
        found.extend(settled.found);
        Settling {
            found,
            github: settled.github,
        }
    }

    /// Settle the open pull requests whose head is the commit a delivery about
    /// its checks named, each as [`Kept::delivered`] settles one. A lookup the
    /// rate limit refuses is reported like any other and does not put off the
    /// asks after it, which the next one finds refused again.
    pub fn delivered_commit(
        &mut self,
        cli: &Cli,
        gh: &dyn Runner,
        settles: &Gates,
        commit: Commit,
        said: Option<&Said>,
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
            let each = self.delivered(cli, gh, settles, pull_request, said, told);
            settling.found.extend(each.found);
            settling.github = settling.github.max(each.github);
            if matches!(settling.found.last(), Some(Found::RateLimited { .. })) {
                break;
            }
        }
        settling
    }

    /// The gates kept that wait on `pull_request`, or nothing where none does.
    fn awaited(&self, pull_request: PullRequest) -> Option<Awaited> {
        let asked = &pull_request;
        let waiting: Vec<Waiting> = self
            .reads()
            .filter_map(|(project, read)| Some((project, read.as_ref().ok()?)))
            .flat_map(|(project, open)| {
                open.pull_requests.iter().filter_map(move |gate| {
                    let wait = gate.awaits.as_ref().ok()?;
                    wait.pull_request
                        .is(asked)
                        .then(|| waiting(&project.name, gate, wait))
                })
            })
            .collect();
        (!waiting.is_empty()).then_some(Awaited {
            pull_request,
            waiting,
        })
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
                    waiting(name, gate, wait),
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
        let settled = gates::settle_together(cli, gh, projects, EVENTS, repository, told);
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
/// limit. A gate whose wait failed is reported on every look while it stays
/// open.
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

/// `gate` in `project`, waiting on what `wait` names.
fn waiting(project: &str, gate: &PrGate, wait: &Wait) -> Waiting {
    Waiting {
        project: project.to_string(),
        until: wait.until,
        blocks: gate.blocks.clone(),
        made: gate.made,
    }
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
    use crate::collect::pr_events;
    use crate::collect::run::testing::FakeRunner;
    use crate::collect::webhook::{self, Delivery, Named, Secret};
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;
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
             (number:{number}){{state {}}}}}}}",
            pr_events::fields(EVENTS)
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
             {{state {fields}}} pr42:pullRequest(number:42){{state {fields}}}}}}}",
            fields = pr_events::fields(EVENTS)
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
                Found::Unseen {
                    pull_request: pr(7),
                    happening: "has passing checks",
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

    /// A delivery naming #`number` to a `bdi gates` that has read nothing yet.
    fn delivered_here(runner: &FakeRunner, settles: &Gates, number: u64) -> Settling {
        deliver(&mut arkham(), runner, settles, number)
    }

    fn deliver(kept: &mut Kept, runner: &FakeRunner, settles: &Gates, number: u64) -> Settling {
        kept.delivered(
            &Cli::new(runner),
            runner,
            settles,
            pr(number),
            None,
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
        arkham().delivered_commit(
            &Cli::new(runner),
            runner,
            settles,
            commit(),
            None,
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

    /// Written into every field of a delivery that holds someone's own words.
    const WRITTEN: &str = "words someone wrote";

    const REPOSITORY: &str =
        r#""repository":{"full_name":"example/ark","html_url":"https://github.com/example/ark"}"#;

    /// `body` as GitHub sends it for `event`, signed, and what it comes to.
    fn signed(event: &str, body: &str) -> Delivery {
        let mut mac = Hmac::<Sha256>::new_from_slice(b"swordfish").expect("any key");
        mac.update(body.as_bytes());
        let digest: String = mac
            .finalize()
            .into_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        match webhook::delivery(
            &Secret::new("swordfish").expect("a secret"),
            Some(event),
            Some(&format!("sha256={digest}")),
            body.as_bytes(),
        ) {
            webhook::Heard::Settle(delivery) => delivery,
            other => panic!("a {event} delivery that settles nothing: {other:?}"),
        }
    }

    fn deliver_signed(
        kept: &mut Kept,
        runner: &FakeRunner,
        delivery: &Delivery,
        told: &Told,
    ) -> Settling {
        let (cli, settles) = (Cli::new(runner), owners(&["example"]));
        let said = delivery.said.as_ref();
        match &delivery.named {
            Named::PullRequest(pull_request) => {
                kept.delivered(&cli, runner, &settles, pull_request.clone(), said, told)
            }
            Named::Commit(commit) => {
                kept.delivered_commit(&cli, runner, &settles, commit.clone(), said, told)
            }
        }
    }

    /// One delivery of each event the hook subscribes to, about open #7, as
    /// `(event, body, what it tells, what a line reporting it says)`. A
    /// submitted review and an inline comment tell nothing of their own, so
    /// [`a_review_or_an_inline_comment_tells_nothing_of_its_own`] has theirs.
    fn each_event() -> Vec<(&'static str, String, String, &'static str)> {
        vec![
            (
                "pull_request",
                format!(
                    r#"{{"action":"labeled","number":7,"label":{{"name":"{WRITTEN}"}},"pull_request":{{"number":7,"title":"{WRITTEN}","body":"{WRITTEN}","html_url":"https://github.com/example/ark/pull/7","head":{{"ref":"{WRITTEN}","sha":"{SHA}"}}}},{REPOSITORY},"sender":{{"login":"alice"}}}}"#
                ),
                "alice acted on pull request example/ark#7: pull_request labeled. \
                 https://github.com/example/ark/pull/7"
                    .to_string(),
                "was acted on",
            ),
            (
                "issue_comment",
                format!(
                    r#"{{"action":"created","issue":{{"number":7,"title":"{WRITTEN}","pull_request":{{"url":"https://api.github.com/repos/example/ark/pulls/7"}}}},"comment":{{"body":"{WRITTEN}","html_url":"https://github.com/example/ark/pull/7#issuecomment-41","user":{{"login":"dave"}}}},{REPOSITORY},"sender":{{"login":"dave"}}}}"#
                ),
                "dave commented on pull request example/ark#7. \
                 https://github.com/example/ark/pull/7#issuecomment-41"
                    .to_string(),
                "was commented on",
            ),
            (
                "check_suite",
                format!(
                    r#"{{"action":"completed","check_suite":{{"head_sha":"{SHA}","head_branch":"{WRITTEN}","conclusion":"failure","head_commit":{{"message":"{WRITTEN}"}}}},{REPOSITORY},"sender":{{"login":"erin"}}}}"#
                ),
                format!(
                    "The checks on {SHA}, the head of pull request example/ark#7, failed, so the \
                     gh:pr gate waiting on it stays open."
                ),
                "has failing checks",
            ),
            (
                "status",
                format!(
                    r#"{{"state":"success","sha":"{SHA}","context":"{WRITTEN}","description":"{WRITTEN}","target_url":"https://ci.invalid/{WRITTEN}","commit":{{"html_url":"https://github.com/example/ark/commit/{SHA}","commit":{{"message":"{WRITTEN}"}}}},{REPOSITORY},"sender":{{"login":"frank"}}}}"#
                ),
                format!(
                    "frank acted on pull request example/ark#7: status success. \
                     https://github.com/example/ark/commit/{SHA}"
                ),
                "was acted on",
            ),
            (
                "pull_request",
                format!(
                    r#"{{"action":"synchronize","number":7,"before":"0badc0de0badc0de0badc0de0badc0de0badc0de","after":"{SHA}","pull_request":{{"number":7,"title":"{WRITTEN}","html_url":"https://github.com/example/ark/pull/7","head":{{"ref":"{WRITTEN}","sha":"{SHA}"}}}},{REPOSITORY},"sender":{{"login":"grace"}}}}"#
                ),
                format!(
                    "Pull request example/ark#7 has a new head, 5eaf00d. \
                     https://github.com/example/ark/commit/{SHA}"
                ),
                "has a new head",
            ),
        ]
    }

    /// A pull_request_review delivery of bob approving #7 with a written body.
    fn review_submitted() -> String {
        format!(
            r#"{{"action":"submitted","review":{{"state":"approved","body":"{WRITTEN}","html_url":"https://github.com/example/ark/pull/7#pullrequestreview-21","user":{{"login":"bob"}}}},"pull_request":{{"number":7,"title":"{WRITTEN}"}},{REPOSITORY},"sender":{{"login":"bob"}}}}"#
        )
    }

    /// arkham's captured tracker, behind a GitHub where #7 is open, headed by
    /// [`SHA`], and has nothing new for the sweep to tell.
    fn seven_open() -> FakeRunner {
        let no_comments = include_str!("../../tests/fixtures/bd_1.3.0_comments_none.json");
        captured(FakeRunner::default(), "arkham")
            .with(&viewed(7), &answer(7, OPEN))
            .with(
                HEADED_BY_SHA,
                &format!(r#"[{{"number":7,"state":"open","head":{{"sha":"{SHA}"}}}}]"#),
            )
            .with(&read("arkham", "comments ark-2ud --json"), no_comments)
            .with(&read("arkham", "comments ark-45c --json"), no_comments)
    }

    /// The one of [`each_event`] that is `event`'s `action`.
    fn delivery_of(event: &str, action: &str) -> (&'static str, String, String) {
        let (event, body, text, _) = each_event()
            .into_iter()
            .find(|(each, body, ..)| {
                *each == event && body.contains(&format!(r#""action":"{action}""#))
            })
            .expect("each_event has one");
        (event, body, text)
    }

    fn comment_added(bead: &str, text: &str) -> String {
        format!("bd -C /nowhere/arkham comments add {bead} {text}")
    }

    fn comments_added(runner: &FakeRunner) -> Vec<String> {
        runner
            .calls()
            .into_iter()
            .map(|call| call.argv)
            .filter(|argv| argv.contains(" comments add "))
            .collect()
    }

    fn told_of_7(bead: &str, happening: &'static str, done: Done) -> Found {
        Found::Settling {
            pull_request: pr(7),
            happening,
            project: "arkham".to_string(),
            bead: bead.to_string(),
            done: Ok(done),
        }
    }

    /// ark-eb1 waits on #7 and holds back ark-2ud and ark-45c.
    #[test]
    fn a_delivery_of_each_event_tells_each_held_back_bead_once_and_quotes_nobody() {
        for (event, body, text, happening) in each_event() {
            let delivery = signed(event, &body);
            let mut runner = seven_open();
            for bead in ["ark-2ud", "ark-45c"] {
                runner = runner.with(&comment_added(bead, &text), "");
            }
            let (mut kept, told) = (arkham(), Told::default());

            let first = deliver_signed(&mut kept, &runner, &delivery, &told).found;
            let again = deliver_signed(&mut kept, &runner, &delivery, &told).found;

            assert_eq!(
                first,
                [
                    told_of_7("ark-2ud", happening, Done::Commented),
                    told_of_7("ark-45c", happening, Done::Commented),
                ],
                "{event}"
            );
            assert_eq!(again, [], "{event}");
            assert_eq!(
                comments_added(&runner),
                [
                    comment_added("ark-2ud", &text),
                    comment_added("ark-45c", &text)
                ],
                "{event}"
            );
            let quoting: Vec<String> = runner
                .calls()
                .into_iter()
                .map(|call| call.argv)
                .filter(|argv| argv.contains(WRITTEN))
                .collect();
            assert_eq!(quoting, Vec::<String>::new(), "{event}");
        }
    }

    #[test]
    fn a_comment_both_a_delivery_and_github_tell_of_is_told_once() {
        let (event, body, text) = delivery_of("issue_comment", "created");
        let commented = r#"{"state":"OPEN","isDraft":false,"mergeCommit":null,"comments":{"nodes":[{"url":"https://github.com/example/ark/pull/7#issuecomment-41","author":{"login":"dave"}}]}}"#;
        let mut runner = seven_open().with(&viewed(7), &answer(7, commented));
        for bead in ["ark-2ud", "ark-45c"] {
            runner = runner.with(&comment_added(bead, &text), "");
        }

        deliver_signed(
            &mut arkham(),
            &runner,
            &signed(event, &body),
            &Told::default(),
        );

        assert_eq!(
            comments_added(&runner),
            [
                comment_added("ark-2ud", &text),
                comment_added("ark-45c", &text)
            ]
        );
    }

    #[test]
    fn a_new_head_both_a_delivery_and_github_tell_of_is_told_once() {
        let (event, body, text) = delivery_of("pull_request", "synchronize");
        let moved = format!(
            r#"{{"state":"OPEN","isDraft":false,"mergeCommit":null,"headRef":{{"target":{{"oid":"{SHA}","url":"https://github.com/example/ark/commit/{SHA}"}}}}}}"#
        );
        let mut runner = seven_open().with(&viewed(7), &answer(7, &moved));
        for bead in ["ark-2ud", "ark-45c"] {
            runner = runner.with(&comment_added(bead, &text), "");
        }

        deliver_signed(
            &mut arkham(),
            &runner,
            &signed(event, &body),
            &Told::default(),
        );

        assert_eq!(
            comments_added(&runner),
            [
                comment_added("ark-2ud", &text),
                comment_added("ark-45c", &text)
            ]
        );
    }

    /// GitHub records an inline comment inside a review, and the look tells a
    /// review's lone inline comment in place of the review, and how many a
    /// larger review holds. Neither delivery says how many comments the
    /// review holds, so each leaves it to the look it prompts, which here has
    /// nothing to tell.
    #[test]
    fn a_review_or_an_inline_comment_tells_nothing_of_its_own() {
        let inline = format!(
            r#"{{"action":"created","comment":{{"body":"{WRITTEN}","html_url":"https://github.com/example/ark/pull/7#discussion_r31","user":{{"login":"carol"}}}},"pull_request":{{"number":7,"title":"{WRITTEN}"}},{REPOSITORY},"sender":{{"login":"carol"}}}}"#
        );
        let approving = review_submitted();
        let bodyless = approving
            .replace(r#""state":"approved""#, r#""state":"commented""#)
            .replace(&format!(r#""body":"{WRITTEN}","#), r#""body":null,"#);
        assert_ne!(bodyless, approving);

        for (event, body) in [
            ("pull_request_review_comment", inline),
            ("pull_request_review", bodyless),
            ("pull_request_review", approving),
        ] {
            let runner = seven_open();
            let found = deliver_signed(
                &mut arkham(),
                &runner,
                &signed(event, &body),
                &Told::default(),
            )
            .found;

            assert_eq!(found, [], "{event}");
            assert_eq!(asked_github(&runner), [viewed(7)], "{event} prompts a look");
            assert_eq!(comments_added(&runner), Vec::<String>::new(), "{event}");
        }
    }

    #[test]
    fn a_review_delivered_is_told_by_the_look_with_its_body_and_inline_comments_once() {
        let reviewed = r#"{"state":"OPEN","isDraft":false,"mergeCommit":null,"reviews":{"nodes":[{"url":"https://github.com/example/ark/pull/7#pullrequestreview-21","state":"APPROVED","body":"WRITTEN","author":{"login":"bob"},"comments":{"totalCount":2,"nodes":[{"url":"https://github.com/example/ark/pull/7#discussion_r31","replyTo":null}]}}]}}"#
            .replace("WRITTEN", WRITTEN);
        let text = "bob reviewed pull request example/ark#7: approved, with a written body and \
                    2 inline comments. https://github.com/example/ark/pull/7#pullrequestreview-21";
        let mut runner = seven_open().with(&viewed(7), &answer(7, &reviewed));
        for bead in ["ark-2ud", "ark-45c"] {
            runner = runner.with(&comment_added(bead, text), "");
        }
        let (mut kept, told) = (arkham(), Told::default());
        let delivery = signed("pull_request_review", &review_submitted());

        let first = deliver_signed(&mut kept, &runner, &delivery, &told).found;
        let again = deliver_signed(&mut kept, &runner, &delivery, &told).found;

        assert_eq!(
            first,
            [
                told_of_7("ark-2ud", "was reviewed", Done::Commented),
                told_of_7("ark-45c", "was reviewed", Done::Commented),
            ]
        );
        assert_eq!(again, []);
        assert_eq!(
            comments_added(&runner),
            [
                comment_added("ark-2ud", text),
                comment_added("ark-45c", text)
            ]
        );
    }

    /// ark-eb1 was made at 2026-10-06T08:24:49Z, so whoever made it could
    /// already see a comment made before then.
    #[test]
    fn a_delivery_of_a_comment_made_before_the_gate_tells_nothing() {
        let (event, body, _) = delivery_of("issue_comment", "created");
        let earlier = body.replace(
            r#""user":{"login":"dave"}"#,
            r#""created_at":"2026-10-01T00:00:00Z","user":{"login":"dave"}"#,
        );
        assert_ne!(earlier, body);
        let runner = seven_open();

        let found = deliver_signed(
            &mut arkham(),
            &runner,
            &signed(event, &earlier),
            &Told::default(),
        )
        .found;

        assert_eq!(found, []);
        assert_eq!(comments_added(&runner), Vec::<String>::new());
    }

    /// ark-0i5 waits on #42 and holds back ark-qca. The runner gives no
    /// comments to read, so asking ark-qca for its comments would fail the
    /// test.
    #[test]
    fn a_delivery_about_a_pull_request_that_merged_closes_its_gate_and_tells_nothing() {
        let body = review_submitted().replace(r#""number":7"#, r#""number":42"#);
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&viewed(42), &answer(42, MERGED))
            .with(&resolving_42("arkham"), "");

        let found = deliver_signed(
            &mut arkham(),
            &runner,
            &signed("pull_request_review", &body),
            &Told::default(),
        )
        .found;

        assert_eq!(found, [settling("arkham")]);
        assert_eq!(comments_added(&runner), Vec::<String>::new());
    }

    /// The runner panics on any call it was not given, so a write would fail
    /// the test.
    #[test]
    fn a_delivery_for_a_pull_request_no_gate_waits_on_asks_github_nothing_and_writes_nothing() {
        let runner = captured(FakeRunner::default(), "arkham");

        let Settling { found, github } = delivered_here(&runner, &Gates::default(), 9);

        assert_eq!(found, []);
        assert_eq!(asked_github(&runner), Vec::<String>::new());
        assert_eq!(github, Read::Unasked);
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

    const OPEN: &str = r#"{"state":"OPEN","isDraft":false,"mergeCommit":null}"#;

    fn bd_started(runner: &FakeRunner) -> usize {
        runner
            .calls()
            .iter()
            .filter(|call| call.argv.starts_with("bd "))
            .count()
    }

    /// arkham, covered by a producer, after the look a `bdi gates` takes as
    /// it starts.
    fn covered_and_looked_at(runner: &FakeRunner) -> Kept {
        let mut kept = arkham();
        let now = Instant::now();
        kept.heard(&covered(), now);
        look_at(&mut kept, runner, now);
        kept
    }

    /// Such as the check deliveries that follow a merge. #7 and #42 stay
    /// open, so nothing a delivery reads of them is new to a gate, and no
    /// gate waits on #9.
    #[test]
    fn a_burst_of_deliveries_on_trackers_unchanged_since_they_were_read_starts_no_bd() {
        let runner = quiet()
            .with(&viewed(7), &answer(7, OPEN))
            .with(&viewed(42), &answer(42, OPEN));
        let mut kept = covered_and_looked_at(&runner);
        let looked = bd_started(&runner);

        for number in [42, 7, 9, 42, 42, 7] {
            let settling = deliver(&mut kept, &runner, &owners(&["example"]), number);
            assert_eq!(settling.found, [], "#{number}");
            kept.settled(&settling);
        }

        assert_eq!(bd_started(&runner), looked);
        assert_eq!(
            asked_github(&runner).len(),
            1 + 5,
            "the look's one query, then one for each delivery a gate waits on"
        );
    }

    #[test]
    fn a_delivery_closes_the_gate_on_a_merged_pull_request_and_reads_no_other_tracker() {
        let runner = captured(FakeRunner::default(), "arkham")
            .with(
                &gate_list("dunwich"),
                include_str!("../../tests/fixtures/bd_1.3.0_gate_list_none.json"),
            )
            .with(&queried("ark"), QUERIED_OPEN)
            .with(&viewed(42), &answer(42, MERGED))
            .with(&resolving_42("arkham"), "");
        let mut kept = Kept::new(vec![project("arkham"), project("dunwich")]);
        let now = Instant::now();
        kept.heard(&covered(), now);
        kept.heard(&Heard::Covered("dunwich".to_string()), now);
        look_at(&mut kept, &runner, now);
        let looked = runner.calls().len();

        let found = deliver(&mut kept, &runner, &owners(&["example"]), 42).found;

        assert_eq!(found, [settling("arkham")]);
        let delivering: Vec<String> = runner.calls()[looked..]
            .iter()
            .map(|call| call.argv.clone())
            .collect();
        assert!(
            delivering
                .iter()
                .all(|argv| !argv.contains("/nowhere/dunwich")),
            "{delivering:?}"
        );
    }

    #[test]
    fn a_tracker_a_producer_says_changed_is_read_again_by_the_next_delivery_alone() {
        let runner = quiet().with(&viewed(42), &answer(42, OPEN));
        let mut kept = covered_and_looked_at(&runner);

        kept.heard(&Heard::Changed("arkham".to_string()), Instant::now());
        deliver(&mut kept, &runner, &owners(&["example"]), 42);
        deliver(&mut kept, &runner, &owners(&["example"]), 42);

        assert_eq!(gate_lists_read(&runner), 2);
    }

    #[test]
    fn a_tracker_whose_read_failed_is_read_again_by_the_next_delivery_and_its_gate_closes() {
        let failing = FakeRunner::default().failing(&gate_list("arkham"), unavailable("bd"));
        let mut kept = covered_and_looked_at(&failing);
        let runner = captured(FakeRunner::default(), "arkham")
            .with(&viewed(42), &answer(42, MERGED))
            .with(&resolving_42("arkham"), "");

        let found = deliver(&mut kept, &runner, &owners(&["example"]), 42).found;

        assert_eq!(found, [settling("arkham")]);
        assert_eq!(
            gate_lists_read(&runner),
            2,
            "read again for the delivery, then once more to settle"
        );
    }

    #[test]
    fn a_delivery_leaves_a_tracker_the_clock_reads_to_the_next_look() {
        let runner = quiet().with(&viewed(42), &answer(42, OPEN));
        let mut kept = arkham();
        look_at(&mut kept, &runner, Instant::now());

        deliver(&mut kept, &runner, &owners(&["example"]), 42);

        assert_eq!(gate_lists_read(&runner), 1);
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

        assert_eq!(
            found,
            [
                Found::TrackerUnread {
                    project: "arkham".to_string(),
                    failure: TrackerFailure::Unavailable,
                },
                no_number("dunwich"),
                settling("dunwich"),
            ],
            "settling #42 asks only dunwich, the tracker holding its gate"
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

    /// ark-689, a bead gate in arkham awaiting dun-ski in dunwich.
    const GATE_LIST_BEAD: &str = include_str!("../../tests/fixtures/bd_1.3.0_gate_list_bead.json");
    const GATE_LIST_NONE: &str = include_str!("../../tests/fixtures/bd_1.3.0_gate_list_none.json");
    const DUN_SKI_OPEN: &str =
        include_str!("../../tests/fixtures/bd_1.3.0_list_id_dun-ski_open.json");
    const DUN_SKI_CLOSED: &str =
        include_str!("../../tests/fixtures/bd_1.3.0_list_id_dun-ski_closed.json");
    const DUN_SKI_CLOSED_REASON: &str = "Bead dun-ski in dunwich closed";

    /// dunwich, whose config states its prefix, as the one that holds dun-ski.
    fn dunwich() -> Project {
        Project {
            prefix: Some("dun".to_string()),
            ..project("dunwich")
        }
    }

    fn arkham_and_dunwich() -> Kept {
        Kept::new(vec![project("arkham"), dunwich()])
    }

    fn awaited_in(project: &str, ids: &str) -> String {
        read(
            project,
            &format!("list --id {ids} --all --include-gates --include-infra --include-templates --limit 0 --json"),
        )
    }

    /// arkham holding ark-689, and dunwich no gate, with dun-ski as `dun_ski`.
    fn bead_gated(dun_ski: &str) -> FakeRunner {
        FakeRunner::default()
            .with(&gate_list("arkham"), GATE_LIST_BEAD)
            .with(&gate_list("dunwich"), GATE_LIST_NONE)
            .with(&awaited_in("dunwich", "dun-ski"), dun_ski)
            .with(&closing_689(), "")
    }

    fn closing_689() -> String {
        format!("bd -C /nowhere/arkham gate resolve ark-689 --reason {DUN_SKI_CLOSED_REASON}.")
    }

    fn dun_ski_closed() -> Found {
        Found::Ended {
            project: "arkham".to_string(),
            gate: "ark-689".to_string(),
            happened: DUN_SKI_CLOSED_REASON.to_string(),
            done: Ok(()),
        }
    }

    fn bead_look(kept: &mut Kept, runner: &FakeRunner) -> Settling {
        timer_look(kept, runner, "2026-10-17T14:00:00Z")
    }

    fn every_gate_list_read(runner: &FakeRunner) -> usize {
        runner
            .calls()
            .iter()
            .filter(|call| call.argv.ends_with(" gate list --limit 0 --json"))
            .count()
    }

    fn awaited_reads(runner: &FakeRunner) -> usize {
        runner
            .calls()
            .iter()
            .filter(|call| call.argv.contains(" list --id "))
            .count()
    }

    fn cover_both(kept: &mut Kept) {
        for project in ["arkham", "dunwich"] {
            kept.heard(&Heard::Covered(project.to_string()), Instant::now());
        }
    }

    #[test]
    fn a_bead_gate_whose_bead_in_another_tracker_has_closed_is_closed_on_the_first_look() {
        let runner = bead_gated(DUN_SKI_CLOSED);

        let settling = bead_look(&mut arkham_and_dunwich(), &runner);

        assert_eq!(
            settling,
            Settling {
                found: vec![dun_ski_closed()],
                github: Read::Unasked,
            }
        );
    }

    #[test]
    fn a_bead_gate_whose_bead_is_in_its_own_tracker_is_settled_from_that_tracker() {
        let own = GATE_LIST_BEAD.replace("dun-ski", "ark-8hm");
        let runner = FakeRunner::default()
            .with(&gate_list("arkham"), &own)
            .with(
                &awaited_in("arkham", "ark-8hm"),
                &DUN_SKI_CLOSED.replace("dun-ski", "ark-8hm"),
            )
            .with(
                "bd -C /nowhere/arkham gate resolve ark-689 --reason Bead ark-8hm in arkham closed.",
                "",
            );

        let settling = bead_look(&mut arkham(), &runner);

        assert!(
            matches!(&settling.found[..], [Found::Ended { done: Ok(()), .. }]),
            "{settling:?}"
        );
    }

    /// The bead is read once as the gate first appears, and again only once
    /// a producer says the tracker holding it changed.
    #[test]
    fn a_covered_bead_gate_is_closed_once_the_tracker_holding_its_bead_changes() {
        let open = bead_gated(DUN_SKI_OPEN);
        let mut kept = arkham_and_dunwich();
        cover_both(&mut kept);

        assert_eq!(bead_look(&mut kept, &open).found, []);
        assert_eq!(bead_look(&mut kept, &open).found, []);
        assert_eq!(kept.changed(&Cli::new(&open), &Gates::default()).found, []);
        assert_eq!(awaited_reads(&open), 1);
        assert_eq!(
            every_gate_list_read(&open),
            2,
            "one listing of each tracker"
        );

        let closed = bead_gated(DUN_SKI_CLOSED);
        kept.heard(&Heard::Changed("dunwich".to_string()), Instant::now());
        let settling = kept.changed(&Cli::new(&closed), &Gates::default());

        assert_eq!(settling.found, [dun_ski_closed()]);
        let calls: Vec<String> = closed.calls().into_iter().map(|call| call.argv).collect();
        assert_eq!(
            calls,
            [
                gate_list("dunwich"),
                awaited_in("dunwich", "dun-ski"),
                closing_689()
            ]
        );
    }

    /// A timer running out reads the tracker a producer said changed, so that
    /// read is news of the beads it holds too.
    #[test]
    fn a_timer_running_out_in_the_tracker_holding_a_gates_bead_settles_that_gate_too() {
        let timer_in_dunwich = GATE_LIST_TIMER.replace("ark-", "dun-");
        let closing_dun_06g =
            format!("bd -C /nowhere/dunwich gate resolve dun-06g --reason {RAN_OUT}.");
        let open = bead_gated(DUN_SKI_OPEN).with(&gate_list("dunwich"), &timer_in_dunwich);
        let mut kept = arkham_and_dunwich();
        cover_both(&mut kept);
        timer_look(&mut kept, &open, "2026-10-12T08:00:00Z");

        let closed = bead_gated(DUN_SKI_CLOSED)
            .with(&gate_list("dunwich"), &timer_in_dunwich)
            .with(&closing_dun_06g, "");
        kept.heard(&Heard::Changed("dunwich".to_string()), Instant::now());
        let settling = deadline_passed(&mut kept, &closed, "2026-10-12T09:32:28Z");

        assert_eq!(
            settling.found,
            [
                Found::Ended {
                    project: "dunwich".to_string(),
                    gate: "dun-06g".to_string(),
                    happened: RAN_OUT.to_string(),
                    done: Ok(()),
                },
                dun_ski_closed()
            ]
        );
    }

    /// A delivery reads the tracker a producer said changed, so that read is
    /// news of the beads it holds too.
    #[test]
    fn a_delivery_reading_the_tracker_holding_a_gates_bead_settles_that_gate_too() {
        let mut kept = arkham_and_dunwich();
        cover_both(&mut kept);
        bead_look(&mut kept, &bead_gated(DUN_SKI_OPEN));

        let closed = bead_gated(DUN_SKI_CLOSED);
        kept.heard(&Heard::Changed("dunwich".to_string()), Instant::now());
        let settling = deliver(&mut kept, &closed, &Gates::default(), 42);

        assert_eq!(settling.found, [dun_ski_closed()]);
    }

    #[test]
    fn a_bead_gate_set_to_await_another_bead_has_that_bead_read() {
        let mut kept = arkham_and_dunwich();
        cover_both(&mut kept);
        bead_look(&mut kept, &bead_gated(DUN_SKI_OPEN));

        let reason = "Bead dun-zzz in dunwich closed";
        let runner = FakeRunner::default()
            .with(
                &gate_list("arkham"),
                &GATE_LIST_BEAD.replace("dun-ski", "dun-zzz"),
            )
            .with(
                &awaited_in("dunwich", "dun-zzz"),
                &DUN_SKI_CLOSED.replace("dun-ski", "dun-zzz"),
            )
            .with(
                &format!("bd -C /nowhere/arkham gate resolve ark-689 --reason {reason}."),
                "",
            );
        kept.heard(&Heard::Changed("arkham".to_string()), Instant::now());
        let settling = kept.changed(&Cli::new(&runner), &Gates::default());

        assert_eq!(
            settling.found,
            [Found::Ended {
                project: "arkham".to_string(),
                gate: "ark-689".to_string(),
                happened: reason.to_string(),
                done: Ok(()),
            }]
        );
    }

    /// A change to the tracker holding the gate, rather than its bead, is not
    /// news of the bead.
    #[test]
    fn a_change_to_the_gates_own_tracker_alone_does_not_read_its_bead_again() {
        let runner = bead_gated(DUN_SKI_OPEN);
        let mut kept = arkham_and_dunwich();
        cover_both(&mut kept);
        bead_look(&mut kept, &runner);

        kept.heard(&Heard::Changed("arkham".to_string()), Instant::now());
        kept.changed(&Cli::new(&runner), &Gates::default());

        assert_eq!(awaited_reads(&runner), 1);
        assert_eq!(every_gate_list_read(&runner), 3);
    }

    #[test]
    fn a_bead_gate_whose_bead_no_one_project_states_the_prefix_of_cannot_be_settled() {
        let runner = FakeRunner::default().with(&gate_list("arkham"), GATE_LIST_BEAD);

        let settling = bead_look(&mut arkham(), &runner);

        assert_eq!(
            settling.found,
            [unsettleable(
                "arkham",
                "ark-689",
                Fault::NoProjectHolds("dun-ski".to_string())
            )]
        );
        assert_eq!(awaited_reads(&runner), 0);
    }

    #[test]
    fn a_bead_gate_whose_bead_its_tracker_does_not_hold_cannot_be_settled() {
        let runner = bead_gated("[]");

        let settling = bead_look(&mut arkham_and_dunwich(), &runner);

        assert_eq!(
            settling.found,
            [unsettleable(
                "arkham",
                "ark-689",
                Fault::NoSuchBead {
                    project: "dunwich".to_string(),
                    bead: "dun-ski".to_string(),
                }
            )]
        );
    }

    /// As a tracker whose gates could not be read is read again on the next
    /// look.
    #[test]
    fn a_bead_gate_whose_bead_could_not_be_read_is_said_and_read_again_on_the_next_look() {
        let runner =
            bead_gated(DUN_SKI_OPEN).failing(&awaited_in("dunwich", "dun-ski"), unavailable("bd"));
        let mut kept = arkham_and_dunwich();
        cover_both(&mut kept);

        let found = bead_look(&mut kept, &runner).found;
        bead_look(&mut kept, &runner);

        assert!(
            matches!(
                &found[..],
                [Found::AwaitedUnread { project, gate, holder, bead, .. }]
                    if project == "arkham" && gate == "ark-689" && holder == "dunwich" && bead == "dun-ski"
            ),
            "{found:?}"
        );
        assert_eq!(awaited_reads(&runner), 2);
    }

    /// The next look reads arkham again for the write, and asks once more.
    #[test]
    fn a_bead_gate_that_would_not_close_is_asked_after_again_on_the_next_look() {
        let runner = bead_gated(DUN_SKI_CLOSED).failing(&closing_689(), unavailable("bd"));
        let mut kept = arkham_and_dunwich();
        cover_both(&mut kept);

        let first = bead_look(&mut kept, &runner);
        kept.settled(&first);
        bead_look(&mut kept, &runner);

        assert_eq!(awaited_reads(&runner), 2);
    }

    #[test]
    fn the_beads_two_gates_await_in_one_tracker_are_read_in_one_listing() {
        let two = GATE_LIST_BEAD.replace(
            "\n]",
            &format!(
                ",\n{}\n]",
                GATE_LIST_BEAD
                    .trim()
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .trim()
                    .replace("ark-689", "ark-68a")
                    .replace("dun-ski", "dun-0tp")
            ),
        );
        let runner = FakeRunner::default()
            .with(&gate_list("arkham"), &two)
            .with(&gate_list("dunwich"), GATE_LIST_NONE)
            .with(&awaited_in("dunwich", "dun-ski,dun-0tp"), DUN_SKI_OPEN);

        let settling = bead_look(&mut arkham_and_dunwich(), &runner);

        assert_eq!(
            settling.found,
            [unsettleable(
                "arkham",
                "ark-68a",
                Fault::NoSuchBead {
                    project: "dunwich".to_string(),
                    bead: "dun-0tp".to_string(),
                }
            )]
        );
        assert_eq!(awaited_reads(&runner), 1);
    }
}
