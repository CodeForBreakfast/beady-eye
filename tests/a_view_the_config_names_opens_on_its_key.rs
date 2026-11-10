//! `v` opens each view the config names in turn, and each lists the beads its
//! conditions select and none of the others.

mod terminal;

use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::ShimmedTracker;
use terminal::{a_home_naming_one_project_settled, contains, ENTER_ALTERNATE_SCREEN};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// A gap this long between bytes means the collection is over and drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// How long a repaint gets before waiting for it is called stalling.
const LONG_ENOUGH_TO_ANSWER: Duration = Duration::from_secs(10);

/// Two views: one of the beads waiting on a signature and not held, and one
/// of the urgent beads.
const TWO_VIEWS: &str = r#"
[[views]]
name = "to sign"

[[views.select]]
when = { labels = "signing" }
unless = { labels = "held" }

[[views]]
name = "urgent work"

[[views.select]]
when = { priority = "0" }
"#;

/// The archive, as `bd list --all --json` writes it.
const THE_ARCHIVE: &str = r#"[
  {"id":"ark-12","title":"restore the archive","status":"open",
   "priority":1,"issue_type":"epic","created_at":"2026-08-01T09:00:00Z"},
  {"id":"ark-12.1","title":"countersign the folios","status":"open","labels":["signing"],
   "dependencies":[{"issue_id":"ark-12.1","depends_on_id":"ark-12","type":"parent-child"}],
   "priority":2,"issue_type":"task","created_at":"2026-08-02T09:00:00Z"},
  {"id":"ark-12.2","title":"countersign the ledgers","status":"open",
   "labels":["held","signing"],
   "dependencies":[{"issue_id":"ark-12.2","depends_on_id":"ark-12","type":"parent-child"}],
   "priority":2,"issue_type":"task","created_at":"2026-08-03T09:00:00Z"},
  {"id":"ark-12.3","title":"dry the flooded stacks","status":"open",
   "dependencies":[{"issue_id":"ark-12.3","depends_on_id":"ark-12","type":"parent-child"}],
   "priority":0,"issue_type":"task","created_at":"2026-08-04T09:00:00Z"}
]"#;

/// One frame with every cell of the screen in it, after the key `pressed`.
fn after_pressing(bdi: &mut Driven, pressed: &[u8], rows: &mut u16) -> Vec<u8> {
    bdi.send(pressed);
    bdi.settle(A_SILENCE, GIVING_UP);
    *rows = if *rows == ROWS { ROWS + 1 } else { ROWS };
    let repainted = bdi.resize(*rows, COLS);
    bdi.answer_to(repainted, LONG_ENOUGH_TO_ANSWER)
}

#[test]
fn each_view_opens_in_turn_and_lists_only_what_it_selects() {
    let home = a_home_naming_one_project_settled("views", TWO_VIEWS);
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_ARCHIVE);
    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &tracker.environment());
    bdi.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);
    let mut rows = ROWS;

    for (view, listed, left_out) in [
        ("to sign · 1", "ark-12.1", ["ark-12.2", "ark-12.3"]),
        ("urgent work · 1", "ark-12.3", ["ark-12.1", "ark-12.2"]),
    ] {
        let screen = after_pressing(&mut bdi, b"v", &mut rows);
        let drawn = |said: &str| contains(&screen, said.as_bytes());
        let what_bdi_drew =
            || format!("{:?}\n{}", String::from_utf8_lossy(&screen), bdi.timeline());

        for said in [view, listed] {
            assert!(drawn(said), "{said} is not on the screen: {}", what_bdi_drew());
        }
        for bead in left_out {
            assert!(
                !drawn(bead),
                "{view} lists {bead}, which it does not select: {}",
                what_bdi_drew()
            );
        }
    }
}
