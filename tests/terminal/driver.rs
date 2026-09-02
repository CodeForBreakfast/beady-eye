//! A `bdi` on a pty that a test can type at, timestamping what comes back.
//!
//! The two tests beside this one read one burst of output and never send
//! anything, so they can wait for a shape and stop. A test about the loop
//! cannot: what it asks is *when* `bdi` answered relative to something else
//! going on, and that needs both halves — synthetic keystrokes written at
//! chosen moments, and a time against every chunk that comes back.
//!
//! Every wait here is a `poll` with a deadline, and no read outlives its
//! poll: the master `a_pty` hands over is non-blocking, and it says why. A
//! wait ends one of three ways — what it waited for arrived; `bdi` exited
//! without it, which is failed at once; or the deadline passed — and the two
//! failures panic at the line of the test that waited, naming what never
//! arrived and what did.

use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::PathBuf;
use std::process::{Child, ExitStatus};
use std::time::{Duration, Instant};

use super::{a_pty, bdi_on, ENTER_ALTERNATE_SCREEN};

/// Long enough that a wait reaching it means something is wrong, and short
/// enough that the harness above the suite does not give up first. Never
/// asserted against: every wait that ends here panics with what it was
/// waiting for.
///
/// The harness above the suite is cargo-mutants, which allows a test run
/// five times its baseline and no less than twenty seconds, and scores a run
/// that outlives that as `Timeout` — a third answer on the tally, which the
/// count cannot tell from a kill. A wait longer than that floor can never
/// fail inside it, so a mutant that leaves `bdi` silent is recorded as a
/// hang rather than as the wait it expired. Ten seconds is under the floor
/// with room for whatever passed before it, and an order of magnitude over
/// the slowest of these test binaries, which takes just over a second.
pub const GIVING_UP: Duration = Duration::from_secs(10);

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
    #[track_caller]
    pub fn read_until(&mut self, said: &[u8], patience: Duration) {
        let wanted = format!("said {:?}", String::from_utf8_lossy(said));
        self.wait_until(patience, &wanted, |driven| {
            super::contains(&driven.everything(), said)
        });
    }

    /// Read until `bdi` has been quiet this long, so what it says next is an
    /// answer to what happens next rather than the tail of the frame before.
    ///
    /// A quiet window taken *after* a keystroke is satisfied by the frame that
    /// preceded it and reports no latency at all, which cost bdi-7ao.43 a
    /// wrong reading. So this is what a test calls before it sends, and
    /// [`Driven::answer_to`] is what it calls after.
    #[track_caller]
    pub fn settle(&mut self, quiet: Duration, patience: Duration) {
        let mut heard = self.said.len();
        let mut silent_since = Instant::now();
        self.wait_until(patience, "stopped drawing", |driven| {
            if driven.said.len() != heard {
                heard = driven.said.len();
                silent_since = Instant::now();
            }
            silent_since.elapsed() >= quiet
        });
    }

    /// Read until `satisfied` says what was waited for has arrived.
    ///
    /// Fails at once, rather than at the deadline, when `bdi` exits without
    /// it: nothing more is coming from a `bdi` that has gone, and a wait that
    /// runs on regardless is a deadline's worth of nothing that says only
    /// "never". Under mutation it says less than that — a mutant that makes
    /// `bdi` exit at once left three of these to outlive cargo-mutants'
    /// timeout, and scored `Timeout` where the suite would have named the
    /// wait (bdi-2bb.28). Its last words are read before the verdict, since
    /// they may be the error it exited with.
    ///
    /// The panic sits at the line of the test that waited, so a test with
    /// seven waits fails at one of them rather than at the one line of this
    /// file every wait shares (bdi-kpu).
    #[track_caller]
    fn wait_until(
        &mut self,
        patience: Duration,
        wanted: &str,
        mut satisfied: impl FnMut(&Self) -> bool,
    ) {
        let giving_up = Instant::now() + patience;
        while Instant::now() < giving_up {
            if satisfied(self) {
                return;
            }
            if let Some(exited) = self.exited() {
                if satisfied(self) {
                    return;
                }
                panic!(
                    "bdi exited ({exited}) and never {wanted}. {}",
                    self.everything_said()
                );
            }
            self.read_some();
        }
        panic!(
            "bdi never {wanted} in {patience:?}. {}",
            self.everything_said()
        );
    }

    /// How `bdi` exited, where it has, with whatever it wrote on the way out
    /// read: the read that saw nothing may have come before its last write.
    fn exited(&mut self) -> Option<ExitStatus> {
        let exited = self.child.try_wait().expect("bdi is ours to ask after")?;
        loop {
            let before = self.said.len();
            self.read_some();
            if self.said.len() == before {
                return Some(exited);
            }
        }
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
    /// first of it.
    #[track_caller]
    pub fn answer_to(&mut self, since: Mark, patience: Duration) -> Vec<u8> {
        let giving_up = Instant::now() + patience;
        self.wait_until(patience, "answered", |driven| driven.said.len() > since.0);
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

    /// The timeline, and then the bytes themselves, for a wait that failed
    /// to say with: what did arrive is the only evidence a failed wait
    /// leaves, and a one-off failure nobody watched has nothing else.
    fn everything_said(&self) -> String {
        format!(
            "{}\nIt wrote: {:?}",
            self.timeline(),
            String::from_utf8_lossy(&self.everything())
        )
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
