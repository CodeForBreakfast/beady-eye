//! A `bdi` on a pty that a test can type at, timestamping what comes back.
//!
//! The two tests beside this one read one burst of output and never send
//! anything, so they can wait for a shape and stop. A test about the loop
//! cannot: what it asks is *when* `bdi` answered relative to something else
//! going on, and that needs both halves — synthetic keystrokes written at
//! chosen moments, and a time against every chunk that comes back.
//!
//! Every wait here is a `poll` with a deadline on a master set `O_NONBLOCK`,
//! and no read outlives its poll. A blocking read after a poll that timed out
//! waits for the next byte instead of reporting that the frame is over, which
//! takes a failing test to its whole giving-up deadline with no assertion
//! text — bdi-2bb.28, measured at 60.05s against 0.66s.

use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::PathBuf;
use std::process::Child;
use std::time::{Duration, Instant};

use super::{a_pty, bdi_on, ENTER_ALTERNATE_SCREEN};

/// Long enough that a wait reaching it means something is wrong, and short
/// enough that a suite hitting it still finishes. Never asserted against:
/// every wait that ends here panics with what it was waiting for.
pub const GIVING_UP: Duration = Duration::from_secs(60);

/// How long a poll blocks before looking at the clock again.
const A_GLANCE: Duration = Duration::from_millis(50);

/// A run of bytes `bdi` wrote, and how long after it started it wrote them.
struct Said {
    at: Duration,
    bytes: Vec<u8>,
}

/// Where in what `bdi` has said a test is reading from. Handed out by
/// [`Driven::send`], so what a keystroke was answered with can be asked for
/// without the frame before it: a chunk is one read, so a chunk taken before
/// the send is wholly before it.
#[derive(Clone, Copy)]
pub struct Mark(usize);

/// A `bdi` drawing on a pty of our own, that we can type at.
pub struct Driven {
    child: Child,
    terminal: OwnedFd,
    home: PathBuf,
    started: Instant,
    said: Vec<Said>,
}

impl Driven {
    /// Start `bdi` on a pty of the given size, with `environment` on top of
    /// what a test binary already carries.
    pub fn bdi(rows: u16, cols: u16, home: PathBuf, environment: &[(String, String)]) -> Self {
        let (ours, theirs) = a_pty(rows, cols);
        // Read without blocking. The child stays alive for the whole test, so
        // no end of file ever arrives to end a read that has outrun its poll.
        unsafe { libc::fcntl(ours.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) };
        let started = Instant::now();
        let child = bdi_on(&theirs, &home, environment);
        drop(theirs);
        Self {
            child,
            terminal: ours,
            home,
            started,
            said: Vec::new(),
        }
    }

    /// The `bdi` being driven, for a test whose subject is the process rather
    /// than what it draws.
    pub fn pid(&self) -> libc::pid_t {
        self.child.id() as libc::pid_t
    }

    /// Read until `bdi` has said this, or give up and say what it did say.
    pub fn read_until(&mut self, said: &[u8], patience: Duration) {
        let giving_up = Instant::now() + patience;
        while Instant::now() < giving_up {
            if super::contains(&self.everything(), said) {
                return;
            }
            self.read_some();
        }
        panic!(
            "bdi never said {:?}. {}",
            String::from_utf8_lossy(said),
            self.timeline()
        );
    }

    /// Read until `bdi` has been quiet this long, so what it says next is an
    /// answer to what happens next rather than the tail of the frame before.
    ///
    /// A quiet window taken *after* a keystroke is satisfied by the frame that
    /// preceded it and reports no latency at all, which cost bdi-7ao.43 a
    /// wrong reading. So this is what a test calls before it sends, and
    /// [`Driven::answer_to`] is what it calls after.
    pub fn settle(&mut self, quiet: Duration, patience: Duration) {
        let giving_up = Instant::now() + patience;
        let mut silent_since = Instant::now();
        while Instant::now() < giving_up {
            let before = self.said.len();
            self.read_some();
            if self.said.len() != before {
                silent_since = Instant::now();
            } else if silent_since.elapsed() >= quiet {
                return;
            }
        }
        panic!("bdi never stopped drawing. {}", self.timeline());
    }

    /// Type at `bdi`, and mark the place in what it has said so far, so its
    /// answer can be told from everything that came before.
    ///
    /// Refused until the alternate screen has been read. Before `bdi` puts
    /// the terminal into raw mode the pty's line discipline holds a key until
    /// a newline that never comes, so a key typed then is not answered late
    /// but never, and a test waiting for the answer waits its whole deadline
    /// to say that nothing arrived. `bdi` enters raw mode and then opens the
    /// screen, so the screen on the wire is the line discipline out of the
    /// way — and a test that has waited for anything drawn after it has read
    /// it too.
    pub fn send(&mut self, keys: &[u8]) -> Mark {
        assert!(
            super::contains(&self.everything(), ENTER_ALTERNATE_SCREEN),
            "typed {:?} before bdi had opened its screen, when the line \
             discipline would hold it and nothing would answer. Wait for \
             `ENTER_ALTERNATE_SCREEN` first, or for anything drawn after it. {}",
            String::from_utf8_lossy(keys),
            self.timeline()
        );
        let at = Mark(self.said.len());
        let mut terminal = self.as_file();
        terminal.write_all(keys).expect("the terminal takes keys");
        terminal.flush().expect("the terminal takes keys");
        std::mem::forget(terminal);
        at
    }

    /// Change the size of the terminal under `bdi`, which the kernel reports
    /// to it as a resize.
    ///
    /// What it is for is reading the screen rather than changing it: the
    /// terminal is written as a difference from the frame before, so a word
    /// that lands where another word already had letters in the same columns
    /// reaches the wire in pieces and no test can look for it. A resize is
    /// answered by drawing every cell again, so what comes back is the screen
    /// as it stands rather than what changed about it.
    pub fn resize(&mut self, rows: u16, cols: u16) -> Mark {
        let at = Mark(self.said.len());
        let size = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        assert_eq!(
            unsafe { libc::ioctl(self.terminal.as_raw_fd(), libc::TIOCSWINSZ, &size) },
            0,
            "the terminal is ours to resize"
        );
        at
    }

    /// Everything `bdi` wrote after `since`, waiting up to `patience` for the
    /// first of it. Empty only where it wrote nothing at all in that time.
    pub fn answer_to(&mut self, since: Mark, patience: Duration) -> Vec<u8> {
        let giving_up = Instant::now() + patience;
        while Instant::now() < giving_up && self.said.len() == since.0 {
            self.read_some();
        }
        // Then drain, so a frame arriving in several reads is answered whole
        // rather than by its first chunk. A poll that returns nothing is the
        // end of it: `bdi` writes a frame in one burst.
        while Instant::now() < giving_up {
            let before = self.said.len();
            self.read_some();
            if self.said.len() == before {
                break;
            }
        }
        self.said[since.0..]
            .iter()
            .flat_map(|said| said.bytes.iter().copied())
            .collect()
    }

    /// When each run of bytes arrived, for an assertion that failed to say
    /// with. A stalled loop and a slow one are told apart by exactly this, so
    /// a failure that only said "nothing arrived" would leave the reader
    /// where the whole bead started.
    pub fn timeline(&self) -> String {
        let mut told = format!("bdi wrote {} times:", self.said.len());
        for said in &self.said {
            told.push_str(&format!(
                "\n  +{:.3}s  {} bytes",
                said.at.as_secs_f64(),
                said.bytes.len()
            ));
        }
        told
    }

    /// Everything `bdi` has written so far, for a test whose subject is what
    /// was never written — a wait can only say what arrived.
    pub fn everything(&self) -> Vec<u8> {
        self.said
            .iter()
            .flat_map(|said| said.bytes.iter().copied())
            .collect()
    }

    /// One poll, and whatever was ready when it returned.
    fn read_some(&mut self) {
        let mut polling = libc::pollfd {
            fd: self.terminal.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        unsafe { libc::poll(&mut polling, 1, A_GLANCE.as_millis() as i32) };
        let mut buffer = [0u8; 8192];
        let mut terminal = self.as_file();
        let read = terminal.read(&mut buffer);
        std::mem::forget(terminal);
        if let Ok(count) = read {
            if count > 0 {
                self.said.push(Said {
                    at: self.started.elapsed(),
                    bytes: buffer[..count].to_vec(),
                });
            }
        }
    }

    /// The master as a `File`, which every caller must `forget` rather than
    /// drop: the fd belongs to `self.terminal`, and closing it twice would
    /// close whatever the next open handed out.
    fn as_file(&self) -> std::fs::File {
        unsafe { std::fs::File::from_raw_fd(self.terminal.as_raw_fd()) }
    }
}

impl Drop for Driven {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.home);
    }
}
