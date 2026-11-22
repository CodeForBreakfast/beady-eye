//! `bdi web`: every configured project's forest as a read-only web page, and
//! each bead at an address of its own, answered over HTTP.
//!
//! One thread keeps the latest snapshot, reading the trackers as a view does,
//! through the watcher where one answers. Each request is drawn afresh from
//! that snapshot, so what a request costs is the drawing and never a read.

use std::collections::BTreeMap;
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Context;
use chrono::{DateTime, Local, Offset, Utc};

use crate::app::{Awaited, Collection, Wanted};
use crate::collect::agents::{Agents, Unasked};
use crate::collect::bd;
use crate::collect::changes::Heard;
use crate::collect::environment::EnvironmentCache;
use crate::collect::http::{self, close, head, Head, Patient, Unread};
use crate::collect::run::RealRunner;
use crate::collect::watched::Through;
use crate::config::{Config, Poll};
use crate::model::snapshot::{Filter, Snapshot};
use crate::view::address::{self, Asked};
use crate::view::draw::html;
use crate::view::phrase;

use super::{read_config, where_this_machine_watches, Launch, Reading};

mod live;

/// The path a readiness probe asks, answered as passing once the trackers
/// have been read.
const HEALTH: &str = "/healthz";

/// The most requests answered at once. Drawing a page is the whole cost of
/// one, so this bounds how much of the machine readers can take at a time.
const MOST_AT_ONCE: usize = 8;

/// How long a request has to arrive in full, and its answer to be sent.
const PATIENCE: Duration = Duration::from_secs(10);

const HTML: &str = "text/html; charset=utf-8";
const TEXT: &str = "text/plain; charset=utf-8";

const UNAVAILABLE: &str = "503 Service Unavailable";
const NOT_FOUND: &str = "404 Not Found";

/// What the reading thread holds for the pages to be drawn from, and word
/// of each change to it for the open pages waiting on one.
struct Latest {
    held: Mutex<Held>,
    changed: Condvar,
}

#[derive(Clone)]
struct Held {
    /// The latest snapshot, where the trackers have been read at all.
    snapshot: Option<Snapshot>,
    /// The read being made, so a page drawn while it is out says so, and
    /// says a tracker has stopped answering once it has been out too long.
    collecting: Option<Awaited>,
    /// Which change this is. The count starts at the time the server
    /// started, in milliseconds, so a page drawn by a server since restarted
    /// never names a change the new one holds, and is sent the new one's.
    change: u64,
}

/// What a page drawn from what is held depends on: the change, and whether
/// the read out has gone unanswered, which time passing changes alone.
type DrawnFrom = (u64, bool);

impl Latest {
    fn new() -> Self {
        let started = Utc::now().timestamp_millis();
        Self::holding(Held {
            snapshot: None,
            collecting: None,
            change: u64::try_from(started).unwrap_or_default(),
        })
    }

    fn holding(held: Held) -> Self {
        Self {
            held: Mutex::new(held),
            changed: Condvar::new(),
        }
    }

    /// What is held now.
    fn now(&self) -> Held {
        self.held().clone()
    }

    /// Change what is held with `change`, and wake every page waiting on it.
    fn change(&self, change: impl FnOnce(&mut Held)) {
        {
            let mut held = self.held();
            change(&mut held);
            held.change += 1;
        }
        self.changed.notify_all();
    }

    /// The reading thread changes what is held by whole assignments, so a
    /// panic leaves it whole, and a poisoned lock is taken all the same.
    fn held(&self) -> MutexGuard<'_, Held> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// What is held once a page drawn from it would differ from one drawn
    /// from `seen`, and what that page is drawn from, or nothing where
    /// `until` comes first. A page drawn from nothing yet seen differs at
    /// once.
    fn after(&self, seen: Option<DrawnFrom>, until: Instant) -> Option<(Held, DrawnFrom)> {
        let mut held = self.held();
        loop {
            let now = Utc::now();
            let from = held.drawn_from(now);
            if Some(from) != seen {
                return Some((held.clone(), from));
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            let unanswered_in = held
                .collecting
                .as_ref()
                .filter(|out| !out.unanswered_at(now))
                .and_then(|out| (out.asked_at + out.patience - now).to_std().ok());
            let wait = unanswered_in.map_or(left, |unanswered_in| left.min(unanswered_in));
            held = self
                .changed
                .wait_timeout(held, wait)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

impl Held {
    fn drawn_from(&self, now: DateTime<Utc>) -> DrawnFrom {
        let unanswered = self
            .collecting
            .as_ref()
            .is_some_and(|out| out.unanswered_at(now));
        (self.change, unanswered)
    }
}

/// Answer requests on `listen` until a signal ends the process, drawing each
/// from the latest read of every project `config` names.
///
/// The config is read once. Replacing what it reads is a restart.
pub(super) fn web(config: &Path, listen: &str) -> anyhow::Result<ExitCode> {
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
    if let Some(project) = cfg
        .read()
        .map(|project| project.name.as_str())
        .find(|name| !address::addressable(name))
    {
        anyhow::bail!("bdi web cannot give the project {project:?} an address a browser keeps; give it another name in the config");
    }
    let listener = TcpListener::bind(listen).with_context(|| format!("listening on {listen}"))?;
    let taken = listener
        .local_addr()
        .with_context(|| format!("reading the address {listen} gave"))?;
    println!("bdi web is answering on http://{taken}");

    let latest = Arc::new(Latest::new());
    let answering = Arc::new(Answering {
        latest: Arc::clone(&latest),
        cfg: cfg.clone(),
        live: live::Streams::default(),
    });
    http::accept(
        listener,
        MOST_AT_ONCE,
        |stream| reply(stream, UNAVAILABLE, TEXT, b"busy\n"),
        move |stream| answer(stream, &answering),
    );
    keep_reading(&cfg, &latest);
    Ok(ExitCode::SUCCESS)
}

/// What every request is answered from.
struct Answering {
    latest: Arc<Latest>,
    cfg: Config,
    live: live::Streams,
}

/// Read every project, then each project again as the watcher says it
/// changed or as its read stops speaking for its tracker, and each project
/// that polls again a refresh interval after the interval's last reads,
/// holding each snapshot as the latest. Runs for as long as the process does.
fn keep_reading(cfg: &Config, latest: &Latest) {
    let refresh = cfg.tui.refresh();
    let (told, heard) = mpsc::channel();
    let trackers = Through::staying(
        where_this_machine_watches(cfg).as_deref(),
        cfg.read().map(|project| project.name.as_str()),
        bd::Cli::new(&RealRunner).caching_environments(EnvironmentCache::here()),
        told,
        refresh,
    );
    let mut collection = Collection::default();
    let (mut wanted, mut interval_read) = (vec![Wanted::Everything], true);
    let mut interval_ends = None;
    let mut speaks_until = BTreeMap::new();
    loop {
        for wanted in wanted {
            let asked_at = Utc::now();
            latest.change(|held| {
                held.collecting = Some(Awaited {
                    wanted: wanted.clone(),
                    asked_at,
                    patience: cfg.tui.unanswered_after(),
                });
            });
            let snapshot =
                collection.collect(cfg, &Unasked, &trackers, &wanted, Filter::All, asked_at);
            speaks_until.clone_from(&snapshot.speaks_until);
            latest.change(|held| {
                held.snapshot = Some(snapshot);
                held.collecting = None;
            });
        }
        // ponytail: a project that polls until it is covered goes on polling
        // under cover; give each its own poll through `app::Reading` if the
        // load on a tracker matters.
        if interval_read {
            interval_ends = interval_from(Instant::now(), refresh);
        }
        let lapse = soonest_lapse(&speaks_until, Utc::now(), Instant::now())
            .filter(|(_, at)| interval_ends.is_none_or(|ends| *at < ends));
        let ends = lapse.as_ref().map(|(_, at)| *at).or(interval_ends);
        (wanted, interval_read) = match (changed_before(&heard, ends), lapse) {
            (Some(project), _) | (None, Some((project, _))) => {
                (vec![Wanted::Project(project)], false)
            }
            (None, None) => (on_the_interval(cfg), true),
        };
    }
}

/// The project whose read stops speaking for its tracker soonest, and the
/// instant it does, as the clock the loop waits on counts it, where that
/// clock can count that far. `now` and `instant` are the same moment.
fn soonest_lapse(
    speaks_until: &BTreeMap<String, DateTime<Utc>>,
    now: DateTime<Utc>,
    instant: Instant,
) -> Option<(String, Instant)> {
    let (project, until) = speaks_until.iter().min_by_key(|(_, until)| **until)?;
    let left = (*until - now).to_std().unwrap_or_default();
    Some((project.clone(), instant.checked_add(left)?))
}

/// When a refresh interval starting `now` ends, or nothing where it is too
/// long to end.
fn interval_from(now: Instant, refresh: Duration) -> Option<Instant> {
    now.checked_add(refresh)
}

/// What a refresh interval reads again: every project where each of them
/// polls, and otherwise each project that does.
fn on_the_interval(cfg: &Config) -> Vec<Wanted> {
    if cfg.read().all(|project| project.poll != Poll::Never) {
        return vec![Wanted::Everything];
    }
    cfg.read()
        .filter(|project| project.poll != Poll::Never)
        .map(|project| Wanted::Project(project.name.clone()))
        .collect()
}

/// The next project the watcher says has changed, or nothing once the
/// interval `ends` first. An interval that never ends waits on the watcher,
/// and for ever once it has gone.
fn changed_before(heard: &Receiver<Heard>, ends: Option<Instant>) -> Option<String> {
    loop {
        let said = match ends {
            Some(ends) => heard.recv_timeout(ends.checked_duration_since(Instant::now())?),
            None => heard.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match said {
            Ok(Heard::Changed(project)) => return Some(project),
            Ok(Heard::Covered(_)) => {}
            Err(RecvTimeoutError::Timeout) => return None,
            Err(RecvTimeoutError::Disconnected) => match ends {
                Some(ends) => {
                    thread::sleep(ends.saturating_duration_since(Instant::now()));
                    return None;
                }
                None => loop {
                    thread::park();
                },
            },
        }
    }
}

/// Read one request from `stream` and answer it, or close it unanswered
/// where it never arrives in full or is not HTTP. A request for the changes
/// to a page is kept open apart from the requests being answered.
fn answer(stream: TcpStream, answering: &Arc<Answering>) {
    let mut stream = Patient {
        stream,
        deadline: Instant::now() + PATIENCE,
    };
    let read = head(&mut stream, &mut Vec::new());
    if let Ok(head) = &read {
        if let Some(asked) = Asked::of(&head.target).filter(|_| live::asks_for_changes(head)) {
            live::keep_current(answering, stream.stream, head, asked);
            return;
        }
    }
    let Some((status, kind, body)) = said(read, &answering.latest, &answering.cfg) else {
        return;
    };
    reply(&stream.stream, status, kind, body.as_bytes());
    close(stream.stream, stream.deadline);
}

/// What a request whose head was `read` comes to, or nothing where it never
/// arrived in full or is not HTTP.
fn said(
    read: Result<Head, Unread>,
    latest: &Latest,
    cfg: &Config,
) -> Option<(&'static str, &'static str, String)> {
    match read {
        Ok(head) => Some(answered(&head, latest, cfg)),
        Err(Unread::TooLong) => Some((
            "414 URI Too Long",
            HTML,
            html::saying(phrase::address_too_long()),
        )),
        Err(Unread::Unreadable) => None,
    }
}

/// What a request comes to: its status, the kind of body, and the body.
fn answered(head: &Head, latest: &Latest, cfg: &Config) -> (&'static str, &'static str, String) {
    if head.method != "GET" {
        return ("405 Method Not Allowed", TEXT, "GET only\n".to_string());
    }
    let held = latest.now();
    if head.path() == HEALTH {
        return match held.snapshot {
            Some(_) => ("200 OK", TEXT, "ok\n".to_string()),
            None => (UNAVAILABLE, TEXT, "reading\n".to_string()),
        };
    }
    let Some(asked) = Asked::of(&head.target) else {
        return (NOT_FOUND, HTML, html::saying(phrase::no_page_here()));
    };
    let (status, page) = drawn(&asked, &held, cfg);
    (status, HTML, page)
}

/// The page `asked` names, drawn from `held`, and its status.
fn drawn(asked: &Asked, held: &Held, cfg: &Config) -> (&'static str, String) {
    let drawn = html::Drawn {
        at: Utc::now(),
        offset: Local::now().offset().fix(),
        collecting: Vec::from_iter(held.collecting.clone()),
        change: held.change,
    };
    let Some(snapshot) = held.snapshot.clone() else {
        // Until the first read answers, the forest is each project's line
        // with its mark saying it is being read, as the terminal opens on.
        // Whether a bead is there cannot be said yet.
        if asked.bead.is_some() {
            let page = html::saying_kept_current(phrase::not_read_yet(), held.change);
            return (UNAVAILABLE, page);
        }
        let awaiting = Snapshot::awaiting(
            cfg.read().map(|project| project.name.clone()).collect(),
            cfg.projects_named_without_git(),
            Unasked.name(),
            cfg.scope.clone(),
            Filter::All,
            drawn.at,
        );
        return (
            UNAVAILABLE,
            html::page(awaiting, &cfg.row, asked, drawn).html,
        );
    };
    let page = html::page(snapshot, &cfg.row, asked, drawn);
    let status = if page.found { "200 OK" } else { NOT_FOUND };
    (status, page.html)
}

fn reply(stream: &TcpStream, status: &str, kind: &str, body: &[u8]) {
    http::reply(stream, status, kind, body, PATIENCE);
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;

    /// What a request for `target` with `method` is answered, with `latest`
    /// held.
    fn answered_to(method: &str, target: &str, latest: &Latest) -> (&'static str, String) {
        let sent = format!("{method} {target} HTTP/1.1\r\nHost: bdi\r\n\r\n");
        let (status, _, body) = said(
            head(&mut sent.as_bytes(), &mut Vec::new()),
            latest,
            &Config::naming(Vec::new()),
        )
        .expect("an answer");
        (status, body)
    }

    /// Every link carries the folds the reader pointed, so a reader who
    /// points enough of them follows a link longer than a request may be.
    #[test]
    fn an_address_too_long_to_read_is_answered_with_a_way_back_to_the_forest() {
        let latest = read();
        let target = format!("/?{}", "open=p%3Aarkham&".repeat(http::LONGEST_HEAD / 16));
        let (status, body) = answered_to("GET", &target, &latest);
        assert_eq!(status, "414 URI Too Long");
        assert!(body.contains("href=\"/\""), "{body}");
    }

    /// The same address is too long whether its end arrives in the read
    /// that crosses the limit or in a read after it.
    #[test]
    fn an_address_is_too_long_however_its_bytes_arrive() {
        let target = format!("/?{}", "open=p%3Aarkham&".repeat(1024));
        let sent = format!("GET {target} HTTP/1.1\r\nHost: bdi\r\n\r\n");
        assert!(sent.len() > http::LONGEST_HEAD && sent.len() < http::LONGEST_HEAD + 4096);
        let (first, rest) = sent.as_bytes().split_at(100);
        let mut arriving = first.chain(rest);
        let (status, _, _) = said(
            head(&mut arriving, &mut Vec::new()),
            &read(),
            &Config::naming(Vec::new()),
        )
        .expect("an answer");
        assert_eq!(status, "414 URI Too Long");
    }

    /// A project that says it does not poll is left to whatever reports its
    /// changes, and is read again only when the watcher says it changed.
    #[test]
    fn a_refresh_interval_reads_again_only_the_projects_that_poll() {
        let cfg = Config::from_toml(
            r#"
[[projects]]
name = "arkham"
path = "/home/user/dev/arkham"

[[projects]]
name = "kadath"
path = "/home/user/dev/kadath"
poll = false
"#,
        )
        .expect("parses");
        assert_eq!(
            on_the_interval(&cfg),
            vec![Wanted::Project("arkham".to_string())]
        );
        assert_eq!(
            on_the_interval(&Config::naming(Vec::new())),
            vec![Wanted::Everything]
        );
    }

    /// A refresh interval too long to end is one that never ends, and what
    /// the watcher says is still heard.
    #[test]
    fn a_refresh_interval_too_long_to_end_still_hears_the_watcher() {
        let cfg = Config::from_toml(
            r#"
[tui]
refresh_seconds = 9223372036854775807

[[projects]]
name = "arkham"
path = "/home/user/dev/arkham"
"#,
        )
        .expect("parses");
        let ends = interval_from(Instant::now(), cfg.tui.refresh());
        assert_eq!(ends, None);
        let (told, heard) = mpsc::channel();
        told.send(Heard::Covered("arkham".to_string()))
            .expect("open");
        told.send(Heard::Changed("arkham".to_string()))
            .expect("open");
        assert_eq!(changed_before(&heard, ends), Some("arkham".to_string()));
    }

    /// A deferred bead coming due writes nothing to its tracker, so the
    /// project holding it is read again when it does, soonest first, and at
    /// once where the instant has passed.
    #[test]
    fn the_read_that_stops_speaking_soonest_is_made_again_when_it_does() {
        let now = Utc::now();
        let instant = Instant::now();
        let speaks_until = BTreeMap::from([
            ("arkham".to_string(), now + chrono::Duration::minutes(5)),
            ("kadath".to_string(), now + chrono::Duration::minutes(2)),
        ]);
        assert_eq!(
            soonest_lapse(&speaks_until, now, instant),
            Some(("kadath".to_string(), instant + Duration::from_secs(120)))
        );
        let passed = BTreeMap::from([("arkham".to_string(), now - chrono::Duration::minutes(1))]);
        assert_eq!(
            soonest_lapse(&passed, now, instant),
            Some(("arkham".to_string(), instant))
        );
        assert_eq!(soonest_lapse(&BTreeMap::new(), now, instant), None);
    }

    /// A snapshot of a config naming no project, which reads nothing.
    fn read() -> Latest {
        let cfg = Config::naming(Vec::new());
        let snapshot = Collection::default().collect(
            &cfg,
            &Unasked,
            &bd::Cli::new(&RealRunner),
            &Wanted::Everything,
            Filter::All,
            Utc::now(),
        );
        Latest::holding(Held {
            snapshot: Some(snapshot),
            collecting: None,
            change: 0,
        })
    }

    #[test]
    fn before_the_trackers_are_read_a_beads_page_says_so_and_the_probe_fails() {
        let latest = Latest::new();
        let (status, body) = answered_to("GET", "/ark/ark-5", &latest);
        assert_eq!(status, "503 Service Unavailable");
        assert!(body.contains(phrase::not_read_yet()), "{body}");
        assert_eq!(
            answered_to("GET", HEALTH, &latest).0,
            "503 Service Unavailable"
        );
    }

    #[test]
    fn before_the_first_read_answers_the_forest_names_each_project_as_being_read() {
        let cfg =
            Config::from_toml("[[projects]]\nname = \"arkham\"\npath = \"/home/user/arkham\"\n")
                .expect("a config");
        let asked_at = Utc::now() - chrono::TimeDelta::seconds(600);
        let latest = Latest::holding(Held {
            snapshot: None,
            collecting: Some(Awaited {
                wanted: Wanted::Everything,
                asked_at,
                patience: cfg.tui.unanswered_after(),
            }),
            change: 0,
        });
        let sent = "GET / HTTP/1.1\r\nHost: bdi\r\n\r\n";
        let head = head(&mut sent.as_bytes(), &mut Vec::new()).expect("a head");
        let (status, _, body) = answered(&head, &latest, &cfg);
        assert_eq!(status, "503 Service Unavailable");
        let line = &body[body.find(">arkham").expect("the project is named")..];
        let line = &line[..line.find("</div>").expect("the line ends")];
        assert!(line.contains("⠿"), "{body}");
    }

    #[test]
    fn once_the_trackers_are_read_the_forest_is_drawn_and_the_probe_passes() {
        let latest = read();
        let (status, body) = answered_to("GET", "/", &latest);
        assert_eq!(status, "200 OK");
        assert!(body.starts_with("<!doctype html>"), "{body}");
        assert_eq!(answered_to("GET", HEALTH, &latest).0, "200 OK");
    }

    #[test]
    fn an_address_with_no_page_is_not_found_and_anything_but_a_get_is_refused() {
        let latest = read();
        let (status, body) = answered_to("GET", "/a/b/c", &latest);
        assert_eq!(status, "404 Not Found");
        assert!(body.contains(phrase::no_page_here()), "{body}");
        assert_eq!(answered_to("GET", "/ark/ark-5", &latest).0, "404 Not Found");
        assert_eq!(
            answered_to("POST", "/", &latest).0,
            "405 Method Not Allowed"
        );
    }
}
