//! A project added to the config under a running `bdi` is read and drawn,
//! and one removed from it leaves the view.
//!
//! Driven through the binary because nothing below it can say this. The
//! collection has always drawn the projects the config it is handed names,
//! and `Reload` has always produced a config — what neither of them can say
//! is that the second reaches the first while a reader is looking at the
//! screen. Every piece of this run is real: the file on disk, the check on
//! the loop's deadline set, the collector on its own thread, and the forest
//! drawn from what came back.
//!
//! One edit does both halves, and that is what there is to wait on. A
//! project drawn is something to read off the wire; a project gone is not,
//! and a test that slept for it would be as long as its slowest machine. The
//! project the edit gained arriving is the same snapshot as the one it lost
//! leaving, so waiting for the first is waiting for the second.
//!
//! Both projects are read from the shimmed tracker, which answers the same
//! rows whichever project asks: what is being watched is which projects the
//! screen draws, and a tracker per project would be two captures to keep in
//! step for no question this test asks. The absence is read off a whole
//! frame, because the rows are drawn as a difference from the frame before
//! and a project that has gone leaves its cells where they were until
//! something repaints them.

mod terminal;

use std::path::{Path, PathBuf};
use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::ShimmedTracker;
use terminal::{a_socket_of_its_own, contains, THE_DESCRIBED_SUBTREE};

const ROWS: u16 = 40;
const COLS: u16 = 120;

const ATLAS: &[u8] = "atlas".as_bytes();
const FERRY: &[u8] = "ferry".as_bytes();

/// Long enough for a check to fall due, the collection it asks for to be
/// made, and the frame that draws what came back. The check's own interval
/// is `tui::reload::CHECKED_EVERY`, which is not a config setting and so
/// cannot be shortened for a test.
const A_RELOAD_AND_ITS_COLLECTION: Duration = Duration::from_secs(20);

/// A `HOME` whose config names `projects`, each in a directory of its own
/// under it.
///
/// Under it rather than at it, so the directory `bdi` is started in — the
/// `HOME` itself — belongs to no project and the run reads every project the
/// file names. A project at the directory `bdi` was started in would scope
/// the run to itself, and a project added beside it would then be left out
/// for that reason rather than for any reason this test is about.
fn a_home_naming(named: &str, projects: &[&str]) -> PathBuf {
    let home = std::env::temp_dir().join(format!("bdi-{named}-{}", std::process::id()));
    std::fs::create_dir_all(home.join(".config/beady-eye")).expect("the directory is ours to make");
    naming(&home, projects);
    home
}

/// The config rewritten to name `projects` and nothing else, as a reader
/// editing the file leaves it.
fn naming(home: &Path, projects: &[&str]) {
    let written: String = projects
        .iter()
        .map(|project| {
            let path = home.join(project);
            std::fs::create_dir_all(&path).expect("the directory is ours to make");
            format!(
                "[[projects]]\nname = \"{project}\"\npath = \"{}\"\n\n",
                path.display()
            )
        })
        .collect();
    std::fs::write(home.join(".config/beady-eye/config.toml"), written)
        .expect("the config is ours to write");
}

#[test]
fn a_project_added_to_the_config_appears_and_one_removed_leaves() {
    let home = a_home_naming("config-collects", &["atlas"]);
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_DESCRIBED_SUBTREE);
    let mut environment = tracker.environment();
    environment.push(a_socket_of_its_own(&home));

    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &environment);
    bdi.read_until(ATLAS, GIVING_UP);

    naming(&home, &["ferry"]);
    bdi.read_until(FERRY, A_RELOAD_AND_ITS_COLLECTION);

    let whole = bdi.resize(ROWS - 1, COLS);
    let frame = bdi.answer_to(whole, GIVING_UP);
    assert!(
        contains(&frame, FERRY),
        "the project the reader added was collected and drawn, with no \
         restart\n{}",
        bdi.timeline()
    );
    assert!(
        !contains(&frame, ATLAS),
        "and the one they took out left no rows behind it\n{}",
        bdi.timeline()
    );
}
