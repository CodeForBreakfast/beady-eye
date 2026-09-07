//! What a test binary that is killed leaves running.
//!
//! Every harness here reaps its `bdi` from `Drop`, and a process that is
//! killed runs no `Drop`. Nothing outside the process can pick up after it
//! either: `own_the_terminal` calls `setsid` so the pty can become the
//! child's controlling terminal, and that same call gives the child a session
//! and a process group of its own, where a killer working by process group
//! never finds it.
//!
//! So the reaping has to be the kernel's. This runs the test binary as its
//! own child, kills it the way a mutation harness kills one that has stopped
//! answering, and asks whether the `bdi` underneath went too.
//!
//! **The pty is opened here and the master stays here**, which is what makes
//! the answer mean anything. A pty hangs up when its last master handle
//! closes, and the kernel then sends `SIGHUP` to the session's foreground
//! process group, which `bdi` handles by exiting. Hand the master down to the
//! half that gets killed and its death hangs the pty up, so the `bdi` is
//! reaped whether or not the parent-death signal is armed and this passes
//! either way. Holding the master here keeps it open across the kill, so no
//! hangup is possible and the arming is the only thing left that can reap.
//!
//! That is also the case the arming exists for. Reaping by hangup runs
//! through `bdi`'s own `SIGHUP` handling, which is product code a mutant is
//! free to break — and these leaks happened under mutation. A killer holding
//! the master open stages that without having to mutate anything.
//!
//! Linux only, because a parent-death signal is. On a system without one the
//! `Drop` is all there is, and this asserts nothing.
#![cfg(target_os = "linux")]

mod terminal;

use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use terminal::driver::GIVING_UP;
use terminal::{a_home_naming_one_project, a_pty, bdi_on, contains, ENTER_ALTERNATE_SCREEN};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// What the half that gets killed says about itself before it does, so the
/// half doing the killing knows what to look for once it is gone.
const PID_SAID: &str = "the bdi is ";
const HOME_SAID: &str = "its home is ";

/// Where the half that gets killed finds the end to draw `bdi` on. `openpty`
/// sets close-on-exec on neither end and only the master is given it, so the
/// slave arrives across the exec at the number it was opened on.
const SLAVE_FD: &str = "BDI_TEST_PTY_SLAVE_FD";

/// Long enough for a process the kernel has signalled to be gone.
const LONG_ENOUGH_TO_DIE: Duration = Duration::from_secs(10);
/// Long enough for the test below to do the killing, and short enough that a
/// fixture whose killer died before doing it ends on its own.
const LONG_ENOUGH_TO_BE_KILLED: Duration = Duration::from_secs(120);
/// How long a poll blocks before looking at the clock again.
const A_GLANCE: Duration = Duration::from_millis(50);

/// A `bdi` and a parent that is about to be killed without reaping it.
///
/// Ignored because it is a fixture rather than a test: it is what
/// [`a_killed_test_leaves_no_bdi_behind`] runs this binary again to reach.
/// The pty comes from that test, so a run reaching this with none — anyone
/// asking a suite for its ignored tests — has nothing to be and does nothing.
/// Left alone rather than killed it tidies up after itself.
#[test]
#[ignore = "the subject of a_killed_test_leaves_no_bdi_behind, which runs it"]
fn a_bdi_whose_parent_is_about_to_be_killed() {
    let Ok(handed_down) = std::env::var(SLAVE_FD) else {
        return;
    };
    let theirs = unsafe {
        std::fs::File::from_raw_fd(
            handed_down
                .parse()
                .unwrap_or_else(|_| panic!("{SLAVE_FD} is a file descriptor: {handed_down:?}")),
        )
    };

    let home = a_home_naming_one_project("orphaned");
    let mut child = bdi_on(&theirs, &home, &[], &[]);
    // Read before anything can reap it: its pid is ours until we do, so what
    // this names is the `bdi` and not a later holder of the number.
    let bdi = Process::named(child.id() as libc::pid_t).expect("it was spawned");

    let mut told = std::io::stdout();
    writeln!(told, "{PID_SAID}{bdi}").expect("stdout takes it");
    writeln!(told, "{HOME_SAID}{}", home.display()).expect("stdout takes it");
    told.flush().expect("stdout takes it");

    std::thread::sleep(LONG_ENOUGH_TO_BE_KILLED);
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&home);
}

/// Asking a suite for its ignored tests reaches the fixture above with no pty
/// handed down, and what it does then is invisible from the run that asks:
/// libtest reports an `#[ignore]`d test that returns and one that was never
/// run the same way. So it is asked for here, where a failure is a failure.
#[test]
fn the_fixture_does_nothing_without_a_pty() {
    let asked = Command::new(std::env::current_exe().expect("this binary has a path"))
        .args([
            "--exact",
            "--ignored",
            "a_bdi_whose_parent_is_about_to_be_killed",
        ])
        .env_remove(SLAVE_FD)
        .output()
        .expect("this binary runs");

    assert!(
        asked.status.success(),
        "the fixture wanted a pty nobody handed it: {}{}",
        String::from_utf8_lossy(&asked.stdout),
        String::from_utf8_lossy(&asked.stderr)
    );
}

#[test]
fn a_killed_test_leaves_no_bdi_behind() {
    let (ours, theirs) = a_pty(ROWS, COLS);
    let mut parent = a_test_binary_holding_a_bdi(&theirs);
    drop(theirs);

    let Some((bdi, home)) = what_it_started(&mut parent) else {
        let _ = parent.kill();
        let _ = parent.wait();
        panic!("the half that gets killed never said which bdi it started");
    };

    // Waiting for the screen is what makes this the fault rather than a
    // sketch of it. The `bdi` processes that outlived their mutation runs had
    // exec'd and drawn, and the arming this holds is done before the exec, so
    // a kill that lands first would say nothing about whether it survives one.
    if let Err(said) = wait_until_drawn(&ours, &bdi) {
        // Dropping a `Child` does not kill it, and a test about processes
        // left running is the last one that should leave any.
        let _ = parent.kill();
        let _ = parent.wait();
        let _ = std::fs::remove_dir_all(&home);
        panic!(
            "bdi never put the terminal on the alternate screen — it is {}; \
             it wrote {} bytes: {:?}",
            state_of(&bdi),
            said.len(),
            String::from_utf8_lossy(&said)
        );
    }

    // A `bdi` that never started would be reported gone by everything below,
    // and this would pass without the fix it is here to hold. Its parent
    // never reaps it, so one that died is a zombie and still says so.
    let started = state_of(&bdi);

    parent.kill().expect("the kill is ours to send");
    parent.wait().expect("it is ours to reap");
    let outlived = still_running_after(LONG_ENOUGH_TO_DIE, || state_of(&bdi));

    // Held open until here on purpose — see the module doc. Let it go any
    // earlier and the pty hangs up, which reaps the `bdi` by the one path
    // this test is written to exclude.
    drop(ours);

    // Whichever way that went, do not become the leak this test is about.
    if outlived.is_running() {
        unsafe { libc::kill(bdi.pid, libc::SIGKILL) };
    }
    let _ = std::fs::remove_dir_all(&home);
    assert!(
        started.is_running(),
        "bdi {bdi} was never running — it was {started} — so nothing was killed"
    );
    assert!(
        !outlived.is_running(),
        "bdi {bdi} was still {outlived} {}s after the test binary that \
         started it was killed, which is how a mutation run leaves one \
         holding the inbound socket",
        LONG_ENOUGH_TO_DIE.as_secs()
    );
}

/// This binary again, running the one test above that starts a `bdi` and
/// waits, drawing on the end of our pty we hand it.
fn a_test_binary_holding_a_bdi(theirs: &std::fs::File) -> Child {
    Command::new(std::env::current_exe().expect("this binary has a path"))
        .args([
            "--exact",
            "--ignored",
            "--nocapture",
            "a_bdi_whose_parent_is_about_to_be_killed",
        ])
        .env(SLAVE_FD, theirs.as_raw_fd().to_string())
        .stdout(Stdio::piped())
        .spawn()
        .expect("this binary runs")
}

/// Read our end until `bdi` has put the terminal on the alternate screen, or
/// give up and hand back what it said instead — at once where `bdi` has
/// stopped running, once its last words are read, since nothing more is
/// coming from it. The `bdi` is not ours to `wait` on, so `/proc` is asked
/// rather than the child.
fn wait_until_drawn(terminal: &OwnedFd, bdi: &Process) -> Result<(), Vec<u8>> {
    let mut said = Vec::new();
    let giving_up = Instant::now() + GIVING_UP;
    while Instant::now() < giving_up {
        if contains(&said, ENTER_ALTERNATE_SCREEN) {
            return Ok(());
        }
        if !state_of(bdi).is_running() {
            while read_some(terminal, &mut said) {}
            if contains(&said, ENTER_ALTERNATE_SCREEN) {
                return Ok(());
            }
            return Err(said);
        }
        read_some(terminal, &mut said);
    }
    Err(said)
}

/// One poll, and whatever was ready when it returned. Says whether anything
/// was.
fn read_some(terminal: &OwnedFd, into: &mut Vec<u8>) -> bool {
    let mut polling = libc::pollfd {
        fd: terminal.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    unsafe { libc::poll(&mut polling, 1, A_GLANCE.as_millis() as i32) };
    let mut buffer = [0u8; 8192];
    let mut reading = unsafe { std::fs::File::from_raw_fd(terminal.as_raw_fd()) };
    let read = reading.read(&mut buffer);
    std::mem::forget(reading);
    match read {
        Ok(count) if count > 0 => {
            into.extend_from_slice(&buffer[..count]);
            true
        }
        _ => false,
    }
}

/// The `bdi` the spawned half started, and the home it made for it. `None`
/// where it never said, so a failure to start is not read as a process that
/// died.
fn what_it_started(parent: &mut Child) -> Option<(Process, PathBuf)> {
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
    let giving_up = Instant::now() + GIVING_UP;
    while (pid.is_none() || home.is_none()) && Instant::now() < giving_up {
        let Ok(line) = heard.recv_timeout(Duration::from_millis(200)) else {
            continue;
        };
        if let Some(said) = line.strip_prefix(PID_SAID) {
            pid = Process::heard(said.trim());
        }
        if let Some(said) = line.strip_prefix(HOME_SAID) {
            home = Some(PathBuf::from(said.trim()));
        }
    }
    Some((pid?, home?))
}

/// What a process is, having waited this long for it to stop.
///
/// A zombie counts as stopped. A killed orphan is reparented, and what reaps
/// it — a subreaper, `init`, a build sandbox's stub — is the machine's choice
/// and not this test's subject.
fn still_running_after(patience: Duration, mut read: impl FnMut() -> State) -> State {
    let giving_up = Instant::now() + patience;
    loop {
        let state = read();
        if !state.is_running() || Instant::now() >= giving_up {
            return state;
        }
        std::thread::sleep(A_GLANCE);
    }
}

/// One process, told apart from any later one the kernel hands its pid to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Process {
    pid: libc::pid_t,
    /// When it started, in clock ticks since boot — `starttime` in
    /// `/proc/<pid>/stat`. A pid is handed on only once its holder is gone,
    /// so no two holders of one pid start on the same tick.
    started: u64,
}

impl Process {
    /// The process this pid names now, or `None` where it names none.
    fn named(pid: libc::pid_t) -> Option<Self> {
        let (_, started) = state_and_start(&stat_of(pid)?)?;
        Some(Self { pid, started })
    }

    /// The process a line of [`Display`](std::fmt::Display) output named.
    fn heard(said: &str) -> Option<Self> {
        let (pid, started) = said.split_once(STARTED_AT)?;
        Some(Self {
            pid: pid.parse().ok()?,
            started: started.parse().ok()?,
        })
    }
}

const STARTED_AT: &str = ", started at tick ";

impl std::fmt::Display for Process {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{STARTED_AT}{}", self.pid, self.started)
    }
}

impl std::fmt::Display for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            State::Alive { in_state } => write!(f, "running, in state {in_state}"),
            State::Zombie => write!(f, "a zombie"),
            State::Dead => write!(f, "dead, in state X, its pid not yet let go of"),
            State::Replaced { by_one_started } => write!(
                f,
                "gone, its pid held by a process started at tick {by_one_started}"
            ),
            State::Gone => write!(f, "gone"),
        }
    }
}

/// The line `/proc/<pid>/stat` holds for this pid, or `None` where it holds
/// none.
fn stat_of(pid: libc::pid_t) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()
}

/// What a line of `/proc/<pid>/stat` says its holder is doing, and when it
/// started. The command name sits in parentheses and may hold spaces, so the
/// fields are counted from the last closing one: `state` is the third field
/// and `starttime` the twenty-second.
fn state_and_start(stat: &str) -> Option<(char, u64)> {
    let after_name = stat.rsplit_once(')')?.1;
    let mut fields = after_name.split_whitespace();
    let state = fields.next()?.chars().next()?;
    let started = fields.nth(22 - 4)?.parse().ok()?;
    Some((state, started))
}

/// What became of a process, as `/proc` tells it.
#[derive(Debug, PartialEq, Eq)]
enum State {
    /// Still going, in the state `/proc` gives it — `D` for one the kernel
    /// has signalled but cannot yet take out of an uninterruptible wait.
    Alive { in_state: char },
    /// Dead, and waiting for a parent to reap it.
    Zombie,
    /// Dead and being reaped: `X`, which `/proc` shows from the moment the
    /// kernel takes a zombie for reaping until it lets the pid go, a few
    /// lines later in `wait_task_zombie`. A window of microseconds, and a
    /// read can land in it.
    Dead,
    /// Dead and reaped, its pid since handed to a process that started on
    /// this tick.
    Replaced { by_one_started: u64 },
    /// Dead and reaped, its pid held by nobody.
    Gone,
}

fn state_of(process: &Process) -> State {
    State::read(process, stat_of(process.pid).as_deref())
}

impl State {
    fn is_running(&self) -> bool {
        matches!(self, State::Alive { .. })
    }

    /// What one reading of `/proc/<pid>/stat` says became of this process,
    /// `None` being a pid the file system no longer has a line for.
    fn read(process: &Process, stat: Option<&str>) -> State {
        match stat.and_then(state_and_start) {
            None => State::Gone,
            Some((_, started)) if started != process.started => State::Replaced {
                by_one_started: started,
            },
            Some(('Z', _)) => State::Zombie,
            Some(('X', _)) => State::Dead,
            Some((in_state, _)) => State::Alive { in_state },
        }
    }
}

/// A process that held our pid before we did is a process that has gone,
/// whatever `/proc` says about the pid now.
#[test]
fn a_pid_handed_on_to_a_later_process_reads_as_gone() {
    let ours = Process::named(std::process::id() as libc::pid_t).expect("we are running");
    let before_us = Process {
        started: ours.started - 1,
        ..ours
    };
    assert_eq!(
        state_of(&before_us),
        State::Replaced {
            by_one_started: ours.started
        }
    );
}

#[test]
fn a_process_that_is_running_reads_as_alive() {
    let ours = Process::named(std::process::id() as libc::pid_t).expect("we are running");
    assert!(
        state_of(&ours).is_running(),
        "we are running: {:?}",
        state_of(&ours)
    );
}

/// `X` is a process past its exit whose pid has not yet been let go of.
/// Nothing is running there, and the letter is what a failure has to show.
#[test]
fn a_process_past_its_exit_reads_as_dead() {
    let ours = Process::named(std::process::id() as libc::pid_t).expect("we are running");
    let stat = stat_of(ours.pid).expect("we are running");
    let read = State::read(&ours, Some(&in_state(&stat, 'X')));
    assert!(!read.is_running(), "past its exit, it was read as {read}");
    assert!(
        read.to_string().contains('X'),
        "the letter is what the run reports, and it is missing from {read:?}: {read}"
    );
}

/// The same line of `/proc/<pid>/stat`, its state letter swapped. The letter
/// is the first field after the closing parenthesis of the name.
fn in_state(stat: &str, letter: char) -> String {
    let (name, after_name) = stat.rsplit_once(')').expect("a name in parentheses");
    let mut fields = after_name.split_whitespace();
    fields.next().expect("a state letter");
    let rest: Vec<&str> = fields.collect();
    format!("{name}) {letter} {}", rest.join(" "))
}

/// A child that has exited and not been reaped keeps its pid, and reads as
/// dead rather than as a process still holding it.
#[test]
fn a_zombie_reads_as_dead() {
    let mut exited = Command::new("true").spawn().expect("true runs");
    let child = Process::named(exited.id() as libc::pid_t).expect("it was spawned");
    let seen = still_running_after(LONG_ENOUGH_TO_DIE, || state_of(&child));
    exited.wait().expect("it is ours to reap");
    assert_eq!(seen, State::Zombie);
}

/// A process that has stopped keeps moving — a zombie is claimed and its pid
/// let go — so a read made after the one that found it stopped is a read of
/// a different instant, and can find it dead. The wait reports the read that
/// ended it.
#[test]
fn the_wait_reports_the_read_that_ended_it() {
    let mut reads = [State::Alive { in_state: 'S' }, State::Zombie, State::Dead].into_iter();
    let seen = still_running_after(LONG_ENOUGH_TO_DIE, || {
        reads
            .next()
            .expect("the wait ends at the first read that finds it stopped")
    });
    assert_eq!(seen, State::Zombie);
}

#[test]
fn a_reaped_process_reads_as_gone() {
    let mut exited = Command::new("true").spawn().expect("true runs");
    let child = Process::named(exited.id() as libc::pid_t).expect("it was spawned");
    exited.wait().expect("it is ours to reap");
    assert_eq!(state_of(&child), State::Gone);
}

/// What tells two holders of a pid apart is when each started, so the tick
/// read has to be one that a later process reads later.
#[test]
fn a_process_spawned_later_started_on_a_later_tick() {
    let mut earlier = Command::new("true").spawn().expect("true runs");
    let first = Process::named(earlier.id() as libc::pid_t).expect("it was spawned");
    earlier.wait().expect("it is ours to reap");

    std::thread::sleep(a_few_ticks());

    let mut later = Command::new("true").spawn().expect("true runs");
    let second = Process::named(later.id() as libc::pid_t).expect("it was spawned");
    later.wait().expect("it is ours to reap");

    assert!(
        second.started > first.started,
        "the later process read tick {}, the earlier tick {}",
        second.started,
        first.started
    );
}

/// Long enough that the clock the kernel stamps a start with has moved on.
fn a_few_ticks() -> Duration {
    let ticks_a_second = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    assert!(ticks_a_second > 0, "the kernel says how fast it ticks");
    Duration::from_secs(1) / ticks_a_second as u32 * 3
}
