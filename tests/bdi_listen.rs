//! `bdi listen` reads every configured project, reads one again when a
//! producer reports it, and will not start beside a listener already running.
//!
//! The cases run the binary, because what is under test is the process a
//! supervisor starts and the socket it leaves on the machine.

mod terminal;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use terminal::shims::ShimmedTracker;
use terminal::{a_home_naming_one_project_settled, Producer, THE_DESCRIBED_SUBTREE};

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
    command
}

/// A listener started over a tracker holding the described subtree, once its
/// first read of the tracker is in.
fn a_listener(named: &str) -> (PathBuf, ShimmedTracker, Child) {
    let home = a_home_naming_one_project_settled(named, NOT_POLLED);
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_DESCRIBED_SUBTREE);
    let listener = bdi_listen(&home, &tracker)
        .spawn()
        .expect("bdi listen starts");
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
fn stopped(listener: Child) -> Output {
    let signalled = Command::new("kill")
        .args(["-TERM", &listener.id().to_string()])
        .status()
        .expect("kill runs");
    assert!(signalled.success());
    listener.wait_with_output().expect("bdi listen exits")
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
