//! What a signal leaves behind on the terminal.
//!
//! `bdi` puts the terminal on the alternate screen, into raw mode and into
//! reporting the mouse, and puts all of it back from `Drop`. A signal with no
//! handler terminates the process outright, so none of that runs and the
//! shell underneath comes back unusable. These tests kill a real `bdi` on a
//! real pty and read what it wrote on the way out.

use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

/// The terminal is on the alternate screen from here.
const ENTER_ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049h";
/// And off it again from here. Its absence after a signal is the bug.
const LEAVE_ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049l";
/// The mouse modes `EnableMouseCapture` turns on, each paired with the reset
/// that has to follow it. A terminal left reporting the mouse writes an
/// escape sequence into whatever runs next for every cell the pointer
/// crosses, which reaches the reader as input typing itself.
const MOUSE_OFF: [(&str, &[u8]); 5] = [
    ("normal tracking", b"\x1b[?1000l"),
    ("button tracking", b"\x1b[?1002l"),
    ("any-event tracking", b"\x1b[?1003l"),
    ("urxvt coordinates", b"\x1b[?1015l"),
    ("SGR coordinates", b"\x1b[?1006l"),
];

/// Long enough for a collection that has no tracker to fail and the screen to
/// open. Only ever a giving-up point: nothing is asserted against the clock.
const LONG_ENOUGH_TO_DRAW: Duration = Duration::from_secs(60);
/// Long enough for a process that is going to die to have died.
const LONG_ENOUGH_TO_DIE: Duration = Duration::from_secs(10);

#[test]
fn a_terminate_puts_the_terminal_back() {
    assert_restored_after(libc::SIGTERM, "SIGTERM");
}

#[test]
fn an_interrupt_puts_the_terminal_back() {
    assert_restored_after(libc::SIGINT, "SIGINT");
}

/// A hangup reaches `bdi` when the terminal it is drawing on goes away, which
/// is how a multiplexer reclaims a pane.
#[test]
fn a_hangup_puts_the_terminal_back() {
    assert_restored_after(libc::SIGHUP, "SIGHUP");
}

/// Run `bdi` on a pty until it is drawing, signal it, and assert it handed
/// the terminal back on the way out.
fn assert_restored_after(signal: i32, named: &str) {
    let mut session = Session::showing_the_forest(named);

    let restoring = session.signal_and_read(signal);

    let said = String::from_utf8_lossy(&restoring);
    assert!(
        contains(&restoring, LEAVE_ALTERNATE_SCREEN),
        "{named} left the terminal on the alternate screen; \
         it wrote {} bytes on the way out: {said:?}",
        restoring.len()
    );
    for (mode, off) in MOUSE_OFF {
        assert!(
            contains(&restoring, off),
            "{named} left the terminal reporting the mouse ({mode}): {said:?}"
        );
    }
}

/// A `bdi` drawing on a pty of our own.
struct Session {
    child: Child,
    terminal: OwnedFd,
    home: PathBuf,
}

impl Session {
    /// Start `bdi` and wait until it is actually on the alternate screen.
    ///
    /// The waiting is the part that matters. `tui::run` makes its first
    /// collection *before* the screen opens, so a test that instead slept a
    /// fixed time would signal a `bdi` that has not drawn anything, find no
    /// restore sequences because there was nothing to restore, and pass
    /// against a build that never restores at all.
    fn showing_the_forest(named: &str) -> Self {
        let home = a_home_naming_one_project(named);
        let (ours, theirs) = a_pty();

        let child = unsafe {
            Command::new(env!("CARGO_BIN_EXE_bdi"))
                .current_dir(&home)
                .env("HOME", &home)
                .env("TERM", "xterm-256color")
                .env_remove("BEADS_DIR")
                .env_remove("COMMY_PROJECT")
                .stdin(theirs.try_clone().expect("the pty is ours to hand over"))
                .stdout(theirs.try_clone().expect("the pty is ours to hand over"))
                .stderr(theirs.try_clone().expect("the pty is ours to hand over"))
                .pre_exec(own_the_terminal)
                .spawn()
        }
        .expect("bdi runs");
        drop(theirs);

        let mut session = Self {
            child,
            terminal: ours,
            home,
        };
        session.read_until(ENTER_ALTERNATE_SCREEN);
        session
    }

    /// Send a signal and collect everything written after it.
    fn signal_and_read(&mut self, signal: i32) -> Vec<u8> {
        let pid = self.child.id() as libc::pid_t;
        assert_eq!(
            unsafe { libc::kill(pid, signal) },
            0,
            "the signal is ours to send"
        );

        let mut restoring = Vec::new();
        let giving_up = Instant::now() + LONG_ENOUGH_TO_DIE;
        while Instant::now() < giving_up {
            if let Ok(Some(_)) = self.child.try_wait() {
                break;
            }
            self.read_some(&mut restoring);
        }
        self.read_some(&mut restoring);
        restoring
    }

    /// Read until the terminal has said this, or until we give up on it.
    fn read_until(&mut self, said: &[u8]) {
        let mut seen = Vec::new();
        let giving_up = Instant::now() + LONG_ENOUGH_TO_DRAW;
        while Instant::now() < giving_up {
            self.read_some(&mut seen);
            if contains(&seen, said) {
                return;
            }
        }
        panic!(
            "bdi never put the terminal on the alternate screen; it wrote {} bytes: {:?}",
            seen.len(),
            String::from_utf8_lossy(&seen)
        );
    }

    fn read_some(&mut self, into: &mut Vec<u8>) {
        wait_for_reading(&self.terminal, Duration::from_millis(200));
        let mut buffer = [0u8; 8192];
        let mut terminal = unsafe { std::fs::File::from_raw_fd(self.terminal.as_raw_fd()) };
        let read = terminal.read(&mut buffer);
        std::mem::forget(terminal);
        if let Ok(count) = read {
            into.extend_from_slice(&buffer[..count]);
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

/// A `HOME` holding a config that names one project, so `bdi` gets past
/// config assembly and as far as drawing. The project's path is the same
/// directory, which holds no tracker, so the collection fails fast.
///
/// What is drawn is still whatever the machine has to say — `bdi` asks herdr
/// for the live agents, and on a machine running one it answers. Nothing here
/// asserts on any of it. These tests read escape sequences, so a frame full of
/// somebody's real panes and a frame saying there is no herdr are the same
/// frame to them.
fn a_home_naming_one_project(named: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("bdi-{named}-{}", std::process::id()));
    std::fs::create_dir_all(home.join(".config/beady-eye")).expect("the directory is ours to make");
    std::fs::write(
        home.join(".config/beady-eye/config.toml"),
        format!(
            "[[projects]]\nname = \"atlas\"\npath = \"{}\"\n",
            home.display()
        ),
    )
    .expect("the config is ours to write");
    home
}

/// A pty: the end the test reads, and the end `bdi` draws on.
fn a_pty() -> (OwnedFd, std::fs::File) {
    let mut ours = 0;
    let mut theirs = 0;
    let size = libc::winsize {
        ws_row: 40,
        ws_col: 120,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    assert_eq!(
        unsafe {
            libc::openpty(
                &mut ours,
                &mut theirs,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &size,
            )
        },
        0,
        "a pty is ours to open"
    );
    unsafe {
        (
            OwnedFd::from_raw_fd(ours),
            std::fs::File::from_raw_fd(theirs),
        )
    }
}

/// Between the fork and the exec: make the pty the child's controlling
/// terminal, so crossterm finds one to read keys from.
///
/// # Safety
///
/// Runs in the forked child before `exec`, so only async-signal-safe calls
/// belong here. Both of these are.
fn own_the_terminal() -> std::io::Result<()> {
    unsafe {
        if libc::setsid() == -1 {
            return Err(std::io::Error::last_os_error());
        }
        if libc::ioctl(libc::STDIN_FILENO, libc::TIOCSCTTY, 0) == -1 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Block until there is something to read, or the wait is up.
fn wait_for_reading(fd: &OwnedFd, patience: Duration) {
    let mut polling = libc::pollfd {
        fd: fd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    unsafe { libc::poll(&mut polling, 1, patience.as_millis() as i32) };
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|at| at == needle)
}
