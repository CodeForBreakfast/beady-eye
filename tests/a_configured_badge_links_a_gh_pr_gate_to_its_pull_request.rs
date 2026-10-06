//! A `gh:pr` gate is linked to its pull request by a `[[badges]]` entry
//! rather than by anything `bdi` knows about GitHub, so the entry the README
//! gives for it has to reach the terminal as a hyperlink.
//!
//! Driven through the binary for the reason `a_badge_that_names_a_url_is_clickable`
//! gives: the sequences are bytes on the wire and nowhere else.

mod terminal;

use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::ShimmedTracker;
use terminal::{
    a_home_naming_one_project_settled, contains, row_of, rows_drawn, rows_of,
    ENTER_ALTERNATE_SCREEN,
};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// A gap this long between bytes means the collection is over and drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// Three beads each blocked by a `gh:pr` gate, as bd 1.3.0 wrote them. One
/// gate names its repository.
const GH_PR_GATES: &str = include_str!("fixtures/bd_1.3.0_gh_pr_gates.json");

/// The entry the README gives for a `gh:pr` gate, word for word.
const THE_READMES_ENTRY: &str = concat!(
    "\n[[badges]]\n",
    "key    = \"metadata.repo\"\n",
    "when   = { await_type = \"gh:pr\", await_id = \"[0-9]+\" }\n",
    "match  = \"(?<owner>[A-Za-z0-9_.-]+)/(?<name>[A-Za-z0-9_.-]+)\"\n",
    "render = \"⇢ {name} #{await_id}\"\n",
    "short  = \"⇢ #{await_id}\"\n",
    "link   = \"https://github.com/{owner}/{name}/pull/{await_id}\"\n",
);

/// `a`, which shows every tree rather than only those with a live agent.
const SHOW_EVERY_TREE: &[u8] = b"a";

/// `E`, which opens every fold in the forest, so each gate is drawn under
/// the bead it blocks.
const EXPAND_THE_FOREST: &[u8] = b"E";

const OSC_8: &str = "\x1b]8;";
const ST: &str = "\x1b\\";

fn link_id(to: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    to.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

#[test]
fn the_readmes_entry_links_a_gh_pr_gate_to_its_pull_request() {
    let home = a_home_naming_one_project_settled("gated", THE_READMES_ENTRY);
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(GH_PR_GATES);

    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &tracker.environment());
    bdi.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);
    bdi.send(SHOW_EVERY_TREE);
    bdi.settle(A_SILENCE, GIVING_UP);
    bdi.send(EXPAND_THE_FOREST);

    let to = "https://github.com/dunwich/arkham/pull/12";
    let id = link_id(to);
    let clickable = format!("{OSC_8}id={id};{to}{ST}⇢ arkham #12{OSC_8};{ST}");
    bdi.read_until(clickable.as_bytes(), GIVING_UP);
}

/// `C`, which shuts every fold in the forest, so each gate is folded away
/// beneath the bead it blocks.
const COLLAPSE_THE_FOREST: &[u8] = b"C";

/// `l`, which opens the selected line's fold. The project's line is selected
/// once the forest is collapsed, and opening it draws its beads still shut.
const EXPAND: &[u8] = b"l";

#[test]
fn a_bead_shut_over_its_gate_draws_the_gates_pull_request_link_on_its_own_row() {
    let home = a_home_naming_one_project_settled("gated", THE_READMES_ENTRY);
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(GH_PR_GATES);

    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &tracker.environment());
    bdi.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);
    bdi.send(SHOW_EVERY_TREE);
    bdi.settle(A_SILENCE, GIVING_UP);
    bdi.send(COLLAPSE_THE_FOREST);
    bdi.settle(A_SILENCE, GIVING_UP);
    bdi.send(EXPAND);
    bdi.settle(A_SILENCE, GIVING_UP);
    let repainted = bdi.resize(ROWS, COLS + 1);
    let screen = bdi.answer_to(repainted, GIVING_UP);
    let frame = rows_drawn(&screen).join("\n");

    let waiting = row_of(&screen, b"dun-bz4")
        .unwrap_or_else(|| panic!("the bead the gate holds back is drawn once, on:\n{frame}"));
    assert_eq!(
        rows_of(&screen, b"dun-auk"),
        Vec::<u16>::new(),
        "the gate is folded away beneath the bead it blocks, on:\n{frame}"
    );
    assert_eq!(
        rows_of(&screen, "⇢ arkham #12".as_bytes()),
        vec![waiting],
        "the gate's badge is drawn on the row of the bead it blocks, on:\n{frame}"
    );
    let to = "https://github.com/dunwich/arkham/pull/12";
    let linked = format!("{OSC_8}id={};{to}{ST}", link_id(to));
    assert!(
        contains(&screen, linked.as_bytes()),
        "the badge keeps its link to the pull request, on:\n{frame}"
    );
}
