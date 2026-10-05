//! `bdi listen` reads every configured project, reads one again when a
//! producer reports it, will not start beside a listener already running, and
//! sends each consumer the beads it watches. A one-shot `bdi` reads through
//! it where one is running, and for itself where none is.
//!
//! The cases run the binary, because what is under test is the process a
//! supervisor starts and the socket it leaves on the machine.

mod terminal;

use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use terminal::driver::{Driven, GIVING_UP as THE_SCREEN_GIVING_UP};
use terminal::shims::ShimmedTracker;
use terminal::{
    a_home_naming_one_project_settled, die_with, Producer, ENTER_ALTERNATE_SCREEN,
    THE_DESCRIBED_SUBTREE,
};

/// `a`, which shows every tree rather than only those with a live agent. No
/// pane sits in the temp `HOME`, so without it the one tree here sits behind
/// its project's *no live agent* line and draws no row of its own.
const SHOW_EVERY_TREE: &[u8] = b"a";

/// The call that reads a tracker in full.
const READ_IN_FULL: &str = "list --all --limit 0 --json";

/// Long enough for a read that is coming to have come, and short enough
/// that one that is not is a failure rather than a hang.
const GIVING_UP: Duration = Duration::from_secs(10);

/// A project that never polls, so every read after the first is one a
/// producer asked for.
const NOT_POLLED: &str = "poll = false\n";

fn listening_at(home: &Path) -> PathBuf {
    home.join("listener.sock")
}

fn bdi_listen(home: &Path, tracker: &ShimmedTracker) -> Command {
    let spawned_by = std::process::id();
    let mut command = Command::new(env!("CARGO_BIN_EXE_bdi"));
    command
        .args(["listen", "--socket"])
        .arg(listening_at(home))
        .current_dir(home)
        .env("HOME", home)
        .env_remove("BEADS_DIR")
        .env_remove("BEADS_DOLT_PASSWORD")
        .env_remove("BDI_PROJECT")
        .envs(tracker.environment())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    // Dropping a `Child` leaves its process running, and a test binary that
    // is killed runs no `Drop`, so the kernel is asked to end the listener
    // with its spawner.
    unsafe { command.pre_exec(move || die_with(spawned_by)) };
    command
}

/// A running `bdi listen` that is killed and reaped if the test ends without
/// stopping it, which a panic does.
struct Listener(Option<Child>);

impl Listener {
    fn stopping(mut self) -> Child {
        self.0.take().expect("the listener is still held")
    }

    fn pid(&self) -> u32 {
        self.0.as_ref().expect("the listener is still held").id()
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// A listener started over a tracker holding the described subtree, once its
/// first read of the tracker is in.
fn a_listener(named: &str) -> (PathBuf, ShimmedTracker, Listener) {
    a_listener_over(named, NOT_POLLED, |_| {})
}

/// The same, with `settings` on the project's entry and the tracker staged by
/// `staging` before the listener starts.
fn a_listener_over(
    named: &str,
    settings: &str,
    staging: impl FnOnce(&ShimmedTracker),
) -> (PathBuf, ShimmedTracker, Listener) {
    let home = a_home_naming_one_project_settled(named, settings);
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_DESCRIBED_SUBTREE);
    staging(&tracker);
    let listener = Listener(Some(
        bdi_listen(&home, &tracker)
            .spawn()
            .expect("bdi listen starts"),
    ));
    until(|| reads_in_full(&tracker) == 1, "the first read");
    until(|| listening_at(&home).exists(), "the socket");
    (home, tracker, listener)
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

/// Ask `listener` to stop as a supervisor does, and take what it said.
fn stopped(listener: Listener) -> Output {
    let signalled = Command::new("kill")
        .args(["-TERM", &listener.pid().to_string()])
        .status()
        .expect("kill runs");
    assert!(signalled.success());
    listener
        .stopping()
        .wait_with_output()
        .expect("bdi listen exits")
}

#[test]
fn a_project_a_producer_reports_is_read_again() {
    let (home, tracker, listener) = a_listener("listen-reads-again");

    let answer = Producer::connected_to(&listening_at(&home)).says("arkham");
    until(
        || reads_in_full(&tracker) == 2,
        "the read the report asked for",
    );

    let out = stopped(listener);
    assert_eq!(answer, "ok arkham");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stderr), "");
    assert!(
        !listening_at(&home).exists(),
        "the socket goes with the listener"
    );
}

#[test]
fn a_second_listener_on_the_same_socket_will_not_start_and_says_where() {
    let (home, tracker, listener) = a_listener("listen-twice");

    let second = bdi_listen(&home, &tracker)
        .output()
        .expect("bdi listen runs");

    stopped(listener);
    let said = String::from_utf8_lossy(&second.stderr);
    assert!(!second.status.success());
    assert!(
        said.contains(&listening_at(&home).display().to_string()),
        "the socket is named: {said}"
    );
    assert_eq!(reads_in_full(&tracker), 1, "the second read nothing");
}

/// Something outside `bdi` watching beads on the listener's socket.
struct Consumer {
    speaking: UnixStream,
    listening: BufReader<UnixStream>,
}

impl Consumer {
    fn connected_to(at: &Path) -> Self {
        let speaking = UnixStream::connect(at)
            .unwrap_or_else(|why| panic!("bdi listen is on {} ({why})", at.display()));
        let listening = speaking
            .try_clone()
            .expect("the connection is ours to read");
        listening
            .set_read_timeout(Some(GIVING_UP))
            .expect("a read nothing answers is ours to give up on");
        Self {
            speaking,
            listening: BufReader::new(listening),
        }
    }

    fn sends(&mut self, line: &str) -> &mut Self {
        writeln!(self.speaking, "{line}").expect("the line is ours to send");
        self
    }

    /// The next line the listener sends, or nothing where it closed the
    /// connection.
    fn hears(&mut self) -> Option<Value> {
        let mut line = String::new();
        match self.listening.read_line(&mut line) {
            Ok(0) => None,
            Ok(_) => Some(
                serde_json::from_str(&line)
                    .unwrap_or_else(|why| panic!("the listener sends JSON lines ({why}): {line}")),
            ),
            Err(why) if why.kind() == ErrorKind::ConnectionReset => None,
            Err(why) => panic!("the listener said nothing in time ({why})"),
        }
    }

    /// Every line up to and including the next freshness line: one answer
    /// for one project.
    fn hears_an_answer(&mut self) -> Vec<Value> {
        let mut answer = Vec::new();
        loop {
            let line = self.hears().expect("the listener stays up");
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
/// listener reads it again.
fn the_tracker_now_holds(home: &Path, tracker: &ShimmedTracker, capture: &str) {
    let before = reads_in_full(tracker);
    tracker.holds(capture);
    Producer::connected_to(&listening_at(home)).says("arkham");
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
    let (home, _tracker, listener) = a_listener("listen-watch");

    let answer = Consumer::connected_to(&listening_at(&home))
        .sends("watch arkham")
        .hears_an_answer();

    stopped(listener);
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

#[test]
fn a_bead_created_after_the_consumer_connected_is_sent_and_one_that_goes_is_gone() {
    let (home, tracker, listener) = a_listener("listen-learns-of-new-beads");
    let mut consumer = Consumer::connected_to(&listening_at(&home));
    consumer.sends("watch arkham").hears_an_answer();

    the_tracker_now_holds(
        &home,
        &tracker,
        &the_described_subtree_with(|rows| {
            let mut created = rows[1].clone();
            created["id"] = json!("dun-0tp.10");
            rows.push(created);
            rows.retain(|row| row["id"] != "dun-0tp.6");
        }),
    );
    let answer = consumer.hears_an_answer();

    stopped(listener);
    assert_eq!(beads_in(&answer), ["dun-0tp.10"]);
    assert!(
        answer.contains(&json!({ "line": "gone", "project": "arkham", "id": "dun-0tp.6" })),
        "{answer:?}"
    );
}

#[test]
fn a_report_that_changes_nothing_is_answered_by_freshness_alone() {
    let (home, tracker, listener) = a_listener("listen-quiet");
    let mut consumer = Consumer::connected_to(&listening_at(&home));
    let first = consumer.sends("watch arkham").hears_an_answer();

    the_tracker_now_holds(&home, &tracker, THE_DESCRIBED_SUBTREE);
    let answer = consumer.hears_an_answer();

    stopped(listener);
    assert_eq!(answer.len(), 1, "{answer:?}");
    assert_ne!(
        answer[0]["as_of"],
        first.last().expect("an answer ends")["as_of"],
        "the second read vouches for a later instant"
    );
}

/// `watch` starts from the beads that are not closed and is told when one
/// closes. `watch-all` starts from every bead, and a reconnect is sent the
/// beads as they now stand.
#[test]
fn a_watch_is_told_of_a_close_and_a_reconnect_is_sent_the_bead_as_it_stands() {
    let (home, tracker, listener) = a_listener("listen-close");
    let mut watching = Consumer::connected_to(&listening_at(&home));
    watching.sends("watch arkham").hears_an_answer();

    the_tracker_now_holds(
        &home,
        &tracker,
        &the_described_subtree_with(|rows| closing(rows, "dun-0tp.7")),
    );
    let told = watching.hears_an_answer();
    let open_only = Consumer::connected_to(&listening_at(&home))
        .sends("watch arkham")
        .hears_an_answer();
    let everything = Consumer::connected_to(&listening_at(&home))
        .sends("watch-all arkham")
        .hears_an_answer();

    stopped(listener);
    assert_eq!(beads_in(&told), ["dun-0tp.7"]);
    assert_eq!(told[0]["row"]["status"], "closed");
    assert_eq!(
        beads_in(&open_only),
        ["dun-0tp", "dun-0tp.6", "dun-0tp.8", "dun-0tp.9"]
    );
    assert_eq!(beads_in(&everything), EVERY_DESCRIBED_BEAD);
}

#[test]
fn a_watch_on_one_bead_is_sent_that_bead_whatever_its_status() {
    let (home, tracker, listener) = a_listener("listen-one-bead");
    let mut waiting = Consumer::connected_to(&listening_at(&home));
    waiting.sends("watch arkham").hears_an_answer();
    the_tracker_now_holds(
        &home,
        &tracker,
        &the_described_subtree_with(|rows| closing(rows, "dun-0tp.7")),
    );
    waiting.hears_an_answer();

    let mut consumer = Consumer::connected_to(&listening_at(&home));
    let closed = consumer.sends("watch arkham dun-0tp.7").hears_an_answer();
    let never_held = consumer.sends("watch arkham dun-0tp.99").hears_an_answer();

    stopped(listener);
    assert_eq!(beads_in(&closed), ["dun-0tp.7"]);
    assert_eq!(closed[0]["row"]["status"], "closed");
    assert_eq!(
        closed.len(),
        2,
        "the bead and its project's freshness: {closed:?}"
    );
    assert_eq!(
        never_held[0],
        json!({ "line": "gone", "project": "arkham", "id": "dun-0tp.99" })
    );
    assert_eq!(never_held.len(), 2, "{never_held:?}");
}

#[test]
fn every_project_is_watched_by_a_bare_watch() {
    let (home, _tracker, listener) = a_listener("listen-everything");

    let answer = Consumer::connected_to(&listening_at(&home))
        .sends("watch")
        .hears_an_answer();

    stopped(listener);
    assert_eq!(beads_in(&answer), EVERY_DESCRIBED_BEAD);
}

#[test]
fn a_line_the_listener_cannot_serve_is_refused_and_the_connection_goes_on() {
    let (home, _tracker, listener) = a_listener("listen-refuses");
    let mut consumer = Consumer::connected_to(&listening_at(&home));

    let unknown = consumer.sends("watch innsmouth").hears();
    let malformed = consumer.sends("watch-all").hears();
    let answer = consumer.sends("watch arkham").hears_an_answer();

    stopped(listener);
    assert_eq!(
        unknown,
        Some(json!({ "line": "refused", "asked": "watch innsmouth", "reason": "unknown-project" }))
    );
    assert_eq!(
        malformed,
        Some(json!({ "line": "refused", "asked": "watch-all", "reason": "malformed" }))
    );
    assert_eq!(beads_in(&answer), EVERY_DESCRIBED_BEAD);
}

/// What a consumer that acts only on what it is told relies on: a listener
/// that goes closes the connection, and the next connect is refused.
#[test]
fn a_listener_that_stops_closes_its_consumers_and_takes_no_more() {
    let (home, _tracker, listener) = a_listener("listen-gone");
    let mut consumer = Consumer::connected_to(&listening_at(&home));
    consumer.sends("watch arkham").hears_an_answer();

    stopped(listener);

    assert_eq!(consumer.hears(), None);
    assert!(UnixStream::connect(listening_at(&home)).is_err());
}

/// The journal's read past where it ended when the listener first read it.
const THE_JOURNAL_SINCE_IT_STARTED: &str = "events tail --since 0";

/// A comment on, a dependency added to and a close of `dun-0tp.7`, and an
/// update to `dun-0tp.8`: the shape bd 1.3.0 writes, onto the described
/// subtree's beads.
fn the_journal_of_dun_0tp_7() -> String {
    let ids = ["dun-0tp.7", "dun-0tp.7", "dun-0tp.8", "dun-0tp.7"];
    include_str!("fixtures/bd_1.3.0_events_tail.jsonl")
        .lines()
        .zip(ids)
        .enumerate()
        .map(|(at, (line, id))| {
            let mut record: Value = serde_json::from_str(line).expect("the capture is bd's JSON");
            record["seq"] = json!(at + 1);
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
    let (home, tracker, listener) = a_listener_over(
        "listen-journal",
        &format!("{NOT_POLLED}events_journal = true\n"),
        |tracker| tracker.answers_with(THE_JOURNAL_SINCE_IT_STARTED, ""),
    );
    let mut consumer = Consumer::connected_to(&listening_at(&home));
    let first = consumer.sends("watch arkham dun-0tp.7").hears_an_answer();

    tracker.answers_with(THE_JOURNAL_SINCE_IT_STARTED, &the_journal_of_dun_0tp_7());
    the_tracker_now_holds(
        &home,
        &tracker,
        &the_described_subtree_with(|rows| closing(rows, "dun-0tp.7")),
    );
    let answer = consumer.hears_an_answer();

    stopped(listener);
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

#[test]
fn a_project_that_claims_no_journal_is_said_to_have_no_events_and_its_journal_is_never_read() {
    let (home, tracker, listener) = a_listener_over("listen-no-journal", NOT_POLLED, |tracker| {
        tracker.answers_with(THE_JOURNAL_SINCE_IT_STARTED, &the_journal_of_dun_0tp_7())
    });
    let mut consumer = Consumer::connected_to(&listening_at(&home));
    consumer.sends("watch arkham").hears_an_answer();

    the_tracker_now_holds(
        &home,
        &tracker,
        &the_described_subtree_with(|rows| closing(rows, "dun-0tp.7")),
    );
    let answer = consumer.hears_an_answer();

    stopped(listener);
    assert_eq!(events_in(&answer), Vec::<&str>::new());
    assert_eq!(answer.last().expect("an answer ends")["events"], "off");
    assert!(
        !tracker
            .calls()
            .iter()
            .any(|call| call.starts_with("events")),
        "{:?}",
        tracker.calls()
    );
}

/// A home whose config tells every run where the listener is, as a setup
/// that starts one says it, holding the described subtree.
fn a_home_with_a_listener_configured(named: &str) -> (PathBuf, ShimmedTracker) {
    let home = a_home_naming_one_project_settled(named, NOT_POLLED);
    let config = home.join(".config/beady-eye/config.toml");
    let mut text = std::fs::read_to_string(&config).expect("the config was just written");
    text.push_str(&format!(
        "\n[listener]\nsocket = \"{}\"\n",
        listening_at(&home).display()
    ));
    std::fs::write(&config, text).expect("the config is ours to write");
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_DESCRIBED_SUBTREE);
    (home, tracker)
}

/// A listener started in `home`, once it has answered for its first read of
/// the tracker, which is when every call of that read has been made.
fn listening_in(home: &Path, tracker: &ShimmedTracker) -> Listener {
    let listener = Listener(Some(
        bdi_listen(home, tracker)
            .spawn()
            .expect("bdi listen starts"),
    ));
    until(|| listening_at(home).exists(), "the socket");
    Consumer::connected_to(&listening_at(home))
        .sends("watch arkham")
        .hears_an_answer();
    listener
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

/// Both one-shots, each beside a listener and then with none, so the two
/// answers can be set side by side.
#[test]
fn a_one_shot_beside_a_listener_reads_no_tracker_and_says_what_its_own_read_says() {
    for (named, args) in [
        ("listened-json", &["--json", "--all"][..]),
        ("listened-beads", &["--beads"][..]),
    ] {
        let (home, tracker) = a_home_with_a_listener_configured(named);
        let listener = listening_in(&home, &tracker);
        let asked_before = tracker.calls();

        let (through_the_listener, dated) = one_shot(&home, &tracker, args);

        let asked_after = tracker.calls();
        stopped(listener);
        let (read_for_itself, _) = one_shot(&home, &tracker, args);
        assert_eq!(asked_after, asked_before, "{args:?} asked bd nothing");
        assert_eq!(through_the_listener, read_for_itself, "{args:?}");
        assert!(dated.is_string(), "{args:?} is dated: {dated}");
    }
}

/// A one-shot is dated to the listener's read rather than to the instant it
/// asked, so it says how old what it says is.
#[test]
fn a_one_shot_beside_a_listener_is_dated_to_the_listeners_read() {
    let (home, tracker) = a_home_with_a_listener_configured("listened-dated");
    let listener = listening_in(&home, &tracker);
    let freshness = Consumer::connected_to(&listening_at(&home))
        .sends("watch arkham")
        .hears_an_answer()
        .pop()
        .expect("an answer ends");

    let (_, dated) = one_shot(&home, &tracker, &["--json", "--all"]);

    stopped(listener);
    assert_eq!(dated, freshness["as_of"]);
}

/// A listener that hangs up without answering is one that is not running,
/// and the run reads its tracker as it would with none configured.
#[test]
fn a_one_shot_whose_listener_hangs_up_reads_for_itself() {
    let (home, tracker) = a_home_with_a_listener_configured("listened-hangs-up");
    let hanging_up = UnixListener::bind(listening_at(&home)).expect("the socket is ours");
    std::thread::spawn(move || {
        for connection in hanging_up.incoming() {
            drop(connection);
        }
    });

    let (written, _) = one_shot(&home, &tracker, &["--json", "--all"]);

    assert_eq!(reads_in_full(&tracker), 1, "the run read for itself");
    assert_eq!(written["failed_projects"], json!([]));
    assert_eq!(written["trees"][0]["root"], "dun-0tp");
}

/// Configured and not running is the ordinary state of a machine whose
/// listener is being restarted.
#[test]
fn a_one_shot_whose_listener_is_not_running_reads_for_itself() {
    let (home, tracker) = a_home_with_a_listener_configured("listened-not-running");

    let (written, _) = one_shot(&home, &tracker, &["--beads"]);

    assert!(tracker.read_the_tracker(), "the run read for itself");
    assert_eq!(written["failed_projects"], json!([]));
    assert!(
        !written["beads"].as_array().expect("beads").is_empty(),
        "{written}"
    );
}

/// A view beside a listener draws what the listener holds and asks bd
/// nothing, and reads the tracker itself as soon as the listener goes.
#[test]
fn a_view_beside_a_listener_reads_no_tracker_until_the_listener_goes() {
    let (home, tracker) = a_home_with_a_listener_configured("listened-view");
    let listener = listening_in(&home, &tracker);
    let asked_before = tracker.calls();

    let mut bdi = Driven::bdi(40, 120, home.clone(), &tracker.environment());
    bdi.read_until(ENTER_ALTERNATE_SCREEN, THE_SCREEN_GIVING_UP);
    bdi.send(SHOW_EVERY_TREE);
    bdi.read_until(b"dun-0tp", THE_SCREEN_GIVING_UP);
    bdi.settle(Duration::from_millis(300), THE_SCREEN_GIVING_UP);
    let asked_while_listening = tracker.calls();
    stopped(listener);

    assert_eq!(
        asked_while_listening, asked_before,
        "the view asked bd nothing"
    );
    until(|| reads_in_full(&tracker) > 1, "the view's own read");
}

/// `^R`, which asks every project for itself again.
const REFRESH: &[u8] = b"\x12";

/// A view draws what the listener holds, so asking for a read is asking
/// the listener for one.
#[test]
fn the_refresh_key_has_the_listener_a_view_reads_through_read_again() {
    let (home, tracker) = a_home_with_a_listener_configured("listened-view-refresh");
    let listener = listening_in(&home, &tracker);
    let mut bdi = Driven::bdi(40, 120, home.clone(), &tracker.environment());
    bdi.read_until(ENTER_ALTERNATE_SCREEN, THE_SCREEN_GIVING_UP);
    bdi.send(SHOW_EVERY_TREE);
    bdi.read_until(b"dun-0tp", THE_SCREEN_GIVING_UP);
    bdi.settle(Duration::from_millis(300), THE_SCREEN_GIVING_UP);
    let asked_before = tracker.calls().len();

    bdi.send(REFRESH);

    until(
        || tracker.calls().len() > asked_before,
        "the listener asking its tracker",
    );
    stopped(listener);
}

/// A test that panics before it stops its listener must not leave it behind.
#[test]
fn a_listener_a_test_abandons_is_killed() {
    let (_home, _tracker, listener) = a_listener("listen-abandoned");
    let pid = listener.pid();

    drop(listener);

    let still_there = Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .expect("kill runs");
    assert!(!still_there.success(), "listener {pid} outlived its test");
}
