//! What `bdi watch` holds of every project, and the seam what it holds comes
//! across.
//!
//! A change source finds out what each tracker holds, by whatever means it
//! has, and says so one project at a time. The watcher keeps what it is told
//! and never asks which source told it, so a source can be replaced without
//! anything that watches or delivers changing.

use std::collections::BTreeMap;
use std::io::{BufWriter, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::collect::changes::{self, Heard, Reported};
use crate::config::Reach;
use crate::model::snapshot::TrackerFailure;
use crate::model::types::Bead;

use super::watching::{self, Events, Interest, Refusal, Watch, ALIVE_LINE};

/// One bead as the watcher holds it: the bead its tracker's row was read
/// into, and the readiness `bdi --beads` gives it.
///
/// The readiness sits beside the row rather than in it, so nothing `bdi` adds
/// can be taken for a field of bd's or collide with one a later bd adds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    /// The bead as its project's read holds it, whose row is passed on as bd
    /// printed it, so a field `bdi` never reads still reaches a consumer.
    pub bead: Arc<Bead>,
    pub ready: bool,
    /// Every bead blocking this one, in its own project or another.
    pub blocked_by: Vec<String>,
    /// What bd itself says, from this one tracker. A consumer reading some
    /// projects through the watcher and the rest itself builds its trees
    /// from these, as it would from reads of its own.
    pub bd: BeadReadiness,
}

/// Whether one bead is ready, and the beads blocking it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct BeadReadiness {
    pub ready: bool,
    pub blocked_by: Vec<String>,
}

/// What a change source says of one project.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    pub project: String,
    pub said: Said,
    /// What the source read of the project's events journal for this
    /// answer, where it read it.
    pub journal: Option<Journal>,
}

/// One read of a project's events journal.
#[derive(Debug, Clone, PartialEq)]
pub enum Journal {
    /// bd's event records since the source last read the journal, oldest
    /// first, each as bd printed it.
    Read(Vec<serde_json::Value>),
    /// The journal would not answer.
    Unreadable(TrackerFailure),
}

/// How current a project is, and the beads it holds where they were read.
#[derive(Debug, Clone, PartialEq)]
pub enum Said {
    /// Every bead the tracker holds, wisps among them, keyed by id, as of
    /// `at`.
    Read {
        at: DateTime<Utc>,
        beads: BTreeMap<String, Held>,
    },
    /// Nothing has moved as of `at`, so the beads held already stand.
    Vouched { at: DateTime<Utc> },
    /// The last attempt to reach the tracker failed. The beads held already
    /// stand as the last known, and as old as when they were last vouched
    /// for.
    Unreachable(TrackerFailure),
}

/// Where the watcher's answers come from.
pub trait ChangeSource: Send {
    /// Block until the source says something of a project, and hand it over.
    /// Nothing once the source has stopped.
    fn next(&mut self) -> Option<Answer>;
}

/// One project as the watcher holds it.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Standing {
    /// Nothing until the source has read the project, which is not the same
    /// as a project holding no beads.
    pub beads: Option<BTreeMap<String, Held>>,
    /// The instant the source last vouched for the beads, by reading them or
    /// by saying nothing had moved.
    pub as_of: Option<DateTime<Utc>>,
    /// Why the last attempt to reach the tracker failed, where it did.
    pub unreachable: Option<TrackerFailure>,
    /// How the watcher's config reaches the tracker.
    pub reach: Reach,
    /// Whether the project's event records are sent.
    pub events: Events,
}

impl Standing {
    /// The lines that bring `interest` up to what this project holds, ending
    /// with how current that is. None until there is something to say: a
    /// project neither read nor found unreachable has a watch wait rather
    /// than be told it is empty.
    fn told(&self, project: &str, interest: &mut Interest) -> Option<Vec<String>> {
        let mut lines = match &self.beads {
            Some(beads) => interest.catch_up(project, beads),
            None if self.unreachable.is_some() => Vec::new(),
            None => return None,
        };
        lines.push(watching::freshness_line(
            project,
            &self.reach,
            self.as_of,
            self.unreachable.as_ref(),
            &self.events,
        ));
        Some(lines)
    }
}

/// Every project's beads as last read, how current each is, and who is
/// watching them.
#[derive(Debug, Default)]
pub struct Hold {
    projects: BTreeMap<String, Standing>,
    consumers: BTreeMap<u64, Consumer>,
    next_consumer: u64,
}

/// One connection watching beads.
#[derive(Debug)]
struct Consumer {
    /// Where its lines go, a batch at a time, to be written by the
    /// connection's own thread.
    telling: SyncSender<Vec<String>>,
    /// The connection, to hang up on where it falls behind.
    connection: UnixStream,
    interests: BTreeMap<String, Interest>,
}

impl Consumer {
    /// Send `lines`, or hang up where the connection is too far behind to
    /// take them. False where it is gone.
    fn tells(&self, lines: Vec<String>) -> bool {
        let sent = self.telling.try_send(lines).is_ok();
        if !sent {
            let _ = self.connection.shutdown(Shutdown::Both);
        }
        sent
    }
}

impl Hold {
    /// Holding nothing yet of each of `projects`, which are the projects a
    /// consumer may watch, each with how its tracker is reached and whether
    /// its config claims an events journal.
    pub fn reading<I: IntoIterator<Item = (String, Reach, bool)>>(projects: I) -> Self {
        Self {
            projects: projects
                .into_iter()
                .map(|(project, reach, journal)| {
                    let standing = Standing {
                        reach,
                        events: if journal { Events::Ok } else { Events::Off },
                        ..Standing::default()
                    };
                    (project, standing)
                })
                .collect(),
            ..Self::default()
        }
    }

    /// Take what a source said of one project, and tell everyone watching it.
    pub fn take(&mut self, answer: Answer) {
        let standing = self.projects.entry(answer.project.clone()).or_default();
        match answer.said {
            Said::Read { at, beads } => {
                standing.beads = Some(beads);
                standing.as_of = standing.as_of.max(Some(at));
                standing.unreachable = None;
            }
            Said::Vouched { at } => standing.as_of = standing.as_of.max(Some(at)),
            Said::Unreachable(failure) => standing.unreachable = Some(failure),
        }
        let records = match answer.journal {
            Some(Journal::Read(records)) => {
                standing.events = Events::Ok;
                records
            }
            Some(Journal::Unreadable(failure)) => {
                standing.events = Events::Unreadable(failure);
                Vec::new()
            }
            None => Vec::new(),
        };

        let project = answer.project;
        self.consumers.retain(|_, consumer| {
            let Some(interest) = consumer.interests.get_mut(&project) else {
                return true;
            };
            let mut lines: Vec<String> = records
                .iter()
                .filter(|record| {
                    record["issue_id"]
                        .as_str()
                        .is_some_and(|id| interest.covers(id))
                })
                .map(|record| watching::event_line(&project, record))
                .collect();
            standing.told(&project, interest).is_none_or(|told| {
                lines.extend(told);
                consumer.tells(lines)
            })
        });
    }

    /// A connection that will watch beads, sent its lines on `telling`.
    pub fn connect(&mut self, telling: SyncSender<Vec<String>>, connection: UnixStream) -> u64 {
        let id = self.next_consumer;
        self.next_consumer += 1;
        let consumer = Consumer {
            telling,
            connection,
            interests: BTreeMap::new(),
        };
        self.consumers.insert(id, consumer);
        id
    }

    /// Have `consumer` watch what `watch` names, and send it what it was not
    /// already sent of that.
    pub fn watch(&mut self, consumer: u64, watch: &Watch) -> Result<(), Refusal> {
        let projects: Vec<&String> = match watch {
            Watch::Everything => self.projects.keys().collect(),
            Watch::Project { project, .. } | Watch::Bead { project, .. } => {
                let (project, _) = self
                    .projects
                    .get_key_value(project)
                    .ok_or(Refusal::UnknownProject)?;
                vec![project]
            }
        };
        let Some(connection) = self.consumers.get_mut(&consumer) else {
            return Ok(());
        };

        let mut lines = Vec::new();
        for project in projects {
            let interest = connection.interests.entry(project.clone()).or_default();
            interest.widen(watch);
            lines.extend(
                self.projects[project]
                    .told(project, interest)
                    .into_iter()
                    .flatten(),
            );
        }
        if !lines.is_empty() && !connection.tells(lines) {
            self.consumers.remove(&consumer);
        }
        Ok(())
    }

    /// Stop telling a connection that has gone.
    pub fn forget(&mut self, consumer: u64) {
        self.consumers.remove(&consumer);
    }

    /// What the watcher holds of `project`, where a source has said anything
    /// of it.
    #[cfg(test)]
    pub fn of(&self, project: &str) -> Option<&Standing> {
        self.projects.get(project)
    }
}

fn locked(hold: &Mutex<Hold>) -> MutexGuard<'_, Hold> {
    hold.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Hold everything `source` says until it stops.
pub fn hold(source: &mut dyn ChangeSource, into: &Mutex<Hold>) {
    while let Some(answer) = source.next() {
        locked(into).take(answer);
        give_back_freed_memory();
    }
}

/// Return the memory the last read and the last consumers' lines freed.
///
/// glibc keeps freed memory for reuse rather than returning it, and a read
/// and a consumer's catch-up each free tens of megabytes among rows the
/// watcher holds for good. Without this the resident set ratchets up with
/// every consumer served, to several times what the watcher holds.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn give_back_freed_memory() {
    // SAFETY: `malloc_trim` takes no pointer and only releases memory that
    // nothing holds.
    unsafe { libc::malloc_trim(0) };
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn give_back_freed_memory() {}

/// How many answers a connection may fall behind by before it is hung up
/// on. A consumer that reconnects is sent the beads as they then stand, so
/// hanging up loses it nothing, where queueing for it without end would cost
/// the watcher memory for as long as it does not read.
const FALLING_BEHIND: usize = 64;

/// How often every connection is told the watcher is alive, whatever else
/// it has been sent.
pub const ALIVE_EVERY: Duration = Duration::from_secs(20);

/// Serve one connection to the watcher's socket until it goes away: its
/// watch lines go to `hold`, and every other line is a producer's.
pub fn serve(
    connection: UnixStream,
    hold: &Mutex<Hold>,
    reported: &Reported,
    changed: &Sender<Heard>,
) {
    let (Ok(writing), Ok(hanging_up)) = (connection.try_clone(), connection.try_clone()) else {
        return;
    };
    let (telling, told) = mpsc::sync_channel(FALLING_BEHIND);
    thread::spawn(move || tell(writing, &told, ALIVE_EVERY));
    let consumer = locked(hold).connect(telling.clone(), hanging_up);

    changes::each_line(connection, |message| {
        let answer = match message.map(|line| (line, watching::asked(line))) {
            Some((line, Some(asked))) => asked
                .and_then(|watch| locked(hold).watch(consumer, &watch))
                .err()
                .map(|refusal| watching::refused_line(line, refusal)),
            _ => {
                let Some(answer) = changes::answered(message, reported, changed) else {
                    return false;
                };
                Some(answer)
            }
        };
        answer.is_none_or(|answer| telling.send(vec![answer]).is_ok())
    });
    locked(hold).forget(consumer);
}

/// Write each batch of lines `told` hands over to `to`, and an alive line
/// every `alive_every`, until nothing is left to hand any over or `to` will
/// not take them.
fn tell(to: UnixStream, told: &Receiver<Vec<String>>, alive_every: Duration) {
    let mut to = BufWriter::new(to);
    let mut alive_at = Instant::now() + alive_every;
    loop {
        let now = Instant::now();
        let lines = if now >= alive_at {
            alive_at += alive_every;
            vec![ALIVE_LINE.to_string()]
        } else {
            match told.recv_timeout(alive_at - now) {
                Ok(lines) => lines,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        };
        let written = lines
            .iter()
            .try_for_each(|line| writeln!(to, "{line}"))
            .and_then(|()| to.flush());
        if written.is_err() {
            let _ = to.get_ref().shutdown(Shutdown::Both);
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::fixtures::now;

    fn a_bead() -> Held {
        let beads = crate::collect::bd::parse_shared_beads(
            r#"[{"id":"dun-1","title":"re-point the dish","status":"open"}]"#,
        )
        .expect("the row reads");
        Held {
            bead: Arc::clone(&beads[0]),
            ready: true,
            blocked_by: Vec::new(),
            bd: BeadReadiness {
                ready: true,
                blocked_by: Vec::new(),
            },
        }
    }

    fn read(project: &str, at: DateTime<Utc>, ids: &[&str]) -> Answer {
        said(
            project,
            Said::Read {
                at,
                beads: ids.iter().map(|id| (id.to_string(), a_bead())).collect(),
            },
        )
    }

    fn said(project: &str, said: Said) -> Answer {
        Answer {
            project: project.to_string(),
            said,
            journal: None,
        }
    }

    fn later(seconds: i64) -> DateTime<Utc> {
        now() + chrono::TimeDelta::seconds(seconds)
    }

    fn ids(standing: &Standing) -> Option<Vec<&str>> {
        standing
            .beads
            .as_ref()
            .map(|beads| beads.keys().map(String::as_str).collect())
    }

    #[test]
    fn a_project_nothing_has_been_said_of_is_not_held() {
        assert_eq!(Hold::default().of("dunwich"), None);
    }

    /// A read replaces every bead, so a bead the tracker no longer holds is
    /// no longer held either.
    #[test]
    fn a_read_replaces_what_was_held() {
        let mut hold = Hold::default();
        hold.take(read("dunwich", now(), &["dun-1", "dun-2"]));

        hold.take(read("dunwich", later(30), &["dun-1"]));

        let standing = hold.of("dunwich").expect("dunwich was read");
        assert_eq!(ids(standing), Some(vec!["dun-1"]));
        assert_eq!(standing.as_of, Some(later(30)));
    }

    #[test]
    fn a_project_vouched_for_keeps_its_beads_and_is_current_as_of_then() {
        let mut hold = Hold::default();
        hold.take(read("dunwich", now(), &["dun-1"]));

        hold.take(said("dunwich", Said::Vouched { at: later(10) }));

        let standing = hold.of("dunwich").expect("dunwich was read");
        assert_eq!(ids(standing), Some(vec!["dun-1"]));
        assert_eq!(standing.as_of, Some(later(10)));
    }

    /// A read made before the last word vouching for the project, handed
    /// over again because another project was read, brings the beads up to
    /// date and leaves the project as current as that word made it.
    #[test]
    fn a_project_is_never_less_current_than_the_last_word_vouching_for_it() {
        let mut hold = Hold::default();
        hold.take(read("dunwich", now(), &["dun-1"]));
        hold.take(said("dunwich", Said::Vouched { at: later(10) }));

        hold.take(read("dunwich", now(), &["dun-1", "dun-2"]));

        let standing = hold.of("dunwich").expect("dunwich was read");
        assert_eq!(ids(standing), Some(vec!["dun-1", "dun-2"]));
        assert_eq!(standing.as_of, Some(later(10)));
    }

    /// The beads stand as the last known, and how old they are is still the
    /// age of the read that brought them.
    #[test]
    fn a_tracker_that_cannot_be_reached_leaves_the_last_known_beads_and_their_age() {
        let mut hold = Hold::default();
        hold.take(read("dunwich", now(), &["dun-1"]));

        hold.take(said(
            "dunwich",
            Said::Unreachable(TrackerFailure::Unavailable),
        ));

        let standing = hold.of("dunwich").expect("dunwich was read");
        assert_eq!(ids(standing), Some(vec!["dun-1"]));
        assert_eq!(standing.as_of, Some(now()));
        assert_eq!(standing.unreachable, Some(TrackerFailure::Unavailable));
    }

    #[test]
    fn a_read_after_a_failure_says_the_tracker_answers_again() {
        let mut hold = Hold::default();
        hold.take(said(
            "dunwich",
            Said::Unreachable(TrackerFailure::Unavailable),
        ));

        hold.take(read("dunwich", later(30), &[]));

        let standing = hold.of("dunwich").expect("dunwich was read");
        assert_eq!(standing.unreachable, None);
        assert_eq!(ids(standing), Some(Vec::new()));
    }

    /// Unread is not the same as empty, so a watch can wait for the first
    /// read rather than be told there is nothing.
    #[test]
    fn a_project_only_vouched_for_has_no_beads_yet() {
        let mut hold = Hold::default();

        hold.take(said("dunwich", Said::Vouched { at: now() }));

        assert_eq!(hold.of("dunwich").and_then(ids), None);
    }

    #[test]
    fn what_is_said_of_one_project_leaves_another_alone() {
        let mut hold = Hold::default();
        hold.take(read("dunwich", now(), &["dun-1"]));

        hold.take(read("ferry", later(5), &["fer-1"]));

        let dunwich = hold.of("dunwich").expect("dunwich was read");
        assert_eq!(ids(dunwich), Some(vec!["dun-1"]));
        assert_eq!(dunwich.as_of, Some(now()));
    }

    /// Long enough for a line that is coming to have come, and short enough
    /// that one that is not is a failure rather than a hang.
    const GIVING_UP: Duration = Duration::from_secs(5);

    fn watching_dunwich() -> Watch {
        Watch::Project {
            project: "dunwich".to_string(),
            closed_too: false,
        }
    }

    fn dunwich() -> Reach {
        Reach {
            path: "/srv/work/dunwich".into(),
            environment_command: None,
        }
    }

    /// A hold reading dunwich, and one connection to it, whose lines arrive
    /// on what this hands back. Room for `behind` batches before it is hung
    /// up on.
    fn a_consumer(behind: usize) -> (Hold, u64, Receiver<Vec<String>>, UnixStream) {
        let mut hold = Hold::reading([("dunwich".to_string(), dunwich(), false)]);
        let Connection {
            consumer,
            told,
            theirs,
            ..
        } = connected(&mut hold, behind);
        (hold, consumer, told, theirs)
    }

    /// One connection to `hold`, and the ends of it the test holds.
    struct Connection {
        consumer: u64,
        told: Receiver<Vec<String>>,
        theirs: UnixStream,
        /// The watcher's end, kept open beside the one the hold has as a
        /// connection's reading thread keeps it, so that only a hang-up
        /// ends the connection.
        _reading: UnixStream,
    }

    fn connected(hold: &mut Hold, behind: usize) -> Connection {
        let (telling, told) = mpsc::sync_channel(behind);
        let (ours, theirs) = UnixStream::pair().expect("a connection");
        let reading = ours.try_clone().expect("the end is ours to share");
        let consumer = hold.connect(telling, ours);
        Connection {
            consumer,
            told,
            theirs,
            _reading: reading,
        }
    }

    /// The kind of each line in each batch sent so far.
    fn kinds(told: &Receiver<Vec<String>>) -> Vec<Vec<String>> {
        told.try_iter()
            .map(|batch| {
                batch
                    .iter()
                    .map(|line| {
                        let line: serde_json::Value = serde_json::from_str(line).expect("JSON");
                        line["line"].as_str().unwrap_or_default().to_string()
                    })
                    .collect()
            })
            .collect()
    }

    /// Unread is not empty: the watch is told nothing until the first read,
    /// and then the beads it starts from.
    #[test]
    fn a_watch_on_a_project_not_yet_read_waits_for_the_read() {
        let (mut hold, consumer, told, _theirs) = a_consumer(4);

        hold.watch(consumer, &watching_dunwich())
            .expect("dunwich is read");
        let before = kinds(&told);
        hold.take(read("dunwich", now(), &["dun-1"]));

        assert_eq!(before, Vec::<Vec<String>>::new());
        assert_eq!(kinds(&told), [["bead", "freshness"]]);
    }

    #[test]
    fn a_watch_is_told_how_the_watcher_reaches_the_tracker() {
        let (mut hold, consumer, told, _theirs) = a_consumer(4);
        hold.watch(consumer, &watching_dunwich())
            .expect("dunwich is read");

        hold.take(read("dunwich", now(), &[]));

        let batch = told.try_recv().expect("a batch");
        let line: serde_json::Value = serde_json::from_str(&batch[0]).expect("JSON");
        assert_eq!(
            line["reach"],
            serde_json::json!({ "path": "/srv/work/dunwich", "environment_command": null })
        );
    }

    /// The one thing a consumer of a tracker that never answered can be
    /// told, and the thing it most needs telling.
    #[test]
    fn a_tracker_that_cannot_be_reached_is_said_before_anything_is_read() {
        let (mut hold, consumer, told, _theirs) = a_consumer(4);
        hold.watch(consumer, &watching_dunwich())
            .expect("dunwich is read");

        hold.take(said("dunwich", Said::Unreachable(TrackerFailure::Auth)));

        let batch = told.try_recv().expect("a batch");
        let line: serde_json::Value = serde_json::from_str(&batch[0]).expect("JSON");
        assert_eq!(batch.len(), 1);
        assert_eq!(line["tracker"]["unreachable"]["reason"], "auth");
    }

    #[test]
    fn a_project_the_watcher_does_not_read_is_refused() {
        let (mut hold, consumer, _told, _theirs) = a_consumer(4);

        let refused = hold.watch(
            consumer,
            &Watch::Bead {
                project: "ferry".to_string(),
                id: "fer-1".to_string(),
            },
        );

        assert_eq!(refused, Err(Refusal::UnknownProject));
    }

    #[test]
    fn a_project_nobody_watches_is_sent_to_nobody() {
        let (mut hold, _consumer, told, _theirs) = a_consumer(4);

        hold.take(read("dunwich", now(), &["dun-1"]));

        assert_eq!(kinds(&told), Vec::<Vec<String>>::new());
    }

    #[test]
    fn a_watch_on_a_project_already_read_is_sent_its_beads_at_once() {
        let (mut hold, consumer, told, _theirs) = a_consumer(4);
        hold.take(read("dunwich", now(), &["dun-1"]));

        hold.watch(consumer, &watching_dunwich())
            .expect("dunwich is read");
        let at_once = kinds(&told);
        hold.take(read("dunwich", later(30), &["dun-1", "dun-2"]));

        assert_eq!(at_once, [["bead", "freshness"]]);
        assert_eq!(kinds(&told), [["bead", "freshness"]], "and after");
    }

    #[test]
    fn each_connection_is_told_for_itself() {
        let mut hold = Hold::reading([("dunwich".to_string(), dunwich(), false)]);
        let first = connected(&mut hold, 4);
        let second = connected(&mut hold, 4);
        hold.watch(first.consumer, &watching_dunwich())
            .expect("dunwich is read");
        hold.watch(second.consumer, &watching_dunwich())
            .expect("dunwich is read");

        hold.take(read("dunwich", now(), &["dun-1"]));

        assert_eq!(kinds(&first.told), [["bead", "freshness"]]);
        assert_eq!(kinds(&second.told), [["bead", "freshness"]]);
    }

    #[test]
    fn a_connection_forgotten_is_told_nothing_more() {
        let mut hold = Hold::reading([("dunwich".to_string(), dunwich(), false)]);
        let gone = connected(&mut hold, 4);
        hold.watch(gone.consumer, &watching_dunwich())
            .expect("dunwich is read");

        hold.forget(gone.consumer);
        hold.take(read("dunwich", now(), &["dun-1"]));

        assert_eq!(kinds(&gone.told), Vec::<Vec<String>>::new());
    }

    #[test]
    fn a_connection_that_keeps_up_stays_open() {
        let mut hold = Hold::reading([("dunwich".to_string(), dunwich(), false)]);
        let keeping_up = connected(&mut hold, 1);
        hold.watch(keeping_up.consumer, &watching_dunwich())
            .expect("dunwich is read");

        hold.take(read("dunwich", now(), &["dun-1"]));

        let mut rest = Vec::new();
        let mut theirs = &keeping_up.theirs;
        theirs
            .set_read_timeout(Some(Duration::from_millis(100)))
            .expect("a read is ours to give up on");
        let still_open = std::io::Read::read_to_end(&mut theirs, &mut rest);
        assert!(still_open.is_err(), "nothing hung up on it");
    }

    #[test]
    fn a_connection_that_falls_behind_is_hung_up_on() {
        let mut hold = Hold::reading([("dunwich".to_string(), dunwich(), false)]);
        let mut behind = connected(&mut hold, 1);
        hold.take(read("dunwich", now(), &["dun-1"]));
        hold.watch(behind.consumer, &watching_dunwich())
            .expect("dunwich is read");

        hold.take(said("dunwich", Said::Vouched { at: later(10) }));
        hold.take(said("dunwich", Said::Vouched { at: later(20) }));

        behind
            .theirs
            .set_read_timeout(Some(GIVING_UP))
            .expect("a read is ours to give up on");
        let mut rest = Vec::new();
        std::io::Read::read_to_end(&mut behind.theirs, &mut rest).expect("the connection ends");
        assert!(hold.consumers.is_empty());
    }

    // ---- bd's events -------------------------------------------------------

    fn record(seq: u64, op: &str, id: &str) -> serde_json::Value {
        serde_json::json!({ "seq": seq, "op": op, "issue_id": id })
    }

    /// A read of dunwich holding `ids`, with `records` read from its journal
    /// just before.
    fn journalled(at: DateTime<Utc>, ids: &[&str], records: Vec<serde_json::Value>) -> Answer {
        Answer {
            journal: Some(Journal::Read(records)),
            ..read("dunwich", at, ids)
        }
    }

    /// A hold reading dunwich, which claims a journal, and one connection
    /// watching what `watch` names after the first read.
    fn watching_a_journal(watch: &Watch) -> (Hold, Connection) {
        let mut hold = Hold::reading([("dunwich".to_string(), dunwich(), true)]);
        let connection = connected(&mut hold, 8);
        hold.take(journalled(now(), &["dun-1", "dun-2"], Vec::new()));
        hold.watch(connection.consumer, watch)
            .expect("dunwich is read");
        connection.told.try_iter().for_each(drop);
        (hold, connection)
    }

    /// Each line of each batch sent so far, as `<kind> <op>` for an event and
    /// the kind alone for anything else.
    fn heard(told: &Receiver<Vec<String>>) -> Vec<Vec<String>> {
        told.try_iter()
            .map(|batch| {
                batch
                    .iter()
                    .map(|line| {
                        let line: serde_json::Value = serde_json::from_str(line).expect("JSON");
                        match line["event"]["op"].as_str() {
                            Some(op) => format!("event {op}"),
                            None => line["line"].as_str().unwrap_or_default().to_string(),
                        }
                    })
                    .collect()
            })
            .collect()
    }

    /// A consumer acting at the freshness line has, for every event, a bead
    /// line at least as new.
    #[test]
    fn a_watch_is_sent_each_record_ahead_of_the_beads_and_the_freshness_that_close_the_answer() {
        let (mut hold, connection) = watching_a_journal(&watching_dunwich());
        let mut closed = read("dunwich", later(30), &["dun-1", "dun-2"]);
        if let Said::Read { beads, .. } = &mut closed.said {
            beads.get_mut("dun-1").expect("held").ready = false;
        }

        hold.take(Answer {
            journal: Some(Journal::Read(vec![
                record(3, "comment", "dun-1"),
                record(4, "dep_add", "dun-1"),
                record(5, "close", "dun-2"),
            ])),
            ..closed
        });

        assert_eq!(
            heard(&connection.told),
            [[
                "event comment",
                "event dep_add",
                "event close",
                "bead",
                "freshness"
            ]]
        );
    }

    #[test]
    fn a_watch_on_one_bead_is_sent_only_the_records_about_it() {
        let (mut hold, connection) = watching_a_journal(&Watch::Bead {
            project: "dunwich".to_string(),
            id: "dun-2".to_string(),
        });

        hold.take(journalled(
            later(30),
            &["dun-1", "dun-2"],
            vec![record(3, "comment", "dun-1"), record(4, "close", "dun-2")],
        ));

        assert_eq!(heard(&connection.told), [["event close", "freshness"]]);
    }

    /// The bead lines a watch starts from already hold the outcome.
    #[test]
    fn a_watch_is_not_sent_the_records_read_before_it_watched() {
        let mut hold = Hold::reading([("dunwich".to_string(), dunwich(), true)]);
        let connection = connected(&mut hold, 8);
        hold.take(journalled(
            now(),
            &["dun-1"],
            vec![record(3, "comment", "dun-1")],
        ));

        hold.watch(connection.consumer, &watching_dunwich())
            .expect("dunwich is read");

        assert_eq!(heard(&connection.told), [["bead", "freshness"]]);
    }

    #[test]
    fn a_project_that_claims_no_journal_is_said_to_have_no_events() {
        let (mut hold, consumer, told, _theirs) = a_consumer(4);
        hold.watch(consumer, &watching_dunwich())
            .expect("dunwich is read");

        hold.take(read("dunwich", now(), &["dun-1"]));

        let batch = told.try_recv().expect("a batch");
        let freshness: serde_json::Value =
            serde_json::from_str(batch.last().expect("a freshness line")).expect("JSON");
        assert_eq!(freshness["events"], "off");
    }

    #[test]
    fn a_journal_that_would_not_answer_is_said_to_be_unreadable_until_it_answers() {
        let (mut hold, connection) = watching_a_journal(&watching_dunwich());
        let events = |told: &Receiver<Vec<String>>| -> Vec<serde_json::Value> {
            told.try_iter()
                .map(|batch| {
                    let line: serde_json::Value =
                        serde_json::from_str(batch.last().expect("a line")).expect("JSON");
                    line["events"].clone()
                })
                .collect()
        };

        hold.take(Answer {
            journal: Some(Journal::Unreadable(TrackerFailure::Unavailable)),
            ..read("dunwich", later(30), &["dun-1", "dun-2"])
        });
        let unreadable = events(&connection.told);
        hold.take(journalled(later(60), &["dun-1", "dun-2"], Vec::new()));

        assert_eq!(
            unreadable,
            [serde_json::json!({ "unreadable": { "reason": "unavailable" } })]
        );
        assert_eq!(events(&connection.told), [serde_json::json!("ok")]);
    }

    #[test]
    fn every_connection_is_told_the_watcher_is_alive_on_the_interval() {
        let (ours, theirs) = UnixStream::pair().expect("a connection");
        let (telling, told) = mpsc::sync_channel(4);
        let writing = thread::spawn(move || tell(ours, &told, Duration::from_millis(50)));
        theirs
            .set_read_timeout(Some(GIVING_UP))
            .expect("a read is ours to give up on");
        let mut hearing = std::io::BufReader::new(theirs);
        let mut lines = Vec::new();

        telling
            .send(vec!["{}".to_string()])
            .expect("the writer takes it");
        for _ in 0..3 {
            let mut line = String::new();
            std::io::BufRead::read_line(&mut hearing, &mut line).expect("a line");
            lines.push(line.trim_end().to_string());
        }
        drop(telling);
        writing
            .join()
            .expect("the writer stops when nothing is left to tell");

        assert_eq!(lines, ["{}", ALIVE_LINE, ALIVE_LINE]);
    }
}
