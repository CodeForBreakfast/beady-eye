//! `[tui] refresh_seconds` written under a running `bdi` is how long each
//! project waits before asking to be read again.
//!
//! **This one was already true and this test is why it stays true.**
//! `bdi-8un.2` folded a project's `poll` and its interval into one value in
//! `Armed::still_due`, where they cannot be separated: what a reload keeps is
//! the deadline the last read armed, and what it takes from the file is how
//! long the next gap is. So a reader shortening the interval gets it at once.
//! Nothing asserted that through the drawn output, and a claim with no test
//! is the one that quietly stops being true when somebody rearranges the
//! thing it rests on.
//!
//! Read off the age on the project's line, because that is where a read
//! coming back is visible. The run opens on ten minutes, so the age climbs
//! and nothing puts it back; the edit is what makes it start again.
//!
//! **Both halves are read off whole frames, and neither could be read off the
//! stream.** An age that ticks changes one cell, so what reaches the wire is
//! the digit and not the phrase, and a needle naming the whole line is in no
//! run of the stream at all. Every age between zero and ten seconds *was* on
//! the wire during the first half besides, so even a needle the stream
//! carried could not say whether one arrived again.

mod terminal;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::ShimmedTracker;
use terminal::{a_socket_of_its_own, contains, THE_DESCRIBED_SUBTREE};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// An age the project only reaches by not being read again, which on the
/// interval the run opens with is every second after the first read.
const A_WHILE_SINCE_IT_WAS_READ: &[u8] = "✓ 5s ago".as_bytes();

/// The age a project wears the moment a read of it comes back. On the
/// interval the reader writes, one lands every second, so a frame taken at
/// any instant catches one soon.
const JUST_READ: &[u8] = "✓ 0s ago".as_bytes();

/// An interval no run of this test outlives.
const ASKING_EVERY_TEN_MINUTES: &str = "\n[tui]\nrefresh_seconds = 600\n";

/// The interval the reader writes.
const ASKING_EVERY_SECOND: &str = "\n[tui]\nrefresh_seconds = 1\n";

/// Long enough for the check to fall due, the read it arms to come back, and
/// a frame to be taken while its age is still zero.
const A_RELOAD: Duration = Duration::from_secs(20);

/// A `HOME` whose config names one project and carries `settings`.
fn a_home_settled(named: &str, settings: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("bdi-{named}-{}", std::process::id()));
    std::fs::create_dir_all(home.join(".config/beady-eye")).expect("the directory is ours to make");
    settling(&home, settings);
    home
}

/// The config rewritten to carry `settings`, as a reader editing the file
/// leaves it.
fn settling(home: &Path, settings: &str) {
    std::fs::write(
        home.join(".config/beady-eye/config.toml"),
        format!(
            "[[projects]]\nname = \"atlas\"\npath = \"{}\"\n{settings}",
            home.display()
        ),
    )
    .expect("the config is ours to write");
}

/// One frame with every cell of the screen in it. A resize is what asks for
/// one, so the height alternates: a resize to the height the terminal already
/// is is no resize at all.
fn a_whole_frame(bdi: &mut Driven, rows: &mut u16) -> Vec<u8> {
    *rows = if *rows == ROWS { ROWS - 1 } else { ROWS };
    let whole = bdi.resize(*rows, COLS);
    bdi.answer_to(whole, GIVING_UP)
}

/// Whole frames until `arrived` is on one, handed back either way — a frame
/// without it is what the assertion needs to fail against.
fn frames_until(bdi: &mut Driven, rows: &mut u16, arrived: &[u8], patience: Duration) -> Vec<u8> {
    let giving_up = Instant::now() + patience;
    loop {
        let frame = a_whole_frame(bdi, rows);
        if contains(&frame, arrived) || Instant::now() >= giving_up {
            return frame;
        }
    }
}

#[test]
fn a_project_takes_the_interval_the_reader_writes_without_a_restart() {
    let home = a_home_settled("poll-interval", ASKING_EVERY_TEN_MINUTES);
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_DESCRIBED_SUBTREE);
    let mut environment = tracker.environment();
    environment.push(a_socket_of_its_own(&home));

    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &environment);
    let mut rows = ROWS;
    let before = frames_until(&mut bdi, &mut rows, A_WHILE_SINCE_IT_WAS_READ, GIVING_UP);
    assert!(
        contains(&before, A_WHILE_SINCE_IT_WAS_READ),
        "the project was read and then left to age, which is what the interval \
         the run opened on asks for\n{}",
        bdi.timeline()
    );

    settling(&home, ASKING_EVERY_SECOND);
    let after = frames_until(&mut bdi, &mut rows, JUST_READ, A_RELOAD);

    assert!(
        contains(&after, JUST_READ),
        "the project was read again on the interval the reader had just \
         written, rather than going on ageing towards the ten minutes they \
         replaced\n{}",
        bdi.timeline()
    );
}
