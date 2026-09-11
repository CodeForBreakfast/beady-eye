//! A badge whose config says `colour = "status"` reaches the terminal in the
//! colour that bead's own status is drawn in, and a different one on a bead
//! whose status differs.
//!
//! Driven through the binary because a colour is bytes on the wire and the
//! whole point of naming the row's own slot is that the reader's terminal
//! resolves it. A buffer test says which style a span carries; only the
//! stream says which escape the backend sent, and only the stream carries
//! both beads' badges to be compared against each other.
//!
//! One run of bytes rather than a needle a repaint scatters: a repaint reaches
//! the wire a word at a time, and the ticket the badge draws is one word held
//! in one cell because the badge is also a link. So the colour, the link and
//! the words are contiguous in the stream.

mod terminal;

use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::ShimmedTracker;
use terminal::{a_home_naming_one_project_settled, ENTER_ALTERNATE_SCREEN};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// A gap this long between bytes means the collection is over and drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// Two beads under one root, alike but for their status, each carrying a
/// ticket in a key of the reader's own. Two rather than one, because a colour
/// that never moves is a colour a single badge cannot be shown to have taken
/// from anywhere.
const THE_TRACKER: &str = r#"[
  {"id":"orb-1","title":"lift the ground station","status":"open",
   "priority":1,"issue_type":"epic"},
  {"id":"orb-1.1","title":"repoint the dish","status":"blocked","parent":"orb-1",
   "dependencies":[{"depends_on_id":"orb-1","type":"parent-child"}],
   "priority":2,"issue_type":"task","metadata":{"jira":"ATLAS-19"}},
  {"id":"orb-1.2","title":"trim the sails","status":"in_progress","parent":"orb-1",
   "dependencies":[{"depends_on_id":"orb-1","type":"parent-child"}],
   "priority":2,"issue_type":"task","metadata":{"jira":"ATLAS-20"}}
]"#;

/// The badge this whole knob is for: a ticket in another tracker, pointing at
/// its page and drawn in the colour of the bead it sits on.
const A_TICKET_TRACKING_THE_BEAD: &str = concat!(
    "\n[[badges]]\n",
    "key    = \"metadata.jira\"\n",
    "match  = \"(?<ticket>[A-Z]+-[0-9]+)\"\n",
    "render = \"{ticket}\"\n",
    "link   = \"https://jira.invalid/browse/{ticket}\"\n",
    "colour = \"status\"\n",
);

/// Every tree rather than only the staffed ones, then down onto the tree and
/// open it, so both beads are on the screen.
const OPEN_THE_TREE: &[u8] = b"agjlgj";

/// The escape that opens an operating-system command naming a hyperlink, and
/// the one that ends any such command.
const OSC_8: &str = "\x1b]8;;";
const ST: &str = "\x1b\\";

/// `bd`'s own colours for the two statuses here, as the backend sends them:
/// the foreground, and the background reset that rides with any style change.
/// Written out rather than read off the palette, because what is under test is
/// the bytes a terminal receives.
const BLOCKED: &str = "\x1b[38;2;242;109;120;49m";
const IN_PROGRESS: &str = "\x1b[38;2;255;180;84;49m";

/// One badge's whole cell: the colour it is drawn in, then the link written
/// round its words.
fn drawn(colour: &str, ticket: &str) -> String {
    format!("{colour}{OSC_8}https://jira.invalid/browse/{ticket}{ST}{ticket}{OSC_8}{ST}")
}

#[test]
fn a_badge_coloured_by_status_takes_a_different_colour_on_each_beads_row() {
    let home = a_home_naming_one_project_settled("statuscolour", A_TICKET_TRACKING_THE_BEAD);
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_TRACKER);

    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &tracker.environment());
    bdi.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);
    bdi.send(OPEN_THE_TREE);

    bdi.read_until(drawn(BLOCKED, "ATLAS-19").as_bytes(), GIVING_UP);
    bdi.read_until(drawn(IN_PROGRESS, "ATLAS-20").as_bytes(), GIVING_UP);
}
