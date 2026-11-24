//! `bdi web`: every configured project's forest as a read-only web page, and
//! each bead at an address of its own, answered over HTTP.
//!
//! One thread keeps the latest snapshot, reading the trackers as a view does,
//! through the watcher where one answers. Each request is drawn afresh from
//! that snapshot, so what a request costs is the drawing and never a read.

use std::net::{IpAddr, Ipv6Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use anyhow::Context;
use chrono::{DateTime, Local, Offset, Utc};

use crate::app::{armed_unread, Awaited, Collection, Outstanding, Refreshing};
use crate::collect::agents::{Agents, Unasked};
use crate::collect::bd;
use crate::collect::changes::{self, Heard, Report, Reported, Socket};
use crate::collect::environment::EnvironmentCache;
use crate::collect::http::{self, close, head, Encoding, Head, Patient, Unread};
use crate::collect::run::RealRunner;
use crate::collect::tracker::Trackers;
use crate::collect::watched::{self, Through};
use crate::config::Config;
use crate::model::snapshot::{Filter, Snapshot};
use crate::view::address::{self, Asked};
use crate::view::draw::html;
use crate::view::phrase;

use super::{
    arming, read_config, where_producers_find_this_run, where_this_machine_watches, Launch,
    Polling, Reading, ViewOptions,
};

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
    /// The projects nothing has vouched for lately.
    lapsed: Vec<String>,
    /// Which change this is.
    change: u64,
}

/// What a page drawn from what is held depends on: the change, and whether
/// the read out has gone unanswered, which time passing changes alone.
type DrawnFrom = (u64, bool);

impl Latest {
    fn new() -> Self {
        Self::holding(Held {
            snapshot: None,
            collecting: None,
            lapsed: Vec::new(),
            change: 0,
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
                .and_then(|out| {
                    out.patience
                        .checked_sub(&(now - out.asked_at))?
                        .to_std()
                        .ok()
                });
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

/// The host names a request may be addressed to, beside `localhost` and an
/// address, which no other page can make a browser resolve to this one.
///
/// A page elsewhere can point a name of its own at this listener and have a
/// visitor's browser read it as that page's own. The request still says the
/// name it was sent to, so a name nobody allowed is refused.
#[derive(Clone, Debug, Default)]
pub(super) struct Hosts(pub(super) Vec<String>);

impl Hosts {
    /// Whether a request whose one Host header says `host` was sent to a
    /// name this listener answers for. The port is not compared.
    fn allow(&self, host: Option<&str>) -> bool {
        let Some(name) = host.and_then(name_of) else {
            return false;
        };
        let name = name.strip_suffix('.').unwrap_or(name);
        name.parse::<IpAddr>().is_ok()
            || name.eq_ignore_ascii_case("localhost")
            || self.0.iter().any(|allowed| {
                name.eq_ignore_ascii_case(allowed.strip_suffix('.').unwrap_or(allowed))
            })
    }
}

/// The name a Host header's `host` sends a request to, without its port, or
/// nothing where it is not a name with an optional port.
fn name_of(host: &str) -> Option<&str> {
    let (name, rest) = match host.strip_prefix('[') {
        Some(bracketed) => {
            let (address, rest) = bracketed.split_once(']')?;
            address.parse::<Ipv6Addr>().ok()?;
            (address, rest)
        }
        None => host.split_at(host.find(':').unwrap_or(host.len())),
    };
    let port_is_sound = match rest.strip_prefix(':') {
        Some(port) => port.bytes().all(|b| b.is_ascii_digit()) && port.parse::<u16>().is_ok(),
        None => rest.is_empty(),
    };
    (port_is_sound && !name.is_empty()).then_some(name)
}

/// Why `bdi web` will not take each of the view's arguments it was given,
/// each named as `--help` names it. The refusals of `--json`, `--beads` and
/// the options that narrow `--beads` are for good. The rest stand until
/// `bdi web` honours the option.
pub(super) fn refusals(view: &ViewOptions) -> Vec<String> {
    const BEADS_PRINTS: &str =
        "it narrows what --beads prints, and bdi web serves a page rather than printing";
    [
        (!view.beads.is_empty(), "BEAD-ID", "bdi web cannot yet root its page at a bead named on its command line"),
        (!view.projects.is_empty(), "--project", "bdi web reads every configured project, and cannot yet read only the ones named"),
        (view.all_projects, "--all-projects", "bdi web reads every configured project already, and does not yet take the option that asks for it"),
        (view.json, "--json", "bdi web serves a page rather than printing the snapshot"),
        (view.each_bead, "--beads", "bdi web serves a page rather than printing each bead"),
        (!view.labels.is_empty(), "--label", BEADS_PRINTS),
        (view.issue_type.is_some(), "--type", BEADS_PRINTS),
        (view.assignee.is_some(), "--assignee", BEADS_PRINTS),
        (view.parent.is_some(), "--parent", BEADS_PRINTS),
        (view.brief, "--brief", BEADS_PRINTS),
        (view.all, "--all", "bdi web asks no agent provider yet, so it cannot tell a tree with a live agent from one without"),
        (view.ready, "--ready", "bdi web opens on the forest, and cannot yet open on the ready list"),
    ]
    .into_iter()
    .filter(|(given, _, _)| *given)
    .map(|(_, named, why)| format!("{named}: {why}"))
    .collect()
}

/// Answer requests on `listen` until a signal ends the process, drawing each
/// from the latest read of every project `config` names, for requests sent to
/// a name `hosts` allows. Each project is polled, and heard from, as `view`
/// says.
///
/// The config is read once. Replacing what it reads is a restart.
pub(super) fn web(
    config: &Path,
    listen: &str,
    hosts: Hosts,
    view: &ViewOptions,
) -> anyhow::Result<ExitCode> {
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

    // Held for the run, so the reading loop never runs out of anyone to
    // hear from.
    let (told, heard) = mpsc::channel();
    let trackers = Arc::new(Through::staying(
        where_this_machine_watches(&cfg).as_deref(),
        cfg.read().map(|project| project.name.as_str()),
        bd::Cli::new(&RealRunner).caching_environments(EnvironmentCache::here()),
        told.clone(),
        cfg.tui.refresh(),
    ));
    let passing_on = Arc::clone(&trackers);
    // Held, not discarded: the socket comes off the filesystem when this
    // returns.
    let (refreshing, _socket) = refreshing_for(
        &cfg,
        Polling::asked_for(&view.polling),
        where_producers_find_this_run(view, &cfg),
        told,
        heard,
        move |heard| passing_on.passes_on(heard.clone()),
    );

    let latest = Arc::new(Latest::new());
    let answering = Arc::new(Answering {
        latest: Arc::clone(&latest),
        cfg: cfg.clone(),
        hosts,
        live: live::Streams::default(),
    });
    http::accept(
        listener,
        MOST_AT_ONCE,
        |stream| reply(stream, UNAVAILABLE, TEXT, b"busy\n"),
        move |stream| answer(stream, &answering),
    );
    keep_reading(&cfg, &latest, &*trackers, refreshing);
    Ok(ExitCode::SUCCESS)
}

/// What every request is answered from.
struct Answering {
    latest: Arc<Latest>,
    cfg: Config,
    hosts: Hosts,
    live: live::Streams,
}

/// The reads `bdi web` makes of `cfg`, as the terminal view makes them: each
/// project polled as `polling` says, read again when the watcher says on
/// `heard` that it changed, and heard from producers on the socket `at`.
/// What a producer says is handed to `passing_on`, then told on `told`.
///
/// The socket comes back beside the reads, and comes off the filesystem
/// when it is dropped. Nothing where it could not be opened, which is said
/// on stderr where it is worth saying, as the terminal view says it.
fn refreshing_for(
    cfg: &Config,
    polling: Polling,
    at: Option<PathBuf>,
    told: Sender<Heard>,
    heard: Receiver<Heard>,
    passing_on: impl Fn(&Heard) + Send + Sync + 'static,
) -> (Refreshing, Option<Socket>) {
    let arms = arming(polling);
    let armed = arms(cfg);
    let reported = Reported::watching(armed.iter().map(|project| project.project().to_string()));
    let opened = changes::watch(at, &reported, PassingOn { passing_on, told });
    let socket = changes::opened_unless_a_watcher_holds_it(opened, || {
        watched::answers(where_this_machine_watches(cfg).as_deref())
    })
    .unwrap_or_else(|refused| {
        eprintln!("bdi web: {refused}");
        None
    });
    let refreshing = Refreshing::new(
        heard,
        Outstanding::for_a_run(cfg.tui.unanswered_after()),
        crate::app::Reading::of(armed, reported).unread(armed_unread(&arms, cfg)),
    );
    (refreshing, socket)
}

/// What a producer says, handed to `passing_on` and then told on `told`.
struct PassingOn<F> {
    passing_on: F,
    told: Sender<Heard>,
}

impl<F: Fn(&Heard) + Send + Sync + 'static> Report for PassingOn<F> {
    fn report(&self, heard: Heard) -> bool {
        (self.passing_on)(&heard);
        self.told.send(heard).is_ok()
    }
}

/// Read each project as `refreshing` says, through `trackers`, holding each
/// snapshot and the projects that have lapsed as the latest. Returns only
/// once nothing can be heard any longer.
fn keep_reading(
    cfg: &Config,
    latest: &Latest,
    trackers: &dyn Trackers,
    mut refreshing: Refreshing,
) {
    let mut collection = Collection::default();
    loop {
        while read_what_is_due(
            cfg,
            latest,
            trackers,
            &mut refreshing,
            &mut collection,
            Utc::now(),
        ) {}
        if !refreshing.wait() {
            return;
        }
    }
}

/// Take what has been heard by `now`, hold the projects that have lapsed,
/// and make the read whose turn has come, holding what it finds. Whether
/// one had.
fn read_what_is_due(
    cfg: &Config,
    latest: &Latest,
    trackers: &dyn Trackers,
    refreshing: &mut Refreshing,
    collection: &mut Collection,
    now: DateTime<Utc>,
) -> bool {
    refreshing.take_heard(now);
    let lapsed = refreshing.lapsed(now);
    if latest.now().lapsed != lapsed {
        latest.change(|held| held.lapsed = lapsed);
    }
    let Some(wanted) = refreshing.due(now) else {
        return false;
    };
    latest.change(|held| {
        held.collecting = Some(Awaited {
            wanted: wanted.clone(),
            asked_at: now,
            patience: cfg.tui.unanswered_after(),
        });
    });
    let snapshot = collection.collect(cfg, &Unasked, trackers, &wanted, Filter::All, now);
    refreshing.came_back(
        &snapshot.projects,
        &snapshot.speaks_until,
        &snapshot.failed_projects,
        Utc::now(),
    );
    latest.change(|held| {
        held.snapshot = Some(snapshot);
        held.collecting = None;
    });
    true
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
    let encoding = read.as_ref().map_or(Encoding::Identity, Head::encoding);
    if let Ok(head) = &read {
        let kept_current = answering.hosts.allow(head.only("Host")) && live::asks_for_changes(head);
        if let Some(asked) = kept_current.then(|| Asked::of(&head.target)).flatten() {
            live::keep_current(answering, stream.stream, head, asked);
            return;
        }
    }
    let Some((status, kind, body)) =
        said(read, &answering.latest, &answering.cfg, &answering.hosts)
    else {
        return;
    };
    answering.live.remember(&body);
    http::reply_in(
        &stream.stream,
        status,
        kind,
        body.as_bytes(),
        encoding,
        PATIENCE,
    );
    close(stream.stream, stream.deadline);
}

/// What a request whose head was `read` comes to, or nothing where it never
/// arrived in full or is not HTTP.
fn said(
    read: Result<Head, Unread>,
    latest: &Latest,
    cfg: &Config,
    hosts: &Hosts,
) -> Option<(&'static str, &'static str, String)> {
    match read {
        Ok(head) => Some(answered(&head, latest, cfg, hosts)),
        Err(Unread::TooLong) => Some((
            "414 URI Too Long",
            HTML,
            html::saying(phrase::address_too_long()),
        )),
        Err(Unread::Unreadable) => None,
    }
}

/// What a request comes to: its status, the kind of body, and the body.
fn answered(
    head: &Head,
    latest: &Latest,
    cfg: &Config,
    hosts: &Hosts,
) -> (&'static str, &'static str, String) {
    if !hosts.allow(head.only("Host")) {
        return (
            "421 Misdirected Request",
            TEXT,
            "not a host bdi web answers for\n".to_string(),
        );
    }
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
        lapsed: held.lapsed.clone(),
    };
    let Some(snapshot) = held.snapshot.clone() else {
        // Until the first read answers, the forest is each project's line
        // with its mark saying it is being read, as the terminal opens on.
        // Whether a bead is there cannot be said yet.
        if asked.bead.is_some() {
            let page = html::saying_kept_current(phrase::not_read_yet());
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
    use std::collections::BTreeMap;
    use std::io::Read;
    use std::thread;

    use clap::Parser;

    use super::*;
    use crate::app::Wanted;
    use crate::collect::tracker::testing::{Fake, Fakes};

    /// What a request for `target` with `method` is answered, with `latest`
    /// held.
    fn answered_to(method: &str, target: &str, latest: &Latest) -> (&'static str, String) {
        answered_from(&format!("Host: {BEADS}"), method, target, latest)
    }

    /// A name the tests' listener is told it answers for.
    const BEADS: &str = "beads.example.invalid";

    fn allowed() -> Hosts {
        Hosts(vec![BEADS.to_string()])
    }

    /// A name given with the root's dot is the same name as one without it.
    #[test]
    fn a_name_allowed_with_the_roots_dot_is_allowed_without_it() {
        let dotted = Hosts(vec![format!("{BEADS}.")]);
        assert!(dotted.allow(Some(BEADS)));
        assert!(dotted.allow(Some(&format!("{BEADS}.:8080"))));
        assert!(!dotted.allow(Some("dunwich.invalid.")));
    }

    /// What a request for `target` with `method` and the header lines
    /// `headers` is answered, with `latest` held.
    fn answered_from(
        headers: &str,
        method: &str,
        target: &str,
        latest: &Latest,
    ) -> (&'static str, String) {
        let sent = format!("{method} {target} HTTP/1.1\r\n{headers}\r\n\r\n");
        let (status, _, body) = said(
            head(&mut sent.as_bytes(), &mut Vec::new()),
            latest,
            &Config::naming(Vec::new()),
            &allowed(),
        )
        .expect("an answer");
        (status, body)
    }

    /// A page elsewhere that points its own name at this listener sends its
    /// own name as the host, so it is refused, and so is a request naming no
    /// host, naming one unsoundly, or naming more than one. The probe is
    /// refused alike, since it says whether trackers answer.
    #[test]
    fn a_request_sent_to_a_name_nobody_allowed_is_refused() {
        let latest = read();
        for headers in [
            "Host: dunwich.invalid",
            "Host: dunwich.invalid:8080",
            "Host: beads.example.invalid.dunwich.invalid",
            "Accept: text/html",
            "Host: ",
            "Host: beads.example.invalid:http",
            "Host: localhost:+8080",
            "Host: beads.example.invalid:8080:80",
            "Host: [::1",
            "Host: [::1]x",
            "Host: [beads.example.invalid]",
            "Host: beads.example.invalid\r\nHost: dunwich.invalid",
        ] {
            for target in ["/", HEALTH] {
                let (status, body) = answered_from(headers, "GET", target, &latest);
                assert_eq!(status, "421 Misdirected Request", "{headers} {target}");
                assert!(!body.contains("<!doctype html>"), "{body}");
            }
        }
    }

    /// An allowed name gets the page on any port, in any case and with or
    /// without the root's dot, and so do `localhost` and an address, which
    /// only this machine or its network can make a browser send.
    #[test]
    fn a_request_sent_to_an_allowed_name_localhost_or_an_address_gets_the_page() {
        let latest = read();
        for host in [
            BEADS,
            "Beads.Example.Invalid:8080",
            "localhost",
            "LOCALHOST:8080",
            "localhost.:8080",
            "beads.example.invalid.",
            "127.0.0.1:8080",
            "10.0.0.7",
            "[::1]:8080",
            "[::1]",
        ] {
            let (status, body) = answered_from(&format!("Host: {host}"), "GET", "/", &latest);
            assert_eq!(status, "200 OK", "{host}");
            assert!(body.starts_with("<!doctype html>"), "{body}");
        }
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
        let sent = format!("GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n");
        assert!(sent.len() > http::LONGEST_HEAD && sent.len() < http::LONGEST_HEAD + 4096);
        let (first, rest) = sent.as_bytes().split_at(100);
        let mut arriving = first.chain(rest);
        let (status, _, _) = said(
            head(&mut arriving, &mut Vec::new()),
            &read(),
            &Config::naming(Vec::new()),
            &Hosts::default(),
        )
        .expect("an answer");
        assert_eq!(status, "414 URI Too Long");
    }

    /// A refresh interval too long to end is one that never ends, and what
    /// a producer says is still heard.
    #[test]
    fn a_refresh_interval_too_long_to_end_still_hears_a_producer() {
        let socket = a_socket("too-long");
        let cfg = Config::from_toml(&format!(
            "[tui]\nrefresh_seconds = 9223372036854775807\n\n\
             [changes]\nsocket = \"{}\"\n\n\
             [[projects]]\nname = \"arkham\"\npath = \"/home/user/dev/arkham\"\npoll = true\n",
            socket.display()
        ))
        .expect("parses");
        let start = Utc::now();
        let (mut refresh, _socket) = refreshing_as(&cfg, Polling::AsConfigured);
        assert_eq!(due_by(&mut refresh, start), Some(Wanted::Everything));
        refresh.came_back(&["arkham".to_string()], &BTreeMap::new(), &[], start);
        let a_year = start + chrono::TimeDelta::days(365);
        assert_eq!(due_by(&mut refresh, a_year), None, "never polled");
        assert_eq!(say(&socket, "arkham"), "ok arkham");
        refresh.take_heard(a_year);
        assert_eq!(
            due_by(&mut refresh, a_year),
            Some(Wanted::Project("arkham".to_string()))
        );
    }

    /// A deferred bead coming due writes nothing to its tracker, so a
    /// project that does not poll is read again when its read stops
    /// speaking for its tracker.
    #[test]
    fn a_read_that_stops_speaking_is_made_again_when_it_does() {
        let cfg = arkham("poll = false", "", &a_socket("speaks-until"));
        let start = Utc::now();
        let (mut refresh, _socket) = refreshing_as(&cfg, Polling::AsConfigured);
        assert_eq!(due_by(&mut refresh, start), Some(Wanted::Everything));
        let speaks_until =
            BTreeMap::from([("arkham".to_string(), start + chrono::TimeDelta::minutes(2))]);
        refresh.came_back(&["arkham".to_string()], &speaks_until, &[], start);
        assert_eq!(
            due_by(&mut refresh, start + chrono::TimeDelta::minutes(1)),
            None
        );
        assert_eq!(
            due_by(&mut refresh, start + chrono::TimeDelta::minutes(2)),
            Some(Wanted::Project("arkham".to_string()))
        );
    }

    /// One project, arkham, whose entry and `[changes]` table say what
    /// `entry` and `changes` say, with producers reporting at `socket`.
    fn arkham(entry: &str, changes: &str, socket: &Path) -> Config {
        Config::from_toml(&format!(
            "[changes]\nsocket = \"{}\"\n{changes}\n\n\
             [[projects]]\nname = \"arkham\"\npath = \"/home/user/dev/arkham\"\n{entry}\n",
            socket.display()
        ))
        .expect("parses")
    }

    /// A socket path no other test takes.
    fn a_socket(named: &str) -> PathBuf {
        crate::collect::changes::testing::socket_temp_dir()
            .join(format!("web-{}-{named}.sock", std::process::id()))
    }

    /// The reads `bdi web` makes of `cfg` as `polling` says, hearing
    /// producers where the config says, and passing nothing on.
    fn refreshing_as(cfg: &Config, polling: Polling) -> (Refreshing, Option<Socket>) {
        let (told, heard) = mpsc::channel();
        refreshing_for(
            cfg,
            polling,
            cfg.changes.socket.clone(),
            told,
            heard,
            |_| {},
        )
    }

    /// The read due by `at`, once the window any ask waits out is over.
    fn due_by(refresh: &mut Refreshing, at: DateTime<Utc>) -> Option<Wanted> {
        refresh
            .due(at)
            .or_else(|| refresh.due(at + chrono::TimeDelta::seconds(1)))
    }

    /// Whether arkham is read again a refresh interval after its first read
    /// comes back, with `said` heard in between.
    fn read_again(cfg: &Config, polling: Polling, said: &[&str]) -> bool {
        let start = Utc::now();
        let (mut refresh, _socket) = refreshing_as(cfg, polling);
        assert_eq!(due_by(&mut refresh, start), Some(Wanted::Everything));
        refresh.came_back(&["arkham".to_string()], &BTreeMap::new(), &[], start);
        let at = cfg.changes.socket.as_deref().expect("a socket");
        for line in said {
            assert_eq!(say(at, line), format!("ok arkham"));
        }
        refresh.take_heard(start + chrono::TimeDelta::seconds(1));
        let interval = chrono::TimeDelta::from_std(cfg.tui.refresh()).expect("an interval");
        due_by(
            &mut refresh,
            start + interval + chrono::TimeDelta::seconds(2),
        )
        .is_some()
    }

    /// Say `line` on the socket at `at` as a producer does, and hand back
    /// the answer.
    fn say(at: &Path, line: &str) -> String {
        use std::io::{BufRead, BufReader, Write};
        let mut to = std::os::unix::net::UnixStream::connect(at).expect("the socket takes it");
        writeln!(to, "{line}").expect("written");
        let mut answer = String::new();
        BufReader::new(to).read_line(&mut answer).expect("answered");
        answer.trim_end().to_string()
    }

    /// Arkham's tracker, holding one open bead, so the page draws its
    /// heading.
    fn arkhams_tracker() -> Fakes {
        let beads = bd::parse_beads(
            r#"[{"id":"ark-1","title":"open the gate","status":"open",
                 "priority":2,"issue_type":"task"}]"#,
        )
        .expect("the rows parse");
        Fakes::default().with("arkham", Fake::holding(beads))
    }

    /// `--poll` polls a project whose config says it does not, as the
    /// terminal view does.
    #[test]
    fn under_poll_a_project_that_does_not_poll_is_polled() {
        let cfg = arkham("poll = false", "", &a_socket("under-poll"));
        assert!(read_again(&cfg, Polling::Everything, &[]));
        assert!(!read_again(&cfg, Polling::AsConfigured, &[]));
    }

    /// `--no-poll` leaves unpolled a project whose config says it polls, as
    /// the terminal view does.
    #[test]
    fn under_no_poll_a_project_that_polls_is_left_unpolled() {
        let cfg = arkham("poll = true", "", &a_socket("under-no-poll"));
        assert!(!read_again(&cfg, Polling::Nothing, &[]));
        assert!(read_again(&cfg, Polling::AsConfigured, &[]));
    }

    /// A project that says nothing about `poll` polls until a producer says
    /// it covers the project, and then stops, as the terminal view's does.
    #[test]
    fn a_project_that_leaves_poll_unset_stops_polling_once_a_producer_covers_it() {
        let cfg = arkham("", "", &a_socket("covered"));
        assert!(read_again(&cfg, Polling::AsConfigured, &[]));
        assert!(!read_again(
            &cfg,
            Polling::AsConfigured,
            &["covered arkham"]
        ));
    }

    /// What a producer says on the socket is passed on to the watcher this
    /// run reads through, as the terminal view passes it on.
    #[test]
    fn what_a_producer_says_is_passed_on_before_it_is_read() {
        let socket = a_socket("passed-on");
        let cfg = arkham("poll = false", "", &socket);
        let passed_on = Arc::new(Mutex::new(Vec::new()));
        let passing = Arc::clone(&passed_on);
        let (told, heard) = mpsc::channel();
        let (mut refresh, _socket) = refreshing_for(
            &cfg,
            Polling::AsConfigured,
            Some(socket.clone()),
            told,
            heard,
            move |heard| {
                passing.lock().expect("held").push(heard.clone());
            },
        );
        assert_eq!(say(&socket, "arkham"), "ok arkham");
        assert_eq!(
            *passed_on.lock().expect("held"),
            vec![Heard::Changed("arkham".to_string())]
        );
        refresh.take_heard(Utc::now());
        assert_eq!(
            due_by(&mut refresh, Utc::now()),
            Some(Wanted::Everything),
            "the first read, with arkham's report folded into it"
        );
    }

    /// A project that does not poll says it has lapsed on its heading once
    /// `[changes] covered_for_seconds` passes with nothing vouching for it,
    /// as the terminal view's line does.
    #[test]
    fn a_project_nothing_vouches_for_lapses_on_the_page_after_covered_for_seconds() {
        let cfg = arkham(
            "poll = false",
            "covered_for_seconds = 60",
            &a_socket("lapses"),
        );
        let start = Utc::now();
        let (mut refresh, _socket) = refreshing_as(&cfg, Polling::AsConfigured);
        let latest = Latest::new();
        let trackers = arkhams_tracker();
        let mut collection = Collection::default();
        let mut at = |seconds| {
            read_what_is_due(
                &cfg,
                &latest,
                &trackers,
                &mut refresh,
                &mut collection,
                start + chrono::TimeDelta::seconds(seconds),
            )
        };
        assert!(at(1), "the first read");
        at(59);
        assert!(latest.now().lapsed.is_empty(), "not yet");
        at(62);
        assert_eq!(latest.now().lapsed, ["arkham"]);
        let page = drawn(&Asked::of("/").expect("the forest"), &latest.now(), &cfg).1;
        assert!(
            page.contains("<span class=\"mark warn\">?</span>"),
            "{page}"
        );
    }

    /// A producer's report on the socket this run watches, the one the
    /// config names or the one `--socket` names over it, has the page drawn
    /// again from a fresh read of the project.
    #[test]
    fn a_report_on_the_socket_this_run_watches_redraws_the_page() {
        for (named, told) in [("configured", false), ("told", true)] {
            let configured = a_socket(&format!("{named}-configured"));
            let other = a_socket(&format!("{named}-told"));
            let cfg = arkham("poll = false", "", &configured);
            let mut args = vec!["bdi", "web", "--listen", "127.0.0.1:0"];
            let other_arg = other.display().to_string();
            if told {
                args.extend(["--socket", other_arg.as_str()]);
            }
            let cli = super::super::Cli::parse_from(args);
            let Some(super::super::Passing::Web { view, .. }) = &cli.passing else {
                panic!("bdi web parsed as something else");
            };
            let at = super::super::where_producers_find_this_run(view, &cfg);
            let reported_at = if told { &other } else { &configured };
            assert_eq!(at.as_ref(), Some(reported_at));

            let (told_by, heard) = mpsc::channel();
            let (refresh, socket) =
                refreshing_for(&cfg, Polling::AsConfigured, at, told_by, heard, |_| {});
            let latest = Arc::new(Latest::new());
            let trackers = Arc::new(arkhams_tracker());
            {
                let (cfg, latest, trackers) =
                    (cfg.clone(), Arc::clone(&latest), Arc::clone(&trackers));
                // ponytail: left blocked on the socket once the test is done,
                // since nothing polls and the socket's writer is never dropped.
                thread::spawn(move || keep_reading(&cfg, &latest, &*trackers, refresh));
            }
            let first = held_once(&latest, None, |held| {
                held.snapshot.is_some() && held.collecting.is_none()
            });
            let asked = trackers.tracker("arkham").asked().len();

            assert_eq!(say(reported_at, "arkham"), "ok arkham");
            held_once(&latest, Some(first), |held| {
                held.snapshot.is_some() && held.collecting.is_none()
            });
            assert!(
                trackers.tracker("arkham").asked().len() > asked,
                "arkham's tracker was read again for the report on {}",
                reported_at.display()
            );
            drop(socket);
        }
    }

    /// What `latest` holds once `holds` says so of a change after `seen`,
    /// and what a page drawn from it is drawn from.
    fn held_once(
        latest: &Latest,
        seen: Option<DrawnFrom>,
        holds: impl Fn(&Held) -> bool,
    ) -> DrawnFrom {
        let until = Instant::now() + Duration::from_secs(10);
        let mut seen = seen;
        loop {
            let (held, from) = latest
                .after(seen, until)
                .expect("what was waited for came within ten seconds");
            if holds(&held) {
                return from;
            }
            seen = Some(from);
        }
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
            lapsed: Vec::new(),
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
            lapsed: Vec::new(),
            change: 0,
        });
        let sent = "GET / HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let head = head(&mut sent.as_bytes(), &mut Vec::new()).expect("a head");
        let (status, _, body) = answered(&head, &latest, &cfg, &Hosts::default());
        assert_eq!(status, "503 Service Unavailable");
        let line = &body[body.find(">arkham").expect("the project is named")..];
        let line = &line[..line.find("</div>").expect("the line ends")];
        assert!(
            line.contains(&format!(
                "aria-label=\"{}\"",
                crate::view::phrase::not_answering()
            )),
            "{body}"
        );
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
