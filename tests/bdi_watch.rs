//! `bdi watch` reads every configured project, reads one again when a
//! producer reports it, will not start beside a watcher already running, and
//! sends each consumer the beads it watches. A one-shot `bdi` reads through
//! it where one is running, and for itself where none is.
//!
//! The cases run the binary, because what is under test is the process a
//! supervisor starts and the socket it leaves on the machine.

mod terminal;

use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use terminal::driver::{Driven, GIVING_UP as THE_SCREEN_GIVING_UP};
use terminal::shims::ShimmedTracker;
use terminal::{
    a_home_naming_one_project_settled, a_socket_of_its_own, die_with, the_socket_under, Producer,
    ENTER_ALTERNATE_SCREEN, THE_DESCRIBED_SUBTREE,
};

/// `a`, which shows every tree rather than only those with a live agent. No
/// pane sits in the temp `HOME`, so without it the one tree here sits behind
/// its project's *no live agent* line and draws no row of its own.
const SHOW_EVERY_TREE: &[u8] = b"a";

/// The call that reads a tracker in full.
const READ_IN_FULL: &str = "list --all --include-gates --limit 0 --json";

/// Long enough for a read that is coming to have come, and short enough
/// that one that is not is a failure rather than a hang.
const GIVING_UP: Duration = Duration::from_secs(10);

/// A project that never polls, so every read after the first is one a
/// producer asked for.
const NOT_POLLED: &str = "poll = false\n";

fn watching_at(home: &Path) -> PathBuf {
    home.join("watcher.sock")
}

fn bdi_watch(home: &Path, tracker: &ShimmedTracker) -> Command {
    let spawned_by = std::process::id();
    let mut command = Command::new(env!("CARGO_BIN_EXE_bdi"));
    command
        .args(["watch", "--socket"])
        .arg(watching_at(home))
        .current_dir(home)
        .env("HOME", home)
        .env_remove("BEADS_DIR")
        .env_remove("BEADS_DOLT_PASSWORD")
        .env_remove("BDI_PROJECT")
        .env("XDG_RUNTIME_DIR", home)
        .envs(tracker.environment())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    // Dropping a `Child` leaves its process running, and a test binary that
    // is killed runs no `Drop`, so the kernel is asked to end the watcher
    // with its spawner.
    unsafe { command.pre_exec(move || die_with(spawned_by)) };
    command
}

/// A running `bdi watch` that is killed and reaped if the test ends without
/// stopping it, which a panic does.
struct Watcher(Option<Child>);

impl Watcher {
    fn stopping(mut self) -> Child {
        self.0.take().expect("the watcher is still held")
    }

    fn pid(&self) -> u32 {
        self.0.as_ref().expect("the watcher is still held").id()
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// A watcher started over a tracker holding the described subtree, once its
/// first read of the tracker is in.
fn a_watcher(named: &str) -> (PathBuf, ShimmedTracker, Watcher) {
    a_watcher_over(named, NOT_POLLED, |_| {})
}

/// The same, with `settings` on the project's entry and the tracker staged by
/// `staging` before the watcher starts.
fn a_watcher_over(
    named: &str,
    settings: &str,
    staging: impl FnOnce(&ShimmedTracker),
) -> (PathBuf, ShimmedTracker, Watcher) {
    let home = a_home_naming_one_project_settled(named, settings);
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_DESCRIBED_SUBTREE);
    staging(&tracker);
    let watcher = Watcher(Some(
        bdi_watch(&home, &tracker)
            .spawn()
            .expect("bdi watch starts"),
    ));
    until(|| reads_in_full(&tracker) == 1, "the first read");
    until(|| watching_at(&home).exists(), "the socket");
    (home, tracker, watcher)
}

fn reads_in_full(tracker: &ShimmedTracker) -> usize {
    tracker
        .calls()
        .iter()
        .filter(|call| *call == READ_IN_FULL)
        .count()
}

fn until(holds: impl Fn() -> bool, awaited: &str) {
    let giving_up = Instant::now() + GIVING_UP;
    while !holds() {
        assert!(Instant::now() < giving_up, "{awaited} never came");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Ask `watcher` to stop as a supervisor does, and take what it said.
fn stopped(watcher: Watcher) -> Output {
    let signalled = Command::new("kill")
        .args(["-TERM", &watcher.pid().to_string()])
        .status()
        .expect("kill runs");
    assert!(signalled.success());
    watcher
        .stopping()
        .wait_with_output()
        .expect("bdi watch exits")
}

#[test]
fn a_project_a_producer_reports_is_read_again() {
    let (home, tracker, watcher) = a_watcher("watch-reads-again");

    let answer = Producer::connected_to(&watching_at(&home)).says("arkham");
    until(
        || reads_in_full(&tracker) == 2,
        "the read the report asked for",
    );

    let out = stopped(watcher);
    assert_eq!(answer, "ok arkham");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stderr), "");
    assert!(
        !watching_at(&home).exists(),
        "the socket goes with the watcher"
    );
}

#[test]
fn a_second_watcher_on_the_same_socket_will_not_start_and_says_where() {
    let (home, tracker, watcher) = a_watcher("watch-twice");

    let second = bdi_watch(&home, &tracker).output().expect("bdi watch runs");

    stopped(watcher);
    let said = String::from_utf8_lossy(&second.stderr);
    assert!(!second.status.success());
    assert!(
        said.contains(&watching_at(&home).display().to_string()),
        "the socket is named: {said}"
    );
    assert_eq!(reads_in_full(&tracker), 1, "the second read nothing");
}

/// Something outside `bdi` watching beads on the watcher's socket.
struct Consumer {
    speaking: UnixStream,
    watching: BufReader<UnixStream>,
}

impl Consumer {
    /// Connected once the watcher takes connections, which is a moment
    /// after its socket appears: `bind` names it before `listen`.
    fn connected_to(at: &Path) -> Self {
        let giving_up = Instant::now() + GIVING_UP;
        let speaking = loop {
            match UnixStream::connect(at) {
                Err(why)
                    if why.kind() == ErrorKind::ConnectionRefused && Instant::now() < giving_up =>
                {
                    std::thread::sleep(Duration::from_millis(20));
                }
                connected => {
                    break connected
                        .unwrap_or_else(|why| panic!("bdi watch is on {} ({why})", at.display()))
                }
            }
        };
        let watching = speaking
            .try_clone()
            .expect("the connection is ours to read");
        watching
            .set_read_timeout(Some(GIVING_UP))
            .expect("a read nothing answers is ours to give up on");
        Self {
            speaking,
            watching: BufReader::new(watching),
        }
    }

    fn sends(&mut self, line: &str) -> &mut Self {
        writeln!(self.speaking, "{line}").expect("the line is ours to send");
        self
    }

    /// The next line the watcher sends, or nothing where it closed the
    /// connection.
    fn hears(&mut self) -> Option<Value> {
        let mut line = String::new();
        match self.watching.read_line(&mut line) {
            Ok(0) => None,
            Ok(_) => Some(
                serde_json::from_str(&line)
                    .unwrap_or_else(|why| panic!("the watcher sends JSON lines ({why}): {line}")),
            ),
            Err(why) if why.kind() == ErrorKind::ConnectionReset => None,
            Err(why) => panic!("the watcher said nothing in time ({why})"),
        }
    }

    /// Every line up to and including the next freshness line: one answer
    /// for one project.
    fn hears_an_answer(&mut self) -> Vec<Value> {
        let mut answer = Vec::new();
        loop {
            let line = self.hears().expect("the watcher stays up");
            let done = line["line"] == "freshness";
            answer.push(line);
            if done {
                return answer;
            }
        }
    }
}

/// The ids an answer sends bead lines for, in the order it sent them.
fn beads_in(answer: &[Value]) -> Vec<&str> {
    answer
        .iter()
        .filter(|line| line["line"] == "bead")
        .map(|line| line["row"]["id"].as_str().expect("a row names its bead"))
        .collect()
}

/// The described subtree with `change` made to its rows.
fn the_described_subtree_with(change: impl FnOnce(&mut Vec<Value>)) -> String {
    let mut rows: Vec<Value> =
        serde_json::from_str(THE_DESCRIBED_SUBTREE).expect("the capture is bd's JSON");
    change(&mut rows);
    serde_json::to_string(&rows).expect("rows serialise")
}

fn closing(rows: &mut [Value], id: &str) {
    let row = rows
        .iter_mut()
        .find(|row| row["id"] == id)
        .expect("the capture holds the bead");
    row["status"] = json!("closed");
}

/// Change what `tracker` holds and have a producer report it, so the
/// watcher reads it again.
fn the_tracker_now_holds(home: &Path, tracker: &ShimmedTracker, capture: &str) {
    let before = reads_in_full(tracker);
    tracker.holds(capture);
    Producer::connected_to(&watching_at(home)).says("arkham");
    until(
        || reads_in_full(tracker) > before,
        "the read the report asked for",
    );
}

const EVERY_DESCRIBED_BEAD: [&str; 5] = [
    "dun-0tp",
    "dun-0tp.6",
    "dun-0tp.7",
    "dun-0tp.8",
    "dun-0tp.9",
];

#[test]
fn a_watch_is_sent_the_projects_beads_then_how_current_they_are() {
    let (home, _tracker, watcher) = a_watcher("watch-watch");

    let answer = Consumer::connected_to(&watching_at(&home))
        .sends("watch arkham")
        .hears_an_answer();

    stopped(watcher);
    assert_eq!(beads_in(&answer), EVERY_DESCRIBED_BEAD);
    let bead = &answer[0];
    assert_eq!(bead["project"], "arkham");
    assert_eq!(bead["ready"], false);
    assert_eq!(bead["blocked_by"], json!([]));
    assert_eq!(bead["bd"], json!({ "ready": false, "blocked_by": [] }));
    assert_eq!(
        bead["row"]["description"]
            .as_str()
            .map(|text| !text.is_empty()),
        Some(true),
        "the row is bd's whole: {bead}"
    );
    let freshness = answer.last().expect("an answer ends");
    assert_eq!(freshness["project"], "arkham");
    assert_eq!(freshness["tracker"], "ok");
    assert_eq!(freshness["events"], "off");
    assert_eq!(freshness["protocol"], 1);
    assert!(freshness["as_of"].is_string(), "{freshness}");
    assert!(freshness["reach"]["path"].is_string(), "{freshness}");
}

/// What a consumer that acts only on what it is told relies on: a watcher
/// that goes closes the connection, and the next connect is refused.
#[test]
fn a_watcher_that_stops_closes_its_consumers_and_takes_no_more() {
    let (home, _tracker, watcher) = a_watcher("watch-gone");
    let mut consumer = Consumer::connected_to(&watching_at(&home));
    consumer.sends("watch arkham").hears_an_answer();

    stopped(watcher);

    assert_eq!(consumer.hears(), None);
    assert!(UnixStream::connect(watching_at(&home)).is_err());
}

/// What a consumer waiting on a deferred bead relies on: nothing is written
/// when its date passes, and the watcher still sends it again, marked as
/// having reached it.
#[test]
fn a_bead_whose_defer_until_passes_is_sent_again_marked_reached() {
    let until = chrono::Utc::now() + chrono::Duration::seconds(2);
    let deferred = the_described_subtree_with(|rows| {
        let row = rows
            .iter_mut()
            .find(|row| row["id"] == "dun-0tp.9")
            .expect("the capture holds the bead");
        row["defer_until"] = json!(until);
    });
    let (home, _tracker, watcher) = a_watcher_over("watch-defer", NOT_POLLED, |tracker| {
        tracker.holds(&deferred)
    });
    let mut consumer = Consumer::connected_to(&watching_at(&home));
    consumer.sends("watch arkham dun-0tp.9");
    let before = consumer.hears_an_answer();

    let after = consumer.hears_an_answer();

    stopped(watcher);
    let reached = |answer: &[Value]| -> Vec<Value> {
        answer
            .iter()
            .filter(|line| line["line"] == "bead")
            .map(|line| line["defer_until_reached"].clone())
            .collect()
    };
    assert_eq!(reached(&before), [json!(false)]);
    assert_eq!(beads_in(&after), ["dun-0tp.9"]);
    assert_eq!(reached(&after), [json!(true)]);
}

/// The journal's read past where it ended when the watcher first read it.
const THE_JOURNAL_SINCE_IT_STARTED: &str = "events tail --since 0";

/// A comment on, a dependency added to and a close of `dun-0tp.7`, and an
/// update to `dun-0tp.8`, written after the record `after`: the shape bd
/// 1.3.0 writes, onto the described subtree's beads.
fn the_journal_of_dun_0tp_7(after: u64) -> String {
    let ids = ["dun-0tp.7", "dun-0tp.7", "dun-0tp.8", "dun-0tp.7"];
    include_str!("fixtures/bd_1.3.0_events_tail.jsonl")
        .lines()
        .zip(ids)
        .enumerate()
        .map(|(at, (line, id))| {
            let mut record: Value = serde_json::from_str(line).expect("the capture is bd's JSON");
            record["seq"] = json!(after + at as u64 + 1);
            record["issue_id"] = json!(id);
            record["issue"]["id"] = json!(id);
            format!("{record}\n")
        })
        .collect()
}

fn events_in(answer: &[Value]) -> Vec<&str> {
    answer
        .iter()
        .filter(|line| line["line"] == "event")
        .map(|line| line["event"]["op"].as_str().expect("a record names its op"))
        .collect()
}

#[test]
fn a_watch_on_a_bead_in_a_project_keeping_a_journal_is_sent_its_records_before_the_answer_closes() {
    let (home, tracker, watcher) = a_watcher_over(
        "watch-journal",
        &format!("{NOT_POLLED}events_journal = true\n"),
        |tracker| tracker.answers_with(THE_JOURNAL_SINCE_IT_STARTED, ""),
    );
    let mut consumer = Consumer::connected_to(&watching_at(&home));
    let first = consumer.sends("watch arkham dun-0tp.7").hears_an_answer();

    tracker.answers_with(THE_JOURNAL_SINCE_IT_STARTED, &the_journal_of_dun_0tp_7(0));
    the_tracker_now_holds(
        &home,
        &tracker,
        &the_described_subtree_with(|rows| closing(rows, "dun-0tp.7")),
    );
    let answer = consumer.hears_an_answer();

    stopped(watcher);
    assert_eq!(first.last().expect("an answer ends")["events"], "ok");
    assert_eq!(events_in(&answer), ["comment", "dep_add", "close"]);
    assert_eq!(answer[0]["project"], "arkham");
    assert_eq!(
        answer[0]["event"]["comment"]["text"],
        "Guard it in the parser."
    );
    let kinds: Vec<&str> = answer
        .iter()
        .map(|line| line["line"].as_str().expect("every line says its kind"))
        .collect();
    assert_eq!(kinds, ["event", "event", "event", "bead", "freshness"]);
}

/// A home whose config tells every run where the watcher is, as a setup
/// that starts one says it, holding the described subtree.
fn a_home_with_a_watcher_configured(named: &str) -> (PathBuf, ShimmedTracker) {
    let home = a_home_naming_one_project_settled(named, NOT_POLLED);
    let config = home.join(".config/beady-eye/config.toml");
    let mut text = std::fs::read_to_string(&config).expect("the config was just written");
    text.push_str(&format!(
        "\n[watcher]\nsocket = \"{}\"\n",
        watching_at(&home).display()
    ));
    std::fs::write(&config, text).expect("the config is ours to write");
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_DESCRIBED_SUBTREE);
    (home, tracker)
}

/// A watcher started in `home`, once it has answered for its first read of
/// the tracker, which is when every call of that read has been made.
fn watching_in(home: &Path, tracker: &ShimmedTracker) -> Watcher {
    let watcher = Watcher(Some(
        bdi_watch(home, tracker).spawn().expect("bdi watch starts"),
    ));
    until(|| watching_at(home).exists(), "the socket");
    Consumer::connected_to(&watching_at(home))
        .sends("watch arkham")
        .hears_an_answer();
    watcher
}

/// What a one-shot `bdi` run in `home` with `args` wrote, read as JSON, with
/// the instant it is dated to taken out to be looked at on its own.
fn one_shot(home: &Path, tracker: &ShimmedTracker, args: &[&str]) -> (Value, Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_bdi"))
        .args(args)
        .current_dir(home)
        .env("HOME", home)
        .env_remove("BEADS_DIR")
        .env_remove("BEADS_DOLT_PASSWORD")
        .env_remove("BDI_PROJECT")
        .envs(tracker.environment())
        .output()
        .expect("bdi runs");
    assert!(
        out.status.success(),
        "bdi exited {}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let mut written: Value = serde_json::from_slice(&out.stdout).expect("bdi writes JSON");
    let dated = written["generated_at"].take();
    (written, dated)
}

/// Both one-shots, each beside a watcher and then with none, so the two
/// answers can be set side by side.
#[test]
fn a_one_shot_beside_a_watcher_reads_no_tracker_and_says_what_its_own_read_says() {
    for (named, args) in [
        ("watched-json", &["--json", "--all"][..]),
        ("watched-beads", &["--beads"][..]),
    ] {
        let (home, tracker) = a_home_with_a_watcher_configured(named);
        let watcher = watching_in(&home, &tracker);
        let asked_before = tracker.calls();

        let (through_the_watcher, dated) = one_shot(&home, &tracker, args);

        let asked_after = tracker.calls();
        stopped(watcher);
        let (read_for_itself, _) = one_shot(&home, &tracker, args);
        assert_eq!(asked_after, asked_before, "{args:?} asked bd nothing");
        assert_eq!(through_the_watcher, read_for_itself, "{args:?}");
        assert!(dated.is_string(), "{args:?} is dated: {dated}");
    }
}

/// A view beside a watcher draws what the watcher holds and asks bd
/// nothing, and reads the tracker itself as soon as the watcher goes.
#[test]
fn a_view_beside_a_watcher_reads_no_tracker_until_the_watcher_goes() {
    let (home, tracker) = a_home_with_a_watcher_configured("watched-view");
    let watcher = watching_in(&home, &tracker);
    let asked_before = tracker.calls();

    let mut bdi = Driven::bdi(40, 120, home.clone(), &tracker.environment());
    bdi.read_until(ENTER_ALTERNATE_SCREEN, THE_SCREEN_GIVING_UP);
    bdi.send(SHOW_EVERY_TREE);
    bdi.read_until(b"dun-0tp", THE_SCREEN_GIVING_UP);
    bdi.settle(Duration::from_millis(300), THE_SCREEN_GIVING_UP);
    let asked_while_watching = tracker.calls();
    stopped(watcher);

    assert_eq!(
        asked_while_watching, asked_before,
        "the view asked bd nothing"
    );
    until(|| reads_in_full(&tracker) > 1, "the view's own read");
}

/// The word the foot's notice for a project the watcher does not answer for
/// says in its full form and its brief one alike. One word, because a repaint
/// reaches the wire a word at a time.
const READ_HERE: &[u8] = b"itself";

/// A view under a config that reaches a project another way than the
/// watcher's reads that project with bd itself, and says so at the foot.
#[test]
fn a_view_the_watcher_does_not_answer_for_says_it_reads_the_project_itself() {
    let (home, tracker) = a_home_with_a_watcher_configured("watched-view-elsewhere");
    let watcher = watching_in(&home, &tracker);
    let config = home.join(".config/beady-eye/config.toml");
    let text = std::fs::read_to_string(&config).expect("the config was just written");
    std::fs::write(
        &config,
        text.replacen(
            NOT_POLLED,
            &format!("{NOT_POLLED}environment_command = \"env\"\n"),
            1,
        ),
    )
    .expect("the config is ours to write");
    let read_by_the_watcher = reads_in_full(&tracker);
    let mut environment = tracker.environment();
    environment.push(a_socket_of_its_own(&home));

    let mut bdi = Driven::bdi(40, 120, home.clone(), &environment);
    bdi.read_until(ENTER_ALTERNATE_SCREEN, THE_SCREEN_GIVING_UP);
    bdi.read_until(READ_HERE, THE_SCREEN_GIVING_UP);
    stopped(watcher);

    assert!(
        reads_in_full(&tracker) > read_by_the_watcher,
        "the view read arkham itself"
    );
}

/// `^R`, which asks every project for itself again.
const REFRESH: &[u8] = b"\x12";

/// A view draws what the watcher holds, so asking for a read is asking
/// the watcher for one.
#[test]
fn the_refresh_key_has_the_watcher_a_view_reads_through_read_again() {
    let (home, tracker) = a_home_with_a_watcher_configured("watched-view-refresh");
    let watcher = watching_in(&home, &tracker);
    let mut bdi = Driven::bdi(40, 120, home.clone(), &tracker.environment());
    bdi.read_until(ENTER_ALTERNATE_SCREEN, THE_SCREEN_GIVING_UP);
    bdi.send(SHOW_EVERY_TREE);
    bdi.read_until(b"dun-0tp", THE_SCREEN_GIVING_UP);
    bdi.settle(Duration::from_millis(300), THE_SCREEN_GIVING_UP);
    let asked_before = tracker.calls().len();

    bdi.send(REFRESH);

    until(
        || tracker.calls().len() > asked_before,
        "the watcher asking its tracker",
    );
    stopped(watcher);
}

/// A board started before the watcher holds the changes socket, and the
/// watcher takes it from the board: a push there reaches every board reading
/// through the watcher, without waiting for this one to find the watcher.
#[test]
fn a_change_pushed_after_a_board_started_first_reaches_the_watchers_boards() {
    let (home, tracker) = a_home_with_a_watcher_configured("watch-takes-changes");
    let mut environment = tracker.environment();
    environment.push(a_socket_of_its_own(&home));
    let mut board = Driven::bdi(40, 120, home.clone(), &environment);
    board.read_until(ENTER_ALTERNATE_SCREEN, THE_SCREEN_GIVING_UP);
    until(|| the_socket_under(&home).exists(), "the board's socket");
    let watcher = watching_in(&home, &tracker);
    let mut another_board = Consumer::connected_to(&watching_at(&home));
    another_board.sends("watch arkham").hears_an_answer();

    let answer = Producer::connected_to(&the_socket_under(&home)).says("arkham");
    let heard = another_board.hears_an_answer();

    stopped(watcher);
    assert_eq!(answer, "ok arkham");
    assert_eq!(heard.len(), 1, "{heard:?}");
}

/// A config naming the watcher's own socket for the views' as well leaves
/// the watcher one socket, which hears producers and serves views alike.
#[test]
fn a_views_socket_named_as_the_watchers_own_stays_the_watchers() {
    let (home, tracker) = a_home_with_a_watcher_configured("watch-one-socket");
    let config = home.join(".config/beady-eye/config.toml");
    let mut text = std::fs::read_to_string(&config).expect("the config was just written");
    text.push_str(&format!(
        "\n[changes]\nsocket = \"{}\"\n",
        watching_at(&home).display()
    ));
    std::fs::write(&config, text).expect("the config is ours to write");
    let watcher = watching_in(&home, &tracker);

    let answer = Producer::connected_to(&watching_at(&home)).says("arkham");
    let watched = Consumer::connected_to(&watching_at(&home))
        .sends("watch arkham")
        .hears_an_answer();

    let out = stopped(watcher);
    assert_eq!(answer, "ok arkham");
    assert!(!watched.is_empty());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(!said.contains("hears producers only"), "{said}");
}

/// A test that panics before it stops its watcher must not leave it behind.
#[test]
fn a_watcher_a_test_abandons_is_killed() {
    let (_home, _tracker, watcher) = a_watcher("watch-abandoned");
    let pid = watcher.pid();

    drop(watcher);

    let still_there = Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .expect("kill runs");
    assert!(!still_there.success(), "watcher {pid} outlived its test");
}

/// A tracker's `bd serve`, which the test speaks for.
struct BdServe {
    listener: TcpListener,
    streaming: Option<TcpStream>,
}

impl BdServe {
    fn listening() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port is ours to take");
        listener
            .set_nonblocking(true)
            .expect("an accept is ours to give up on");
        Self {
            listener,
            streaming: None,
        }
    }

    /// The project entry's line naming where this listens.
    fn named(&self) -> String {
        let at = self.listener.local_addr().expect("the port is bound");
        format!("serve = \"{at}\"\n")
    }

    /// The path the watcher's next request asks for, and the connection to
    /// answer it on.
    fn asked(&self) -> (String, TcpStream) {
        let giving_up = Instant::now() + GIVING_UP;
        let connection = loop {
            match self.listener.accept() {
                Ok((connection, _)) => break connection,
                Err(why) if why.kind() == ErrorKind::WouldBlock => {
                    assert!(Instant::now() < giving_up, "the watcher asked nothing");
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(why) => panic!("the watcher's connection is accepted ({why})"),
            }
        };
        connection
            .set_nonblocking(false)
            .expect("the connection is ours to block on");
        let mut request = BufReader::new(
            connection
                .try_clone()
                .expect("the connection is ours to read"),
        );
        let mut asked = String::new();
        request.read_line(&mut asked).expect("the request comes");
        let mut header = String::new();
        while header != "\r\n" {
            header.clear();
            request.read_line(&mut header).expect("the request ends");
        }
        let path = asked
            .split(' ')
            .nth(1)
            .expect("a request line names its path")
            .to_string();
        (path, connection)
    }

    fn answers_the_head(&self, head: u64) {
        let (path, mut connection) = self.asked();
        assert_eq!(path, "/v0/beads/events?since=0&limit=1");
        write!(
            connection,
            "HTTP/1.0 200 OK\r\n\r\n{{\"head\":{head},\"records\":[]}}"
        )
        .expect("the watcher reads the head");
    }

    fn opens_the_stream_after(&mut self, since: u64) {
        let (path, mut connection) = self.asked();
        assert_eq!(path, format!("/v0/beads/events:watch?since={since}"));
        write!(
            connection,
            "HTTP/1.0 200 OK\r\nContent-Type: text/event-stream\r\n\r\nretry: 3000\n\n"
        )
        .expect("the watcher reads the stream");
        self.streaming = Some(connection);
    }

    fn sends(&mut self, frame: &str) {
        self.streaming
            .as_mut()
            .expect("a stream is open")
            .write_all(frame.as_bytes())
            .expect("the watcher reads the stream");
    }

    fn hangs_up(&mut self) {
        self.streaming = None;
    }
}

/// A watcher following arkham's stream from the journal's head at 7, and a
/// consumer watching arkham.
fn a_watcher_following_a_stream(named: &str) -> (BdServe, ShimmedTracker, Watcher, Consumer) {
    let mut serve = BdServe::listening();
    let (home, tracker, watcher) = a_watcher_over(named, &serve.named(), |_| {});
    serve.answers_the_head(7);
    serve.opens_the_stream_after(7);
    let mut consumer = Consumer::connected_to(&watching_at(&home));
    consumer.sends("watch arkham").hears_an_answer();
    (serve, tracker, watcher, consumer)
}

/// Every answer `consumer` hears until one sends `id` closed.
fn until_told_closed(consumer: &mut Consumer, id: &str) {
    loop {
        let answer = consumer.hears_an_answer();
        let closed = answer.iter().any(|line| {
            line["line"] == "bead" && line["row"]["id"] == id && line["row"]["status"] == "closed"
        });
        if closed {
            return;
        }
    }
}

/// The record stands for a write by any bd client, on this machine or
/// another: the stream is the tracker's journal, whoever wrote to it. The
/// project names no `poll`, so nothing reads it on a clock before the test
/// is over.
#[test]
fn a_write_on_a_projects_stream_reaches_a_consumer_with_no_poll_behind_it() {
    let (mut serve, tracker, watcher, mut consumer) = a_watcher_following_a_stream("watch-stream");

    tracker.holds(&the_described_subtree_with(|rows| {
        closing(rows, "dun-0tp.9")
    }));
    serve.sends("id: 8\ndata: {\"seq\":8}\n\n");

    until_told_closed(&mut consumer, "dun-0tp.9");
    let out = stopped(watcher);
    assert_eq!(String::from_utf8_lossy(&out.stderr), "");
}

#[test]
fn a_dropped_stream_is_resumed_after_the_last_record_the_watcher_read() {
    let (mut serve, tracker, watcher, mut consumer) =
        a_watcher_following_a_stream("watch-stream-dropped");
    serve.sends("id: 8\ndata: {\"seq\":8}\n\n");

    serve.hangs_up();
    serve.opens_the_stream_after(8);
    tracker.holds(&the_described_subtree_with(|rows| {
        closing(rows, "dun-0tp.9")
    }));
    serve.sends("id: 9\ndata: {\"seq\":9}\n\n");

    until_told_closed(&mut consumer, "dun-0tp.9");
    stopped(watcher);
}

/// Nothing polls a project while its stream is down, so only the watcher
/// can tell a consumer that the beads it holds are no longer vouched for.
#[test]
fn a_stream_down_past_covered_for_seconds_has_its_consumers_told_the_project_lapsed() {
    let mut serve = BdServe::listening();
    let settings = format!("{}[changes]\ncovered_for_seconds = 1\n", serve.named());
    let (home, _tracker, watcher) = a_watcher_over("watch-stream-lapsed", &settings, |_| {});
    serve.answers_the_head(7);
    serve.opens_the_stream_after(7);
    let mut consumer = Consumer::connected_to(&watching_at(&home));
    let first = consumer.sends("watch arkham").hears_an_answer();
    assert_eq!(first.last().expect("a freshness line")["lapsed"], false);

    serve.hangs_up();
    let lapsed = loop {
        let line = consumer.hears().expect("the watcher stays up");
        if line["line"] == "freshness" && line["lapsed"] == true {
            break line;
        }
    };
    serve.opens_the_stream_after(7);
    let resumed = loop {
        let line = consumer.hears().expect("the watcher stays up");
        if line["line"] == "freshness" {
            break line;
        }
    };

    assert_eq!(resumed["lapsed"], false);
    assert!(resumed["as_of"].as_str() > lapsed["as_of"].as_str());
    stopped(watcher);
}

/// No record says what was written while the stream could not reach it, so
/// the project is read in full, and the consumer is sent what that read finds.
#[test]
fn a_stream_the_journal_was_pruned_under_is_read_in_full_and_followed_from_its_head() {
    let (mut serve, tracker, watcher, mut consumer) =
        a_watcher_following_a_stream("watch-stream-pruned");

    tracker.holds(&the_described_subtree_with(|rows| {
        closing(rows, "dun-0tp.9")
    }));
    serve.sends("event: truncated\ndata: {\"code\":\"events_journal_truncated\",\"head\":31}\n\n");
    serve.hangs_up();
    serve.answers_the_head(31);

    until_told_closed(&mut consumer, "dun-0tp.9");
    serve.opens_the_stream_after(31);
    stopped(watcher);
}
