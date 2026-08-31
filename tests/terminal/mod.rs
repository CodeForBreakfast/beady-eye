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
//! * a sized pty and a `bdi` owning it — [`a_pty`], [`own_the_terminal`],
//!   [`bdi_on`];
//! * typing at it and timestamping what comes back — [`driver::Driven`];
//! * making `bd` slow, so a stalled loop can be told from a slow one —
//!   [`shims::ShimmedTracker`] and `tests/shims/`.
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

use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

/// The terminal is on the alternate screen from here. Both tests wait for it
/// rather than sleeping: `tui::run` makes its first collection *before* the
/// screen opens, so a fixed sleep watches a `bdi` that has not drawn.
pub const ENTER_ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049h";

/// A pty: the end the test reads, and the end `bdi` draws on.
pub fn a_pty(rows: u16, cols: u16) -> (OwnedFd, std::fs::File) {
    let mut ours = 0;
    let mut theirs = 0;
    let size = libc::winsize {
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
pub fn own_the_terminal() -> std::io::Result<()> {
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

/// A `HOME` holding a config that names one project, so `bdi` gets past
/// config assembly and as far as drawing. The project's path is the same
/// directory, which holds no tracker, so the collection fails fast.
///
/// What is drawn is still whatever the machine has to say — `bdi` asks herdr
/// for the live agents, and on a machine running one it answers. Nothing here
/// asserts on any of it. These tests read escape sequences, so a frame full of
/// somebody's real panes and a frame saying there is no herdr are the same
/// frame to them.
pub fn a_home_naming_one_project(named: &str) -> PathBuf {
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

/// A `HOME` whose one project reaches its tracker without direnv.
///
/// `credential_command` is the escape hatch for a tracker outside direnv's
/// reach, and it is what a test about `bd` needs: without it the first thing
/// a collection does is run direnv, and on a machine without one the project
/// fails there and `bd` is never reached at all.
pub fn a_home_naming_one_project_read_without_direnv(named: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("bdi-{named}-{}", std::process::id()));
    std::fs::create_dir_all(home.join(".config/beady-eye")).expect("the directory is ours to make");
    std::fs::write(
        home.join(".config/beady-eye/config.toml"),
        format!(
            "[[projects]]\nname = \"atlas\"\npath = \"{}\"\n\
             credential_command = \"printf ''\"\n",
            home.display()
        ),
    )
    .expect("the config is ours to write");
    home
}

/// A `bdi` drawing on the far end of a pty, with `environment` on top of what
/// the test binary carries.
pub fn bdi_on(theirs: &std::fs::File, home: &Path, environment: &[(String, String)]) -> Child {
    unsafe {
        Command::new(env!("CARGO_BIN_EXE_bdi"))
            .current_dir(home)
            .env("HOME", home)
            .env("TERM", "xterm-256color")
            .env_remove("BEADS_DIR")
            .env_remove("COMMY_PROJECT")
            .envs(environment.iter().map(|(named, value)| (named, value)))
            .stdin(theirs.try_clone().expect("the pty is ours to hand over"))
            .stdout(theirs.try_clone().expect("the pty is ours to hand over"))
            .stderr(theirs.try_clone().expect("the pty is ours to hand over"))
            .pre_exec(own_the_terminal)
            .spawn()
    }
    .expect("bdi runs")
}

pub fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|at| at == needle)
}
