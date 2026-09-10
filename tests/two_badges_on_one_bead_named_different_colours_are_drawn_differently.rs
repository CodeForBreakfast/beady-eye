//! Two badges on one bead, one naming a slot of the palette and the other an
//! absolute colour, reach the terminal in two different colours.
//!
//! Driven through the binary because a live config is what exercises this. A
//! buffer test can say which style a span carries, but the two kinds of name
//! differ in what the reader's terminal is asked to resolve — a slot arrives
//! as one of the sixteen the theme owns, an absolute colour as the value
//! written in the config — and only the stream says which of those was sent.
//!
//! One run of bytes per badge rather than a needle a repaint scatters: a
//! repaint reaches the wire a word at a time, and each badge here draws one
//! word held in one cell because it is also a link. So the colour, the link
//! and the words are contiguous in the stream.

mod terminal;

use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::ShimmedTracker;
use terminal::{a_home_naming_one_project_settled, ENTER_ALTERNATE_SCREEN};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// A gap this long between bytes means the collection is over and drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// One bead carrying two references in two keys of the reader's own, so the
/// two badges being compared sit on one row and take one status between them.
/// Whatever separates them on the wire is the colour each was named, because
/// there is nothing else left for it to be.
const THE_TRACKER: &str = r#"[
  {"id":"orb-1","title":"lift the ground station","status":"open",
   "priority":1,"issue_type":"epic"},
  {"id":"orb-1.1","title":"repoint the dish","status":"open","parent":"orb-1",
   "dependencies":[{"depends_on_id":"orb-1","type":"parent-child"}],
   "priority":2,"issue_type":"task",
   "metadata":{"jira":"ATLAS-19","design":"DISH-4"}}
]"#;

/// The two kinds of name, side by side on one bead. `agent` is a slot of the
/// palette, which the theme resolves; `#c71585` is a colour the reader wrote,
/// which no theme can move.
const A_SLOT_AND_A_COLOUR: &str = concat!(
    "\n[[badges]]\n",
    "key    = \"jira\"\n",
    "match  = \"(?<ticket>[A-Z]+-[0-9]+)\"\n",
    "render = \"{ticket}\"\n",
    "link   = \"https://jira.invalid/browse/{ticket}\"\n",
    "colour = \"agent\"\n",
    "\n[[badges]]\n",
    "key    = \"design\"\n",
    "match  = \"(?<sheet>[A-Z]+-[0-9]+)\"\n",
    "render = \"{sheet}\"\n",
    "link   = \"https://design.invalid/{sheet}\"\n",
    "colour = \"#c71585\"\n",
);

/// Every tree rather than only the staffed ones, then down onto the tree and
/// open it, so the bead carrying both badges is on the screen.
const OPEN_THE_TREE: &[u8] = b"agjlgj";

/// The escape that opens an operating-system command naming a hyperlink, and
/// the one that ends any such command.
const OSC_8: &str = "\x1b]8;;";
const ST: &str = "\x1b\\";

/// The two colours as the backend sends them: the foreground, and the
/// background reset that rides with any style change. Written out rather than
/// read off the palette, because what is under test is the bytes a terminal
/// receives — and the two forms are the whole point. `agent` leaves as an
/// index into the theme's own sixteen; the reader's own colour leaves as the
/// value they wrote.
const AGENT: &str = "\x1b[38;5;2;49m";
const THE_READERS_OWN: &str = "\x1b[38;2;199;21;133;49m";

/// One badge's whole cell: the colour it is drawn in, then the link written
/// round its words.
fn drawn(colour: &str, host: &str, reference: &str) -> String {
    format!("{colour}{OSC_8}https://{host}{reference}{ST}{reference}{OSC_8}{ST}")
}

#[test]
fn a_badge_naming_a_slot_and_one_naming_a_colour_leave_in_different_colours() {
    let home = a_home_naming_one_project_settled("twocolours", A_SLOT_AND_A_COLOUR);
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_TRACKER);

    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &tracker.environment());
    bdi.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);
    bdi.send(OPEN_THE_TREE);

    bdi.read_until(
        drawn(AGENT, "jira.invalid/browse/", "ATLAS-19").as_bytes(),
        GIVING_UP,
    );
    bdi.read_until(
        drawn(THE_READERS_OWN, "design.invalid/", "DISH-4").as_bytes(),
        GIVING_UP,
    );
}
