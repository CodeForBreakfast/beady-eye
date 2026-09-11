//! A badge reads the bead's own external reference where its key names no
//! metadata, and a badge reading metadata is unchanged beside it.
//!
//! Driven through the binary because the whole point of the field is a tracker
//! `bdi` does not have: the reference arrives from `bd` on the wire, is parsed,
//! joined, badged and drawn, and only the terminal sees the end of that. A test
//! at any one layer would hold the value it is about.
//!
//! One bead carrying both sorts, because what the two of them draw side by side
//! is the claim: the key says where to read, and nothing else about a badge
//! changes with the answer.

mod terminal;

use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::ShimmedTracker;
use terminal::{a_home_naming_one_project_settled, ENTER_ALTERNATE_SCREEN};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// A gap this long between bytes means the collection is over and drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// One tracker whose sync adapter has filled the field on its root, and whose
/// reader has also written a reference by hand in metadata.
const THE_TRACKER: &str = r#"[
  {"id":"orb-1","title":"lift the ground station","status":"open",
   "priority":1,"issue_type":"epic",
   "external_ref":"https://jira.invalid/browse/HELIO-412",
   "metadata":{"jira":"ATLAS-19"}}
]"#;

/// A badge on the field and a badge on metadata. The field's is written as a
/// link, so what reaches the wire is one contiguous run a repaint cannot
/// scatter.
const A_BADGE_ON_EACH: &str = concat!(
    "\n[[badges]]\n",
    "key    = \"external_ref\"\n",
    "match  = \".*/(?<ticket>[A-Z]+-[0-9]+)\"\n",
    "render = \"{ticket}\"\n",
    "link   = \"https://jira.invalid/browse/{ticket}\"\n",
    "\n[[badges]]\n",
    "key    = \"metadata.jira\"\n",
    "render = \"{}\"\n",
);

/// `a`, which shows every tree rather than only those with a live agent. No
/// pane sits in the temp `HOME`, so without it the one tree here sits behind
/// its project's *no live agent* line and draws no row of its own.
const SHOW_EVERY_TREE: &[u8] = b"a";

/// The escape that opens an operating-system command naming a hyperlink, and
/// the one that ends any such command.
const OSC_8: &str = "\x1b]8;;";
const ST: &str = "\x1b\\";

#[test]
fn a_badge_on_the_external_reference_is_drawn_beside_one_on_metadata() {
    let home = a_home_naming_one_project_settled("external-ref", A_BADGE_ON_EACH);
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_TRACKER);

    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &tracker.environment());
    bdi.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);
    bdi.send(SHOW_EVERY_TREE);

    let from_the_field =
        format!("{OSC_8}https://jira.invalid/browse/HELIO-412{ST}HELIO-412{OSC_8}{ST}");
    bdi.read_until(from_the_field.as_bytes(), GIVING_UP);

    // Badges are drawn in the order their config names them, so the one on
    // metadata follows on the same row rather than waiting for a repaint.
    bdi.read_until(b"ATLAS-19", GIVING_UP);
}
