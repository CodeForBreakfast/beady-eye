//! A tracker holding rows bd wrote while it spelled `metadata` as a string
//! draws its beads, in the binary a reader actually runs.
//!
//! A tracker is read whole, so what a row bdi refuses costs is not one field
//! on one bead but every bead in that project — the screen the reporter of
//! GitHub issue #37 met was a project with nothing under it. That is why this
//! is asserted at the screen rather than at the parser: the parser answering
//! is only half of the claim, and the half a reader never sees.
//!
//! The capture holds one row of every shape the parser used to refuse, and
//! every row is a loose root, so the forest draws each one under the whole of
//! its id rather than under the part that is its own.

mod terminal;

use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::ShimmedTracker;
use terminal::{a_home_naming_one_project_settled, contains, ENTER_ALTERNATE_SCREEN};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// A gap this long between bytes means the collection is over and drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// How long a keystroke gets before waiting for it is called stalling.
const LONG_ENOUGH_TO_ANSWER: Duration = Duration::from_secs(10);

/// What `bd list --all --json` says about a tracker written before the
/// cutover.
const THE_TRACKER: &str = include_str!("fixtures/bd_before_the_metadata_cutover.json");

/// Every bead that capture holds, by the id its row is drawn under.
const EVERY_BEAD: [&str; 6] = ["orb-v1", "orb-v2", "orb-v3", "orb-v4", "orb-v5", "orb-v6"];

/// A badge on the key one row's metadata spells from inside a string.
const A_BADGE_ON_PHASE: &str = "\n[[badges]]\nkey = \"metadata.phase\"\nrender = \"{}\"\n";

/// What that row holds under `phase`. Nothing else on the screen says it, and
/// nothing says it at all unless the string was read as the object it spells
/// rather than merely tolerated.
const ITS_PHASE: &[u8] = "vacuum-soak".as_bytes();

/// `a`, which shows every tree rather than only those with a live agent. No
/// pane sits in the temp `HOME`, so without it these trees sit behind their
/// project's *no live agent* line and draw no rows of their own.
const SHOW_EVERY_TREE: &[u8] = b"a";

#[test]
fn a_tracker_written_before_the_metadata_cutover_draws_its_beads() {
    let home = a_home_naming_one_project_settled("pre-cutover", A_BADGE_ON_PHASE);
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_TRACKER);
    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &tracker.environment());
    bdi.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);

    assert_eq!(
        tracker.unanswered(),
        Vec::<String>::new(),
        "bd was asked something the shim had no answer for, so the real bd \
         answered instead and the project was read as having no tracker"
    );

    bdi.send(SHOW_EVERY_TREE);
    bdi.settle(A_SILENCE, LONG_ENOUGH_TO_ANSWER);
    let repainted = bdi.resize(ROWS + 1, COLS);
    let screen = bdi.answer_to(repainted, LONG_ENOUGH_TO_ANSWER);

    for bead in EVERY_BEAD {
        assert!(
            contains(&screen, bead.as_bytes()),
            "{bead} is not on the screen, so the row bd wrote it as still \
             costs the whole tracker. The screen bdi drew: {:?}\n{}",
            String::from_utf8_lossy(&screen),
            bdi.timeline()
        );
    }

    assert!(
        contains(&screen, ITS_PHASE),
        "the badge on a key spelled inside the string is not drawn, so the \
         string was tolerated rather than read as the object it spells. The \
         screen bdi drew: {:?}\n{}",
        String::from_utf8_lossy(&screen),
        bdi.timeline()
    );
}
