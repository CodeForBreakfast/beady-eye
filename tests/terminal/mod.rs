//! A terminal of our own to run `bdi` on: the pty harness the suite drives
//! `bdi` with.
//!
//! `bdi` only behaves like a terminal application when it has one, so the
//! tests that read what it writes open a pty and hand it the far end. What is
//! shared here is the machinery for getting a `bdi` onto one; how each test
//! then reads it differs, and stays with the test.
//!
//! Start here rather than writing another one. What it can do is listed
//! rather than left to be found:
//!
//! * a sized pty and a `bdi` owning it, on a command line of the test's own
//!   — [`a_pty`], [`own_the_terminal`], [`bdi_on`],
//!   [`driver::Driven::bdi_with_arguments`];
//! * a `bdi` that dies with the binary that spawned it, so a test binary
//!   killed before its `Drop` leaves nothing running — [`own_the_terminal`];
//! * typing at it and timestamping what comes back — [`driver::Driven`];
//! * making `bd` slow, so a stalled loop can be told from a slow one —
//!   [`shims::ShimmedTracker`] and `tests/shims/`;
//! * a run whose inbound socket is its own, so its foot carries no notice
//!   about having failed to open one — [`a_socket_of_its_own`];
//! * something outside `bdi` speaking on that socket and reading what it was
//!   answered — [`Producer`], [`the_socket_under`];
//! * a forest with a tracker's beads in it, opened and walked to a known
//!   row — [`over_the_described_subtree`] and [`THE_DESCRIBED_SUBTREE`];
//! * a forest with more lines than a short screen has room for, for a test
//!   about what scrolls — [`over_the_loose_roots`] and [`THE_LOOSE_ROOTS`].
//!
//! The size is why this is a harness rather than a shell one-liner: `script
//! -T` with stdout to a file gives a 0x0 pty, and ratatui then draws an empty
//! frame and yields perfectly plausible timings for a screen with nothing on
//! it. Here it is an argument to `openpty` and cannot be forgotten.

// Each test binary uses the part of this that binary needs, so an unused item
// here is not a dead one.
#![allow(dead_code)]

pub mod driver;
pub mod shims;

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::Duration;

/// The terminal is on the alternate screen from here. Waited for rather than
/// slept through: the screen opens in tens of milliseconds and every tracker
/// is read after it, so a sleep long enough to be safe is a sleep spent
/// watching a `bdi` that has been drawing for most of it.
pub const ENTER_ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049h";

/// A pty: the end the test reads, and the end `bdi` draws on.
///
/// The master stays with whoever calls this, which is what decides who can
/// hang the pty up: it hangs up when the last *master* handle closes, so a
/// spawner whose child inherited a copy can never hang up on it. End of file
/// on the master is a different question with a different answer — it arrives
/// when the last *slave* handle closes, and every one of those is `bdi`'s own
/// stdio, so a read here sees no end of file while `bdi` is alive.
///
/// Which is why the master comes back non-blocking. A frame ends in silence,
/// and with no end of file coming there is nothing else to end a read: one
/// that outlives its `poll` waits for the next frame instead of reporting
/// that this one is over, and a test that waits like that runs to its whole
/// deadline — bdi-2bb.28, measured at 60.05s against 0.66s. Set here, every
/// read on a pty from this harness is bounded by the poll before it. A `bdi`
/// that dies still ends a read the same way it always did: end of file is a
/// read of nothing, blocking or not.
pub fn a_pty(rows: u16, cols: u16) -> (OwnedFd, std::fs::File) {
    let mut ours = 0;
    let mut theirs = 0;
    // Apple's `openpty` takes the size as `*mut winsize`, Linux's as
    // `*const`; a `*mut` coerces to either.
    let mut size = libc::winsize {
        ws_row: rows,
        ws_col: cols,
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
                std::ptr::from_mut(&mut size),
            )
        },
        0,
        "a pty is ours to open"
    );
    // `openpty` hands back a master with no close-on-exec, and `Command`
    // closes only the fds it opened itself, so without this every `bdi` the
    // suite spawns inherits a handle to the terminal it is the far end of.
    // The slave is left as it comes: the child holds three dups of it as its
    // stdio anyway, so a fourth copy is one more handle that dies with it.
    assert_eq!(
        unsafe { libc::fcntl(ours, libc::F_SETFD, libc::FD_CLOEXEC) },
        0,
        "the master is ours to keep to ourselves"
    );
    assert_eq!(
        unsafe { libc::fcntl(ours, libc::F_SETFL, libc::O_NONBLOCK) },
        0,
        "the master is ours to read without blocking"
    );
    unsafe {
        (
            OwnedFd::from_raw_fd(ours),
            std::fs::File::from_raw_fd(theirs),
        )
    }
}

/// Between the fork and the exec: tie the child's life to the process that
/// spawned it, and make the pty its controlling terminal so crossterm finds
/// one to read keys from.
///
/// # Safety
///
/// Runs in the forked child before `exec`, so only async-signal-safe calls
/// belong here. All of these are.
pub fn own_the_terminal(spawned_by: u32) -> std::io::Result<()> {
    die_with(spawned_by)?;
    unsafe {
        if libc::setsid() == -1 {
            return Err(std::io::Error::last_os_error());
        }
        // `ioctl` takes a `c_ulong` request everywhere, but Apple's libc
        // declares `TIOCSCTTY` a `c_uint`, so it is widened to whatever the
        // platform's `ioctl` asks for.
        if libc::ioctl(libc::STDIN_FILENO, libc::TIOCSCTTY as _, 0) == -1 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Ask the kernel to kill this child when the process that spawned it dies.
///
/// Every harness here reaps its `bdi` from `Drop`, and a test binary that is
/// killed runs no `Drop`. Nothing outside the process can pick up after it:
/// the `setsid` above gives the child a session and a process group of its
/// own, so a killer working by process group never sees it. Left to itself
/// the child outlives everything and keeps whatever it bound, which is how
/// `bdi` processes came to hold the inbound socket for a whole afternoon.
///
/// The pty hanging up is not a second answer to that. `a_pty` keeps the
/// master to the spawner, so a spawner that dies does hang the pty up — but
/// what that reaps the child by is `bdi`'s own `SIGHUP` handling, which is
/// product code. These leaks happen under mutation, where product code is
/// precisely what is being changed, so a guarantee resting on it is not one.
/// This arming is done before the exec and depends on nothing a mutant can
/// reach, which is why it holds whatever else does.
///
/// Linux only, because a parent-death signal is. On a system without one the
/// `Drop` is all there is.
#[cfg(target_os = "linux")]
fn die_with(spawned_by: u32) -> std::io::Result<()> {
    unsafe {
        if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) == -1 {
            return Err(std::io::Error::last_os_error());
        }
        // The signal fires on a death, so a spawner that died between the
        // fork and the line above has already spent it and this child would
        // be the leak. Nothing has been exec'd yet, so there is nothing to
        // unwind.
        if libc::getppid() != spawned_by as libc::pid_t {
            libc::_exit(1);
        }
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn die_with(_spawned_by: u32) -> std::io::Result<()> {
    Ok(())
}

/// A `HOME` holding a config that names one project by its path and nothing
/// else, so `bdi` gets past config assembly and as far as drawing. The
/// project's path is the same directory, which holds no tracker, so the
/// collection fails fast unless a test puts a [`shims::ShimmedTracker`] on
/// PATH to answer for one.
///
/// A path and nothing else is read in `bdi`'s own environment, which is what
/// a test about `bd` needs: nothing else is run before `bd` is reached, on
/// this machine or on one with no direnv.
///
/// What is drawn is still whatever the machine has to say, where a test gives
/// `bdi` no `PATH` of its own: `bdi` asks herdr for the live agents by a
/// `PATH` lookup, and on a machine running one it answers with that reader's
/// own panes. Nothing here asserts on any of it. These tests read escape
/// sequences, so a frame full of somebody's real panes and a frame saying
/// there is no herdr are the same frame to them.
///
/// A test that puts the shims on `PATH` no longer reaches that herdr: the
/// shim refuses a call it has no answer for rather than handing it on. What
/// is left is the run with no `PATH` at all, which is a read and never a
/// focus.
pub fn a_home_naming_one_project(named: &str) -> PathBuf {
    a_home_naming_one_project_settled(named, "")
}

/// The same, with `settings` appended to the config — a `[tui]` table, say,
/// for a test whose subject is something `bdi` is configured to wait for.
///
/// Handed as text rather than as fields because the config is what a reader
/// of the test has to picture, and every one of these tests is about what
/// `bdi` does with a config a person could have written.
pub fn a_home_naming_one_project_settled(named: &str, settings: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("bdi-{named}-{}", std::process::id()));
    std::fs::create_dir_all(home.join(".config/beady-eye")).expect("the directory is ours to make");
    std::fs::write(
        home.join(".config/beady-eye/config.toml"),
        format!(
            "[[projects]]\nname = \"atlas\"\npath = \"{}\"\n{settings}",
            home.display()
        ),
    )
    .expect("the config is ours to write");
    home
}

/// A `HOME` whose config does not parse, for a test whose subject is a `bdi`
/// that exits before it has drawn anything: reading the config is the first
/// thing `bdi` does, and a config it cannot read is an error on the way out.
pub fn a_home_whose_config_does_not_parse(named: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("bdi-{named}-{}", std::process::id()));
    std::fs::create_dir_all(home.join(".config/beady-eye")).expect("the directory is ours to make");
    std::fs::write(
        home.join(".config/beady-eye/config.toml"),
        "[[projects]\nthis is not toml\n",
    )
    .expect("the config is ours to write");
    home
}

/// What a panic said, for a test whose subject is a refusal: the payload is
/// a `String` where the message was formatted and a `&str` where it was not.
pub fn said_by(panic: &Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|said| said.to_string()))
        .expect("the refusal is a message")
}

/// A `bdi` drawing on the far end of a pty, given `arguments` on its command
/// line and `environment` on top of what the test binary carries.
///
/// The runtime directory a run has is the test's to give and never the
/// machine's: [`a_socket_of_its_own`] gives one, and what a run without one
/// draws is then the same wherever the suite is run rather than following
/// whether whoever is sitting there has a runtime directory and what else is
/// already listening in it.
pub fn bdi_on(
    theirs: &std::fs::File,
    home: &Path,
    arguments: &[&str],
    environment: &[(String, String)],
) -> Child {
    let spawned_by = std::process::id();
    unsafe {
        Command::new(env!("CARGO_BIN_EXE_bdi"))
            .args(arguments)
            .current_dir(home)
            .env("HOME", home)
            .env("TERM", "xterm-256color")
            .env_remove("BEADS_DIR")
            .env_remove("BDI_PROJECT")
            .env_remove("XDG_RUNTIME_DIR")
            .envs(environment.iter().map(|(named, value)| (named, value)))
            .stdin(theirs.try_clone().expect("the pty is ours to hand over"))
            .stdout(theirs.try_clone().expect("the pty is ours to hand over"))
            .stderr(theirs.try_clone().expect("the pty is ours to hand over"))
            .pre_exec(move || own_the_terminal(spawned_by))
            .spawn()
    }
    .expect("bdi runs")
}

pub fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|at| at == needle)
}

/// The row of the screen `bdi` drew some text on, counted from zero.
///
/// The stream is put back together into the screen it would have drawn, and
/// the needle looked for there, because a row does not reach the pty as one
/// run: a frame is written as the cells that differ from a blank screen, so
/// every cell already blank is skipped and a row's words arrive with a cursor
/// move between each of them. Every row drawn at the terminal's own
/// foreground is that row — a staffed one, and under this scale an ordinary
/// one too — and a search over the bytes finds nothing on any of them while
/// finding whole rows elsewhere.
///
/// The alternative for a test that needs a row is to work it out from the
/// geometry it is testing, which is that arithmetic written a second time and
/// green whenever both copies are wrong the same way.
///
/// Nothing where the text was drawn on no row, or in more than one place: a
/// needle met twice would hand back whichever came first, and a test built on
/// it would press somewhere nobody chose. Read a frame `bdi` was made to
/// repaint whole, since one drawn as a difference from the frame before holds
/// only the cells that moved and the rest of this screen is the frame before,
/// which is not here to be put back.
pub fn row_of(screen: &[u8], needle: &[u8]) -> Option<u16> {
    match rows_of(screen, needle).as_slice() {
        [only] => Some(*only),
        _ => None,
    }
}

/// Every row of the screen the text was drawn on, once for each time it was
/// drawn there, which is how a test says *twice*. A search over the stream
/// says it for neither, and for the reason above: the words of a row arrive
/// with a cursor move between them, so the needle is on the wire in pieces.
pub fn rows_of(screen: &[u8], needle: &[u8]) -> Vec<u16> {
    let Ok(needle) = std::str::from_utf8(needle) else {
        return Vec::new();
    };
    drawn_rows(screen)
        .into_iter()
        .flat_map(|(row, said)| said.matches(needle).map(|_| row).collect::<Vec<_>>())
        .collect()
}

/// The escape that opens a control sequence.
const CSI: &[u8] = b"\x1b[";

/// Every row the stream put anything on, as the text that would be on it,
/// with the cells nothing was written to left as the blanks they are.
fn drawn_rows(screen: &[u8]) -> BTreeMap<u16, String> {
    let mut rows: BTreeMap<u16, Vec<char>> = BTreeMap::new();
    let mut at = (0u16, 0u16);
    let mut rest = screen;

    while !rest.is_empty() {
        if rest[0] == ESC {
            let length = escape(rest);
            if let Some(moved) = move_to(&rest[..length]) {
                at = moved;
            }
            rest = &rest[length..];
            continue;
        }

        let text = rest
            .iter()
            .position(|byte| *byte == ESC)
            .unwrap_or(rest.len());
        let said = String::from_utf8_lossy(&rest[..text]).into_owned();
        let row = rows.entry(at.0).or_default();
        for glyph in said.chars() {
            let column = at.1 as usize;
            if row.len() <= column {
                row.resize(column + 1, ' ');
            }
            row[column] = glyph;
            at.1 += 1;
        }
        rest = &rest[text..];
    }

    rows.into_iter()
        .map(|(row, said)| (row, said.into_iter().collect()))
        .collect()
}

/// The byte an escape sequence opens with.
const ESC: u8 = 0x1b;

/// How long the escape sequence at the head of this stream is. A control
/// sequence runs to its final byte; anything else is taken as the two bytes
/// of the escape alone, which is right for the escapes ratatui and crossterm
/// send here and wrong for a string one — nothing draws with those, and a
/// stream that gained one would put its payload on a row as text.
fn escape(sequence: &[u8]) -> usize {
    if !sequence.starts_with(CSI) {
        return sequence.len().min(2);
    }
    sequence[CSI.len()..]
        .iter()
        .position(|byte| (0x40..=0x7e).contains(byte))
        .map_or(sequence.len(), |end| CSI.len() + end + 1)
}

/// Where a cursor move puts the cursor, counted from zero — where the
/// sequence at hand is a cursor move at all.
fn move_to(sequence: &[u8]) -> Option<(u16, u16)> {
    let inside = sequence.strip_prefix(CSI)?;
    let (parameters, end) = inside.split_at(inside.len().checked_sub(1)?);
    if end != b"H" {
        return None;
    }
    let parameters = std::str::from_utf8(parameters).ok()?;
    let (row, column) = match parameters.split_once(';') {
        Some(both) => both,
        None if parameters.is_empty() => ("1", "1"),
        None => (parameters, "1"),
    };
    Some((
        row.parse::<u16>().ok()?.checked_sub(1)?,
        column.parse::<u16>().ok()?.checked_sub(1)?,
    ))
}

/// A runtime directory of this run's own, so it opens its own inbound socket
/// and carries no notice about having failed to.
///
/// The foot gives up the keys to make room for notices, and a notice shifts
/// the rows of the forest a window is drawn over. A run with no runtime
/// directory and nothing telling it where to listen has nowhere to put a
/// socket, so it carries that notice — and [`bdi_on`] takes the machine's
/// runtime directory away from every run, which is what makes the two
/// screens a test can be given the two it chooses between.
///
/// A run told a path by `--socket` or by its config gets its channel that
/// way instead, and this is the environment's way of saying the same thing.
pub fn a_socket_of_its_own(home: &Path) -> (String, String) {
    ("XDG_RUNTIME_DIR".to_string(), home.display().to_string())
}

/// Where a run given [`a_socket_of_its_own`] listens, for a test that wants
/// to speak to it.
pub fn the_socket_under(runtime_directory: &Path) -> PathBuf {
    runtime_directory.join("beady-eye/changes.sock")
}

/// Long enough that an answer which was coming has, and short enough that a
/// test waiting for one that is not is a failure rather than a hang.
const AN_ANSWER: Duration = Duration::from_secs(10);

/// Something outside `bdi` saying a project's work has moved on, holding its
/// connection open the way a real one does: a long-running producer connects
/// once and speaks whenever it has something to say.
///
/// Held open rather than reconnected because that is the shape the protocol
/// is for, and because a fresh connection is entitled to a fresh reading of
/// anything — a test that reconnected would ask the weaker question.
pub struct Producer {
    speaking: UnixStream,
    listening: BufReader<UnixStream>,
}

impl Producer {
    /// Connected to whichever run is listening on this socket.
    pub fn connected_to(at: &Path) -> Self {
        let speaking = UnixStream::connect(at)
            .unwrap_or_else(|why| panic!("bdi is listening on {} ({why})", at.display()));
        let listening = speaking
            .try_clone()
            .expect("the connection is ours to read");
        listening
            .set_read_timeout(Some(AN_ANSWER))
            .expect("a read that is not answered is ours to give up on");
        Self {
            speaking,
            listening: BufReader::new(listening),
        }
    }

    /// Say one project's work has moved, and hand back what `bdi` answered.
    pub fn says(&mut self, project: &str) -> String {
        writeln!(self.speaking, "{project}").expect("the message is ours to send");
        let mut answer = String::new();
        self.listening
            .read_line(&mut answer)
            .expect("bdi answers every line");
        answer.trim_end().to_string()
    }
}

/// What `bd list --all --limit 0 --json` said about one open epic of this
/// project's own tracker and the four open beads under it.
///
/// The captures beside it carry no descriptions, and a bead window with
/// nothing in it is one that cannot be scrolled and can hardly be told from
/// the next bead's. These rows carry the prose their beads were written
/// with, which is what the window is for.
pub const THE_DESCRIBED_SUBTREE: &str = include_str!("../fixtures/bd_described_subtree.json");

/// The keys that open that capture's tree and leave the selection on its
/// header: every tree rather than only the staffed ones, back to the first
/// row, down onto the tree, open it — and back to the first row and down
/// again.
///
/// `a` is needed because no pane sits in the temp `HOME`, so the live-agent
/// filter holds the one tree there is behind its project's *no live agent*
/// line rather than drawing its row. The moves are what make the walk the
/// same on every machine: which row the selection rests on at startup follows
/// what herdr says about the panes this machine is running, and `g` is deaf
/// to all of it. They are done twice because the key that opens a tree is
/// also the key that steps into one, so where the selection is afterwards is
/// a fact about the fold rather than about the walk.
const OPEN_THE_TREE: &[u8] = b"agjlgj";

/// Where that walk leaves the forest, counting the rows of the screen from
/// zero: the project, its one tree, the anomaly its dangling edges raise,
/// and then the four beads. Whatever herdr says about this machine's panes
/// is drawn under all of them, so these rows are the same everywhere.
pub const THE_TREES_HEADER: u16 = 1;
pub const THE_FIRST_BEAD: u16 = 3;

/// A `bdi` on a pty of this size over [`THE_DESCRIBED_SUBTREE`], with its
/// tree open and the selection on the tree's header.
///
/// The tracker comes back with it because a test that refreshes has to say
/// what the next collection finds.
pub fn over_the_described_subtree(
    named: &str,
    rows: u16,
    cols: u16,
    settled: Duration,
) -> (driver::Driven, shims::ShimmedTracker) {
    over(
        named,
        THE_DESCRIBED_SUBTREE,
        OPEN_THE_TREE,
        rows,
        cols,
        settled,
    )
}

/// What the same command said about a project of six loose roots and two
/// small trees.
///
/// The described subtree draws seven lines however tall the screen is, and a
/// screen short enough for seven lines to scroll by three has a forest band
/// of four rows — so the third row of the scroll is the last one there is,
/// and a view that moved by five would land in the same place. These roots
/// draw enough lines for the distance to be the distance.
pub const THE_LOOSE_ROOTS: &str = include_str!("../fixtures/bulk_loose_roots.json");

/// The keys that draw every one of those roots and leave the selection on the
/// first: every tree rather than only the staffed ones, back to the first row
/// — both for the reasons [`OPEN_THE_TREE`] gives — and down onto the root
/// under the project, which is a bead and so has a window to name it.
const SHOW_EVERY_ROOT: &[u8] = b"agj";

/// Where that walk leaves the selection, and the whole of the window's title
/// over it. Whole because the id of a root is a prefix of nothing else here,
/// but the title of the window is what says a window is over *this* bead.
pub const THE_FIRST_ROOTS_WINDOW: &str = "orb-c3 · Esc to go back";

/// A `bdi` on a pty of this size over [`THE_LOOSE_ROOTS`], with every root
/// drawn and the selection on the first of them.
pub fn over_the_loose_roots(
    named: &str,
    rows: u16,
    cols: u16,
    settled: Duration,
) -> (driver::Driven, shims::ShimmedTracker) {
    over(named, THE_LOOSE_ROOTS, SHOW_EVERY_ROOT, rows, cols, settled)
}

/// A `bdi` on a pty of this size over one capture, walked to where the tests
/// using it start.
fn over(
    named: &str,
    holds: &str,
    walk: &[u8],
    rows: u16,
    cols: u16,
    settled: Duration,
) -> (driver::Driven, shims::ShimmedTracker) {
    let home = a_home_naming_one_project(named);
    let tracker = shims::ShimmedTracker::beside(&home);
    tracker.holds(holds);
    let mut environment = tracker.environment();
    environment.push(a_socket_of_its_own(&home));

    let mut bdi = driver::Driven::bdi(rows, cols, home, &environment);
    bdi.read_until(ENTER_ALTERNATE_SCREEN, driver::GIVING_UP);
    bdi.settle(settled, driver::GIVING_UP);
    bdi.send(walk);
    bdi.settle(settled, driver::GIVING_UP);
    (bdi, tracker)
}
