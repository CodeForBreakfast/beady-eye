//! `[tui] unanswered_after_seconds` written under a running `bdi` is how long
//! the next read may go unanswered.
//!
//! The third key of `[tui]`, and the one the bead this landed under did not
//! count: `docs/design.md` named all three and the tracker named two. It is
//! the loop's rather than the screen's — `Outstanding` stamps each read with
//! how long it may wait, and the project line draws the mark that says a read
//! has stopped getting anywhere rather than that it is on its way.
//!
//! **The run opens on a patience no test outlives.** Five minutes, against a
//! test that gives up in twenty seconds — so the mark arriving at all is the
//! whole assertion, and there is no window in which the old value could have
//! produced it. That is why this waits for something to appear rather than
//! measuring how long it took: a duration would be a reading of the machine,
//! and the appearance is only possible under the value the reader wrote.
//!
//! The mark is one glyph, which is what makes it a needle. A read that has
//! stopped getting anywhere is the turning mark held still — where the frames
//! were and made of the same dots, so a reader watching one turn sees it stop
//! rather than sees something else appear.

mod terminal;

use std::path::{Path, PathBuf};
use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::ShimmedTracker;
use terminal::{a_socket_of_its_own, THE_DESCRIBED_SUBTREE};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// The mark a project wears once its read has stopped getting anywhere,
/// `view::phrase`'s `UNANSWERED`.
const STOPPED_GETTING_ANYWHERE: &[u8] = "⠿".as_bytes();

/// The mark a project wears when it has been read, which is what says the
/// first collection came back before the tracker was told to hold.
const READ: &[u8] = "✓".as_bytes();

/// A patience nothing in a run of this test reaches.
const WAITING_FIVE_MINUTES: &str = "\n[tui]\nunanswered_after_seconds = 300\n";

/// The patience the reader writes, shorter than the gap between one config
/// check and the next.
const WAITING_A_SECOND: &str = "\n[tui]\nunanswered_after_seconds = 1\n";

/// Long enough for the check to fall due, the read it asks for to be held,
/// and the second that read may then wait. `tui::reload::CHECKED_EVERY` is a
/// constant rather than a config setting, so nothing here can shorten it.
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

#[test]
fn a_read_waits_out_the_patience_the_reader_writes_without_a_restart() {
    let home = a_home_settled("patience", WAITING_FIVE_MINUTES);
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_DESCRIBED_SUBTREE);
    let mut environment = tracker.environment();
    environment.push(a_socket_of_its_own(&home));

    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &environment);
    bdi.read_until(READ, GIVING_UP);

    // From here every `bd` call is held, so the read the reload asks for
    // never comes back and the only question is how long the screen waits
    // before saying so.
    tracker.hang();
    settling(&home, WAITING_A_SECOND);

    bdi.read_until(STOPPED_GETTING_ANYWHERE, A_RELOAD);
}
