//! `bdi --ready <root>` opens on that root's ready list: the beads bd calls
//! ready, then the beads in progress, and none of the tree's other beads.

mod terminal;

use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::ShimmedTracker;
use terminal::{a_home_naming_one_project, contains, ENTER_ALTERNATE_SCREEN};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// A gap this long between bytes means the collection is over and drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// How long a repaint gets before waiting for it is called stalling.
const LONG_ENOUGH_TO_ANSWER: Duration = Duration::from_secs(10);

/// The archive, as `bd list --all --json` writes it: one bead ready, one in
/// progress, and one waiting on the ready one.
const THE_ARCHIVE: &str = r#"[
  {"id":"ark-12","title":"restore the archive","status":"open",
   "priority":1,"issue_type":"epic","created_at":"2026-08-01T09:00:00Z"},
  {"id":"ark-12.1","title":"catalogue the folios","status":"open",
   "dependencies":[{"issue_id":"ark-12.1","depends_on_id":"ark-12","type":"parent-child"}],
   "priority":2,"issue_type":"task","created_at":"2026-08-02T09:00:00Z"},
  {"id":"ark-12.2","title":"rebind the ledgers","status":"in_progress",
   "dependencies":[{"issue_id":"ark-12.2","depends_on_id":"ark-12","type":"parent-child"}],
   "priority":2,"issue_type":"task","created_at":"2026-08-03T09:00:00Z"},
  {"id":"ark-12.3","title":"shelve the bindings","status":"open",
   "dependencies":[{"issue_id":"ark-12.3","depends_on_id":"ark-12","type":"parent-child"},
                   {"issue_id":"ark-12.3","depends_on_id":"ark-12.1","type":"blocks"}],
   "priority":2,"issue_type":"task","created_at":"2026-08-04T09:00:00Z"}
]"#;

/// What `bd ready` answers in the archive.
const READY: &str = r#"[
  {"id":"ark-12.1","title":"catalogue the folios","status":"open",
   "priority":2,"issue_type":"task"}
]"#;

#[test]
fn ready_with_a_root_opens_on_that_roots_ready_list() {
    let home = a_home_naming_one_project("ready-list");
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_ARCHIVE);
    tracker.answers_with("ready --limit 0 --json", READY);
    let mut bdi = Driven::bdi_with_arguments(
        ROWS,
        COLS,
        home.clone(),
        &["--ready", "ark-12"],
        &tracker.environment(),
    );
    bdi.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);
    assert_eq!(tracker.unanswered(), Vec::<String>::new());

    let repainted = bdi.resize(ROWS + 1, COLS);
    let screen = bdi.answer_to(repainted, LONG_ENOUGH_TO_ANSWER);
    let drawn = |said: &str| contains(&screen, said.as_bytes());
    let what_bdi_drew = || format!("{:?}\n{}", String::from_utf8_lossy(&screen), bdi.timeline());

    for said in ["Ready", "progress", "ark-12.1", "folios", "ark-12.2", "ledgers"] {
        assert!(drawn(said), "{said} is not on the screen: {}", what_bdi_drew());
    }
    // Neither ready nor in progress, so the forest draws it and the list
    // does not.
    assert!(!drawn("ark-12.3"), "the waiting bead is listed: {}", what_bdi_drew());
}
