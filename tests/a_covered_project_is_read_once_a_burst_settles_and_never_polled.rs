//! A project a producer covers has its poll stood down, and is read once for
//! each burst of reports about it, while a project nothing covers goes on
//! polling beside it.
//!
//! The producer replaces the poll rather than backing it up, so the assertion
//! that matters is an absence: after the cover, the only reads of the covered
//! project are the ones its reports asked for. Driven through the binary,
//! because the line is parsed in `collect::changes`, the window is the loop's
//! and the stand-down is the poll's, and they meet nowhere else.

mod terminal;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use terminal::driver::Driven;
use terminal::shims::ShimmedTracker;
use terminal::{a_home_naming_one_project_settled, Producer, THE_DESCRIBED_SUBTREE};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// The call that reads a tracker in full, which every read of the shim's
/// trackers makes, since the shim refuses the probe that would skip it.
const READ_IN_FULL: &str = "list --all --include-gates --limit 0 --json";

/// The shortest poll the config can name, so that a poll left standing would
/// have read the covered project several times over while the test watched.
const REFRESH: Duration = Duration::from_secs(1);

/// Long enough that a read which was coming has, and short enough that one
/// that is not is a failure rather than a hang.
const GIVING_UP: Duration = Duration::from_secs(10);

/// How many reports a burst carries, all in one write.
const A_BURST: [&str; 5] = ["arkham"; 5];

/// A `HOME` that is arkham's directory, naming ferry in a directory called
/// `ferry` elsewhere. Neither says anything about `poll`, so each polls until
/// a producer covers it, and arkham is never left long enough to lapse.
fn a_home_reading_arkham_and_ferry(socket: &Path) -> PathBuf {
    let ferry = std::env::temp_dir()
        .join(format!("bdi-per-burst-{}", std::process::id()))
        .join("ferry");
    std::fs::create_dir_all(&ferry).expect("the directory is ours to make");
    a_home_naming_one_project_settled(
        "covered-per-burst",
        &format!(
            "\n[[projects]]\nname = \"ferry\"\npath = \"{ferry}\"\n\n\
             [tui]\nrefresh_seconds = {refresh}\n\n\
             [changes]\nsocket = \"{socket}\"\ncovered_for_seconds = 600\n",
            ferry = ferry.display(),
            refresh = REFRESH.as_secs(),
            socket = socket.display(),
        ),
    )
}

fn reads_of(tracker: &ShimmedTracker, named: &str) -> usize {
    tracker
        .calls_for(named)
        .iter()
        .filter(|call| *call == READ_IN_FULL)
        .count()
}

fn until(holds: impl Fn() -> bool, awaited: &str) {
    let giving_up = Instant::now() + GIVING_UP;
    while !holds() {
        assert!(Instant::now() < giving_up, "{awaited} never came");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_covered_project_is_read_once_per_burst_and_never_polled_while_its_neighbour_polls() {
    let socket = std::env::temp_dir().join(format!("bdi-per-burst-{}.sock", std::process::id()));
    let home = a_home_reading_arkham_and_ferry(&socket);
    let arkham = home
        .file_name()
        .and_then(|name| name.to_str())
        .expect("the home is named")
        .to_string();
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_DESCRIBED_SUBTREE);
    let _bdi = Driven::bdi_with_arguments(
        ROWS,
        COLS,
        home.clone(),
        &["--all-projects"],
        &tracker.environment(),
    );
    until(
        || reads_of(&tracker, &arkham) >= 1 && reads_of(&tracker, "ferry") >= 1,
        "the first read of each project",
    );
    let mut producer = Producer::connected_to(&socket);

    assert_eq!(producer.says("covered arkham"), "ok arkham");
    // A poll of arkham may have been on its way when the word came, and the
    // read it brings back arms nothing. One interval and its read is enough
    // for it to have landed.
    std::thread::sleep(REFRESH * 2);
    let covered = reads_of(&tracker, &arkham);
    let ferry_when_covered = reads_of(&tracker, "ferry");
    let watching_since = Instant::now();

    for burst in 1..=2 {
        assert_eq!(producer.says_at_once(&A_BURST), ["ok arkham"; 5]);
        until(
            || reads_of(&tracker, &arkham) >= covered + burst,
            "the read the burst asked for",
        );
        std::thread::sleep(REFRESH * 2);
        assert_eq!(
            reads_of(&tracker, &arkham),
            covered + burst,
            "burst {burst} cost one read, and no poll came after it"
        );
    }

    until(
        || reads_of(&tracker, "ferry") >= ferry_when_covered + 3,
        "ferry's polls",
    );
    assert!(
        watching_since.elapsed() >= REFRESH * 3,
        "ferry polled at least three times while arkham was watched"
    );
    assert_eq!(
        reads_of(&tracker, &arkham),
        covered + 2,
        "arkham was read only for its two bursts while ferry polled"
    );

    let _ = std::fs::remove_file(&socket);
}
