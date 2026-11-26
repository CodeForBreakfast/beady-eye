//! `bdi web`: the forest of the projects it reads as a read-only web page,
//! and each bead at an address of its own, answered over HTTP.
//!
//! One thread keeps the latest snapshot, reading the trackers as a view does,
//! through the watcher where one answers. Each request is drawn afresh from
//! that snapshot, so what a request costs is the drawing and never a read.

use std::io::Read;
use std::net::{IpAddr, Ipv6Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, TryLockError};
use std::time::{Duration, Instant};

use anyhow::Context;
use chrono::{DateTime, Local, Offset, Utc};

use crate::app::{
    armed_unread, Arming, Awaited, Collection, Outstanding, Parses, Refreshing, Reload, Reloaded,
    CHECKED_EVERY,
};
use crate::collect::agents::{Agents, Unasked};
use crate::collect::bd;
use crate::collect::changes::{self, Heard, Report, Reported, Socket};
use crate::collect::environment::{self, EnvironmentCache};
use crate::collect::http::{self, close, head, Encoding, Head, Patient, Unbodied, Unread};
use crate::collect::run::{RealRunner, Runner};
use crate::collect::tracker::Trackers;
use crate::collect::watched::{self, Through};
use crate::config::Config;
use crate::model::snapshot::{Filter, Snapshot};
use crate::view::address::{self, Asked, Start, Tokens, COMMENT, TEXT as COMMENT_TEXT, TOKEN};
use crate::view::draw::html;
use crate::view::forest::listed;
use crate::view::{phrase, Notice};

use super::{
    arming, parses_as_launched, read_config, where_producers_find_this_run,
    where_this_machine_watches, Launch, Polling, Reading, ViewOptions,
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

/// The most of a write's form read: whatever the comment form lets through.
const LARGEST_WRITE: usize = address::LARGEST_COMMENT_FORM;

/// Which trees a page draws: every one, since `bdi web` asks no agent
/// provider and so cannot tell a tree with a live agent from one without.
/// It is what `--all` asks of the view, so `--all` changes nothing here.
pub(super) const EVERY_TREE: Filter = Filter::All;
const NOT_FOUND: &str = "404 Not Found";

const FORBIDDEN: &str = "403 Forbidden";

/// What the reading thread holds for the pages to be drawn from, and word
/// of each change to it for the open pages waiting on one.
struct Latest {
    held: Mutex<Held>,
    changed: Condvar,
}

#[derive(Clone)]
struct Held {
    /// The config the reading thread works to, which the pages are drawn
    /// to as well.
    cfg: Arc<Config>,
    /// The latest snapshot, where the trackers have been read at all.
    snapshot: Option<Snapshot>,
    /// The config the snapshot was read under, which a write to one of its
    /// beads goes by, since a config taken up since may name another
    /// tracker for the same project.
    read_under: Arc<Config>,
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
    fn new(cfg: Config) -> Self {
        let cfg = Arc::new(cfg);
        Self::holding(Held {
            read_under: Arc::clone(&cfg),
            cfg,
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

    /// The config held now.
    fn cfg(&self) -> Arc<Config> {
        Arc::clone(&self.held().cfg)
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
/// each named as `--help` names it: `--json`, `--beads` and the options that
/// narrow `--beads`, since `bdi web` prints nothing.
pub(super) fn refusals(view: &ViewOptions) -> Vec<String> {
    const BEADS_PRINTS: &str =
        "it narrows what --beads prints, and bdi web serves a page rather than printing";
    [
        (
            view.json,
            "--json",
            "bdi web serves a page rather than printing the snapshot",
        ),
        (
            view.each_bead,
            "--beads",
            "bdi web serves a page rather than printing each bead",
        ),
        (!view.labels.is_empty(), "--label", BEADS_PRINTS),
        (view.issue_type.is_some(), "--type", BEADS_PRINTS),
        (view.assignee.is_some(), "--assignee", BEADS_PRINTS),
        (view.parent.is_some(), "--parent", BEADS_PRINTS),
        (view.brief, "--brief", BEADS_PRINTS),
    ]
    .into_iter()
    .filter(|(given, _, _)| *given)
    .map(|(_, named, why)| format!("{named}: {why}"))
    .collect()
}

/// Which projects `bdi web` reads: the ones `view` names, as the view reads
/// them, and every configured project where it names none, since a page has
/// no directory it was started in.
pub(super) fn reading(view: &ViewOptions) -> Reading {
    match Reading::asked_for(view) {
        Reading::WhereBdiWasStarted => Reading::EveryProject,
        named => named,
    }
}

/// Answer requests on `listen` until a signal ends the process, drawing each
/// from the latest read of the projects `config` and `view` name, for
/// requests sent to a name `hosts` allows. Each project is polled, and heard
/// from, as `view` says. A write is taken only where `writable`, and every
/// other run refuses it.
///
/// The config is read again as the terminal view reads it, and an edit is
/// taken up a couple of seconds after it is saved.
pub(super) fn web(
    config: &Path,
    listen: &str,
    hosts: Hosts,
    writable: bool,
    view: &ViewOptions,
) -> anyhow::Result<ExitCode> {
    if writable {
        // Before any thread starts, so a write reaches the project's own
        // tracker as one through `bdi bd` does.
        environment::forget_the_callers_tracker();
    }
    let cwd = std::env::current_dir().context("finding the current directory")?;
    let cfg = addressable(
        read_config(
            &RealRunner,
            config,
            &Launch {
                cwd: &cwd,
                reading: reading(view),
                roots: &view.beads,
            },
        )?
        .config,
    )?;
    let writes_as = match writable {
        true => Some(cfg.web.actor()?.to_string()),
        false => None,
    };
    let start = Start {
        list: listed(None, view.ready),
        roots: cfg.roots.named_beads(),
    };
    let reload = Reload::watching(
        config.to_path_buf(),
        CHECKED_EVERY,
        cfg.clone(),
        each_addressable(parses_as_launched(cwd, reading(view), view.beads.clone())),
        Utc::now(),
    );
    let listener = TcpListener::bind(listen).with_context(|| format!("listening on {listen}"))?;
    let taken = listener
        .local_addr()
        .with_context(|| format!("reading the address {listen} gave"))?;
    println!("bdi web is answering on http://{taken}");

    // `told` is held until the run ends, because a run with no watcher and no
    // socket has nobody else to keep the reading loop waiting.
    let (told, heard) = mpsc::channel();
    let trackers = Arc::new(Through::staying(
        where_this_machine_watches(&cfg).as_deref(),
        cfg.read().map(|project| project.name.as_str()),
        bd::Cli::new(&RealRunner).caching_environments(EnvironmentCache::here()),
        told.clone(),
        cfg.tui.refresh(),
    ));
    let passing_on = Arc::clone(&trackers);
    let arms = arming(Polling::asked_for(&view.polling));
    // Held, not discarded: the socket comes off the filesystem when this
    // returns.
    let (refreshing, _socket) = refreshing_for(
        &cfg,
        &arms,
        where_producers_find_this_run(view, &cfg),
        told.clone(),
        heard,
        move |heard| passing_on.passes_on(heard.clone()),
    );

    let writes = match writes_as {
        Some(actor) => {
            let tokens = Tokens::new().context("reading a key for the write forms' tokens")?;
            Some(Writer::through(&RealRunner, actor, tokens))
        }
        None => None,
    };
    let latest = Arc::new(Latest::new(cfg));
    let answering = Arc::new(Answering {
        latest: Arc::clone(&latest),
        hosts,
        start,
        writes,
        live: live::Streams::default(),
    });
    http::accept(
        listener,
        MOST_AT_ONCE,
        |stream| reply(stream, UNAVAILABLE, TEXT, b"busy\n"),
        move |stream| answer(stream, &answering),
    );
    keep_reading(&latest, &*trackers, refreshing, reload, &arms);
    drop(told);
    Ok(ExitCode::SUCCESS)
}

/// `cfg`, where every project it reads has an address a browser keeps.
fn addressable(cfg: Config) -> anyhow::Result<Config> {
    if let Some(project) = cfg
        .read()
        .map(|project| project.name.as_str())
        .find(|name| !address::addressable(name))
    {
        anyhow::bail!("bdi web cannot give the project {project:?} an address a browser keeps; give it another name in the config");
    }
    Ok(cfg)
}

/// What `parses` reads, refused where a project it reads has no address.
fn each_addressable(parses: Parses) -> Parses {
    Box::new(move |text| addressable(parses(text)?))
}

/// What every request is answered from.
struct Answering {
    latest: Arc<Latest>,
    hosts: Hosts,
    /// What a page shows where its address does not say.
    start: Start,
    /// What makes a write, where this run takes writes.
    writes: Option<Writer<'static>>,
    live: live::Streams,
}

/// What makes a write: the runner bd is run through, one write at a time, so
/// a tracker that stalls holds one of the request slots and not all of them,
/// the actor bd records every write under, and the tokens each write's form
/// carries.
struct Writer<'a> {
    runner: &'a dyn Runner,
    actor: String,
    one_at_a_time: Mutex<()>,
    tokens: Tokens,
}

impl<'a> Writer<'a> {
    fn through(runner: &'a dyn Runner, actor: String, tokens: Tokens) -> Self {
        Self {
            runner,
            actor,
            one_at_a_time: Mutex::new(()),
            tokens,
        }
    }
}

/// The reads `bdi web` makes of `cfg`, as the terminal view makes them: each
/// project polled as `arms` arms it, read again when the watcher says on
/// `heard` that it changed, and heard from producers on the socket `at`.
/// What a producer says is handed to `passing_on`, then told on `told`.
///
/// The socket comes back beside the reads, and comes off the filesystem
/// when it is dropped. Nothing where it could not be opened, which is said
/// on stderr where it is worth saying, as the terminal view says it.
fn refreshing_for(
    cfg: &Config,
    arms: &Arming,
    at: Option<PathBuf>,
    told: Sender<Heard>,
    heard: Receiver<Heard>,
    passing_on: impl Fn(&Heard) + Send + Sync + 'static,
) -> (Refreshing, Option<Socket>) {
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
        crate::app::Reading::of(armed, reported).unread(armed_unread(arms, cfg)),
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
/// snapshot and the projects that have lapsed as the latest. A config the
/// reader writes where `reload` looks is held and read under from then on,
/// each project armed by `arms`. Returns only once nothing can be heard any
/// longer.
fn keep_reading(
    latest: &Latest,
    trackers: &dyn Trackers,
    mut refreshing: Refreshing,
    mut reload: Reload,
    arms: &Arming,
) {
    let mut collection = Collection::default();
    let mut broken = false;
    loop {
        let now = Utc::now();
        taken_up(latest, &mut refreshing, &mut reload, arms, &mut broken, now);
        if read_what_is_due(latest, trackers, &mut refreshing, &mut collection, now) {
            continue;
        }
        if !refreshing.wait(now, reload.checks_in(now)) {
            return;
        }
    }
}

/// Look at the config file where the check is due, and take up a config the
/// reader has written: held for the pages, and read under from here on.
///
/// A config that will not load leaves the one in force standing, as in the
/// terminal view, and is said on stderr, which is where a server's reader
/// looks. `broken` is whether the last check that said anything found it so.
fn taken_up(
    latest: &Latest,
    refreshing: &mut Refreshing,
    reload: &mut Reload,
    arms: &Arming,
    broken: &mut bool,
    now: DateTime<Utc>,
) {
    let reloaded = reload.checks(now);
    if newly_broken(&reloaded, broken) {
        eprintln!("bdi web: {}", phrase::notice(&Notice::ConfigWouldNotReload));
    }
    if let Reloaded::Fresh(written) = reloaded {
        refreshing.reads_under(written, arms, now);
        let written = Arc::new(written.clone());
        latest.change(|held| held.cfg = written);
    }
}

/// Whether `reloaded` says the config will not load where `broken`, the
/// last check that said anything, did not.
fn newly_broken(reloaded: &Reloaded, broken: &mut bool) -> bool {
    match reloaded {
        Reloaded::Broken => !std::mem::replace(broken, true),
        Reloaded::Fresh(_) | Reloaded::Unchanged => {
            *broken = false;
            false
        }
        Reloaded::Untouched => false,
    }
}

/// Take what has been heard by `now`, hold the projects that have lapsed,
/// and make the read whose turn has come, holding what it finds. Whether
/// one had.
fn read_what_is_due(
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
    let cfg = latest.cfg();
    latest.change(|held| {
        held.collecting = Some(Awaited {
            wanted: wanted.clone(),
            asked_at: now,
            patience: cfg.tui.unanswered_after(),
        });
    });
    let snapshot = collection.collect(&cfg, &Unasked, trackers, &wanted, EVERY_TREE, now);
    refreshing.came_back(
        &snapshot.projects,
        &snapshot.speaks_until,
        &snapshot.failed_projects,
        Utc::now(),
    );
    latest.change(|held| {
        held.snapshot = Some(snapshot);
        held.read_under = cfg;
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
    let mut held = Vec::new();
    let read = head(&mut stream, &mut held);
    let encoding = read.as_ref().map_or(Encoding::Identity, Head::encoding);
    if let Ok(head) = &read {
        let kept_current = answering.hosts.allow(head.only("Host")) && live::asks_for_changes(head);
        let asked = kept_current.then(|| Asked::started_on(&head.target, &answering.start));
        if let Some(asked) = asked.flatten() {
            live::keep_current(answering, stream.stream, head, asked);
            return;
        }
    }
    let Some(reply) = said(
        read,
        &mut stream,
        held,
        &answering.latest,
        &answering.hosts,
        &answering.start,
        answering.writes.as_ref(),
    ) else {
        return;
    };
    answering.live.remember(&reply.body);
    http::reply_in(
        &stream.stream,
        reply.status,
        reply.kind,
        reply.body.as_bytes(),
        encoding,
        &headers(&reply, answering.writes.is_some()),
        PATIENCE,
    );
    close(stream.stream, stream.deadline);
}

/// The header lines `reply` is sent with: where it sends the reader on, and,
/// from a run that takes writes, that a kept copy is asked for again before
/// it is shown, since a page carries tokens a later run will not take. The
/// copy is still kept, because the page's script reads the page from it.
fn headers(reply: &Reply, writable: bool) -> String {
    let sent_on = reply
        .sent_to
        .as_ref()
        .map(|to| format!("Location: {to}\r\n"))
        .unwrap_or_default();
    let kept = if writable {
        "Cache-Control: no-cache\r\n"
    } else {
        ""
    };
    sent_on + kept
}

/// What a request is answered: its status, the kind of body and the body,
/// and the address the reader is sent on to, where they are sent on.
struct Reply {
    status: &'static str,
    kind: &'static str,
    body: String,
    sent_to: Option<String>,
}

impl From<(&'static str, &'static str, String)> for Reply {
    fn from((status, kind, body): (&'static str, &'static str, String)) -> Self {
        Self {
            status,
            kind,
            body,
            sent_to: None,
        }
    }
}

/// What a request whose head was `read` into `held` comes to, or nothing
/// where it never arrived in full or is not HTTP. A page shows what `start`
/// says where its address does not say. A post sent to an allowed name is a
/// write, whose form is read on from `rest`, and only a run with a writer in
/// `writes` takes one.
fn said(
    read: Result<Head, Unread>,
    rest: &mut impl Read,
    held: Vec<u8>,
    latest: &Latest,
    hosts: &Hosts,
    start: &Start,
    writes: Option<&Writer>,
) -> Option<Reply> {
    let head = match read {
        Ok(head) => head,
        Err(Unread::TooLong) => {
            return Some(
                (
                    "414 URI Too Long",
                    HTML,
                    html::saying(phrase::address_too_long()),
                )
                    .into(),
            )
        }
        Err(Unread::Unreadable) => return None,
    };
    match writes {
        Some(writer) if head.method == "POST" && hosts.allow(head.only("Host")) => {
            if !from_its_own_page(&head) {
                let said = phrase::posted_from_elsewhere(
                    head.only("Origin"),
                    head.only("Host").unwrap_or_default(),
                );
                return Some((FORBIDDEN, HTML, html::saying(&said)).into());
            }
            let form = match http::body(rest, held, &head, LARGEST_WRITE) {
                Ok(form) => form,
                Err(Unbodied::Unmeasured) => {
                    return Some(
                        ("411 Length Required", TEXT, "length required\n".to_string()).into(),
                    )
                }
                Err(Unbodied::TooLarge) => {
                    return Some(("413 Content Too Large", TEXT, "too large\n".to_string()).into())
                }
                Err(Unbodied::Unreadable) => return None,
            };
            if let Some(refused) = not_issued(&head, &form, &writer.tokens) {
                return Some(refused);
            }
            let _making = match writer.one_at_a_time.try_lock() {
                Ok(making) => making,
                Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
                Err(TryLockError::WouldBlock) => {
                    return Some(
                        (
                            UNAVAILABLE,
                            TEXT,
                            "another write is being made\n".to_string(),
                        )
                            .into(),
                    )
                }
            };
            let held = latest.now();
            Some(taken(&head, &form, &held, writer))
        }
        _ => {
            let tokens = writes.map(|writer| writer.tokens);
            Some(answered(&head, latest, hosts, start, tokens).into())
        }
    }
}

/// Whether a post whose head is `head` came from a page at the address it
/// was sent to: its one Origin names the host and port its one Host does. A
/// browser sends an Origin with every form it posts, and a page on another
/// site cannot send this listener's.
fn from_its_own_page(head: &Head) -> bool {
    match (head.only("Origin"), head.only("Host")) {
        (Some(origin), Some(host)) => origin
            .split_once("://")
            .is_some_and(|(_, at)| at.eq_ignore_ascii_case(host)),
        _ => false,
    }
}

/// A write a post asks of a bead's tracker.
enum Write {
    /// Add the text to the bead as a comment.
    Comment(String),
}

impl Write {
    /// Whether a page offers a write named `named`, with a form of its own.
    fn offered(named: &str) -> bool {
        named == COMMENT
    }

    /// The write a post to an address ending in `named` asks for with the
    /// form `form`, or nothing where it asks for none.
    fn asked(named: &str, form: &str) -> Option<Self> {
        match named {
            COMMENT => address::field(form, COMMENT_TEXT)
                .filter(|text| !text.trim().is_empty())
                .map(Self::Comment),
            _ => None,
        }
    }

    /// What `bdi bd` is asked for to make this write to `bead`.
    fn words(self, bead: &str) -> Vec<String> {
        match self {
            Self::Comment(text) => ["comments", "add", bead, "--", &text]
                .map(String::from)
                .to_vec(),
        }
    }
}

/// The refusal of a post to a write's address whose form carries no token
/// `tokens` issued for that write, which links back to the bead's page for
/// a fresh form. Nothing for a post whose form carries one, or for a post to
/// an address that names no write a page offers.
fn not_issued(head: &Head, form: &[u8], tokens: &Tokens) -> Option<Reply> {
    let (bead, named) = address::written(head.path()).filter(|(_, named)| Write::offered(named))?;
    let issued = std::str::from_utf8(form)
        .ok()
        .and_then(|form| address::field(form, TOKEN))
        .is_some_and(|token| tokens.issued(&token, &bead, named));
    (!issued).then(|| {
        let back = address::written_back(&bead, &head.target);
        let said = html::saying_back_to(phrase::form_not_issued(), &back);
        (FORBIDDEN, HTML, said).into()
    })
}

/// What a post whose head is `head` and whose form is `form` comes to: the
/// write it asks for made through `bdi bd` by `writer` to a bead `held`
/// draws, and the reader sent back to the bead's page. Otherwise, why the
/// write was not made.
fn taken(head: &Head, form: &[u8], held: &Held, writer: &Writer) -> Reply {
    let Some((bead, named)) = address::written(head.path()) else {
        return (NOT_FOUND, HTML, html::saying(phrase::no_page_here())).into();
    };
    let form = std::str::from_utf8(form).ok();
    let runner = writer.runner;
    let Some(snapshot) = &held.snapshot else {
        return (UNAVAILABLE, HTML, html::saying(phrase::not_read_yet())).into();
    };
    // Any project the config names, since a scope widens to read a blocker
    // and the bead is drawn either way.
    let Some(project) = held
        .read_under
        .projects
        .iter()
        .find(|project| project.name == bead.project)
    else {
        return (NOT_FOUND, HTML, html::saying(phrase::no_page_here())).into();
    };
    // bd takes the start of an id, and routes an id it does not hold to
    // another tracker, so only an id the page draws names the bead exactly.
    if snapshot.node(&bead).is_none() {
        return (NOT_FOUND, HTML, html::saying(phrase::no_page_here())).into();
    }
    let Some(write) = form.and_then(|form| Write::asked(named, form)) else {
        return (
            "400 Bad Request",
            TEXT,
            "no write in the form\n".to_string(),
        )
            .into();
    };
    let words = write.words(&bead.id);
    let made = bd::writing(runner, project, &words, Some(&writer.actor)).and_then(|(argv, env)| {
        let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
        Ok(runner.run("bd", &argv, Some(&project.path), &env)?)
    });
    match made {
        Ok(_) => Reply {
            status: "303 See Other",
            kind: TEXT,
            body: String::new(),
            sent_to: Some(address::written_back(&bead, &head.target)),
        },
        Err(why) => (
            "502 Bad Gateway",
            HTML,
            html::saying(&phrase::not_written(&format!("{why:#}"))),
        )
            .into(),
    }
}

/// What a request comes to: its status, the kind of body, and the body. A
/// page offers writes with the tokens `writes` issues, where it issues any.
fn answered(
    head: &Head,
    latest: &Latest,
    hosts: &Hosts,
    start: &Start,
    writes: Option<Tokens>,
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
    let Some(asked) = Asked::started_on(&head.target, start) else {
        return (NOT_FOUND, HTML, html::saying(phrase::no_page_here()));
    };
    let (status, page) = drawn(&asked, &held, writes);
    (status, HTML, page)
}

/// The page `asked` names, drawn from `held`, and its status, offering
/// writes with the tokens `writes` issues, where it issues any. A page
/// started on the command line's roots is drawn at them as the held config
/// places them, since an edit can move them after the page was asked for.
fn drawn(asked: &Asked, held: &Held, writes: Option<Tokens>) -> (&'static str, String) {
    let cfg = &held.cfg;
    let asked = &Asked {
        start: Start {
            roots: cfg.roots.named_beads(),
            ..asked.start.clone()
        },
        ..asked.clone()
    };
    let drawn = html::Drawn {
        at: Utc::now(),
        offset: Local::now().offset().fix(),
        collecting: Vec::from_iter(held.collecting.clone()),
        lapsed: held.lapsed.clone(),
        writes,
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
            EVERY_TREE,
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
    use crate::collect::run::testing::FakeRunner;
    use crate::collect::run::RunFailure;
    use crate::collect::tracker::testing::{Fake, Fakes};
    use crate::model::join::BeadKey;

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
        let reply = replied(&sent, latest, None);
        (reply.status, reply.body)
    }

    /// What the request `sent` is answered, with `latest` held, by a run
    /// whose writes `writes` runs, where it takes them.
    fn replied(sent: &str, latest: &Latest, writes: Option<&dyn Runner>) -> Reply {
        replied_by(
            sent,
            latest,
            writes
                .map(|runner| Writer::through(runner, ACTOR.to_string(), tokens()))
                .as_ref(),
        )
    }

    /// The tokens a test's run issues.
    fn tokens() -> Tokens {
        Tokens::keyed([7; 32])
    }

    /// The same, by a run whose writes `writer` makes.
    fn replied_by(sent: &str, latest: &Latest, writes: Option<&Writer>) -> Reply {
        let mut arriving = sent.as_bytes();
        let mut held = Vec::new();
        let read = head(&mut arriving, &mut held);
        said(
            read,
            &mut arriving,
            held,
            latest,
            &allowed(),
            &Start::default(),
            writes,
        )
        .expect("an answer")
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
        let reply = said(
            head(&mut arriving, &mut Vec::new()),
            &mut std::io::empty(),
            Vec::new(),
            &read(),
            &Hosts::default(),
            &Start::default(),
            None,
        )
        .expect("an answer");
        assert_eq!(reply.status, "414 URI Too Long");
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
        Config::from_toml(&arkham_written(entry, changes, socket)).expect("parses")
    }

    /// The text of the config [`arkham`] reads.
    fn arkham_written(entry: &str, changes: &str, socket: &Path) -> String {
        format!(
            "[changes]\nsocket = \"{}\"\n{changes}\n\n\
             [[projects]]\nname = \"arkham\"\npath = \"/home/user/dev/arkham\"\n{entry}\n",
            socket.display()
        )
    }

    /// A reload that never falls due, so a run reads under `cfg` alone.
    fn never_reloading(cfg: &Config) -> Reload {
        Reload::watching(
            PathBuf::new(),
            Duration::MAX,
            cfg.clone(),
            Box::new(Config::from_toml),
            Utc::now(),
        )
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
            &arming(polling),
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
            assert_eq!(say(at, line), "ok arkham");
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
            &arming(Polling::AsConfigured),
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
        let latest = Latest::new(cfg);
        let trackers = arkhams_tracker();
        let mut collection = Collection::default();
        let mut at = |seconds| {
            read_what_is_due(
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
        let page = drawn(&Asked::of("/").expect("the forest"), &latest.now(), None).1;
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
            let (refresh, socket) = refreshing_for(
                &cfg,
                &arming(Polling::AsConfigured),
                at,
                told_by,
                heard,
                |_| {},
            );
            let latest = Arc::new(Latest::new(cfg.clone()));
            let trackers = Arc::new(arkhams_tracker());
            {
                let (cfg, latest, trackers) =
                    (cfg.clone(), Arc::clone(&latest), Arc::clone(&trackers));
                // ponytail: left blocked on the socket once the test is done,
                // since nothing polls and the socket's writer is never dropped.
                thread::spawn(move || {
                    keep_reading(
                        &latest,
                        &*trackers,
                        refresh,
                        never_reloading(&cfg),
                        &arming(Polling::AsConfigured),
                    )
                });
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

    /// A project the reader writes into the config is read and drawn on the
    /// page once the file is next looked at, with no restart, as the
    /// terminal view takes up an edit.
    #[test]
    fn a_project_written_into_the_config_is_drawn_without_a_restart() {
        let before = arkham_written("poll = false", "", &a_socket("edited"));
        let path = std::env::temp_dir().join(format!("bdi-web-{}-edited.toml", std::process::id()));
        std::fs::write(&path, &before).expect("the config is ours to write");
        let cfg = Config::from_toml(&before).expect("parses");
        let dunwich = bd::parse_beads(
            r#"[{"id":"dun-1","title":"lay the feeder cable","status":"open",
                 "priority":2,"issue_type":"task"}]"#,
        )
        .expect("the rows parse");
        let trackers = Arc::new(arkhams_tracker().with("dunwich", Fake::holding(dunwich)));
        // Kept here, so the reading thread ends once the test drops it.
        let (told, heard) = mpsc::channel();
        let (refresh, _) = refreshing_for(
            &cfg,
            &arming(Polling::AsConfigured),
            None,
            told.clone(),
            heard,
            |_| {},
        );
        let latest = Arc::new(Latest::new(cfg.clone()));
        let reading = {
            let (path, latest, trackers) =
                (path.clone(), Arc::clone(&latest), Arc::clone(&trackers));
            thread::spawn(move || {
                let reload = Reload::watching(
                    path,
                    Duration::from_millis(20),
                    cfg,
                    Box::new(Config::from_toml),
                    Utc::now(),
                );
                keep_reading(
                    &latest,
                    &*trackers,
                    refresh,
                    reload,
                    &arming(Polling::AsConfigured),
                );
            })
        };
        let reads = |project: &'static str| {
            move |held: &Held| {
                held.collecting.is_none()
                    && held
                        .snapshot
                        .as_ref()
                        .is_some_and(|read| read.projects.iter().any(|named| named == project))
            }
        };
        let first = held_once(&latest, None, reads("arkham"));
        let forest = Asked::of("/").expect("the forest");
        assert!(!drawn(&forest, &latest.now(), None).1.contains(">dunwich"));

        std::fs::write(
            &path,
            format!("{before}\n[[projects]]\nname = \"dunwich\"\npath = \"/srv/work/dunwich\"\npoll = false\n"),
        )
        .expect("the config is ours to write");

        held_once(&latest, Some(first), reads("dunwich"));
        let page = drawn(&forest, &latest.now(), None).1;
        assert!(page.contains(">dunwich"), "{page}");
        assert!(page.contains("lay the feeder cable"), "{page}");
        drop(told);
        reading.join().expect("the reading thread ended");
        std::fs::remove_file(&path).expect("the config is ours to remove");
    }

    /// The page asks no agent provider, so a claimed bead's page says its
    /// agent is unknown rather than that no agent is on it.
    #[test]
    fn a_claimed_beads_page_says_its_agent_is_unknown_rather_than_absent() {
        let cfg =
            Config::from_toml("[[projects]]\nname = \"arkham\"\npath = \"/home/user/arkham\"\n")
                .expect("a config");
        let claimed = bd::parse_beads(
            r#"[{"id":"ark-1","title":"open the gate","status":"in_progress",
                 "priority":2,"issue_type":"task","metadata":{"agent_pane":"wG:p2"}}]"#,
        )
        .expect("the rows parse");
        let trackers = Fakes::default().with("arkham", Fake::holding(claimed));
        let snapshot = Collection::default().collect(
            &cfg,
            &Unasked,
            &trackers,
            &Wanted::Everything,
            Filter::All,
            Utc::now(),
        );
        let latest = Latest::holding(Held {
            read_under: Arc::new(cfg.clone()),
            cfg: Arc::new(cfg),
            snapshot: Some(snapshot),
            collecting: None,
            lapsed: Vec::new(),
            change: 0,
        });
        let (status, page) = answered_to("GET", "/arkham/ark-1", &latest);
        assert_eq!(status, "200 OK");
        assert!(!page.contains("no agent on it"), "{page}");
        assert!(
            page.contains(&format!(
                "<dt>{}</dt><dd>{}</dd>",
                phrase::field::AGENT,
                phrase::agent_unknown()
            )),
            "{page}"
        );
    }

    /// A page started on the roots the command line named takes those
    /// roots from the config in force, so a page opened before an edit
    /// moved them is drawn at where they are now.
    #[test]
    fn a_page_is_rooted_where_the_config_in_force_places_its_roots() {
        let cfg = Config::from_toml(
            "[[projects]]\nname = \"arkham\"\npath = \"/home/user/arkham\"\n\
             [[projects]]\nname = \"dunwich\"\npath = \"/srv/work/dunwich\"\n",
        )
        .expect("a config")
        .with_roots_named_on_the_command_line(&["dunwich:dun-1".to_string()])
        .expect("dunwich is read");
        let dunwich = bd::parse_beads(
            r#"[{"id":"dun-1","title":"lay the feeder cable","status":"open",
                 "priority":2,"issue_type":"task"}]"#,
        )
        .expect("the rows parse");
        let trackers = arkhams_tracker().with("dunwich", Fake::holding(dunwich));
        let snapshot = Collection::default().collect(
            &cfg,
            &Unasked,
            &trackers,
            &Wanted::Everything,
            Filter::All,
            Utc::now(),
        );
        let held = Held {
            read_under: Arc::new(cfg.clone()),
            cfg: Arc::new(cfg),
            snapshot: Some(snapshot),
            collecting: None,
            lapsed: Vec::new(),
            change: 0,
        };
        let as_launched = Start {
            list: None,
            roots: vec![BeadKey {
                project: "arkham".to_string(),
                id: "ark-1".to_string(),
            }],
        };
        let asked = Asked::started_on("/", &as_launched).expect("the forest");
        let page = drawn(&asked, &held, None).1;
        assert!(page.contains("lay the feeder cable"), "{page}");
        assert!(!page.contains("open the gate"), "{page}");
    }

    /// A config that stays unloadable, such as one that has gone from its
    /// path, is said once rather than at every check, and again only once
    /// it has loaded in between.
    #[test]
    fn a_config_that_will_not_load_is_said_once_until_it_loads() {
        let mut broken = false;
        assert!(newly_broken(&Reloaded::Broken, &mut broken));
        assert!(!newly_broken(&Reloaded::Broken, &mut broken));
        assert!(!newly_broken(&Reloaded::Untouched, &mut broken));
        assert!(!newly_broken(&Reloaded::Broken, &mut broken));
        assert!(!newly_broken(&Reloaded::Unchanged, &mut broken));
        assert!(newly_broken(&Reloaded::Broken, &mut broken));
    }

    /// An edit naming a project with no address a browser keeps is refused
    /// as a config that will not load, as it is at startup.
    #[test]
    fn a_config_naming_a_project_with_no_address_will_not_load() {
        let parses = each_addressable(Box::new(Config::from_toml));
        assert!(parses("[[projects]]\nname = \"arkham\"\npath = \"/home/user/arkham\"\n").is_ok());
        assert!(parses("[[projects]]\nname = \"..\"\npath = \"/home/user/arkham\"\n").is_err());
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
            read_under: Arc::new(cfg.clone()),
            cfg: Arc::new(cfg),
            snapshot: Some(snapshot),
            collecting: None,
            lapsed: Vec::new(),
            change: 0,
        })
    }

    #[test]
    fn before_the_trackers_are_read_a_beads_page_says_so_and_the_probe_fails() {
        let latest = Latest::new(Config::naming(Vec::new()));
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
            cfg: Arc::new(cfg.clone()),
            read_under: Arc::new(cfg.clone()),
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
        let (status, _, body) =
            answered(&head, &latest, &Hosts::default(), &Start::default(), None);
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

    /// A tracker of arkham's, read from a directory nothing enters, so a
    /// write reaches bd with nothing run before it.
    fn arkham_held() -> Latest {
        let cfg =
            Config::from_toml("[[projects]]\nname = \"arkham\"\npath = \"/nowhere/arkham\"\n")
                .expect("a config");
        let snapshot = Collection::default().collect(
            &cfg,
            &Unasked,
            &arkhams_tracker(),
            &Wanted::Everything,
            Filter::All,
            Utc::now(),
        );
        Latest::holding(Held {
            read_under: Arc::new(cfg.clone()),
            cfg: Arc::new(cfg),
            snapshot: Some(snapshot),
            collecting: None,
            lapsed: Vec::new(),
            change: 0,
        })
    }

    /// A post of the comment form `form` to `target` from the bead's own
    /// page, as a browser sends it, with the token that page issued.
    fn posted(target: &str, form: &str) -> String {
        let path = target.split('?').next().unwrap_or_default();
        let form = match address::written(path) {
            Some((bead, write)) => format!("{form}&{TOKEN}={}", tokens().issue(&bead, write)),
            None => form.to_string(),
        };
        posted_with(
            target,
            &format!("Host: {BEADS}\r\nOrigin: http://{BEADS}\r\n"),
            &form,
        )
    }

    /// A post of the form `form` to `target` with the header lines `headers`.
    fn posted_with(target: &str, headers: &str, form: &str) -> String {
        format!(
            "POST {target} HTTP/1.1\r\n{headers}\
             Content-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n{form}",
            form.len()
        )
    }

    /// A run that takes writes asks that a kept copy of what it answers be
    /// asked for again before it is shown, since a page it drew carries
    /// tokens a later run will not take. A read-only run's pages may be
    /// shown as they were kept.
    #[test]
    fn a_kept_copy_of_what_a_run_that_takes_writes_answers_is_asked_for_again() {
        let latest = arkham_held();
        let page = replied(
            &format!("GET /arkham/ark-1 HTTP/1.1\r\nHost: {BEADS}\r\n\r\n"),
            &latest,
            None,
        );
        assert_eq!(headers(&page, true), "Cache-Control: no-cache\r\n");
        assert_eq!(headers(&page, false), "");

        let runner = FakeRunner::default().with(COMMENTED, "");
        let written = replied(
            &posted("/arkham/ark-1/comment", "text=the+gate+is+shut%2C+again"),
            &latest,
            Some(&runner),
        );
        assert_eq!(
            headers(&written, true),
            "Location: /arkham/ark-1#shown\r\nCache-Control: no-cache\r\n"
        );
    }

    /// A post from a page on another site, or from a page that will not say
    /// where it is, is refused before any bd runs, and says it was the origin.
    #[test]
    fn a_write_posted_from_another_site_is_refused() {
        let latest = arkham_held();
        let runner = FakeRunner::default();
        let form = format!(
            "text=hello&{TOKEN}={}",
            tokens().issue(&arkham_gate(), COMMENT)
        );
        for (host, origin) in [
            (BEADS, Some("http://dunwich.invalid")),
            ("localhost:8080", Some("http://localhost:3000")),
            (BEADS, Some("null")),
            (BEADS, None),
        ] {
            let origin_line =
                origin.map_or(String::new(), |origin| format!("Origin: {origin}\r\n"));
            let reply = replied(
                &posted_with(
                    "/arkham/ark-1/comment",
                    &format!("Host: {host}\r\n{origin_line}"),
                    &form,
                ),
                &latest,
                Some(&runner),
            );
            assert_eq!(reply.status, "403 Forbidden", "{origin:?}");
            assert_eq!(
                reply.body,
                html::saying(&phrase::posted_from_elsewhere(origin, host))
            );
            assert!(!reply.body.contains("from null"), "{}", reply.body);
            assert_eq!(reply.sent_to, None);
        }
        assert_eq!(runner.calls(), []);
    }

    /// A proxy that sends bdi web a Host other than the one the browser
    /// asked for makes every write look posted from elsewhere. It is refused,
    /// and the refusal names both, so whoever runs the proxy can see why.
    #[test]
    fn a_write_through_a_proxy_that_rewrites_the_host_says_the_origin_did_not_match() {
        let latest = arkham_held();
        let runner = FakeRunner::default();
        let form = format!(
            "text=hello&{TOKEN}={}",
            tokens().issue(&arkham_gate(), COMMENT)
        );
        let reply = replied(
            &posted_with(
                "/arkham/ark-1/comment",
                "Host: 127.0.0.1:8080\r\nOrigin: https://beads.example.invalid\r\n",
                &form,
            ),
            &latest,
            Some(&runner),
        );
        assert_eq!(reply.status, "403 Forbidden");
        let said =
            phrase::posted_from_elsewhere(Some("https://beads.example.invalid"), "127.0.0.1:8080");
        assert!(said.contains("https://beads.example.invalid"), "{said}");
        assert!(said.contains("127.0.0.1:8080"), "{said}");
        assert_eq!(reply.body, html::saying(&said));
        assert_eq!(runner.calls(), []);
    }

    /// A post carrying no token, or one this run did not issue for that
    /// bead's form, is refused before any bd runs.
    #[test]
    fn a_write_whose_token_the_page_did_not_issue_is_refused() {
        let latest = arkham_held();
        let runner = FakeRunner::default();
        let elsewhere = BeadKey {
            id: "ark-2".to_string(),
            ..arkham_gate()
        };
        for token in [
            None,
            Some(String::new()),
            Some("guessed".to_string()),
            Some(tokens().issue(&elsewhere, COMMENT)),
            Some(Tokens::keyed([8; 32]).issue(&arkham_gate(), COMMENT)),
        ] {
            let form = match &token {
                Some(token) => format!("text=hello&{TOKEN}={token}"),
                None => "text=hello".to_string(),
            };
            let reply = replied(
                &posted_with(
                    "/arkham/ark-1/comment",
                    &format!("Host: {BEADS}\r\nOrigin: http://{BEADS}\r\n"),
                    &form,
                ),
                &latest,
                Some(&runner),
            );
            assert_eq!(reply.status, "403 Forbidden", "{token:?}");
            assert_eq!(
                reply.body,
                html::saying_back_to(phrase::form_not_issued(), "/arkham/ark-1#shown")
            );
            assert!(
                reply.body.contains("href=\"/arkham/ark-1#shown\""),
                "the refusal links to the bead's page, which issues a fresh form, \
                 rather than leave the reader to reload the post"
            );
            assert_eq!(reply.sent_to, None);
        }
        assert_eq!(runner.calls(), []);
    }

    /// The bead arkham's tracker holds that every write here is made to.
    fn arkham_gate() -> BeadKey {
        BeadKey {
            project: "arkham".to_string(),
            id: "ark-1".to_string(),
        }
    }

    /// The actor a run taking writes is configured to write as.
    const ACTOR: &str = "carter";

    const COMMENTED: &str =
        "bd --actor=carter -C /nowhere/arkham comments add ark-1 -- the gate is shut, again";

    /// A bead the page draws from a project its scope took in to read a
    /// blocker takes the comment its page offers.
    #[test]
    fn a_write_to_a_bead_drawn_from_beyond_the_scope_is_made() {
        let mut held = arkham_held().now();
        held.read_under = Arc::new(
            Config::from_toml(
                "[[projects]]\nname = \"arkham\"\npath = \"/nowhere/arkham\"\n\
                 [[projects]]\nname = \"dunwich\"\npath = \"/nowhere/dunwich\"\n",
            )
            .expect("a config")
            .scoped_to(&["dunwich".to_string()])
            .expect("dunwich is configured"),
        );
        let runner = FakeRunner::default().with(COMMENTED, "");
        let reply = replied(
            &posted("/arkham/ark-1/comment", "text=the+gate+is+shut%2C+again"),
            &Latest::holding(held),
            Some(&runner),
        );
        assert_eq!(reply.status, "303 See Other");
        assert_eq!(runner.calls().len(), 1);
    }

    /// A write goes to the tracker its bead was read from, so a config that
    /// has since pointed the project elsewhere does not send it there.
    #[test]
    fn a_write_goes_to_the_tracker_its_bead_was_read_from() {
        let mut held = arkham_held().now();
        held.cfg = Arc::new(
            Config::from_toml("[[projects]]\nname = \"arkham\"\npath = \"/elsewhere/arkham\"\n")
                .expect("a config"),
        );
        let runner = FakeRunner::default().with(COMMENTED, "");
        let reply = replied(
            &posted("/arkham/ark-1/comment", "text=the+gate+is+shut%2C+again"),
            &Latest::holding(held),
            Some(&runner),
        );
        assert_eq!(reply.status, "303 See Other");
        assert_eq!(runner.calls().len(), 1);
    }

    #[test]
    fn a_run_that_takes_no_writes_refuses_a_write() {
        let latest = arkham_held();
        let reply = replied(
            &posted("/arkham/ark-1/comment", "text=the+gate+is+shut"),
            &latest,
            None,
        );
        assert_eq!(reply.status, "405 Method Not Allowed");
        assert_eq!(reply.sent_to, None);
    }

    #[test]
    fn a_run_that_takes_writes_makes_a_comment_through_bdi_bd_and_sends_the_reader_back() {
        let latest = arkham_held();
        let runner = FakeRunner::default().with(COMMENTED, "");
        let reply = replied(
            &posted("/arkham/ark-1/comment", "text=the+gate+is+shut%2C+again"),
            &latest,
            Some(&runner),
        );
        assert_eq!(reply.status, "303 See Other", "{}", reply.body);
        assert_eq!(reply.sent_to.as_deref(), Some("/arkham/ark-1#shown"));
        assert_eq!(
            runner.call(COMMENTED).cwd.as_deref(),
            Some(Path::new("/nowhere/arkham"))
        );
    }

    /// The reader is sent back to the page as they left it: the folds, the
    /// list and the copy the form's address carried.
    #[test]
    fn a_write_sends_the_reader_back_to_the_page_as_they_left_it() {
        let latest = arkham_held();
        let runner = FakeRunner::default().with(COMMENTED, "");
        let reply = replied(
            &posted(
                "/arkham/ark-1/comment?shut=gate",
                "text=the+gate+is+shut%2C+again",
            ),
            &latest,
            Some(&runner),
        );
        assert_eq!(
            reply.sent_to.as_deref(),
            Some("/arkham/ark-1?shut=gate#shown")
        );
    }

    /// A comment that starts with a dash is still the comment, since the
    /// words go after `--`.
    #[test]
    fn a_comment_that_looks_like_a_flag_is_written_as_the_comment() {
        let latest = arkham_held();
        let flagged = "bd --actor=carter -C /nowhere/arkham comments add ark-1 -- --db=elsewhere";
        let runner = FakeRunner::default().with(flagged, "");
        let reply = replied(
            &posted("/arkham/ark-1/comment", "text=--db%3Delsewhere"),
            &latest,
            Some(&runner),
        );
        assert_eq!(reply.status, "303 See Other", "{}", reply.body);
        runner.call(flagged);
    }

    #[test]
    fn a_write_bd_refuses_says_why_and_sends_the_reader_nowhere() {
        let latest = arkham_held();
        let refusal = RunFailure::from_exit("bd", Some(1), "Error: connection refused");
        let runner = FakeRunner::default().failing(COMMENTED, refusal.clone());
        let reply = replied(
            &posted("/arkham/ark-1/comment", "text=the+gate+is+shut%2C+again"),
            &latest,
            Some(&runner),
        );
        assert_eq!(reply.status, "502 Bad Gateway");
        assert_eq!(reply.sent_to, None);
        assert_eq!(
            reply.body,
            html::saying(&phrase::not_written(&refusal.to_string()))
        );
    }

    /// A post that names no write, no text, a project the run does not read
    /// or a name with no form behind it runs nothing.
    #[test]
    fn a_post_that_asks_for_no_write_this_run_makes_runs_nothing() {
        let latest = arkham_held();
        let runner = FakeRunner::default();
        for (target, form, status) in [
            ("/arkham/ark-1/comment", "", "400 Bad Request"),
            ("/arkham/ark-1/comment", "text=+%0A", "400 Bad Request"),
            ("/arkham/ark-1/comment", "said=hello", "400 Bad Request"),
            ("/arkham/ark-1/burn", "text=hello", "400 Bad Request"),
            ("/dunwich/dun-1/comment", "text=hello", "404 Not Found"),
            ("/arkham/ark-1", "text=hello", "404 Not Found"),
            ("/", "text=hello", "404 Not Found"),
        ] {
            let reply = replied(&posted(target, form), &latest, Some(&runner));
            assert_eq!(reply.status, status, "{target} {form}");
        }
        assert_eq!(runner.calls(), []);
    }

    /// A write's form names its length, and one too long to take is refused
    /// before any of it is read.
    #[test]
    fn a_write_with_no_length_or_too_long_a_one_is_refused() {
        let latest = arkham_held();
        let runner = FakeRunner::default();
        let unmeasured = format!("POST /arkham/ark-1/comment HTTP/1.1\r\nHost: {BEADS}\r\nOrigin: http://{BEADS}\r\n\r\n");
        assert_eq!(
            replied(&unmeasured, &latest, Some(&runner)).status,
            "411 Length Required"
        );
        let too_long = format!(
            "POST /arkham/ark-1/comment HTTP/1.1\r\nHost: {BEADS}\r\nOrigin: http://{BEADS}\r\nContent-Length: {}\r\n\r\n",
            LARGEST_WRITE + 1
        );
        assert_eq!(
            replied(&too_long, &latest, Some(&runner)).status,
            "413 Content Too Large"
        );
        assert_eq!(runner.calls(), []);
    }

    /// An address whose bead looks like a flag names no bead the page draws,
    /// so bd is never run.
    #[test]
    fn a_write_to_a_bead_named_like_a_flag_runs_nothing() {
        let latest = arkham_held();
        let runner = FakeRunner::default();
        let reply = replied(
            &posted("/arkham/--db=elsewhere/comment", "text=hello"),
            &latest,
            Some(&runner),
        );
        assert_eq!(reply.status, NOT_FOUND);
        assert_eq!(reply.sent_to, None);
        assert_eq!(runner.calls(), []);
    }

    /// A write names the bead by the id its page drew, so an id that is only
    /// the start of one, or that bd would route to another tracker, runs
    /// nothing.
    #[test]
    fn a_write_to_a_bead_the_page_does_not_draw_runs_nothing() {
        let latest = arkham_held();
        let runner = FakeRunner::default();
        let reply = replied(
            &posted("/arkham/ark/comment", "text=hello"),
            &latest,
            Some(&runner),
        );
        assert_eq!(reply.status, NOT_FOUND);
        assert_eq!(reply.sent_to, None);
        assert_eq!(runner.calls(), []);
    }

    /// Until the first read answers, whether the bead is there cannot be
    /// said, so a write waits for it.
    #[test]
    fn a_write_before_the_first_read_answers_runs_nothing() {
        let cfg =
            Config::from_toml("[[projects]]\nname = \"arkham\"\npath = \"/nowhere/arkham\"\n")
                .expect("a config");
        let runner = FakeRunner::default();
        let reply = replied(
            &posted("/arkham/ark-1/comment", "text=hello"),
            &Latest::new(cfg),
            Some(&runner),
        );
        assert_eq!(reply.status, UNAVAILABLE);
        assert_eq!(runner.calls(), []);
    }

    /// A tracker that stalls holds up one write and not every request, so a
    /// write sent while another is being made is refused for now.
    #[test]
    fn a_write_sent_while_another_is_being_made_is_refused_for_now() {
        let latest = arkham_held();
        let runner = FakeRunner::default().with(COMMENTED, "");
        let writer = Writer::through(&runner, ACTOR.to_string(), tokens());
        let posting = posted("/arkham/ark-1/comment", "text=the+gate+is+shut%2C+again");
        {
            let _making = writer.one_at_a_time.lock().expect("nothing else holds it");
            assert_eq!(
                replied_by(&posting, &latest, Some(&writer)).status,
                UNAVAILABLE
            );
            assert_eq!(runner.calls(), []);
        }
        assert_eq!(
            replied_by(&posting, &latest, Some(&writer)).status,
            "303 See Other"
        );
    }

    /// A post to a write no page offers says it asks for none, rather than
    /// that its form is stale, since no page issues a form for it.
    #[test]
    fn a_post_to_a_write_no_page_offers_is_not_told_its_form_is_stale() {
        let runner = FakeRunner::default();
        let reply = replied(
            &posted_with(
                "/arkham/ark-1/commnet",
                &format!("Host: {BEADS}\r\nOrigin: http://{BEADS}\r\n"),
                "text=hello",
            ),
            &arkham_held(),
            Some(&runner),
        );
        assert_eq!(reply.status, "400 Bad Request", "{}", reply.body);
        assert_eq!(runner.calls(), []);
    }

    /// A form this run did not issue is refused while another write is being
    /// made too, since sending it again later would be refused the same.
    #[test]
    fn a_form_not_issued_is_refused_while_another_write_is_being_made() {
        let latest = arkham_held();
        let runner = FakeRunner::default();
        let writer = Writer::through(&runner, ACTOR.to_string(), tokens());
        let posting = posted_with(
            "/arkham/ark-1/comment",
            &format!("Host: {BEADS}\r\nOrigin: http://{BEADS}\r\n"),
            "text=hello&token=guessed",
        );
        let _making = writer.one_at_a_time.lock().expect("nothing else holds it");
        assert_eq!(
            replied_by(&posting, &latest, Some(&writer)).status,
            FORBIDDEN
        );
    }

    /// The longest comment the form lets through is taken whatever it is
    /// written in, so a draft is never lost to a refusal the form allowed.
    #[test]
    fn the_longest_comment_the_form_takes_is_not_too_long() {
        let latest = arkham_held();
        let commented = format!(
            "bd --actor=carter -C /nowhere/arkham comments add ark-1 -- {}",
            "€".repeat(address::LONGEST_COMMENT)
        );
        let runner = FakeRunner::default().with(&commented, "");
        let euros = "%E2%82%AC".repeat(address::LONGEST_COMMENT);
        let reply = replied(
            &posted("/arkham/ark-1/comment", &format!("text={euros}")),
            &latest,
            Some(&runner),
        );
        assert_eq!(reply.status, "303 See Other", "{}", reply.body);
    }

    /// A write sent to a name nobody allowed is refused as a read is.
    #[test]
    fn a_write_sent_to_a_name_nobody_allowed_is_refused() {
        let latest = arkham_held();
        let runner = FakeRunner::default();
        let elsewhere =
            posted("/arkham/ark-1/comment", "text=hello").replace(BEADS, "dunwich.invalid");
        assert_eq!(
            replied(&elsewhere, &latest, Some(&runner)).status,
            "421 Misdirected Request"
        );
        assert_eq!(runner.calls(), []);
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
