//! A project that does not poll says so once nothing has vouched for it for
//! longer than `[changes] covered_for_seconds`, and a producer saying it
//! covers the project takes that back without a read.
//!
//! With no poll behind it, a producer that has died leaves rows looking
//! exactly as current as a producer covering a tracker nobody touches. The
//! mark beside the project's name is the one thing that can tell them apart.
//!
//! Driven through the binary because the halves meet nowhere else: the line
//! is parsed in `collect::changes`, the lapse is the loop's, and the mark is
//! the view's.
//!
//! The age is what says the word asked for no read. It goes on counting from
//! the run's first read, so a word that had caused one would have put it back
//! to nought.

mod terminal;

use std::path::{Path, PathBuf};
use std::time::Instant;

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::ShimmedTracker;
use terminal::{a_home_naming_one_project_settled, rows_drawn, Producer, THE_DESCRIBED_SUBTREE};

const ROWS: u16 = 40;
const COLS: u16 = 120;

const ARKHAM: &[u8] = "arkham".as_bytes();

/// Long enough that a frame drawn after the word is on the screen well before
/// the project lapses again, and short enough to wait out.
const COVERED_FOR_SECONDS: u64 = 3;

/// A `HOME` whose one project does not poll, listening on `socket`.
fn a_home_that_does_not_poll(socket: &Path) -> PathBuf {
    a_home_naming_one_project_settled(
        "lapses",
        &format!(
            "poll = false\n\n[changes]\nsocket = \"{}\"\ncovered_for_seconds = {COVERED_FOR_SECONDS}\n",
            socket.display()
        ),
    )
}

/// One frame with every cell of the screen in it. A resize is what asks for
/// one, so the height alternates: a resize to the height the terminal already
/// is is no resize at all.
fn a_whole_frame(bdi: &mut Driven, rows: &mut u16) -> Vec<u8> {
    *rows = if *rows == ROWS { ROWS - 1 } else { ROWS };
    let whole = bdi.resize(*rows, COLS);
    bdi.answer_to(whole, GIVING_UP)
}

/// The project's own line, which is the one row saying how long ago its
/// tracker was read.
fn arkhams_line(frame: &[u8]) -> Option<String> {
    rows_drawn(frame)
        .into_iter()
        .find(|row| row.contains("arkham") && row.contains(" ago"))
}

/// Whole frames until the project's line carries `mark`, handing back the
/// line either way — one that never did is what the assertion fails against.
fn line_once_it_wears(bdi: &mut Driven, rows: &mut u16, mark: char) -> Option<String> {
    let giving_up = Instant::now() + GIVING_UP;
    loop {
        let line = arkhams_line(&a_whole_frame(bdi, rows));
        let wears = line.as_deref().is_some_and(|line| line.contains(mark));
        if wears || Instant::now() >= giving_up {
            return line;
        }
    }
}

/// How many seconds old the line says the rows are.
fn age_in_seconds(line: &str) -> Option<u64> {
    let before = line.split("s ago").next()?;
    before.rsplit(' ').next()?.parse().ok()
}

#[test]
fn a_project_nothing_vouches_for_says_so_until_something_covers_it() {
    let socket = std::env::temp_dir().join(format!("bdi-lapses-{}.sock", std::process::id()));
    let home = a_home_that_does_not_poll(&socket);
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_DESCRIBED_SUBTREE);
    let mut bdi = Driven::bdi(ROWS, COLS, home, &tracker.environment());
    bdi.read_until(ARKHAM, GIVING_UP);
    let mut rows = ROWS;

    let lapsed = line_once_it_wears(&mut bdi, &mut rows, '?');
    assert!(
        lapsed.as_deref().is_some_and(|line| line.contains('?')),
        "nothing covered arkham and it does not poll, so its line says so: \
         {lapsed:?}\n{}",
        bdi.timeline()
    );

    assert_eq!(
        Producer::connected_to(&socket).says("covered arkham"),
        "ok arkham"
    );

    let covered = line_once_it_wears(&mut bdi, &mut rows, '✓');
    assert!(
        covered.as_deref().is_some_and(|line| line.contains('✓')),
        "a producer said it covers arkham, so its line says it was read: \
         {covered:?}\n{}",
        bdi.timeline()
    );
    assert!(
        covered
            .as_deref()
            .and_then(age_in_seconds)
            .is_some_and(|age| age >= COVERED_FOR_SECONDS),
        "and the rows are as old as the run's first read, since the word \
         asked for no read of its own: {covered:?}"
    );

    let _ = std::fs::remove_file(&socket);
}
