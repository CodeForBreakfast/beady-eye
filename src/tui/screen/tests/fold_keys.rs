//! Every key that moves a fold, or moves the selection through the folds,
//! leaves the forest it leaves today — pinned over the layout and the fold
//! map that draw it.
//!
//! The assertion is the forest band: every row of it as text. Not one word of
//! a row, because the thing being pinned is which lines are drawn and in what
//! order, and a word says a line is there without saying what stands above or
//! below it.
//!
//! One invented tracker, with every kind of fold in it. A live pane on a
//! grandchild rests the root and the spine down to it open, a second root
//! nobody is on gives the filter a tree to hold back, three closed siblings
//! under the root make a run, and a blocker with a child of its own, hung
//! under the two beads it blocks as well as under its parent, is one bead on
//! three rows with its subtree under each copy. What is drawn of it once `S`
//! has put every copy of a bead on the spine, counting the rows of the screen
//! from the top:
//!
//! ```text
//! 0 ▾ arkham  ✓ 1s ago                                   3/10  1 agent
//! 1   ├── ○ ark-1    raise the beacon                            3/8
//! 2   │   ├── ○ .1       cast the bracket                        0/2
//! 3   │   │   └── ◐ .1       pour the iron          ◍ wT:p2 · working
//! 4   │   ├── ○ .2       glaze the lantern                       0/3
//! 5   │   │   └┄▸ ○ ark-1.1  cast the bracket  0/2  ◍ 1 agent beneath
//! 6   │   ├── ○ .3       mount the lens                          0/3
//! 7   │   │   └┄▸ ○ ark-1.1  cast the bracket  0/2  ◍ 1 agent beneath
//! 8   │   └─▸ ✓ 3 more finished beads with no agent
//! 9   └─▸ 1 tree with no live agent
//! ```
//!
//! A row's columns are where `view::fitted` puts them for the width the
//! screen happens to be, so a run of blanks is compared as one gap. Every
//! glyph, marker, id, title and note is compared as drawn.

use super::*;
use crate::collect::agents::testing::Fake as Herdr;
use crate::collect::bd::parse_beads;
use crate::collect::herdr::parse_agent_list;
use crate::collect::tracker::testing::{Fake, Fakes};
use crate::model::types::testing::A_SESSION;
use crate::tui::keys::tests::control;
use pretty_assertions::assert_eq;

const COLS: u16 = 120;

/// A screen `bdi` is drawn on: its height, and the most rows of it that are
/// the forest's — the rest are the tail's and the key bar's, by the
/// arithmetic in `view::draw::bands`.
struct Height {
    rows: u16,
    band: usize,
}

/// Tall enough that the whole forest is on screen however far it is opened.
const TALL: Height = Height { rows: 40, band: 32 };

/// Short enough that the forest opened by `E` does not fit, so a motion has
/// somewhere to scroll to.
const SHORT: Height = Height { rows: 13, band: 7 };

/// What `bd list --all --limit 0 --json` says about the tracker described at
/// the top of this file.
const THE_TRACKER: &str = include_str!("../../../../tests/fixtures/bd_every_fold_kind.json");

/// The pane on `ark-1.1.1`, which is what rests the root open and holds the
/// tree in front of the filter.
const THE_PANE: &str = "wT:p2";

/// The same tracker after a second seat has started on a bead under
/// `ark-1.3`, which was quiet until then, and the pane it sits in.
const THE_TRACKER_WITH_NEW_WORK: &str =
    include_str!("../../../../tests/fixtures/bd_every_fold_kind_and_new_work.json");
const THE_NEW_PANE: &str = "wT:p3";

/// The same tracker after the seat on `ark-1.1.1` has finished it and gone,
/// which leaves nothing under `ark-1.1` to rest it open.
const THE_TRACKER_WITH_FINISHED_WORK: &str =
    include_str!("../../../../tests/fixtures/bd_every_fold_kind_and_finished_work.json");

const EXPAND_THE_FOREST: KeyCode = KeyCode::Char('E');
const COLLAPSE_THE_FOREST: KeyCode = KeyCode::Char('C');
const RESTORE_THE_FOREST: KeyCode = KeyCode::Char('D');
const EXPAND_THE_SUBTREE: KeyCode = KeyCode::Char('e');
const COLLAPSE_THE_SUBTREE: KeyCode = KeyCode::Char('c');
const RESTORE_THE_SUBTREE: KeyCode = KeyCode::Char('d');
const OPEN_OR_STEP_IN: KeyCode = KeyCode::Char('l');
const SHUT_OR_STEP_OUT: KeyCode = KeyCode::Char('h');
const TOGGLE: KeyCode = KeyCode::Char(' ');
const DOWN: KeyCode = KeyCode::Char('j');
const UP: KeyCode = KeyCode::Char('k');
const FIRST_ROW: KeyCode = KeyCode::Char('g');
const LAST_ROW: KeyCode = KeyCode::Char('G');
const SHOW_EVERY_TREE: KeyCode = KeyCode::Char('a');
const NEXT_RULE_OVER_THE_FOREST: KeyCode = KeyCode::Char('S');

/// Rows of the first screen, counted from the top, that the tests select.
const RAISE_THE_KADATH: u16 = 1;
const GLAZE_THE_LANTERN: u16 = 4;
const THE_COPY_UNDER_IT: u16 = 5;
const MOUNT_THE_LENS: u16 = 6;

/// The forest as a fresh `bdi` draws it, under the deepest rule: one copy of
/// the blocker is opened to and the other two rest shut over the same work.
const FRESH: &[&str] = &[
    "▾ arkham  ✓ 0s ago  3/10  1 agent",
    "  ├── ○ ark-1  raise the beacon  3/8",
    "  │   ├─▸ ○ .1  cast the bracket  0/2  ◍ 1 agent beneath",
    "  │   ├── ○ .2  glaze the lantern  0/3",
    "  │   │   └┄┄ ○ ark-1.1  cast the bracket  0/2",
    "  │   │       └── ◐ .1  pour the iron  ◍ wT:p2 · working",
    "  │   ├─▸ ○ .3  mount the lens  0/3  ◍ 1 agent beneath",
    "  │   └─▸ ✓ 3 more finished beads with no agent",
    "  └─▸ 1 tree with no live agent",
];

/// The forest at rest once every copy of a bead is on the spine, in the
/// words the assertions use.
const AT_REST: &[&str] = &[
    "▾ arkham  ✓ 0s ago  3/10  1 agent",
    "  ├── ○ ark-1  raise the beacon  3/8",
    "  │   ├── ○ .1  cast the bracket  0/2",
    "  │   │   └── ◐ .1  pour the iron  ◍ wT:p2 · working",
    "  │   ├── ○ .2  glaze the lantern  0/3",
    "  │   │   └┄▸ ○ ark-1.1  cast the bracket  0/2  ◍ 1 agent beneath",
    "  │   ├── ○ .3  mount the lens  0/3",
    "  │   │   └┄▸ ○ ark-1.1  cast the bracket  0/2  ◍ 1 agent beneath",
    "  │   └─▸ ✓ 3 more finished beads with no agent",
    "  └─▸ 1 tree with no live agent",
];

/// The forest with every fold open: the run's members, the subtree under each
/// copy of the blocker, and the tree the filter was holding back.
const EVERYTHING_OPEN: &[&str] = &[
    "▾ arkham  ✓ 0s ago  3/10  1 agent",
    "  ├── ○ ark-1  raise the beacon  3/8",
    "  │   ├── ○ .1  cast the bracket  0/2",
    "  │   │   └── ◐ .1  pour the iron  ◍ wT:p2 · working",
    "  │   ├── ○ .2  glaze the lantern  0/3",
    "  │   │   └┄┄ ○ ark-1.1  cast the bracket  0/2",
    "  │   │       └── ◐ .1  pour the iron  ◍ wT:p2 · working",
    "  │   ├── ○ .3  mount the lens  0/3",
    "  │   │   └┄┄ ○ ark-1.1  cast the bracket  0/2",
    "  │   │       └── ◐ .1  pour the iron  ◍ wT:p2 · working",
    "  │   └── ✓ 3 more finished beads with no agent",
    "  │       ├── ✓ .4  survey the headland",
    "  │       ├── ✓ .5  draw up the plans",
    "  │       └── ✓ .6  clear the site",
    "  └── 1 tree with no live agent",
    "      └── ○ ark-2  dredge the harbour  0/2",
    "          └── ○ .1  survey the silt",
];

/// The forest with every fold shut, which is the project's line alone.
const EVERYTHING_SHUT: &[&str] = &["▸ arkham  ✓ 0s ago  3/10  1 agent"];

/// The forest with `glaze the lantern` shut by hand and nothing else moved.
const GLAZE_THE_LANTERN_SHUT: &[&str] = &[
    "▾ arkham  ✓ 0s ago  3/10  1 agent",
    "  ├── ○ ark-1  raise the beacon  3/8",
    "  │   ├── ○ .1  cast the bracket  0/2",
    "  │   │   └── ◐ .1  pour the iron  ◍ wT:p2 · working",
    "  │   ├─▸ ○ .2  glaze the lantern  0/3  ◍ 1 agent beneath",
    "  │   ├── ○ .3  mount the lens  0/3",
    "  │   │   └┄▸ ○ ark-1.1  cast the bracket  0/2  ◍ 1 agent beneath",
    "  │   └─▸ ✓ 3 more finished beads with no agent",
    "  └─▸ 1 tree with no live agent",
];

/// The forest with the copy of the blocker under `glaze the lantern` open,
/// and nothing else moved.
const THE_COPY_OPEN: &[&str] = &[
    "▾ arkham  ✓ 0s ago  3/10  1 agent",
    "  ├── ○ ark-1  raise the beacon  3/8",
    "  │   ├── ○ .1  cast the bracket  0/2",
    "  │   │   └── ◐ .1  pour the iron  ◍ wT:p2 · working",
    "  │   ├── ○ .2  glaze the lantern  0/3",
    "  │   │   └┄┄ ○ ark-1.1  cast the bracket  0/2",
    "  │   │       └── ◐ .1  pour the iron  ◍ wT:p2 · working",
    "  │   ├── ○ .3  mount the lens  0/3",
    "  │   │   └┄▸ ○ ark-1.1  cast the bracket  0/2  ◍ 1 agent beneath",
    "  │   └─▸ ✓ 3 more finished beads with no agent",
    "  └─▸ 1 tree with no live agent",
];

/// A fresh `bdi` opens each bead's deepest copy before the reader presses
/// anything, and one `S` puts every copy on the spine.
#[test]
fn a_fresh_bdi_opens_each_beads_deepest_copy() {
    let mut shown = launched();
    assert_eq!(forest(&mut shown, &TALL), FRESH);

    press(&mut shown, NEXT_RULE_OVER_THE_FOREST);
    assert_eq!(forest(&mut shown, &TALL), AT_REST);
}

/// `E` opens every fold there is, `C` shuts every fold there is, and `D`
/// hands every one of them back to where it rests.
#[test]
fn e_and_c_fold_the_whole_forest_and_d_puts_it_back() {
    let mut shown = over_every_fold_kind();
    assert_eq!(forest(&mut shown, &TALL), AT_REST);

    press(&mut shown, EXPAND_THE_FOREST);
    assert_eq!(forest(&mut shown, &TALL), EVERYTHING_OPEN);

    press(&mut shown, COLLAPSE_THE_FOREST);
    assert_eq!(forest(&mut shown, &TALL), EVERYTHING_SHUT);

    press(&mut shown, RESTORE_THE_FOREST);
    assert_eq!(forest(&mut shown, &TALL), AT_REST);
}

/// `e`, `c` and `d` do the same to the selected node and what hangs under it,
/// and to nothing else. The node is `glaze the lantern`, whose one child is
/// the second copy of the blocker: `e` opens the copy as well, `c` shuts the
/// node over both, and `d` rests the node open and the copy shut.
#[test]
fn e_c_and_d_fold_the_subtree_under_the_selection() {
    let mut shown = over_every_fold_kind();
    clicked_on(&mut shown, &TALL, GLAZE_THE_LANTERN);

    press(&mut shown, EXPAND_THE_SUBTREE);
    assert_eq!(forest(&mut shown, &TALL), THE_COPY_OPEN);

    press(&mut shown, COLLAPSE_THE_SUBTREE);
    assert_eq!(forest(&mut shown, &TALL), GLAZE_THE_LANTERN_SHUT);

    press(&mut shown, RESTORE_THE_SUBTREE);
    assert_eq!(forest(&mut shown, &TALL), AT_REST);
}

/// `l` opens a shut node and steps into an open one; `h` shuts an open node
/// and steps out of a shut one; Space turns the fold whichever way it is not
/// pointing. All on one row, the second copy of the blocker.
#[test]
fn l_h_and_space_move_one_fold_and_the_selection_through_it() {
    let mut shown = over_every_fold_kind();
    clicked_on(&mut shown, &TALL, THE_COPY_UNDER_IT);

    press(&mut shown, OPEN_OR_STEP_IN);
    assert_eq!(forest(&mut shown, &TALL), THE_COPY_OPEN);

    press(&mut shown, OPEN_OR_STEP_IN);
    assert_eq!(forest(&mut shown, &TALL), THE_COPY_OPEN);
    assert_eq!(selected(&shown), "ark-1.1.1");

    press(&mut shown, SHUT_OR_STEP_OUT);
    assert_eq!(forest(&mut shown, &TALL), THE_COPY_OPEN);
    assert_eq!(selected(&shown), "ark-1.1");

    press(&mut shown, SHUT_OR_STEP_OUT);
    assert_eq!(forest(&mut shown, &TALL), AT_REST);

    press(&mut shown, TOGGLE);
    assert_eq!(forest(&mut shown, &TALL), THE_COPY_OPEN);

    press(&mut shown, TOGGLE);
    assert_eq!(forest(&mut shown, &TALL), AT_REST);
}

/// The motions walk the opened forest row by row, and the view follows the
/// selection by the least it can: down to the end, back up by half a band
/// until the selection is above the view and the view moves to it, and back
/// to the top.
#[test]
fn the_motions_walk_the_opened_forest_and_scroll_the_view_after_it() {
    let mut shown = over_every_fold_kind();

    press(&mut shown, EXPAND_THE_FOREST);
    assert_eq!(forest(&mut shown, &SHORT), &EVERYTHING_OPEN[..7]);

    press(&mut shown, DOWN);
    assert_eq!(forest(&mut shown, &SHORT), &EVERYTHING_OPEN[..7]);
    assert_eq!(selected(&shown), "ark-1");

    press(&mut shown, LAST_ROW);
    assert_eq!(forest(&mut shown, &SHORT), &EVERYTHING_OPEN[10..]);
    assert_eq!(selected(&shown), "ark-2.1");

    press(&mut shown, UP);
    assert_eq!(forest(&mut shown, &SHORT), &EVERYTHING_OPEN[10..]);
    assert_eq!(selected(&shown), "ark-2");

    half_screen_up(&mut shown);
    assert_eq!(forest(&mut shown, &SHORT), &EVERYTHING_OPEN[10..]);
    assert_eq!(selected(&shown), "ark-1.5");

    half_screen_up(&mut shown);
    assert_eq!(forest(&mut shown, &SHORT), &EVERYTHING_OPEN[9..16]);
    assert_eq!(selected(&shown), "ark-1.1.1");

    half_screen_down(&mut shown);
    assert_eq!(forest(&mut shown, &SHORT), &EVERYTHING_OPEN[9..16]);
    assert_eq!(selected(&shown), "ark-1.5");

    // The first row is the project's, and a project has no bead to name it,
    // so the row below says where `g` went.
    press(&mut shown, FIRST_ROW);
    assert_eq!(forest(&mut shown, &SHORT), &EVERYTHING_OPEN[..7]);
    press(&mut shown, DOWN);
    assert_eq!(selected(&shown), "ark-1");
}

/// `a` draws the tree the filter was holding back among the others, resting
/// shut, and takes the line that stood for it away; the fold the reader set
/// by hand stays where they set it, through the change and back.
#[test]
fn a_moves_the_hidden_tree_and_leaves_the_folds_alone() {
    let mut shown = over_every_fold_kind();
    clicked_on(&mut shown, &TALL, GLAZE_THE_LANTERN);
    press(&mut shown, COLLAPSE_THE_SUBTREE);
    assert_eq!(forest(&mut shown, &TALL), GLAZE_THE_LANTERN_SHUT);

    press(&mut shown, SHOW_EVERY_TREE);
    assert_eq!(
        forest(&mut shown, &TALL),
        &[
            "▾ arkham  ✓ 0s ago  3/10  1 agent",
            "  ├── ○ ark-1  raise the beacon  3/8",
            "  │   ├── ○ .1  cast the bracket  0/2",
            "  │   │   └── ◐ .1  pour the iron  ◍ wT:p2 · working",
            "  │   ├─▸ ○ .2  glaze the lantern  0/3  ◍ 1 agent beneath",
            "  │   ├── ○ .3  mount the lens  0/3",
            "  │   │   └┄▸ ○ ark-1.1  cast the bracket  0/2  ◍ 1 agent beneath",
            "  │   └─▸ ✓ 3 more finished beads with no agent",
            "  └─▸ ○ ark-2  dredge the harbour  0/2",
        ]
    );

    press(&mut shown, SHOW_EVERY_TREE);
    assert_eq!(forest(&mut shown, &TALL), GLAZE_THE_LANTERN_SHUT);
}

/// A collection that finds the same work under a fold the reader shut leaves
/// it shut.
#[test]
fn a_refresh_keeps_a_fold_shut_over_work_that_has_not_changed() {
    let mut shown = over_every_fold_kind();
    clicked_on(&mut shown, &TALL, GLAZE_THE_LANTERN);
    press(&mut shown, COLLAPSE_THE_SUBTREE);
    assert_eq!(forest(&mut shown, &TALL), GLAZE_THE_LANTERN_SHUT);

    refresh(&mut shown, THE_TRACKER, &[THE_PANE]);
    assert_eq!(forest(&mut shown, &TALL), GLAZE_THE_LANTERN_SHUT);
}

/// A collection that finds live work under a fold the reader shut, that was
/// not there when they shut it, lets the fold go: the node rests open again,
/// onto the new work.
#[test]
fn a_refresh_lets_a_fold_go_when_live_work_arrives_under_it() {
    let mut shown = over_every_fold_kind();
    clicked_on(&mut shown, &TALL, MOUNT_THE_LENS);
    press(&mut shown, COLLAPSE_THE_SUBTREE);
    assert_eq!(
        forest(&mut shown, &TALL),
        &[
            "▾ arkham  ✓ 0s ago  3/10  1 agent",
            "  ├── ○ ark-1  raise the beacon  3/8",
            "  │   ├── ○ .1  cast the bracket  0/2",
            "  │   │   └── ◐ .1  pour the iron  ◍ wT:p2 · working",
            "  │   ├── ○ .2  glaze the lantern  0/3",
            "  │   │   └┄▸ ○ ark-1.1  cast the bracket  0/2  ◍ 1 agent beneath",
            "  │   ├─▸ ○ .3  mount the lens  0/3  ◍ 1 agent beneath",
            "  │   └─▸ ✓ 3 more finished beads with no agent",
            "  └─▸ 1 tree with no live agent",
        ]
    );

    refresh(
        &mut shown,
        THE_TRACKER_WITH_NEW_WORK,
        &[THE_PANE, THE_NEW_PANE],
    );
    assert_eq!(
        forest(&mut shown, &TALL),
        &[
            "▾ arkham  ✓ 0s ago  3/11  2 agents",
            "  ├── ○ ark-1  raise the beacon  3/9",
            "  │   ├── ○ .1  cast the bracket  0/2",
            "  │   │   └── ◐ .1  pour the iron  ◍ wT:p2 · working",
            "  │   ├── ○ .2  glaze the lantern  0/3",
            "  │   │   └┄▸ ○ ark-1.1  cast the bracket  0/2  ◍ 1 agent beneath",
            "  │   ├── ○ .3  mount the lens  0/4",
            "  │   │   ├── ◐ .1  grind the glass  ◍ wT:p3 · working",
            "  │   │   └┄▸ ○ ark-1.1  cast the bracket  0/2  ◍ 1 agent beneath",
            "  │   └─▸ ✓ 3 more finished beads with no agent",
            "  └─▸ 1 tree with no live agent",
        ]
    );
}

/// `e` holds every fold under it open as the key set it, the ones it found
/// resting open included. `cast the bracket` rested open onto the seat on
/// `pour the iron`; once that bead is finished and the seat gone, the fold
/// would rest shut, and stays open under the `e` instead. Every tree is
/// shown first, so that the tree is still drawn once nobody is on it.
#[test]
fn a_refresh_keeps_a_fold_e_found_resting_open_open_when_the_work_under_it_finishes() {
    let mut shown = over_every_fold_kind();
    press(&mut shown, SHOW_EVERY_TREE);
    clicked_on(&mut shown, &TALL, RAISE_THE_KADATH);
    press(&mut shown, EXPAND_THE_SUBTREE);
    assert_eq!(
        forest(&mut shown, &TALL),
        &[
            "▾ arkham  ✓ 0s ago  3/10  1 agent",
            "  ├── ○ ark-1  raise the beacon  3/8",
            "  │   ├── ○ .1  cast the bracket  0/2",
            "  │   │   └── ◐ .1  pour the iron  ◍ wT:p2 · working",
            "  │   ├── ○ .2  glaze the lantern  0/3",
            "  │   │   └┄┄ ○ ark-1.1  cast the bracket  0/2",
            "  │   │       └── ◐ .1  pour the iron  ◍ wT:p2 · working",
            "  │   ├── ○ .3  mount the lens  0/3",
            "  │   │   └┄┄ ○ ark-1.1  cast the bracket  0/2",
            "  │   │       └── ◐ .1  pour the iron  ◍ wT:p2 · working",
            "  │   └── ✓ 3 more finished beads with no agent",
            "  │       ├── ✓ .4  survey the headland",
            "  │       ├── ✓ .5  draw up the plans",
            "  │       └── ✓ .6  clear the site",
            "  └─▸ ○ ark-2  dredge the harbour  0/2",
        ]
    );

    refresh(&mut shown, THE_TRACKER_WITH_FINISHED_WORK, &[]);
    assert_eq!(
        forest(&mut shown, &TALL),
        &[
            "▾ arkham  ✓ 0s ago  4/10",
            "  ├── ○ ark-1  raise the beacon  4/8",
            "  │   ├── ○ .1  cast the bracket  1/2",
            "  │   │   └── ✓ .1  pour the iron",
            "  │   ├── ○ .2  glaze the lantern  1/3",
            "  │   │   └┄┄ ○ ark-1.1  cast the bracket  1/2",
            "  │   │       └── ✓ .1  pour the iron",
            "  │   ├── ○ .3  mount the lens  1/3",
            "  │   │   └┄┄ ○ ark-1.1  cast the bracket  1/2",
            "  │   │       └── ✓ .1  pour the iron",
            "  │   └── ✓ 3 more finished beads with no agent",
            "  │       ├── ✓ .4  survey the headland",
            "  │       ├── ✓ .5  draw up the plans",
            "  │       └── ✓ .6  clear the site",
            "  └─▸ ○ ark-2  dredge the harbour  0/2",
        ]
    );
}

/// A search for a bead under a shut fold opens what stands over it — its
/// forebears in its tree, any run the way down to it goes through, and the
/// project — and no other fold. The run under the root counts closed
/// siblings the way down passes by, so it stays shut.
#[test]
fn a_search_opens_the_folds_over_the_bead_it_lands_on() {
    let mut shown = over_every_fold_kind();
    press(&mut shown, COLLAPSE_THE_FOREST);
    assert_eq!(forest(&mut shown, &TALL), EVERYTHING_SHUT);

    press(&mut shown, KeyCode::Char('/'));
    type_into_prompt(&mut shown, "ark-1.1.1");
    at_prompt(&mut shown, Typing::Sought);
    assert_eq!(
        forest(&mut shown, &TALL),
        &[
            "▾ arkham  ✓ 0s ago  3/10  1 agent",
            "  ├── ○ ark-1  raise the beacon  3/8",
            "  │   ├── ○ .1  cast the bracket  0/2",
            "  │   │   └── ◐ .1  pour the iron  ◍ wT:p2 · working",
            "  │   ├─▸ ○ .2  glaze the lantern  0/3  ◍ 1 agent beneath",
            "  │   ├─▸ ○ .3  mount the lens  0/3  ◍ 1 agent beneath",
            "  │   └─▸ ✓ 3 more finished beads with no agent",
            "  └─▸ 1 tree with no live agent",
        ]
    );
    assert_eq!(selected(&shown), "ark-1.1.1");
}

/// A click on a row of the opened forest selects the line drawn there, which
/// after `E` is a line the first screen never drew.
#[test]
fn a_click_after_e_selects_the_row_it_lands_on() {
    let mut shown = over_every_fold_kind();
    press(&mut shown, EXPAND_THE_FOREST);
    assert_eq!(forest(&mut shown, &TALL), EVERYTHING_OPEN);

    let survey_the_silt = 16;
    clicked_on(&mut shown, &TALL, survey_the_silt);
    assert_eq!(forest(&mut shown, &TALL), EVERYTHING_OPEN);
    assert_eq!(selected(&shown), "ark-2.1");
}

/// What the collection after a `^R` finds: this tracker, with these panes
/// working in the project's directory.
fn collected(tracker: &str, panes: &[&str]) -> Snapshot {
    let cfg = Config::from_toml(&format!(
        "[[projects]]\nname = \"arkham\"\npath = \"{ARKHAM}\"\n"
    ))
    .expect("the config parses");
    let trackers = Fakes::default().with(
        "arkham",
        Fake::holding(parse_beads(tracker).expect("the fixture parses")),
    );
    let agents =
        Herdr::holding(parse_agent_list(A_SESSION, &agents_listed(panes)).expect("parses"));
    crate::app::run(&cfg, &agents, &trackers, Filter::LiveAgents, an_instant())
}

/// The project's directory, which is what ties a pane to the project.
const ARKHAM: &str = "/srv/work/arkham";

/// What `herdr agent list` answers: these panes, every one working in the
/// project's directory.
fn agents_listed(panes: &[&str]) -> String {
    let agents: Vec<String> = panes
        .iter()
        .map(|pane| format!(r#"{{"pane_id":"{pane}","cwd":"{ARKHAM}","agent_status":"working"}}"#))
        .collect();
    format!(r#"{{"result":{{"agents":[{}]}}}}"#, agents.join(","))
}

/// A `bdi` over the tracker at the top of this file, with the selection where
/// a run leaves it: on the project's line. A run opens on a screen that is
/// still waiting for its first collection, and the collection arrives under it.
fn launched() -> Shown {
    let awaiting = Snapshot::awaiting(
        vec!["arkham".to_string()],
        Vec::new(),
        "herdr",
        Scope::default(),
        Filter::LiveAgents,
        an_instant(),
    );
    let mut shown = shown(awaiting);
    shown.collected(collected(THE_TRACKER, &[THE_PANE]));
    shown
}

/// A `launched` `bdi` with every copy of a bead on the spine: the screen the
/// fold keys are pinned against.
fn over_every_fold_kind() -> Shown {
    let mut shown = launched();
    press(&mut shown, NEXT_RULE_OVER_THE_FOREST);
    shown
}

/// `^R`: the key is the one the table binds to a refresh, and the collection
/// it asks for finds `tracker` and `panes`.
fn refresh(shown: &mut Shown, tracker: &str, panes: &[&str]) {
    assert_eq!(action(control('r')), Some(Action::Refresh));
    shown.collected(collected(tracker, panes));
}

fn half_screen_down(shown: &mut Shown) {
    apply(shown, control('d'));
}

fn half_screen_up(shown: &mut Shown) {
    apply(shown, control('u'));
}

fn apply(shown: &mut Shown, key: ratatui::crossterm::event::KeyEvent) {
    let asked = action(key).unwrap_or_else(|| panic!("{key:?} is bound to nothing"));
    shown.apply(asked);
}

/// A click on one row of the screen, counted from the top.
fn clicked_on(shown: &mut Shown, height: &Height, row: u16) {
    forest_band(shown, COLS, height.rows);
    shown.clicked(Rect::new(0, 0, COLS, height.rows), row);
}

/// The forest band as it stands, one string per row, in the words the
/// assertions use: a row's blanks squeezed to one gap, and the rows below the
/// last drawn one dropped.
#[track_caller]
fn forest(shown: &mut Shown, height: &Height) -> Vec<String> {
    let mut rows: Vec<String> = forest_band(shown, COLS, height.rows)
        .iter()
        .take(height.band)
        .map(|row| squeezed(row.trim_end()))
        .collect();
    while rows.last().is_some_and(|row| row.is_empty()) {
        rows.pop();
    }
    rows
}

/// The id of the bead the selection is on.
#[track_caller]
fn selected(shown: &Shown) -> &str {
    &cursor(shown)
        .expect("the selection is on a bead and not on a project's line")
        .id
}

/// Every run of blanks past the box-drawing as two, so a row says what is
/// on it and not where `view::fitted` put each cell for this width. The
/// box-drawing stays as drawn: its blanks are where a line hangs, which is
/// the fold's to say.
fn squeezed(row: &str) -> String {
    let prefix = row
        .find(|glyph: char| !" │├└─┄▸▾".contains(glyph))
        .unwrap_or(row.len());
    let mut said = row[..prefix].to_string();
    let mut blanks = 0;
    for glyph in row[prefix..].chars() {
        if glyph == ' ' {
            blanks += 1;
            continue;
        }
        if blanks > 0 {
            said.push_str(if blanks > 1 { "  " } else { " " });
            blanks = 0;
        }
        said.push(glyph);
    }
    said
}
