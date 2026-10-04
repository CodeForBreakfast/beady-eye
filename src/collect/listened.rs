//! What `bdi listen` says of the projects it reads, asked on its socket.
//!
//! A run that finds a listener reads its trackers through it: it watches each
//! project it reads, takes the beads it is sent as a read of its own, and
//! hangs up when it is done. A view stays, and is told of each answer as it
//! arrives. A project the listener does not answer for is
//! read as it would be with no listener at all, so a listener that is down,
//! wedged or reading other projects costs a run what it would have saved and
//! never its answer. `docs/design.md`'s *A view reads through the listener*
//! has the design.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::collect::bd::bead_of;
use crate::collect::changes::{self, Heard};
use crate::collect::run::{FailureKind, RunFailure};
use crate::collect::tracker::{OpenFailure, Tracker, Trackers};
use crate::config::{Project, Reach};
use crate::model::snapshot::{TrackerFailure, TrackerState};
use crate::model::types::{Bead, Printed};

/// The version of the lines the listener sends a consumer, which every
/// freshness line carries. It moves only for a change a consumer cannot read
/// as it read the version before.
pub const PROTOCOL: u32 = 1;

/// How long a listener may leave a project unanswered before it is taken to
/// have wedged. It sends every connection a line every 20 seconds, so a
/// minute of silence is three missed.
const WEDGED_AFTER: Duration = Duration::from_secs(60);

/// Each configured project's tracker, read through the listener where it
/// answers and through `otherwise` where it does not.
pub struct Through<O> {
    at: Option<PathBuf>,
    listener: Arc<Shared>,
    otherwise: O,
}

/// What the listener has said, shared between whoever asks for a project and
/// the thread that hears it.
struct Shared {
    said: Mutex<Listener>,
    /// Woken at every line taken, and when a connection comes or goes.
    moved: Condvar,
}

/// What a run that stays does with what it hears.
struct Staying {
    /// Where each project the listener has answered for is said, as a
    /// producer's report is, and each project it can no longer answer for.
    telling: Sender<Heard>,
    /// How long after losing a listener, or failing to find one, to look
    /// for it again.
    again_every: Duration,
}

impl<O: Trackers> Through<O> {
    /// Watch each of `projects` on the listener at `at`, reading through
    /// `otherwise` every project it does not answer for. A listener that is
    /// not there leaves every project to `otherwise`.
    pub fn listener_at<'p>(
        at: Option<&Path>,
        projects: impl IntoIterator<Item = &'p str>,
        otherwise: O,
    ) -> Self {
        Self::hearing(at, projects, otherwise, WEDGED_AFTER, None)
    }

    /// As [`Self::listener_at`], for a run that stays. Each answer the
    /// listener closes is said on `telling`, as a producer's report is. A
    /// listener that goes has every project it answered for said there too,
    /// since each is read through `otherwise` from then on, and it is looked
    /// for again every `again_every`, as one that was not there is.
    pub fn staying<'p>(
        at: Option<&Path>,
        projects: impl IntoIterator<Item = &'p str>,
        otherwise: O,
        telling: Sender<Heard>,
        again_every: Duration,
    ) -> Self {
        let staying = Staying {
            telling,
            again_every,
        };
        Self::hearing(at, projects, otherwise, WEDGED_AFTER, Some(staying))
    }

    fn hearing<'p>(
        at: Option<&Path>,
        projects: impl IntoIterator<Item = &'p str>,
        otherwise: O,
        patience: Duration,
        staying: Option<Staying>,
    ) -> Self {
        let listener = Arc::new(Shared {
            said: Mutex::new(Listener {
                connecting: at.is_some(),
                patience,
                asked: projects.into_iter().map(str::to_string).collect(),
                ..Listener::default()
            }),
            moved: Condvar::new(),
        });
        if let Some(at) = at {
            let at = at.to_path_buf();
            let hearing = Arc::clone(&listener);
            thread::spawn(move || hear(&at, &hearing, patience, staying.as_ref()));
        }
        Self {
            at: at.map(Path::to_path_buf),
            listener,
            otherwise,
        }
    }

    /// Have the listener read again every project it is watched for, as a
    /// producer's report has it read a project.
    pub fn asks_again(&self) {
        let asked = self.listener.said().asked.clone();
        self.says(asked.into_iter().map(Heard::Changed).collect());
    }

    /// Say to the listener what a producer said to this run, so that what
    /// it holds, which this run reads, takes it in.
    pub fn passes_on(&self, heard: Heard) {
        self.says(vec![heard]);
    }

    /// Say each of `heard` to the listener, as a producer would. Nothing
    /// where there is no listener to say it to.
    fn says(&self, heard: Vec<Heard>) {
        let Some(at) = self.at.clone() else {
            return;
        };
        if self.listener.said().writing.is_none() {
            return;
        }
        // On a thread of its own, so that a listener slow to take the
        // connection holds up nobody.
        thread::spawn(move || {
            let Some((mut to, _)) = connected_to(&at, WEDGED_AFTER) else {
                return;
            };
            for heard in heard {
                if writeln!(to, "{}", heard.line()).is_err() {
                    return;
                }
            }
        });
    }
}

impl<O> Drop for Through<O> {
    fn drop(&mut self) {
        let mut said = self.listener.said();
        said.stopped = true;
        said.hang_up();
    }
}

impl<O: Trackers> Trackers for Through<O> {
    fn of(&self, project: &Project) -> Result<Box<dyn Tracker + '_>, OpenFailure> {
        let told = self.listener.answer_for(&project.name);
        match told.filter(|told| told.reach.as_ref() == Some(&project.reach())) {
            Some(told) => match &told.unreachable {
                Some(failure) => Err(reached_as(failure)),
                None => Ok(Box::new(Answered(told))),
            },
            None => self.otherwise.of(project),
        }
    }
}

/// What the listener has said on the connection to it.
#[derive(Default)]
struct Listener {
    /// Where watch lines go, for as long as there is a connection. Nothing
    /// once the listener has gone, wedged or said something this run cannot
    /// read.
    writing: Option<UnixStream>,
    /// Whether the first connection is still being made, which an ask waits
    /// out.
    connecting: bool,
    /// Whether the run has finished with the listener.
    stopped: bool,
    patience: Duration,
    /// The projects the run watches, each sent a watch line on every
    /// connection.
    asked: BTreeSet<String>,
    /// The projects the listener said it does not read.
    refused: BTreeSet<String>,
    /// Each project as of the last freshness line closing an answer for it.
    current: BTreeMap<String, Arc<Told>>,
    /// The answers that have begun and not yet closed.
    arriving: BTreeMap<String, Told>,
}

/// One project as the listener last said it stood.
#[derive(Debug, Clone, Default)]
struct Told {
    beads: BTreeMap<String, Listed>,
    as_of: Option<DateTime<Utc>>,
    unreachable: Option<TrackerFailure>,
    /// How the listener's config reaches the tracker, where it said.
    reach: Option<Reach>,
}

/// One bead as the listener sent it, with the readiness bd gives it.
#[derive(Debug, Clone)]
struct Listed {
    bead: Bead,
    bd: BeadReadiness,
}

#[derive(Debug, Clone, Deserialize)]
struct BeadReadiness {
    ready: bool,
    blocked_by: Vec<String>,
}

/// Every line the listener sends about a watch, with the kinds this run
/// does nothing with taken as one.
#[derive(Deserialize)]
#[serde(tag = "line", rename_all = "kebab-case")]
enum Line {
    Bead {
        project: String,
        bd: BeadReadiness,
        row: Printed,
    },
    Gone {
        project: String,
        id: String,
    },
    Freshness {
        project: String,
        as_of: Option<DateTime<Utc>>,
        tracker: TrackerState,
        protocol: Option<u32>,
        reach: Option<Reach>,
    },
    Refused {
        asked: String,
    },
    #[serde(other)]
    Other,
}

const WATCH_ALL: &str = "watch-all";

impl Shared {
    fn said(&self) -> MutexGuard<'_, Listener> {
        self.said.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// What the listener says of `project`, waiting until it has said it.
    /// Nothing where it will not say, which leaves the project to be read
    /// some other way.
    fn answer_for(&self, project: &str) -> Option<Arc<Told>> {
        let mut said = self.said();
        said.ask(project);
        let giving_up = Instant::now() + said.patience;
        loop {
            if let Some(told) = said.current.get(project) {
                return Some(Arc::clone(told));
            }
            let listening = said.connecting || said.writing.is_some();
            if said.refused.contains(project) || !listening {
                return None;
            }
            let Some(left) = giving_up
                .checked_duration_since(Instant::now())
                .filter(|left| !left.is_zero())
            else {
                said.hang_up();
                return None;
            };
            said = self
                .moved
                .wait_timeout(said, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

impl Listener {
    /// Watch `project` from now on, sending its watch line once.
    fn ask(&mut self, project: &str) {
        if self.asked.insert(project.to_string()) {
            self.watch(project);
        }
    }

    /// Send the watch line for `project`, where there is a connection to
    /// send it on.
    fn watch(&mut self, project: &str) {
        let sent = self
            .writing
            .as_mut()
            .map(|to| writeln!(to, "{WATCH_ALL} {project}").is_ok());
        if sent == Some(false) {
            self.hang_up();
        }
    }

    /// Take `to` as the connection to the listener, and watch every project
    /// on it. False where the run has finished with the listener.
    fn connected(&mut self, to: UnixStream) -> bool {
        self.connecting = false;
        if self.stopped {
            let _ = to.shutdown(Shutdown::Both);
            return false;
        }
        self.writing = Some(to);
        for project in self.asked.clone() {
            self.watch(&project);
        }
        true
    }

    /// Stop listening, so that the thread hearing the listener finds the
    /// connection closed.
    fn hang_up(&mut self) {
        self.connecting = false;
        if let Some(to) = self.writing.take() {
            let _ = to.shutdown(Shutdown::Both);
        }
    }

    /// The listener has gone. A run that stays forgets every answer, which
    /// it can no longer keep current, and is told which projects to read
    /// some other way.
    fn gone(&mut self, staying: bool) -> Vec<String> {
        self.hang_up();
        if !staying {
            return Vec::new();
        }
        self.arriving.clear();
        self.refused.clear();
        std::mem::take(&mut self.current).into_keys().collect()
    }

    /// Take one line into what has been said, and say which project's
    /// answer it closed. Nothing where it cannot be taken, which is a
    /// listener this run cannot read.
    fn take(&mut self, line: Line) -> Option<Option<String>> {
        match line {
            Line::Bead { project, bd, row } => {
                let bead = bead_of(row, false).ok()?;
                let listed = Listed { bead, bd };
                self.arriving(&project)
                    .beads
                    .insert(listed.bead.id.clone(), listed);
            }
            Line::Gone { project, id } => {
                self.arriving(&project).beads.remove(&id);
            }
            Line::Freshness {
                project,
                as_of,
                tracker,
                protocol,
                reach,
            } => {
                if protocol != Some(PROTOCOL) {
                    return None;
                }
                let mut told = self
                    .arriving
                    .remove(&project)
                    .or_else(|| self.current.get(&project).map(|told| (**told).clone()))
                    .unwrap_or_default();
                told.as_of = as_of;
                told.reach = reach;
                told.unreachable = match tracker {
                    TrackerState::Unreachable(failure) => Some(failure),
                    _ => None,
                };
                self.current.insert(project.clone(), Arc::new(told));
                return Some(Some(project));
            }
            Line::Refused { asked } => {
                let project = asked.strip_prefix(WATCH_ALL)?.trim();
                self.refused.insert(project.to_string());
            }
            Line::Other => {}
        }
        Some(None)
    }

    /// The answer for `project` that has begun, beginning it from what was
    /// last said of the project where it has not.
    fn arriving(&mut self, project: &str) -> &mut Told {
        let current = &self.current;
        self.arriving.entry(project.to_string()).or_insert_with(|| {
            current
                .get(project)
                .map(|told| (**told).clone())
                .unwrap_or_default()
        })
    }
}

/// Connect to the listener at `at` and hear it until it goes, then look for
/// it again where the run stays.
fn hear(at: &Path, shared: &Shared, patience: Duration, staying: Option<&Staying>) {
    loop {
        let connection = connected_to(at, patience);
        let heard = connection.is_some_and(|(to, from)| {
            let watching = shared.said().connected(to);
            shared.moved.notify_all();
            watching && {
                listen(from, shared, patience, staying);
                true
            }
        });
        let gone = shared.said().gone(staying.is_some());
        shared.moved.notify_all();
        let Some(staying) = staying else {
            return;
        };
        if heard {
            for project in gone {
                if staying.telling.send(Heard::Changed(project)).is_err() {
                    return;
                }
            }
        }
        thread::sleep(staying.again_every);
        if shared.said().stopped {
            return;
        }
    }
}

/// Take every line the listener sends on `from` until it goes, wedges or
/// says something this run cannot read.
fn listen(
    mut from: BufReader<UnixStream>,
    shared: &Shared,
    patience: Duration,
    staying: Option<&Staying>,
) {
    loop {
        let heard = next(&mut from, Instant::now() + patience);
        let taken = heard.and_then(|line| shared.said().take(line));
        shared.moved.notify_all();
        match (taken, staying) {
            (None, _) => return,
            (Some(Some(project)), Some(staying)) => {
                if staying.telling.send(Heard::Changed(project)).is_err() {
                    return;
                }
            }
            (Some(_), _) => {}
        }
    }
}

/// A connection to the listener at `at`, giving up on it where it takes no
/// connection, or any one write, within `patience`. Nothing where the socket
/// is one another user could have put there.
fn connected_to(at: &Path, patience: Duration) -> Option<(UnixStream, BufReader<UnixStream>)> {
    if !changes::only_this_user_holds(at) {
        return None;
    }
    let (connected, connection) = mpsc::channel();
    let at = at.to_path_buf();
    // ponytail: a connection never taken leaves its thread waiting until the
    // listener takes it or the run exits.
    thread::spawn(move || connected.send(UnixStream::connect(at)));
    let to = connection.recv_timeout(patience).ok()?.ok()?;
    to.set_write_timeout(Some(patience)).ok()?;
    let from = BufReader::new(to.try_clone().ok()?);
    Some((to, from))
}

/// The next line the listener finishes sending on `from` before
/// `giving_up`. Nothing where it finished none, closed the connection, or
/// sent something that is not a line about a watch.
fn next(from: &mut BufReader<UnixStream>, giving_up: Instant) -> Option<Line> {
    let mut line = Vec::new();
    loop {
        let left = giving_up
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())?;
        from.get_ref().set_read_timeout(Some(left)).ok()?;
        let arrived = from.fill_buf().ok()?;
        if arrived.is_empty() {
            return None;
        }
        match arrived.iter().position(|&byte| byte == b'\n') {
            Some(end) => {
                line.extend_from_slice(&arrived[..end]);
                from.consume(end + 1);
                return serde_json::from_slice(&line).ok();
            }
            None => {
                let taken = arrived.len();
                line.extend_from_slice(arrived);
                from.consume(taken);
            }
        }
    }
}

/// A tracker answering from what the listener said of it.
struct Answered(Arc<Told>);

impl Tracker for Answered {
    /// Nothing to compare against: the listener has done the comparing.
    fn fingerprint(&self) -> Option<Result<String, RunFailure>> {
        None
    }

    fn all(&self) -> Result<Vec<Bead>, RunFailure> {
        Ok(self
            .0
            .beads
            .values()
            .map(|listed| listed.bead.clone())
            .collect())
    }

    fn ready(&self) -> Result<BTreeSet<String>, RunFailure> {
        Ok(self
            .0
            .beads
            .iter()
            .filter(|(_, listed)| listed.bd.ready)
            .map(|(id, _)| id.clone())
            .collect())
    }

    fn blocked(&self) -> Result<BTreeMap<String, Vec<String>>, RunFailure> {
        Ok(self
            .0
            .beads
            .iter()
            .filter(|(_, listed)| !listed.bd.blocked_by.is_empty())
            .map(|(id, listed)| (id.clone(), listed.bd.blocked_by.clone()))
            .collect())
    }

    fn as_of(&self) -> Option<DateTime<Utc>> {
        self.0.as_of
    }
}

/// The failure a read of its own would have met, as the listener reported
/// it: each kind back to the one that is drawn as it.
fn reached_as(failure: &TrackerFailure) -> OpenFailure {
    let kind = match failure {
        TrackerFailure::NoEnvironment => return OpenFailure::NoEnvironment,
        TrackerFailure::NoCredential => return OpenFailure::NoCredential,
        TrackerFailure::Auth => FailureKind::Auth,
        TrackerFailure::Unavailable => FailureKind::Unavailable,
        TrackerFailure::NotInstalled => FailureKind::NotInstalled,
        TrackerFailure::Unstartable => FailureKind::Unstartable,
        TrackerFailure::InstalledUnstartable => FailureKind::InstalledUnstartable,
        TrackerFailure::Parse(_) => FailureKind::Parse,
        TrackerFailure::UnknownFlag => FailureKind::UnknownFlag,
    };
    let unreadable = match failure {
        TrackerFailure::Parse(unreadable) => Some(unreadable.clone()),
        _ => None,
    };
    OpenFailure::Refused(RunFailure {
        kind,
        program: "bd".to_string(),
        detail: "the listener could not read the tracker".to_string(),
        unreadable,
    })
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;
    use std::thread;

    use serde_json::json;

    use super::*;
    use crate::collect::tracker::testing::{Fake, Fakes};
    use crate::config::Config;

    /// Long enough that a listener which was going to answer has, and short
    /// enough that a test waiting in vain is not a hang.
    const A_MOMENT: Duration = Duration::from_secs(5);

    fn a_socket(named: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bdi-{named}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a directory to put the socket in");
        dir.join("listener.sock")
    }

    /// A listener that answers the first connection with `lines`, then holds
    /// it open saying nothing more until the run hangs up.
    fn a_listener_saying(named: &str, lines: Vec<String>) -> PathBuf {
        a_listener_at(a_socket(named), lines)
    }

    fn a_listener_at(at: PathBuf, lines: Vec<String>) -> PathBuf {
        let listening = UnixListener::bind(&at).expect("the socket is ours");
        thread::spawn(move || {
            let (mut connection, _) = listening.accept().expect("the run connects");
            for line in lines {
                writeln!(connection, "{line}").expect("the run reads");
            }
            let _ = connection.read_to_end(&mut Vec::new());
        });
        at
    }

    /// A bead bd calls `ready` and blocked by `blocked_by`, which the
    /// listener's own trees held back on a bead in another project.
    fn bead(project: &str, id: &str, ready: bool, blocked_by: &[&str]) -> String {
        json!({
            "line": "bead",
            "project": project,
            "ready": false,
            "blocked_by": ["fer-9"],
            "bd": { "ready": ready, "blocked_by": blocked_by },
            "row": { "id": id, "title": "re-point the dish", "status": "open",
                     "priority": 2, "issue_type": "task" },
        })
        .to_string()
    }

    fn fresh(project: &str, tracker: serde_json::Value) -> String {
        fresh_reaching(
            project,
            tracker,
            json!({ "path": format!("/srv/work/{project}"), "environment_command": null }),
        )
    }

    fn fresh_reaching(
        project: &str,
        tracker: serde_json::Value,
        reach: serde_json::Value,
    ) -> String {
        json!({
            "line": "freshness",
            "project": project,
            "as_of": "2026-08-30T11:59:30Z",
            "tracker": tracker,
            "events": "off",
            "protocol": 1,
            "reach": reach,
        })
        .to_string()
    }

    fn projects() -> Config {
        Config::from_toml(
            r#"
[[projects]]
name = "dunwich"
path = "/srv/work/dunwich"

[[projects]]
name = "ferry"
path = "/srv/work/ferry"
"#,
        )
        .expect("the config parses")
    }

    fn project<'c>(cfg: &'c Config, named: &str) -> &'c Project {
        cfg.projects
            .iter()
            .find(|project| project.name == named)
            .expect("a configured project")
    }

    /// Both projects' own trackers, which the listener should leave unasked
    /// wherever it answers.
    fn own_trackers() -> Fakes {
        Fakes::default()
            .with("dunwich", Fake::holding(Vec::new()))
            .with("ferry", Fake::holding(Vec::new()))
    }

    fn ids(tracker: &dyn Tracker) -> Vec<String> {
        tracker
            .all()
            .expect("the tracker answers")
            .into_iter()
            .map(|bead| bead.id)
            .collect()
    }

    fn through<'t>(at: &Path, own: &'t Fakes, patience: Duration) -> Through<&'t Fakes> {
        Through::hearing(Some(at), ["dunwich", "ferry"], own, patience, None)
    }

    /// The readiness taken is bd's, which is what a read of this run's own
    /// would have been told. The listener's own trees decided `bdi`'s, and
    /// this run's trees decide it again.
    #[test]
    fn a_project_the_listener_answers_for_is_read_from_its_answer_alone() {
        let at = a_listener_saying(
            "listened-answers",
            vec![
                bead("dunwich", "dun-1", true, &[]),
                bead("dunwich", "dun-2", false, &["dun-1"]),
                bead("dunwich", "dun-3", true, &[]),
                fresh("dunwich", json!("ok")),
            ],
        );
        let own = own_trackers();
        let cfg = projects();

        let through = through(&at, &own, A_MOMENT);

        let tracker = through
            .of(project(&cfg, "dunwich"))
            .unwrap_or_else(|_| panic!("the listener answers"));

        assert_eq!(ids(tracker.as_ref()), ["dun-1", "dun-2", "dun-3"]);
        assert_eq!(
            tracker.ready(),
            Ok(BTreeSet::from(["dun-1".to_string(), "dun-3".to_string()]))
        );
        assert_eq!(
            tracker.blocked(),
            Ok(BTreeMap::from([(
                "dun-2".to_string(),
                vec!["dun-1".to_string()]
            )]))
        );
        assert_eq!(
            tracker.as_of(),
            Some("2026-08-30T11:59:30Z".parse().expect("an instant"))
        );
        assert!(own.tracker("dunwich").asked().is_empty());
    }

    /// An answer is what stands at its freshness line, so a change that has
    /// begun arriving and not closed is not read as half of one.
    #[test]
    fn a_change_is_taken_whole_at_the_freshness_line_closing_it() {
        let at = a_listener_saying(
            "listened-whole",
            vec![
                bead("dunwich", "dun-1", true, &[]),
                fresh("dunwich", json!("ok")),
                bead("dunwich", "dun-2", true, &[]),
                json!({ "line": "gone", "project": "dunwich", "id": "dun-1" }).to_string(),
                fresh("ferry", json!("ok")),
            ],
        );
        let own = own_trackers();
        let cfg = projects();
        let through = through(&at, &own, A_MOMENT);
        let first = through.of(project(&cfg, "dunwich")).expect("answered");
        through.of(project(&cfg, "ferry")).expect("answered");
        let midway = through.of(project(&cfg, "dunwich")).expect("answered");

        assert_eq!(ids(first.as_ref()), ["dun-1"]);
        assert_eq!(ids(midway.as_ref()), ["dun-1"]);
    }

    fn staying<'t>(
        at: &Path,
        own: &'t Fakes,
        again_every: Duration,
    ) -> (Through<&'t Fakes>, mpsc::Receiver<Heard>) {
        let (telling, told) = mpsc::channel();
        let through = Through::hearing(
            Some(at),
            ["dunwich", "ferry"],
            own,
            A_MOMENT,
            Some(Staying {
                telling,
                again_every,
            }),
        );
        (through, told)
    }

    fn changed(project: &str) -> Heard {
        Heard::Changed(project.to_string())
    }

    #[test]
    fn a_run_that_stays_is_told_of_each_answer_as_it_closes() {
        let at = a_listener_saying(
            "listened-staying",
            vec![
                bead("dunwich", "dun-1", true, &[]),
                fresh("dunwich", json!("ok")),
                fresh("ferry", json!("ok")),
            ],
        );
        let own = own_trackers();
        let cfg = projects();
        let (through, told) = staying(&at, &own, A_MOMENT);

        assert_eq!(told.recv_timeout(A_MOMENT), Ok(changed("dunwich")));
        assert_eq!(told.recv_timeout(A_MOMENT), Ok(changed("ferry")));
        let read = through
            .of(project(&cfg, "dunwich"))
            .map(|tracker| ids(tracker.as_ref()));
        assert_eq!(read, Ok(vec!["dun-1".to_string()]));
        assert!(own.tracker("dunwich").asked().is_empty());
    }

    /// Its answers would go stale with nothing to say so, so it reads every
    /// project itself, and is told to at once rather than at its next poll.
    #[test]
    fn a_run_that_stays_reads_for_itself_once_its_listener_goes() {
        let at = a_socket("listened-staying-goes");
        let listening = UnixListener::bind(&at).expect("the socket is ours");
        thread::spawn(move || {
            let (mut connection, _) = listening.accept().expect("the run connects");
            writeln!(connection, "{}", bead("dunwich", "dun-1", true, &[])).expect("sent");
            writeln!(connection, "{}", fresh("dunwich", json!("ok"))).expect("sent");
        });
        let own = own_trackers();
        let cfg = projects();
        let (through, told) = staying(&at, &own, 2 * A_MOMENT);

        let heard: Vec<Heard> = (0..2)
            .filter_map(|_| told.recv_timeout(A_MOMENT).ok())
            .collect();
        let read = through
            .of(project(&cfg, "dunwich"))
            .map(|tracker| ids(tracker.as_ref()));

        assert_eq!(heard, [changed("dunwich"), changed("dunwich")]);
        assert_eq!(read, Ok(Vec::new()));
        assert!(!own.tracker("dunwich").asked().is_empty());
    }

    /// Nothing moving is not a listener gone: the alive line it sends every
    /// connection keeps a watch open for as long as it is sent.
    #[test]
    fn a_run_that_stays_keeps_a_quiet_listener_that_is_alive() {
        let patience = Duration::from_millis(100);
        let at = a_socket("listened-staying-quiet");
        let listening = UnixListener::bind(&at).expect("the socket is ours");
        thread::spawn(move || {
            let (mut connection, _) = listening.accept().expect("the run connects");
            writeln!(connection, "{}", bead("dunwich", "dun-1", true, &[])).expect("sent");
            writeln!(connection, "{}", fresh("dunwich", json!("ok"))).expect("sent");
            while writeln!(connection, r#"{{"line":"alive"}}"#).is_ok() {
                thread::sleep(patience / 4);
            }
        });
        let own = own_trackers();
        let cfg = projects();
        let (telling, told) = mpsc::channel();
        let staying = Staying {
            telling,
            again_every: A_MOMENT,
        };
        let through = Through::hearing(Some(&at), ["dunwich"], &own, patience, Some(staying));

        let answered = told.recv_timeout(A_MOMENT);
        thread::sleep(5 * patience);
        let read = through
            .of(project(&cfg, "dunwich"))
            .map(|tracker| ids(tracker.as_ref()));

        assert_eq!(answered, Ok(changed("dunwich")));
        assert_eq!(told.try_recv().ok(), None, "nothing went");
        assert_eq!(read, Ok(vec!["dun-1".to_string()]));
    }

    #[test]
    fn a_run_that_stays_finds_a_listener_started_after_it() {
        let at = a_socket("listened-staying-late");
        let own = own_trackers();
        let cfg = projects();
        let (through, told) = staying(&at, &own, Duration::from_millis(20));
        let before = through
            .of(project(&cfg, "dunwich"))
            .map(|tracker| ids(tracker.as_ref()));

        a_listener_at(
            at.clone(),
            vec![
                bead("dunwich", "dun-1", true, &[]),
                fresh("dunwich", json!("ok")),
            ],
        );

        assert_eq!(before, Ok(Vec::new()));
        assert_eq!(told.recv_timeout(A_MOMENT), Ok(changed("dunwich")));
        let after = through
            .of(project(&cfg, "dunwich"))
            .map(|tracker| ids(tracker.as_ref()));
        assert_eq!(after, Ok(vec!["dun-1".to_string()]));
    }

    #[test]
    fn asking_again_names_each_project_to_the_listener_as_a_producer_does() {
        let at = a_socket("listened-asks-again");
        let listening = UnixListener::bind(&at).expect("the socket is ours");
        let (hearing, heard) = mpsc::channel();
        thread::spawn(move || {
            let (watching, _) = listening.accept().expect("the run watches");
            let mut answering = watching.try_clone().expect("ours to write");
            writeln!(answering, "{}", fresh("dunwich", json!("ok"))).expect("sent");
            let (asking, _) = listening.accept().expect("the run asks again");
            let _ = hearing.send(
                BufReader::new(asking)
                    .lines()
                    .map_while(Result::ok)
                    .collect::<Vec<String>>(),
            );
            drop(watching);
        });
        let own = own_trackers();
        let cfg = projects();
        let through = through(&at, &own, A_MOMENT);
        through.of(project(&cfg, "dunwich")).expect("answered");

        through.asks_again();

        assert_eq!(
            heard.recv_timeout(A_MOMENT),
            Ok(vec!["dunwich".to_string(), "ferry".to_string()])
        );
    }

    #[test]
    fn a_report_passed_on_is_said_to_the_listener_as_its_producer_said_it() {
        let at = a_socket("listened-passes-on");
        let listening = UnixListener::bind(&at).expect("the socket is ours");
        let (hearing, heard) = mpsc::channel();
        thread::spawn(move || {
            let (watching, _) = listening.accept().expect("the run watches");
            let mut answering = watching.try_clone().expect("ours to write");
            writeln!(answering, "{}", fresh("dunwich", json!("ok"))).expect("sent");
            let (passing_on, _) = listening.accept().expect("the run passes a report on");
            let _ = hearing.send(
                BufReader::new(passing_on)
                    .lines()
                    .map_while(Result::ok)
                    .collect::<Vec<String>>(),
            );
            drop(watching);
        });
        let own = own_trackers();
        let cfg = projects();
        let through = through(&at, &own, A_MOMENT);
        through.of(project(&cfg, "dunwich")).expect("answered");

        through.passes_on(Heard::Covered("ferry".to_string()));

        assert_eq!(
            heard.recv_timeout(A_MOMENT),
            Ok(vec!["covered ferry".to_string()])
        );
    }

    /// A listener started on another config, or on this one before it
    /// changed, can read another tracker under the same name.
    #[test]
    fn a_project_the_listener_reaches_another_way_is_read_for_itself() {
        let at = a_listener_saying(
            "listened-elsewhere",
            vec![
                bead("ferry", "fer-1", true, &[]),
                fresh_reaching(
                    "ferry",
                    json!("ok"),
                    json!({ "path": "/srv/work/ferry", "environment_command": ["direnv", "exec", "."] }),
                ),
                fresh_reaching(
                    "dunwich",
                    json!("ok"),
                    json!({ "path": "/srv/old/dunwich", "environment_command": null }),
                ),
            ],
        );
        let own = own_trackers();
        let cfg = projects();
        let through = through(&at, &own, A_MOMENT);

        for named in ["ferry", "dunwich"] {
            let read = through
                .of(project(&cfg, named))
                .map(|tracker| ids(tracker.as_ref()));
            assert_eq!(read, Ok(Vec::new()), "{named} is read for itself");
            assert!(!own.tracker(named).asked().is_empty());
        }
    }

    #[test]
    fn a_listener_that_does_not_say_how_it_reaches_a_tracker_is_read_around() {
        let at = a_listener_saying(
            "listened-unsaid",
            vec![json!({
                "line": "freshness", "project": "dunwich", "as_of": null,
                "tracker": "ok", "events": "off", "protocol": 1,
            })
            .to_string()],
        );
        let own = own_trackers();
        let cfg = projects();
        let through = through(&at, &own, A_MOMENT);

        through
            .of(project(&cfg, "dunwich"))
            .map(|tracker| ids(tracker.as_ref()))
            .expect("read for itself");

        assert!(!own.tracker("dunwich").asked().is_empty());
    }

    /// Another user could have bound the name, and answered with whatever
    /// beads they liked.
    #[test]
    fn a_listener_at_a_name_another_user_may_take_is_left_unasked() {
        let around = a_socket("listened-shared")
            .parent()
            .expect("the socket is in a directory")
            .to_path_buf();
        std::fs::set_permissions(&around, std::fs::Permissions::from_mode(0o700))
            .expect("nobody else may enter it");
        let shared = around.join("shared");
        std::fs::create_dir(&shared).expect("a directory to open up");
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o777))
            .expect("the directory is ours to open up");
        let at = a_listener_at(
            shared.join("listener.sock"),
            vec![
                bead("dunwich", "dun-1", true, &[]),
                fresh("dunwich", json!("ok")),
            ],
        );
        let own = own_trackers();
        let cfg = projects();
        let through = through(&at, &own, A_MOMENT);

        let read = through
            .of(project(&cfg, "dunwich"))
            .map(|tracker| ids(tracker.as_ref()));

        assert_eq!(read, Ok(Vec::new()), "read for itself");
        assert!(!own.tracker("dunwich").asked().is_empty());
    }

    #[test]
    fn a_project_the_listener_does_not_read_is_read_for_itself() {
        let at = a_listener_saying(
            "listened-refuses",
            vec![
                json!({ "line": "refused", "asked": "watch-all ferry", "reason": "unknown-project" })
                    .to_string(),
                fresh("dunwich", json!("ok")),
            ],
        );
        let own = own_trackers();
        let cfg = projects();
        let through = through(&at, &own, A_MOMENT);
        let asked = Instant::now();

        through
            .of(project(&cfg, "ferry"))
            .map(|tracker| ids(tracker.as_ref()))
            .expect("read for itself");
        let left_at = asked.elapsed();
        through.of(project(&cfg, "dunwich")).expect("answered");

        assert!(left_at < A_MOMENT, "left on the refusal, not the patience");
        assert!(!own.tracker("ferry").asked().is_empty());
        assert!(own.tracker("dunwich").asked().is_empty());
    }

    /// Each project is watched once, however often a collection opens it.
    #[test]
    fn each_project_is_watched_once() {
        let at = a_socket("listened-watched-once");
        let listening = UnixListener::bind(&at).expect("the socket is ours");
        let (hearing, heard) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let (connection, _) = listening.accept().expect("the run connects");
            let mut answering = connection.try_clone().expect("ours to write");
            for project in ["dunwich", "ferry"] {
                writeln!(answering, "{}", fresh(project, json!("ok"))).expect("sent");
            }
            let _ = hearing.send(
                BufReader::new(connection)
                    .lines()
                    .map_while(Result::ok)
                    .collect::<Vec<String>>(),
            );
        });
        let own = own_trackers();
        let cfg = projects();
        let through = through(&at, &own, A_MOMENT);

        for named in ["dunwich", "ferry", "dunwich"] {
            through.of(project(&cfg, named)).expect("answered");
        }
        drop(through);

        assert_eq!(
            heard.recv_timeout(A_MOMENT),
            Ok(vec![
                "watch-all dunwich".to_string(),
                "watch-all ferry".to_string()
            ])
        );
    }

    /// A listener that cannot reach a tracker says why, and the project is
    /// read as having failed that way rather than read again here.
    #[test]
    fn a_tracker_the_listener_cannot_reach_fails_as_it_failed_there() {
        let at = a_listener_saying(
            "listened-unreachable",
            vec![fresh(
                "dunwich",
                json!({ "unreachable": { "reason": "auth" } }),
            )],
        );
        let own = own_trackers();
        let cfg = projects();

        let through = through(&at, &own, A_MOMENT);

        let failed = through.of(project(&cfg, "dunwich")).map(|_| ());

        assert!(
            matches!(&failed, Err(OpenFailure::Refused(failure)) if failure.kind == FailureKind::Auth),
            "{failed:?}"
        );
        assert!(own.tracker("dunwich").asked().is_empty());
    }

    /// What a run says of a tracker the listener could not reach is what the
    /// listener said, whichever way it failed.
    #[test]
    fn every_failure_the_listener_reports_is_the_failure_the_run_reports() {
        let every = [
            TrackerFailure::NoEnvironment,
            TrackerFailure::NoCredential,
            TrackerFailure::Auth,
            TrackerFailure::Unavailable,
            TrackerFailure::NotInstalled,
            TrackerFailure::Unstartable,
            TrackerFailure::InstalledUnstartable,
            TrackerFailure::Parse(crate::model::types::Unreadable {
                read: "list".to_string(),
                cause: "invalid type: null, expected a string at line 1 column 25".to_string(),
            }),
            TrackerFailure::UnknownFlag,
        ];
        let cfg = Config::from_toml(
            r#"
[[projects]]
name = "dunwich"
path = "/srv/work/dunwich"
"#,
        )
        .expect("the config parses");
        let own = own_trackers();

        for failure in every {
            let at = a_listener_saying(
                "listened-every-failure",
                vec![fresh(
                    "dunwich",
                    json!({ "unreachable": serde_json::to_value(&failure).expect("serialises") }),
                )],
            );
            let snapshot = crate::app::run(
                &cfg,
                &crate::collect::agents::Unasked,
                &Through::listener_at(Some(&at), ["dunwich"], &own),
                crate::model::snapshot::Filter::All,
                "2026-08-30T12:00:00Z".parse().expect("an instant"),
            );

            let reported: Vec<&TrackerFailure> = snapshot
                .failed_projects
                .iter()
                .map(|failed| &failed.tracker)
                .collect();
            assert_eq!(reported, [&failure]);
        }
        assert!(own.tracker("dunwich").asked().is_empty());
    }

    /// The listener reads every project, so its `ready` already counts a
    /// blocker in a project a narrower run leaves out. The narrower run
    /// counts it too, from the project it did not read.
    #[test]
    fn a_run_narrower_than_the_listener_says_what_its_own_read_says() {
        let row = |id: &str, status: &str, blocks_on: &[&str]| {
            let dependencies: Vec<_> = blocks_on
                .iter()
                .map(|on| json!({ "depends_on_id": on, "type": "blocks" }))
                .collect();
            json!({ "id": id, "title": "re-point the dish", "status": status,
                    "priority": 2, "issue_type": "task", "dependencies": dependencies })
        };
        let held = |row: &serde_json::Value| {
            bead_of(row.as_object().expect("a row is an object").clone(), false)
                .expect("the row reads")
        };
        let cfg = projects()
            .scoped_to(&["dunwich".to_string()])
            .expect("dunwich is configured");
        let readiness_of = |trackers: &dyn Trackers| {
            let snapshot = crate::app::run(
                &cfg,
                &crate::collect::agents::Unasked,
                trackers,
                crate::model::snapshot::Filter::All,
                "2026-08-30T12:00:00Z".parse().expect("an instant"),
            );
            snapshot
                .collected
                .iter()
                .flat_map(|tree| &tree.beads)
                .find(|node| node.id == "dun-1")
                .map(|node| (node.ready, node.blocked_by.clone()))
        };
        let waiting = row("dun-1", "open", &["fer-1"]);

        for status in ["open", "closed"] {
            let blocker = row("fer-1", status, &[]);
            let unfinished = status == "open";
            let own = Fakes::default()
                .with(
                    "dunwich",
                    Fake::holding(vec![held(&waiting)]).ready(["dun-1"]),
                )
                .with("ferry", Fake::holding(vec![held(&blocker)]));
            let listened_blocked_by: &[&str] = if unfinished { &["fer-1"] } else { &[] };
            let at = a_listener_saying(
                &format!("listened-narrower-{status}"),
                vec![
                    json!({ "line": "bead", "project": "dunwich", "ready": !unfinished,
                            "blocked_by": listened_blocked_by,
                            "bd": { "ready": true, "blocked_by": [] }, "row": waiting })
                    .to_string(),
                    fresh("dunwich", json!("ok")),
                    json!({ "line": "bead", "project": "ferry", "ready": unfinished,
                            "blocked_by": [], "bd": { "ready": unfinished, "blocked_by": [] },
                            "row": blocker })
                    .to_string(),
                    fresh("ferry", json!("ok")),
                ],
            );
            let unasked = own_trackers();

            assert_eq!(
                readiness_of(&own),
                Some((false, vec!["fer-1".to_string()])),
                "a blocker in a project the run did not read may be unfinished"
            );
            assert_eq!(
                readiness_of(&Through::listener_at(Some(&at), ["dunwich"], &unasked)),
                readiness_of(&own),
                "a blocker that is {status}"
            );
            assert!(unasked.tracker("dunwich").asked().is_empty());
        }
    }

    /// A run answered from two reads of different ages is as old as the
    /// older of them.
    #[test]
    fn a_run_is_dated_to_the_oldest_read_it_was_drawn_from() {
        let mut later: serde_json::Value =
            serde_json::from_str(&fresh("ferry", json!("ok"))).expect("JSON");
        later["as_of"] = json!("2026-08-30T11:59:50Z");
        let at = a_listener_saying(
            "listened-dated",
            vec![later.to_string(), fresh("dunwich", json!("ok"))],
        );
        let own = own_trackers();

        let snapshot = crate::app::run(
            &projects(),
            &crate::collect::agents::Unasked,
            &through(&at, &own, A_MOMENT),
            crate::model::snapshot::Filter::All,
            "2026-08-30T12:00:00Z".parse().expect("an instant"),
        );

        assert_eq!(
            snapshot.generated_at,
            "2026-08-30T11:59:30Z"
                .parse::<DateTime<Utc>>()
                .expect("an instant")
        );
    }

    /// The listener went with one project answered and the other not: the
    /// answered one stands, and the other is read here in full.
    #[test]
    fn a_listener_that_goes_part_way_leaves_the_rest_to_be_read_for_itself() {
        let at = a_socket("listened-goes");
        let listening = UnixListener::bind(&at).expect("the socket is ours");
        thread::spawn(move || {
            let (mut connection, _) = listening.accept().expect("the run connects");
            let mut asked = BufReader::new(connection.try_clone().expect("ours to read"));
            for _ in ["dunwich", "ferry"] {
                asked
                    .read_line(&mut String::new())
                    .expect("the run watches");
            }
            writeln!(connection, "{}", bead("dunwich", "dun-1", true, &[])).expect("sent");
            writeln!(connection, "{}", fresh("dunwich", json!("ok"))).expect("sent");
        });
        let own = own_trackers();
        let cfg = projects();
        let through = through(&at, &own, A_MOMENT);

        let dunwich = through.of(project(&cfg, "dunwich")).expect("answered");
        let asked = Instant::now();
        through
            .of(project(&cfg, "ferry"))
            .map(|tracker| ids(tracker.as_ref()))
            .expect("read for itself");
        let left_at = asked.elapsed();

        assert_eq!(ids(dunwich.as_ref()), ["dun-1"]);
        assert!(own.tracker("dunwich").asked().is_empty());
        assert!(!own.tracker("ferry").asked().is_empty());
        assert!(left_at < A_MOMENT, "left on the hang-up, not the patience");
    }

    #[test]
    fn a_listener_that_says_nothing_for_its_patience_is_left_behind() {
        let at = a_listener_saying("listened-wedged", Vec::new());
        let own = own_trackers();
        let cfg = projects();

        through(&at, &own, Duration::from_millis(100))
            .of(project(&cfg, "dunwich"))
            .map(|tracker| ids(tracker.as_ref()))
            .expect("read for itself");

        assert!(!own.tracker("dunwich").asked().is_empty());
    }

    #[test]
    fn a_listener_that_takes_no_connection_for_its_patience_is_left_behind() {
        use std::os::fd::AsRawFd;

        let at = a_socket("listened-not-accepting");
        let listening = UnixListener::bind(&at).expect("the socket is ours");
        // SAFETY: `listening` is a listening socket this test owns, and
        // listening again only shortens its queue to the one connection.
        assert_eq!(unsafe { libc::listen(listening.as_raw_fd(), 0) }, 0);
        let _queued = UnixStream::connect(&at).expect("the queue holds one");
        let (done, finished) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let own = own_trackers();
            let cfg = projects();
            let read = through(&at, &own, Duration::from_millis(100))
                .of(project(&cfg, "dunwich"))
                .map(|tracker| ids(tracker.as_ref()))
                .is_ok();
            let _ = done.send(read && !own.tracker("dunwich").asked().is_empty());
        });

        assert_eq!(finished.recv_timeout(A_MOMENT), Ok(true));
        drop(listening);
    }

    #[test]
    fn a_listener_that_never_finishes_a_line_is_left_behind() {
        let at = a_socket("listened-dribbling");
        let listening = UnixListener::bind(&at).expect("the socket is ours");
        thread::spawn(move || {
            let (mut connection, _) = listening.accept().expect("the run connects");
            while connection.write_all(b" ").is_ok() {
                thread::sleep(Duration::from_millis(20));
            }
        });
        let (done, finished) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let own = own_trackers();
            let cfg = projects();
            let read = through(&at, &own, Duration::from_millis(100))
                .of(project(&cfg, "dunwich"))
                .map(|tracker| ids(tracker.as_ref()))
                .is_ok();
            let _ = done.send(read && !own.tracker("dunwich").asked().is_empty());
        });

        assert_eq!(finished.recv_timeout(A_MOMENT), Ok(true));
    }

    #[test]
    fn a_listener_that_takes_nothing_for_its_patience_is_left_behind() {
        let at = a_socket("listened-not-reading");
        let listening = UnixListener::bind(&at).expect("the socket is ours");
        thread::spawn(move || {
            let (_connection, _) = listening.accept().expect("the run connects");
            thread::sleep(2 * A_MOMENT);
        });
        let more_than_the_socket_holds = "dunwich".repeat(1 << 20);
        let (done, finished) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let own = own_trackers();
            let cfg = projects();
            let read = Through::hearing(
                Some(&at),
                [more_than_the_socket_holds.as_str(), "dunwich"],
                &own,
                Duration::from_millis(100),
                None,
            )
            .of(project(&cfg, "dunwich"))
            .map(|tracker| ids(tracker.as_ref()))
            .is_ok();
            let _ = done.send(read && !own.tracker("dunwich").asked().is_empty());
        });

        assert_eq!(finished.recv_timeout(A_MOMENT), Ok(true));
    }

    /// A bead whose row this run cannot read is a listener it cannot read,
    /// and nothing more is taken from it.
    #[test]
    fn a_line_that_will_not_read_leaves_the_listener_behind() {
        let at = a_listener_saying(
            "listened-unreadable",
            vec![
                json!({ "line": "bead", "project": "dunwich", "ready": true,
                        "blocked_by": [], "row": { "id": "dun-1" } })
                .to_string(),
                fresh("dunwich", json!("ok")),
            ],
        );
        let own = own_trackers();
        let cfg = projects();

        through(&at, &own, A_MOMENT)
            .of(project(&cfg, "dunwich"))
            .map(|tracker| ids(tracker.as_ref()))
            .expect("read for itself");

        assert!(!own.tracker("dunwich").asked().is_empty());
    }

    /// A listener older or newer than this run may mean something else by
    /// the same lines, so one that does not say it speaks this run's
    /// protocol is not believed.
    #[test]
    fn a_listener_speaking_another_protocol_is_left_behind() {
        for (named, protocol) in [
            ("listened-no-protocol", None),
            ("listened-next-protocol", Some(2)),
        ] {
            let mut freshness: serde_json::Value =
                serde_json::from_str(&fresh("dunwich", json!("ok"))).expect("JSON");
            match protocol {
                Some(protocol) => freshness["protocol"] = json!(protocol),
                None => {
                    freshness
                        .as_object_mut()
                        .expect("an object")
                        .remove("protocol");
                }
            }
            let at = a_listener_saying(
                named,
                vec![bead("dunwich", "dun-1", true, &[]), freshness.to_string()],
            );
            let own = own_trackers();
            let cfg = projects();

            through(&at, &own, A_MOMENT)
                .of(project(&cfg, "dunwich"))
                .map(|tracker| ids(tracker.as_ref()))
                .expect("read for itself");

            assert!(!own.tracker("dunwich").asked().is_empty(), "{named}");
        }
    }

    #[test]
    fn with_no_listener_every_project_is_read_for_itself() {
        let own = own_trackers();
        let cfg = projects();

        Through::listener_at(None, ["dunwich"], &own)
            .of(project(&cfg, "dunwich"))
            .map(|tracker| ids(tracker.as_ref()))
            .expect("read for itself");

        assert!(!own.tracker("dunwich").asked().is_empty());
    }
}
