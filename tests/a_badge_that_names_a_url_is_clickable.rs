//! A badge whose config names a `link` reaches the terminal as an OSC 8
//! hyperlink, so the reader clicks it instead of retyping the URL.
//!
//! Driven through the binary because the sequences are bytes on the wire and
//! nowhere else. `Fitted` writes them into a cell rather than into a span, and
//! a buffer test can say the cell holds them — but whether the backend prints
//! a symbol it did not measure, and prints it whole, is the terminal's answer
//! to give. The bytes are what a pane's outer terminal acts on.
//!
//! One run of bytes rather than a needle a repaint scatters: a repaint reaches
//! the wire a word at a time with a cursor move where each space would be, and
//! this badge says `⇢ #12` with a space in it. The whole link is one cell, so
//! the backend prints it with one write and the opening sequence, the words
//! and the closer are contiguous in the stream — which is the same fact the
//! screen rests on, read from the other side.

mod terminal;

use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::ShimmedTracker;
use terminal::{a_home_naming_one_project_settled, ENTER_ALTERNATE_SCREEN};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// A gap this long between bytes means the collection is over and drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// One tracker, whose root carries a reference to somewhere a reader can go.
const THE_TRACKER: &str = r#"[
  {"id":"orb-1","title":"lift the ground station","status":"open",
   "priority":1,"issue_type":"epic",
   "metadata":{"delivery_pr":"orbital/atlas#12"}}
]"#;

/// A badge on that key, drawn as a reference and pointing at the page the
/// reference stands for.
const A_BADGE_POINTING_SOMEWHERE: &str = concat!(
    "\n[[badges]]\n",
    "key    = \"delivery_pr\"\n",
    "match  = \"(?<owner>[^/]+)/(?<repo>[^#]+)#(?<number>[0-9]+)\"\n",
    "render = \"⇢ #{number}\"\n",
    "link   = \"https://forge.invalid/{owner}/{repo}/pull/{number}\"\n",
);

/// `a`, which shows every tree rather than only those with a live agent. No
/// pane sits in the temp `HOME`, so without it the one tree here sits behind
/// its project's *no live agent* line and draws no row of its own.
const SHOW_EVERY_TREE: &[u8] = b"a";

/// The escape that opens an operating-system command naming a hyperlink, and
/// the one that ends any such command. Written out rather than read off the
/// view, because what is under test is the bytes a terminal receives.
const OSC_8: &str = "\x1b]8;;";
const ST: &str = "\x1b\\";

#[test]
fn a_badge_that_names_a_url_reaches_the_terminal_as_a_hyperlink() {
    let home = a_home_naming_one_project_settled("clickable", A_BADGE_POINTING_SOMEWHERE);
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_TRACKER);

    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &tracker.environment());
    bdi.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);
    bdi.send(SHOW_EVERY_TREE);

    let clickable = format!(
        "{OSC_8}https://forge.invalid/orbital/atlas/pull/12{ST}⇢ #12{OSC_8}{ST}",
    );
    bdi.read_until(clickable.as_bytes(), GIVING_UP);
}
