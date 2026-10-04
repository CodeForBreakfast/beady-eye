//! What `bdi listen` says of the projects it reads, asked on its socket.
//!
//! A run that finds a listener reads its trackers through it: it watches each
//! project it reads, takes the beads it is sent as a read of its own, and
//! hangs up when it is done. A project the listener does not answer for is
//! read as it would be with no listener at all, so a listener that is down,
//! wedged or reading other projects costs a run what it would have saved and
//! never its answer. `docs/design.md`'s *A view reads through the listener*
//! has the design.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::collect::bd::bead_of;
use crate::collect::run::{FailureKind, RunFailure};
use crate::collect::tracker::{OpenFailure, Tracker, Trackers};
use crate::config::Project;
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
pub struct Through<'t> {
    listener: Mutex<Listener>,
    otherwise: &'t dyn Trackers,
}

impl<'t> Through<'t> {
    /// Watch each of `projects` on the listener at `at`, reading through
    /// `otherwise` every project it does not answer for. A listener that is
    /// not there leaves every project to `otherwise`.
    pub fn listener_at<'p>(
        at: Option<&Path>,
        projects: impl IntoIterator<Item = &'p str>,
        otherwise: &'t dyn Trackers,
    ) -> Self {
        Self::waiting(at, projects, otherwise, WEDGED_AFTER)
    }

    fn waiting<'p>(
        at: Option<&Path>,
        projects: impl IntoIterator<Item = &'p str>,
        otherwise: &'t dyn Trackers,
        patience: Duration,
    ) -> Self {
        let mut listener = Listener {
            connection: at.and_then(|at| Connection::to(at, patience)),
            patience,
            ..Listener::default()
        };
        for project in projects {
            listener.ask(project);
        }
        Self {
            listener: Mutex::new(listener),
            otherwise,
        }
    }
}

impl Trackers for Through<'_> {
    fn of(&self, project: &Project) -> Result<Box<dyn Tracker + '_>, OpenFailure> {
        let told = self
            .listener
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .answer_for(&project.name);
        match told {
            Some(told) => match &told.unreachable {
                Some(failure) => Err(reached_as(failure)),
                None => Ok(Box::new(Answered(told))),
            },
            None => self.otherwise.of(project),
        }
    }
}

/// One connection to the listener, and what it has said on it.
#[derive(Default)]
struct Listener {
    /// Nothing once the listener has gone, wedged or said something this
    /// run cannot read.
    connection: Option<Connection>,
    patience: Duration,
    /// The projects a watch line has gone out for.
    asked: BTreeSet<String>,
    /// The projects the listener said it does not read.
    refused: BTreeSet<String>,
    /// Each project as of the last freshness line closing an answer for it.
    current: BTreeMap<String, Arc<Told>>,
    /// The answers that have begun and not yet closed.
    arriving: BTreeMap<String, Told>,
}

struct Connection {
    to: UnixStream,
    from: BufReader<UnixStream>,
}

/// One project as the listener last said it stood.
#[derive(Debug, Clone, Default)]
struct Told {
    beads: BTreeMap<String, Listed>,
    as_of: Option<DateTime<Utc>>,
    unreachable: Option<TrackerFailure>,
}

/// One bead as the listener sent it.
#[derive(Debug, Clone)]
struct Listed {
    bead: Bead,
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
        ready: bool,
        blocked_by: Vec<String>,
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
    },
    Refused {
        asked: String,
    },
    #[serde(other)]
    Other,
}

const WATCH_ALL: &str = "watch-all";

impl Listener {
    /// Send the watch line for `project`, once.
    fn ask(&mut self, project: &str) {
        if self.asked.contains(project) {
            return;
        }
        self.asked.insert(project.to_string());
        let sent = self
            .connection
            .as_mut()
            .is_some_and(|connection| writeln!(connection.to, "{WATCH_ALL} {project}").is_ok());
        if !sent {
            self.connection = None;
        }
    }

    /// What the listener says of `project`, reading until it has said it.
    /// Nothing where it will not say, which leaves the project to be read
    /// some other way.
    fn answer_for(&mut self, project: &str) -> Option<Arc<Told>> {
        self.ask(project);
        let giving_up = Instant::now() + self.patience;
        loop {
            if let Some(told) = self.current.get(project) {
                return Some(Arc::clone(told));
            }
            if self.refused.contains(project) {
                return None;
            }
            let heard = self.connection.as_mut()?.next(giving_up);
            if heard.and_then(|line| self.take(line)).is_none() {
                self.connection = None;
            }
        }
    }

    /// Take one line into what has been said. Nothing where it cannot be
    /// taken, which is a listener this run cannot read.
    fn take(&mut self, line: Line) -> Option<()> {
        match line {
            Line::Bead {
                project,
                ready,
                blocked_by,
                row,
            } => {
                let bead = bead_of(row, false).ok()?;
                let listed = Listed {
                    bead,
                    ready,
                    blocked_by,
                };
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
                told.unreachable = match tracker {
                    TrackerState::Unreachable(failure) => Some(failure),
                    _ => None,
                };
                self.current.insert(project, Arc::new(told));
            }
            Line::Refused { asked } => {
                let project = asked.strip_prefix(WATCH_ALL)?.trim();
                self.refused.insert(project.to_string());
            }
            Line::Other => {}
        }
        Some(())
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

impl Connection {
    /// A connection to the listener at `at`, giving up on any one write it
    /// will not take within `patience`.
    fn to(at: &Path, patience: Duration) -> Option<Self> {
        let to = UnixStream::connect(at).ok()?;
        to.set_write_timeout(Some(patience)).ok()?;
        let from = BufReader::new(to.try_clone().ok()?);
        Some(Self { to, from })
    }

    /// The next line the listener finishes sending before `giving_up`.
    /// Nothing where it finished none, closed the connection, or sent
    /// something that is not a line about a watch.
    fn next(&mut self, giving_up: Instant) -> Option<Line> {
        let mut line = Vec::new();
        loop {
            let left = giving_up
                .checked_duration_since(Instant::now())
                .filter(|left| !left.is_zero())?;
            self.from.get_ref().set_read_timeout(Some(left)).ok()?;
            let arrived = self.from.fill_buf().ok()?;
            if arrived.is_empty() {
                return None;
            }
            match arrived.iter().position(|&byte| byte == b'\n') {
                Some(end) => {
                    line.extend_from_slice(&arrived[..end]);
                    self.from.consume(end + 1);
                    return serde_json::from_slice(&line).ok();
                }
                None => {
                    let taken = arrived.len();
                    line.extend_from_slice(arrived);
                    self.from.consume(taken);
                }
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
            .filter(|(_, listed)| listed.ready)
            .map(|(id, _)| id.clone())
            .collect())
    }

    /// The blockers the project itself holds, as bd names them. The listener
    /// sends each bead's blockers as `bdi` drew them, bd's own and then those
    /// a tree found in other projects, and those depend on which tree drew
    /// the bead. So they are left for this run's own trees to find again.
    fn blocked(&self) -> Result<BTreeMap<String, Vec<String>>, RunFailure> {
        Ok(self
            .0
            .beads
            .iter()
            .map(|(id, listed)| {
                let held_here: Vec<String> = listed
                    .blocked_by
                    .iter()
                    .filter(|blocker| self.0.beads.contains_key(*blocker))
                    .cloned()
                    .collect();
                (id.clone(), held_here)
            })
            .filter(|(_, held_here)| !held_here.is_empty())
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
        let at = a_socket(named);
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

    fn bead(project: &str, id: &str, ready: bool, blocked_by: &[&str]) -> String {
        json!({
            "line": "bead",
            "project": project,
            "ready": ready,
            "blocked_by": blocked_by,
            "row": { "id": id, "title": "re-point the dish", "status": "open",
                     "priority": 2, "issue_type": "task" },
        })
        .to_string()
    }

    fn fresh(project: &str, tracker: serde_json::Value) -> String {
        json!({
            "line": "freshness",
            "project": project,
            "as_of": "2026-08-30T11:59:30Z",
            "tracker": tracker,
            "events": "off",
            "protocol": 1,
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

    fn through<'t>(at: &Path, own: &'t Fakes, patience: Duration) -> Through<'t> {
        Through::waiting(Some(at), ["dunwich", "ferry"], own, patience)
    }

    #[test]
    fn a_project_the_listener_answers_for_is_read_from_its_answer_alone() {
        let at = a_listener_saying(
            "listened-answers",
            vec![
                bead("dunwich", "dun-1", true, &[]),
                bead("dunwich", "dun-2", false, &["fer-9", "dun-1"]),
                bead("dunwich", "dun-3", false, &["fer-9"]),
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
        assert_eq!(tracker.ready(), Ok(BTreeSet::from(["dun-1".to_string()])));
        assert_eq!(
            tracker.blocked(),
            Ok(BTreeMap::from([(
                "dun-2".to_string(),
                vec!["dun-1".to_string()]
            )])),
            "a blocker in another project is this run's to find"
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
                fresh("dunwich", json!("ok")),
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
            let read = Through::waiting(
                Some(&at),
                [more_than_the_socket_holds.as_str(), "dunwich"],
                &own,
                Duration::from_millis(100),
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
