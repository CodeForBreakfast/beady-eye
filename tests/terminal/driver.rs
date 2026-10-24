//! A `bdi` on a pty that a test can type at, timestamping what comes back.
//!
//! The two tests beside this one read one burst of output and never send
//! anything, so they can wait for a shape and stop. A test about the loop
//! cannot: what it asks is *when* `bdi` answered relative to something else
//! going on, and that needs both halves — synthetic keystrokes written at
//! chosen moments, and a time against every chunk that comes back.
//!
//! The master is drained by a thread of its own, from the moment `bdi`
//! starts until it has been reaped, rather than by whichever wait happens to
//! be running. A terminal a person is sitting at drains continuously, and a
//! harness that reads only when a test asks it to puts `bdi` under
//! backpressure no terminal applies: a tty's output queue fills, the next
//! frame blocks in `write`, and whatever `bdi` would have done after drawing
//! never happens. macOS gives a tty a far smaller queue than Linux does, so
//! it fills during the first frame — measured on 2026-09-02, where
//! `forest_before_the_first_collection` waited ten seconds for a collection
//! `bdi` had not reached the code to begin, and began it 120ms after the
//! master was first read. Draining is the terminal's half of the bargain and
//! it is not a test's to schedule.
//!
//! Every wait here is a deadline over what that thread has heard. A wait ends
//! one of three ways — what it waited for arrived; `bdi` exited without it,
//! which is failed at once; or the deadline passed — and the two failures
//! panic at the line of the test that waited, naming what never arrived and
//! what did.

use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::process::{Child, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::{a_pty, bdi_on, ENTER_ALTERNATE_SCREEN};

/// Long enough that a wait reaching it means something is wrong, and short
/// enough that the harness above the suite does not give up first. Never
/// asserted against: every wait that ends here panics with what it was
/// waiting for.
///
/// The harness above the suite is cargo-mutants, which this crate's config
/// allows 180 seconds a test run, and scores a run
/// that outlives that as `Timeout` — a third answer on the tally, which the
/// count cannot tell from a kill. A wait longer than that floor can never
/// fail inside it, so a mutant that leaves `bdi` silent is recorded as a
/// hang rather than as the wait it expired. Ten seconds is under the floor
/// with room for whatever passed before it, and an order of magnitude over
/// the slowest of these test binaries, which takes just over a second.
pub const GIVING_UP: Duration = Duration::from_secs(10);

/// How long a poll blocks before looking at the clock again.
const A_GLANCE: Duration = Duration::from_millis(50);

/// One notch of the wheel away from the reader, in xterm's SGR encoding,
/// which is what crossterm reads and what `bdi` asks the terminal for when it
/// takes the mouse. It
/// carries a row because a mouse report always does; nothing reads it, since
/// a notch is answered by whatever is on the screen rather than by what is
/// under the pointer.
pub const A_NOTCH_DOWN: &[u8] = b"\x1b[<65;1;1M";

/// A run of bytes `bdi` wrote, and how long after it started it wrote them.
struct Said {
    at: Duration,
    bytes: Vec<u8>,
}

/// Where in what `bdi` has said a test is reading from. Handed out by
/// [`Driven::send`], so what a keystroke was answered with can be asked for
/// without the frame before it.
///
/// What makes it divide the two is that `send` empties the pty and takes the
/// mark under the lock the drain appends through, and writes the key without
/// letting go: nothing `bdi` had already written can arrive after it, and
/// nothing can be appended between the mark and the key. Neither half is
/// spare. Without the drain, a frame `bdi` wrote before the key but that the
/// drain had not yet picked up is counted as the answer to it — which is a
/// test proving the loop answered during a hang passing without proving it,
/// and passing quietly.
#[derive(Clone, Copy)]
pub struct Mark {
    at: usize,
    /// Whether the answer is the whole screen drawn again, as it is to a
    /// resize, rather than a frame drawing only what changed.
    whole_screen: bool,
}

/// What ratatui writes once a frame's cells are out. It hides the cursor, or,
/// where the frame asked for one, as the search prompt does, shows it and
/// then moves it there.
const THE_CURSOR_HIDDEN: &[u8] = b"\x1b[?25l";
const THE_CURSOR_SHOWN: &[u8] = b"\x1b[?25h";

/// What ratatui writes first when the terminal has changed size, flushed on
/// its own before it paints anything.
const THE_SCREEN_CLEARS: &[u8] = b"\x1b[2J";

/// A `bdi` drawing on a pty of our own, that we can type at.
pub struct Driven {
    child: Child,
    terminal: Arc<OwnedFd>,
    /// The far end of `terminal`, which is `bdi`'s.
    their_end: PathBuf,
    home: PathBuf,
    started: Instant,
    said: Arc<Mutex<Vec<Said>>>,
    draining: Arc<AtomicBool>,
    drain: Option<JoinHandle<()>>,
}

impl Driven {
    /// Start `bdi` on a pty of the given size, with `environment` on top of
    /// what a test binary already carries.
    pub fn bdi(rows: u16, cols: u16, home: PathBuf, environment: &[(String, String)]) -> Self {
        Self::on_a_pty(rows, cols, home, &[], environment, Duration::ZERO)
    }

    /// The same, on a command line of the test's own — `--socket`, say, for a
    /// test whose subject is where this run watches.
    pub fn bdi_with_arguments(
        rows: u16,
        cols: u16,
        home: PathBuf,
        arguments: &[&str],
        environment: &[(String, String)],
    ) -> Self {
        Self::on_a_pty(rows, cols, home, arguments, environment, Duration::ZERO)
    }

    /// The same, with the drain thread held back this long before each read.
    ///
    /// A wait has to be right however far behind the drain has fallen, and on
    /// a machine that is not loaded it never falls behind at all — so that is
    /// a state a test can only reach by putting the driver in it, the way
    /// `tests/shims/` reaches a hung tracker. Held back longer than the glance
    /// a wait takes between looks, the pty keeps what `bdi` wrote across one,
    /// which is where a wait that reads what has been appended and a wait that
    /// empties the pty itself part company.
    pub fn bdi_with_the_drain_held_back(
        rows: u16,
        cols: u16,
        home: PathBuf,
        environment: &[(String, String)],
        held_back: Duration,
    ) -> Self {
        Self::on_a_pty(rows, cols, home, &[], environment, held_back)
    }

    /// Everything the three above have in common: the pty, the `bdi` on it,
    /// and the thread draining it from the moment it starts.
    fn on_a_pty(
        rows: u16,
        cols: u16,
        home: PathBuf,
        arguments: &[&str],
        environment: &[(String, String)],
        held_back: Duration,
    ) -> Self {
        let (ours, theirs) = a_pty(rows, cols);
        let started = Instant::now();
        let child = bdi_on(&theirs, &home, arguments, environment);
        let their_end = the_path_of(&theirs);
        drop(theirs);

        let terminal = Arc::new(ours);
        let said = Arc::new(Mutex::new(Vec::new()));
        let draining = Arc::new(AtomicBool::new(true));
        let drain = std::thread::spawn({
            let terminal = Arc::clone(&terminal);
            let said = Arc::clone(&said);
            let draining = Arc::clone(&draining);
            move || drain_into(&terminal, &said, started, &draining, held_back)
        });

        Self {
            child,
            terminal,
            their_end,
            home,
            started,
            said,
            draining,
            drain: Some(drain),
        }
    }

    /// The `bdi` being driven, for a test whose subject is the process rather
    /// than what it draws.
    pub fn pid(&self) -> libc::pid_t {
        self.child.id() as libc::pid_t
    }

    /// Whether the drain is still reading the terminal.
    ///
    /// It stops of its own accord once the pty has hung up and given up what
    /// it held, so this is false from a little after `bdi` goes. Nothing in
    /// the driver asks it — a test does, because a drain that never stopped
    /// would poll a hung-up pty as fast as the machine allows and there is
    /// nothing else to see that from.
    pub fn still_reading(&self) -> bool {
        self.drain
            .as_ref()
            .is_some_and(|drain| !drain.is_finished())
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
    ///
    /// The quiet is timed from when the last bytes arrived, not from when a
    /// wait saw them: a wait that wakes late finds a write since its last
    /// look nearly every time, and would never see a gap that was there.
    #[track_caller]
    pub fn settle(&mut self, quiet: Duration, patience: Duration) {
        let settling_from = self.started.elapsed();
        self.wait_until(patience, "stopped drawing", |driven| {
            let silent_since = driven.last_heard_at().max(settling_from);
            driven.started.elapsed() - silent_since >= quiet
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
            self.hear_what_is_waiting();
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
            std::thread::sleep(A_GLANCE);
        }
        panic!(
            "bdi never {wanted} in {patience:?}. {}",
            self.everything_said()
        );
    }

    /// Stop the terminal taking what `bdi` writes, as a terminal nobody drains
    /// does once its queue is full: the next write blocks, and so does every
    /// one after it. Taken back up when what this returns is dropped.
    ///
    /// Stopped rather than left undrained, because a full queue is a race
    /// with the drain thread and a stop is not: with the slave's output
    /// stopped, a write blocks however little it has to say. Raw mode does
    /// not undo it, since it is a stop asked for by `tcflow` rather than by a
    /// character the terminal read.
    pub fn stop_taking_output(&self) -> OutputStopped {
        let slave = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOCTTY)
            .open(&self.their_end)
            .expect("the far end of our own pty is ours to open");
        assert_eq!(
            unsafe { libc::tcflow(slave.as_raw_fd(), libc::TCOOFF) },
            0,
            "the terminal's output is ours to stop"
        );
        OutputStopped(slave)
    }

    /// How `bdi` exited, where it has, with whatever it wrote on the way out
    /// read.
    ///
    /// Emptied here rather than waited out. A `bdi` that has been reaped
    /// writes nothing more, so what the pty holds is the whole of its last
    /// words and a read that finds it empty has them all — where a quiet
    /// interval says only that the drain did not run in it, and a drain the
    /// scheduler held off for one would have this report a `bdi` that never
    /// said what it said on the way out.
    pub fn exited(&mut self) -> Option<ExitStatus> {
        let exited = self.child.try_wait().expect("bdi is ours to ask after")?;
        self.hear_what_is_waiting();
        Some(exited)
    }

    /// Take everything the pty holds now, under the lock the drain appends
    /// through — so no read is in flight, nothing read is unappended, and
    /// what is left behind is a pty a read would find empty.
    ///
    /// This is what makes a wait's view of what `bdi` has said exact at the
    /// moment it looks, rather than as fresh as the drain thread last
    /// happened to be scheduled.
    fn hear_what_is_waiting(&self) {
        let mut said = self.said.lock().expect("the drain is running");
        hear_what_is_waiting(&self.terminal, self.started, &mut said);
    }

    /// Mark the place in what `bdi` has said so far, without typing at it, so
    /// what it writes after something the test does by other means — a signal
    /// — can be told from the frame before.
    ///
    /// [`Driven::send`] marks the same place for a keystroke and cannot call
    /// this: it holds the lock across the mark and the write, which is what
    /// makes the key the first thing on the far side of the mark.
    pub fn mark(&self) -> Mark {
        let mut said = self.said.lock().expect("the drain is running");
        hear_what_is_waiting(&self.terminal, self.started, &mut said);
        Mark {
            at: said.len(),
            whole_screen: false,
        }
    }

    /// Read until `bdi` has died, and hand back everything it wrote after
    /// `since`.
    ///
    /// The drain has to be running for this to terminate at all, which is the
    /// fact [`Driven::drop`] below turns on and `CLAUDE.md` has the Darwin
    /// measurements for. Here it means the wait cannot be the reader.
    ///
    /// A `bdi` still alive at the deadline is left to the caller with
    /// whatever it wrote, since what it failed to write is the assertion
    /// these tests came to make.
    pub fn last_words(&mut self, since: Mark, patience: Duration) -> Vec<u8> {
        let giving_up = Instant::now() + patience;
        while Instant::now() < giving_up {
            if self.exited().is_some() {
                break;
            }
            std::thread::sleep(A_GLANCE);
        }
        self.hear_what_is_waiting();
        self.said_since(since)
    }

    /// Type at `bdi`, and mark the place in what it has said so far, so its
    /// answer can be told from everything that came before.
    ///
    /// Held back until the alternate screen has been read. Before `bdi` puts
    /// the terminal into raw mode the pty's line discipline holds a key until
    /// a newline that never comes, so a key typed then is not answered late
    /// but never, and a test waiting for the answer waits its whole deadline
    /// to say that nothing arrived. `bdi` enters raw mode and then opens the
    /// screen, so the screen on the wire is the line discipline out of the
    /// way.
    ///
    /// A wait rather than a refusal, because the driver drains from the
    /// moment `bdi` starts: whether the screen has arrived by the time a test
    /// types is then a race the test cannot see or control, and a guard that
    /// asserts on it fires on some runs and is vacuous on the rest. Waiting
    /// makes the property hold instead of noticing it did not — and a `bdi`
    /// that never opens its screen still ends here, at once, naming what was
    /// waited for.
    #[track_caller]
    pub fn send(&mut self, keys: &[u8]) -> Mark {
        self.wait_until(
            GIVING_UP,
            &format!(
                "opened its screen, so {:?} could be typed at it rather than \
                 held by the line discipline and never answered — this waits \
                 for `ENTER_ALTERNATE_SCREEN`",
                String::from_utf8_lossy(keys)
            ),
            |driven| super::contains(&driven.everything(), ENTER_ALTERNATE_SCREEN),
        );
        let mut said = self.said.lock().expect("the drain is running");
        hear_what_is_waiting(&self.terminal, self.started, &mut said);
        let at = Mark {
            at: said.len(),
            whole_screen: false,
        };
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
        let mut said = self.said.lock().expect("the drain is running");
        hear_what_is_waiting(&self.terminal, self.started, &mut said);
        let at = Mark {
            at: said.len(),
            whole_screen: true,
        };
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

    /// What `bdi` wrote after `since`, once it has finished a frame there.
    /// After a resize that is the whole screen drawn again, from the clear
    /// that opens it, so a frame `bdi` drew before it read the resize is not
    /// taken for the screen.
    ///
    /// A frame is waited for by its last bytes rather than by a quiet glance.
    /// `bdi` does not write a frame in one burst: the clear goes out alone
    /// before anything is painted, and the cells follow through stdout's
    /// buffer a kilobyte at a time. On a loaded machine `bdi` is held off
    /// between any two of those writes for longer than a glance.
    #[track_caller]
    pub fn answer_to(&mut self, since: Mark, patience: Duration) -> Vec<u8> {
        let wanted = if since.whole_screen {
            "drew the whole screen again"
        } else {
            "finished a frame"
        };
        let mut answer = Vec::new();
        self.wait_until(patience, wanted, |driven| {
            match the_answer(&driven.said_since(since), since.whole_screen) {
                Some(finished) => {
                    answer = finished.to_vec();
                    true
                }
                None => false,
            }
        });
        answer
    }

    /// Everything `bdi` wrote after `since`, as it stands now.
    fn said_since(&self, since: Mark) -> Vec<u8> {
        self.said.lock().expect("the drain is running")[since.at..]
            .iter()
            .flat_map(|said| said.bytes.iter().copied())
            .collect()
    }

    /// When each run of bytes arrived, for an assertion that failed to say
    /// with. A stalled loop and a slow one are told apart by exactly this, so
    /// a failure that only said "nothing arrived" would leave the reader
    /// where the whole bead started.
    pub fn timeline(&self) -> String {
        let said = self.said.lock().expect("the drain is running");
        let mut told = format!("bdi wrote {} times:", said.len());
        for said in said.iter() {
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
    ///
    /// Its stderr as well as its screen: both ends are the one pty, and the
    /// lines `bdi` writes before it opens the alternate screen are in here
    /// with the frames. So a test asserting that a phrase is **absent** is
    /// asserting it of both, and a word the screen never draws can still be
    /// in here — `bdi` names the session and the runtime directory on stderr
    /// when it cannot open its inbound socket, which is every run on a
    /// machine with no `XDG_RUNTIME_DIR`, which is the build sandbox.
    pub fn everything(&self) -> Vec<u8> {
        self.said
            .lock()
            .expect("the drain is running")
            .iter()
            .flat_map(|said| said.bytes.iter().copied())
            .collect()
    }

    /// When the last run of bytes arrived, counted from when `bdi` started.
    fn last_heard_at(&self) -> Duration {
        let said = self.said.lock().expect("the drain is running");
        said.last().map_or(Duration::ZERO, |said| said.at)
    }

    /// The master as a `File`, which every caller must `forget` rather than
    /// drop: the fd belongs to `self.terminal`, and closing it twice would
    /// close whatever the next open handed out.
    fn as_file(&self) -> std::fs::File {
        unsafe { std::fs::File::from_raw_fd(self.terminal.as_raw_fd()) }
    }
}

/// A terminal whose output [`Driven::stop_taking_output`] stopped.
///
/// Dropped before the `Driven` it came from, so declare it after: a `bdi`
/// still writing to a stopped terminal is killed mid-write by that drop, and
/// on Darwin a process cannot finish dying while its terminal holds output
/// nobody will take.
pub struct OutputStopped(std::fs::File);

/// Where a terminal lives, asked with the reentrant call because the tests
/// in a binary run on threads of their own.
fn the_path_of(terminal: &std::fs::File) -> PathBuf {
    let mut name = [0 as libc::c_char; 256];
    assert_eq!(
        unsafe { libc::ttyname_r(terminal.as_raw_fd(), name.as_mut_ptr(), name.len()) },
        0,
        "our own pty has a name"
    );
    let name = unsafe { std::ffi::CStr::from_ptr(name.as_ptr()) };
    PathBuf::from(std::ffi::OsStr::from_bytes(name.to_bytes()))
}

impl Drop for OutputStopped {
    fn drop(&mut self) {
        unsafe { libc::tcflow(self.0.as_raw_fd(), libc::TCOON) };
    }
}

impl Drop for Driven {
    /// Reap `bdi` while the master is still being drained, and only then stop
    /// draining. A process killed mid-write is waiting on a queue somebody has
    /// to empty, and the only handle to the far end is the one this holds:
    /// stopping first is a `wait` for a child that cannot finish dying.
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.draining.store(false, Ordering::Relaxed);
        if let Some(drain) = self.drain.take() {
            let _ = drain.join();
        }
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

/// Read the master until told to stop, timestamping each run of bytes from
/// when `bdi` started.
///
/// One poll and one read a turn, so a stop is answered within a glance. The
/// poll is outside the lock and the read inside it, which is what lets
/// [`Driven::send`] hold the lock and know no read is in flight: a mark taken
/// there divides what `bdi` wrote before the key from what it wrote after,
/// and a read that had already happened but not yet been appended would land
/// on the wrong side of it.
///
/// It gives up when the pty has hung up and given up the last of what it
/// held. A hangup is the only slave handles there are closing, which is
/// `bdi`'s own stdio and so `bdi` gone, and a poll reports it whatever it was
/// asked to wait for — so a loop that read on and polled again would come
/// straight back every time, and spin a core for as long as the test kept the
/// `Driven`. Draining after that guards nothing either: what is left to hear
/// from a process that has been reaped is what the pty already holds, and the
/// waits empty it themselves.
fn drain_into(
    terminal: &OwnedFd,
    said: &Mutex<Vec<Said>>,
    started: Instant,
    draining: &AtomicBool,
    held_back: Duration,
) {
    while draining.load(Ordering::Relaxed) {
        if !held_back.is_zero() {
            std::thread::sleep(held_back);
        }
        let mut polling = libc::pollfd {
            fd: terminal.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        unsafe { libc::poll(&mut polling, 1, A_GLANCE.as_millis() as i32) };
        let hung_up = polling.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0;
        let mut said = said.lock().expect("the drain owns what it heard");
        match read_now(terminal, started) {
            Some(heard) => said.push(heard),
            // Nothing left, on a pty nothing will write to again.
            None if hung_up => return,
            None => {}
        }
    }
}

/// Everything the pty holds now, appended. The caller holds the lock the
/// drain appends through, so no read is in flight and nothing read is
/// unappended: what this leaves behind is a pty a read would find empty.
fn hear_what_is_waiting(terminal: &OwnedFd, started: Instant, said: &mut Vec<Said>) {
    while let Some(waiting) = read_now(terminal, started) {
        said.push(waiting);
    }
}

/// One read of whatever is ready, or `None` where nothing is.
///
/// Never blocks: the master is non-blocking, and `a_pty` says why.
fn read_now(terminal: &OwnedFd, started: Instant) -> Option<Said> {
    let mut buffer = [0u8; 8192];
    let read = unsafe {
        libc::read(
            terminal.as_raw_fd(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
        )
    };
    (read > 0).then(|| Said {
        at: started.elapsed(),
        bytes: buffer[..read as usize].to_vec(),
    })
}

/// Everything said after a mark, or after a resize everything from the last
/// clear, once a frame there has finished. `None` while a frame is still
/// arriving.
fn the_answer(said: &[u8], whole_screen: bool) -> Option<&[u8]> {
    let answer = if whole_screen {
        let cleared = said
            .windows(THE_SCREEN_CLEARS.len())
            .rposition(|bytes| bytes == THE_SCREEN_CLEARS)?;
        &said[cleared..]
    } else {
        said
    };
    ends_a_frame(answer).then_some(answer)
}

/// Whether nothing but the cursor's move follows the cursor being hidden or
/// shown, which is the last thing a frame writes.
fn ends_a_frame(said: &[u8]) -> bool {
    let Some(toggled) = said
        .windows(THE_CURSOR_HIDDEN.len())
        .rposition(|bytes| bytes == THE_CURSOR_HIDDEN || bytes == THE_CURSOR_SHOWN)
    else {
        return false;
    };
    match &said[toggled + THE_CURSOR_HIDDEN.len()..] {
        [] => true,
        [b'\x1b', b'[', at @ .., b'H'] => at.iter().all(|b| b.is_ascii_digit() || *b == b';'),
        _ => false,
    }
}
