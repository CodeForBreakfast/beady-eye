//! Two badges on one key, and the one below never runs: the row reaches the
//! terminal carrying what the first badge to read the value drew, and nothing
//! the second would have drawn from the same value.
//!
//! Driven through the binary because a live config is what exercises this. The
//! model can be asked which badges a bead came to, but what a reader sees is a
//! row, and the choice between two entries on one key is invisible in a row
//! that carries one badge either way — only a screen holding one and not the
//! other says which entry was read.
//!
//! The bead's second key is the other half of the rule, and the half that used
//! to speak: a value no entry for its key reads draws nothing and says nothing.
//! The needle for that is the key's own name, because the sentence it used to
//! say named the key and nothing else on this screen does. Weakening the badge
//! into one that reads the value and then cannot fill its `link` puts the name
//! back on the screen, so the absence is the rule holding rather than the needle
//! being unreachable.
//!
//! Every needle is one word with no space in it. A repaint reaches the wire a
//! word at a time, so a needle spanning a space is one a cursor move can split
//! and a passing test can miss.

mod terminal;

use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::ShimmedTracker;
use terminal::{a_home_naming_one_project_settled, contains, ENTER_ALTERNATE_SCREEN};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// A gap this long between bytes means the collection is over and drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// One bead carrying two references. `delivery_pr` is written in the shape the
/// first badge on that key was written for, so the entry below it is the one
/// that must not run. `jira` is written in a shape no badge on its key reads,
/// which is the value that has to leave the row silent.
const THE_TRACKER: &str = r#"[
  {"id":"orb-1","title":"lift the ground station","status":"open",
   "priority":1,"issue_type":"epic"},
  {"id":"orb-1.1","title":"repoint the dish","status":"blocked","parent":"orb-1",
   "dependencies":[{"depends_on_id":"orb-1","type":"parent-child"}],
   "priority":2,"issue_type":"task",
   "metadata":{"delivery_pr":"orbital/atlas#12","jira":"a note to self"}}
]"#;

/// A key read from the shape expected down to the shape settled for, and a key
/// read in one shape only.
///
/// The permissive entry names no `match`, which is the most permissive form
/// there is, and renders the whole value — so what it would draw is the value
/// itself and there is nothing else on the screen it could be mistaken for.
///
/// The `jira` entry names a `link`, because a badge promising somewhere to go
/// is the one with the most to report if anything is still reporting.
const A_CHAIN_AND_A_LONE_ENTRY: &str = concat!(
    "\n[[badges]]\n",
    "key    = \"metadata.delivery_pr\"\n",
    "match  = \"(?<owner>[^/]+)/(?<repo>[^#]+)#(?<number>[0-9]+)\"\n",
    "render = \"⇢{repo}#{number}\"\n",
    "\n[[badges]]\n",
    "key    = \"metadata.delivery_pr\"\n",
    "render = \"{}\"\n",
    "\n[[badges]]\n",
    "key    = \"metadata.jira\"\n",
    "match  = \"(?<ticket>[A-Z]+-[0-9]+)\"\n",
    "render = \"{ticket}\"\n",
    "link   = \"https://jira.invalid/browse/{ticket}\"\n",
);

/// Every tree rather than only the staffed ones, then down onto the tree and
/// open it, so the bead carrying the badges is on the screen.
const OPEN_THE_TREE: &[u8] = b"agjlgj";

/// What the first entry for the key draws, which is the row arriving at all.
const THE_SHAPE_IT_WAS_WRITTEN_FOR: &[u8] = "⇢atlas#12".as_bytes();

/// What the entry below it would draw from the same value.
const THE_WHOLE_VALUE: &[u8] = b"orbital/atlas#12";

/// The key a value no badge reads belongs to, which is what the retired
/// sentence named.
const THE_UNREAD_KEY: &[u8] = b"metadata.jira";

#[test]
fn a_key_draws_the_first_badge_that_reads_it_and_stays_silent_where_none_does() {
    let home = a_home_naming_one_project_settled("fallthrough", A_CHAIN_AND_A_LONE_ENTRY);
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_TRACKER);

    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &tracker.environment());
    bdi.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);
    bdi.send(OPEN_THE_TREE);

    bdi.read_until(THE_SHAPE_IT_WAS_WRITTEN_FOR, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);

    let drawn = bdi.everything();
    assert!(
        !contains(&drawn, THE_WHOLE_VALUE),
        "the entry below the one that read the value drew as well"
    );
    assert!(
        !contains(&drawn, THE_UNREAD_KEY),
        "a value no badge on its key reads still reported"
    );
}
