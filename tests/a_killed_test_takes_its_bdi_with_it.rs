//! What a test binary that is killed leaves running.
//!
//! Every harness here reaps its `bdi` from `Drop`, and a process that is
//! killed runs no `Drop`. Nothing outside the process can pick up after it
//! either: `own_the_terminal` calls `setsid` so the pty can become the
//! child's controlling terminal, and that same call gives the child a session
//! and a process group of its own, where a killer working by process group
//! never finds it. Nor does the pty hang up when the test binary's fds close,
//! because `openpty` hands back a master with no close-on-exec and the child
//! inherits a copy of it.
//!
//! So the reaping has to be the kernel's. This runs the test binary as its
//! own child, kills it the way a mutation harness kills one that has stopped
//! answering, and asks whether the `bdi` underneath went too.
//!
//! Linux only, because a parent-death signal is. On a system without one the
//! `Drop` is all there is, and this asserts nothing.
#![cfg(target_os = "linux")]

mod terminal;

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use terminal::driver::{Driven, GIVING_UP};
use terminal::{a_home_naming_one_project, ENTER_ALTERNATE_SCREEN};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// What the half that gets killed says about itself before it does, so the
/// half doing the killing knows what to look for once it is gone.
const PID_SAID: &str = "the bdi is ";
const HOME_SAID: &str = "its home is ";

/// Long enough for a machine under a mutation run to start a `bdi` and say
/// which one. Only ever a giving-up point: nothing is asserted against the
/// clock.
const LONG_ENOUGH_TO_SPAWN: Duration = Duration::from_secs(60);
/// Long enough for a process the kernel has signalled to be gone.
const LONG_ENOUGH_TO_DIE: Duration = Duration::from_secs(10);
/// Long enough for the test below to do the killing, and short enough that a
/// stray `--ignored` run of the suite ends rather than hangs.
const LONG_ENOUGH_TO_BE_KILLED: Duration = Duration::from_secs(120);

/// A `bdi` and a parent that is about to be killed without reaping it.
///
/// Ignored because it is a fixture rather than a test: it is what
/// [`a_killed_test_leaves_no_bdi_behind`] runs this binary again to reach.
/// Run on its own it waits to be killed, then tidies up after itself.
#[test]
#[ignore = "the subject of a_killed_test_leaves_no_bdi_behind, which runs it"]
fn a_bdi_whose_parent_is_about_to_be_killed() {
    let home = a_home_naming_one_project("orphaned");
    let mut driven = Driven::bdi(ROWS, COLS, home.clone(), &[]);
    // Waiting for the screen is what makes this the fault rather than a
    // sketch of it. The `bdi` processes that outlived their mutation runs had
    // exec'd and drawn, and the arming this holds is done before the exec, so
    // a kill that lands first would say nothing about whether it survives one.
    driven.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP);

    let mut told = std::io::stdout();
    writeln!(told, "{PID_SAID}{}", driven.pid()).expect("stdout takes it");
    writeln!(told, "{HOME_SAID}{}", home.display()).expect("stdout takes it");
    told.flush().expect("stdout takes it");

    // `Driven` holds the pty master, so nothing about this end going away is
    // a hangup — which is the whole of why these outlive their spawner.
    std::thread::sleep(LONG_ENOUGH_TO_BE_KILLED);
    drop(driven);
}

#[test]
fn a_killed_test_leaves_no_bdi_behind() {
    let mut parent = a_test_binary_holding_a_bdi();
    let Some((bdi, home)) = what_it_started(&mut parent) else {
        let _ = parent.kill();
        let _ = parent.wait();
        panic!("the half that gets killed never said which bdi it started");
    };

    // A `bdi` that never started would be reported gone by everything below,
    // and this would pass without the fix it is here to hold. Its parent
    // never reaps it, so one that died is a zombie and still says so.
    let started = running(bdi);

    parent.kill().expect("the kill is ours to send");
    parent.wait().expect("it is ours to reap");
    let outlived = still_running_after(bdi, LONG_ENOUGH_TO_DIE);

    // Whichever way that went, do not become the leak this test is about.
    if outlived {
        unsafe { libc::kill(bdi, libc::SIGKILL) };
    }
    let _ = std::fs::remove_dir_all(&home);
    assert!(
        started,
        "bdi {bdi} was never running, so nothing was killed"
    );
    assert!(
        !outlived,
        "bdi {bdi} was still running {}s after the test binary that started \
         it was killed, which is how a mutation run leaves one holding the \
         inbound socket",
        LONG_ENOUGH_TO_DIE.as_secs()
    );
}

/// This binary again, running the one test above that starts a `bdi` and
/// waits.
fn a_test_binary_holding_a_bdi() -> Child {
    Command::new(std::env::current_exe().expect("this binary has a path"))
        .args([
            "--exact",
            "--ignored",
            "--nocapture",
            "a_bdi_whose_parent_is_about_to_be_killed",
        ])
        .stdout(Stdio::piped())
        .spawn()
        .expect("this binary runs")
}

/// The `bdi` the spawned half started, and the home it made for it. `None`
/// where it never said, so a failure to start is not read as a process that
/// died.
fn what_it_started(parent: &mut Child) -> Option<(libc::pid_t, PathBuf)> {
    let told = BufReader::new(parent.stdout.take().expect("the pipe is ours"));
    let (said, heard) = mpsc::channel();
    std::thread::spawn(move || {
        for line in told.lines().map_while(Result::ok) {
            if said.send(line).is_err() {
                return;
            }
        }
    });

    let mut pid = None;
    let mut home = None;
    let giving_up = Instant::now() + LONG_ENOUGH_TO_SPAWN;
    while (pid.is_none() || home.is_none()) && Instant::now() < giving_up {
        let Ok(line) = heard.recv_timeout(Duration::from_millis(200)) else {
            continue;
        };
        if let Some(said) = line.strip_prefix(PID_SAID) {
            pid = said.trim().parse().ok();
        }
        if let Some(said) = line.strip_prefix(HOME_SAID) {
            home = Some(PathBuf::from(said.trim()));
        }
    }
    Some((pid?, home?))
}

/// Whether this process is still running, having waited this long for it to
/// stop.
///
/// A zombie counts as stopped. A killed orphan is reparented, and what reaps
/// it — a subreaper, `init`, a build sandbox's stub — is the machine's choice
/// and not this test's subject.
fn still_running_after(pid: libc::pid_t, patience: Duration) -> bool {
    let giving_up = Instant::now() + patience;
    while running(pid) && Instant::now() < giving_up {
        std::thread::sleep(Duration::from_millis(50));
    }
    running(pid)
}

fn running(pid: libc::pid_t) -> bool {
    let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status")) else {
        return false;
    };
    !status
        .lines()
        .any(|line| line.starts_with("State:") && line.contains('Z'))
}
