//! Every key that moves a fold, or moves the selection through the folds,
//! leaves the forest it leaves today — pinned from outside, through the
//! binary, before the layout and the fold map are rebuilt underneath it.
//!
//! The assertion is the forest band: every row of it as text, read off a
//! frame `bdi` was made to repaint whole. Not one word of a row, because the
//! thing being pinned is which lines are drawn and in what order, and a word
//! says a line is there without saying what stands above or below it.
//!
//! One invented tracker, with every kind of fold in it. A live pane on a
//! grandchild rests the root and the spine down to it open, a second root
//! nobody is on gives the filter a tree to hold back, three closed siblings
//! under the root make a run, and a blocker with a child of its own, hung
//! under the two beads it blocks as well as under its parent, is one bead on
//! three rows with its subtree under each copy. What the binary draws of it
//! before any key is pressed, counting the rows of the screen from the top:
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
//! 8   │   └─▸ ✓ 3 more beads · finished, and nobody on them
//! 9   └─▸ 1 tree with no live agent
//! ```
//!
//! Two things on a row are not the fold's, and are taken out before a row is
//! compared. The project's line says how long ago it was read, which is a
//! clock; and a row's columns are where `view::fitted` puts them for the
//! width the screen happens to be, so a run of blanks is compared as one gap.
//! Every glyph, marker, id, title and note is compared as drawn.
//!
//! Where the selection went is read off the bead window, as every test
//! about the selection reads it: the window is drawn from the selection and
//! titled with the selected bead's id, and nothing on a row says whether it
//! is the selected one.

mod terminal;

use std::path::PathBuf;
use std::time::Duration;

use pretty_assertions::assert_eq;
use terminal::driver::{clicked_on, Driven, GIVING_UP};
use terminal::shims::{ShimmedHerdr, ShimmedTracker};
use terminal::{a_socket_of_its_own, rows_drawn, window_over, ENTER_ALTERNATE_SCREEN};

const COLS: u16 = 120;

/// A screen `bdi` is drawn on: its height, and how many rows of it are the
/// forest's — the rest are the tail's and the key bar's, by the arithmetic
/// in `view::draw::bands`.
struct Screen {
    rows: u16,
    band: usize,
}

/// Tall enough that the whole forest is on screen however far it is opened.
const TALL: Screen = Screen { rows: 40, band: 32 };

/// Short enough that the forest opened by `E` does not fit, so a motion has
/// somewhere to scroll to. Thirteen rows and fourteen draw the same
/// seven-row band, so the resize that asks for a repaint does not move the
/// rows it is asked about.
const SHORT: Screen = Screen { rows: 13, band: 7 };

/// A gap this long between bytes means the frame is drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// What `bd list --all --limit 0 --json` says about the tracker described at
/// the top of this file.
const THE_TRACKER: &str = include_str!("fixtures/bd_every_fold_kind.json");

/// The pane on `ark-1.1.1`, which is what rests the root open and holds the
/// tree in front of the filter.
const THE_PANE: &str = "wT:p2";

/// The same tracker after a second seat has started on a bead under
/// `ark-1.3`, which was quiet until then, and the pane it sits in.
const THE_TRACKER_WITH_NEW_WORK: &str =
    include_str!("fixtures/bd_every_fold_kind_and_new_work.json");
const THE_NEW_PANE: &str = "wT:p3";

/// The same tracker after the seat on `ark-1.1.1` has finished it and gone,
/// which leaves nothing under `ark-1.1` to rest it open.
const THE_TRACKER_WITH_FINISHED_WORK: &str =
    include_str!("fixtures/bd_every_fold_kind_and_finished_work.json");

const EXPAND_THE_FOREST: &[u8] = b"E";
const COLLAPSE_THE_FOREST: &[u8] = b"C";
const RESTORE_THE_FOREST: &[u8] = b"D";
const EXPAND_THE_SUBTREE: &[u8] = b"e";
const COLLAPSE_THE_SUBTREE: &[u8] = b"c";
const RESTORE_THE_SUBTREE: &[u8] = b"d";
const OPEN_OR_STEP_IN: &[u8] = b"l";
const SHUT_OR_STEP_OUT: &[u8] = b"h";
const TOGGLE: &[u8] = b" ";
const DOWN: &[u8] = b"j";
const UP: &[u8] = b"k";
const FIRST_ROW: &[u8] = b"g";
const LAST_ROW: &[u8] = b"G";
const HALF_SCREEN_DOWN: &[u8] = b"\x04";
const HALF_SCREEN_UP: &[u8] = b"\x15";
const SHOW_EVERY_TREE: &[u8] = b"a";
const REFRESH: &[u8] = b"\x12";
const SHOW_IT: &[u8] = b"\r";
const BACK: &[u8] = b"\x1b";

/// Rows of the first screen, counted from the top, that the tests select.
const RAISE_THE_KADATH: u16 = 1;
const GLAZE_THE_LANTERN: u16 = 4;
const THE_COPY_UNDER_IT: u16 = 5;
const MOUNT_THE_LENS: u16 = 6;

/// The forest as the first screen draws it, in the words the assertions use.
const AT_REST: &[&str] = &[
    "▾ arkham  ✓ <age> ago  3/10  1 agent",
    "  ├── ○ ark-1  raise the beacon  3/8",
    "  │   ├── ○ .1  cast the bracket  0/2",
    "  │   │   └── ◐ .1  pour the iron  ◍ wT:p2 · working",
    "  │   ├── ○ .2  glaze the lantern  0/3",
    "  │   │   └┄▸ ○ ark-1.1  cast the bracket  0/2  ◍ 1 agent beneath",
    "  │   ├── ○ .3  mount the lens  0/3",
    "  │   │   └┄▸ ○ ark-1.1  cast the bracket  0/2  ◍ 1 agent beneath",
    "  │   └─▸ ✓ 3 more beads · finished, and nobody on them",
    "  └─▸ 1 tree with no live agent",
];

/// The forest with every fold open: the run's members, the subtree under each
/// copy of the blocker, and the tree the filter was holding back.
const EVERYTHING_OPEN: &[&str] = &[
    "▾ arkham  ✓ <age> ago  3/10  1 agent",
    "  ├── ○ ark-1  raise the beacon  3/8",
    "  │   ├── ○ .1  cast the bracket  0/2",
    "  │   │   └── ◐ .1  pour the iron  ◍ wT:p2 · working",
    "  │   ├── ○ .2  glaze the lantern  0/3",
    "  │   │   └┄┄ ○ ark-1.1  cast the bracket  0/2",
    "  │   │       └── ◐ .1  pour the iron  ◍ wT:p2 · working",
    "  │   ├── ○ .3  mount the lens  0/3",
    "  │   │   └┄┄ ○ ark-1.1  cast the bracket  0/2",
    "  │   │       └── ◐ .1  pour the iron  ◍ wT:p2 · working",
    "  │   └── ✓ 3 more beads · finished, and nobody on them",
    "  │       ├── ✓ .4  survey the headland",
    "  │       ├── ✓ .5  draw up the plans",
    "  │       └── ✓ .6  clear the site",
    "  └── 1 tree with no live agent",
    "      └── ○ ark-2  dredge the harbour  0/2",
    "          └── ○ .1  survey the silt",
];

/// The forest with every fold shut, which is the project's line alone.
const EVERYTHING_SHUT: &[&str] = &["▸ arkham  ✓ <age> ago  3/10  1 agent"];

/// The forest with `glaze the lantern` shut by hand and nothing else moved.
const GLAZE_THE_LANTERN_SHUT: &[&str] = &[
    "▾ arkham  ✓ <age> ago  3/10  1 agent",
    "  ├── ○ ark-1  raise the beacon  3/8",
    "  │   ├── ○ .1  cast the bracket  0/2",
    "  │   │   └── ◐ .1  pour the iron  ◍ wT:p2 · working",
    "  │   ├─▸ ○ .2  glaze the lantern  0/3  ◍ 1 agent beneath",
    "  │   ├── ○ .3  mount the lens  0/3",
    "  │   │   └┄▸ ○ ark-1.1  cast the bracket  0/2  ◍ 1 agent beneath",
    "  │   └─▸ ✓ 3 more beads · finished, and nobody on them",
    "  └─▸ 1 tree with no live agent",
];

/// The forest with the copy of the blocker under `glaze the lantern` open,
/// and nothing else moved.
const THE_COPY_OPEN: &[&str] = &[
    "▾ arkham  ✓ <age> ago  3/10  1 agent",
    "  ├── ○ ark-1  raise the beacon  3/8",
    "  │   ├── ○ .1  cast the bracket  0/2",
    "  │   │   └── ◐ .1  pour the iron  ◍ wT:p2 · working",
    "  │   ├── ○ .2  glaze the lantern  0/3",
    "  │   │   └┄┄ ○ ark-1.1  cast the bracket  0/2",
    "  │   │       └── ◐ .1  pour the iron  ◍ wT:p2 · working",
    "  │   ├── ○ .3  mount the lens  0/3",
    "  │   │   └┄▸ ○ ark-1.1  cast the bracket  0/2  ◍ 1 agent beneath",
    "  │   └─▸ ✓ 3 more beads · finished, and nobody on them",
    "  └─▸ 1 tree with no live agent",
];

/// `E` opens every fold there is, `C` shuts every fold there is, and `D`
/// hands every one of them back to where it rests.
#[test]
fn e_and_c_fold_the_whole_forest_and_d_puts_it_back() {
    let (mut bdi, _fixture) = over_every_fold_kind("whole-forest", &TALL);
    assert_eq!(forest(&mut bdi, &TALL), AT_REST);

    bdi.send(EXPAND_THE_FOREST);
    assert_eq!(forest(&mut bdi, &TALL), EVERYTHING_OPEN);

    bdi.send(COLLAPSE_THE_FOREST);
    assert_eq!(forest(&mut bdi, &TALL), EVERYTHING_SHUT);

    bdi.send(RESTORE_THE_FOREST);
    assert_eq!(forest(&mut bdi, &TALL), AT_REST);
}

/// `e`, `c` and `d` do the same to the selected node and what hangs under it,
/// and to nothing else. The node is `glaze the lantern`, whose one child is
/// the second copy of the blocker: `e` opens the copy as well, `c` shuts the
/// node over both, and `d` rests the node open and the copy shut.
#[test]
fn e_c_and_d_fold_the_subtree_under_the_selection() {
    let (mut bdi, _fixture) = over_every_fold_kind("subtree", &TALL);
    bdi.send(&clicked_on(GLAZE_THE_LANTERN));

    bdi.send(EXPAND_THE_SUBTREE);
    assert_eq!(forest(&mut bdi, &TALL), THE_COPY_OPEN);

    bdi.send(COLLAPSE_THE_SUBTREE);
    assert_eq!(forest(&mut bdi, &TALL), GLAZE_THE_LANTERN_SHUT);

    bdi.send(RESTORE_THE_SUBTREE);
    assert_eq!(forest(&mut bdi, &TALL), AT_REST);
}

/// `l` opens a shut node and steps into an open one; `h` shuts an open node
/// and steps out of a shut one; Space turns the fold whichever way it is not
/// pointing. All on one row, the second copy of the blocker.
#[test]
fn l_h_and_space_move_one_fold_and_the_selection_through_it() {
    let (mut bdi, _fixture) = over_every_fold_kind("one-row", &TALL);
    bdi.send(&clicked_on(THE_COPY_UNDER_IT));

    bdi.send(OPEN_OR_STEP_IN);
    assert_eq!(forest(&mut bdi, &TALL), THE_COPY_OPEN);

    bdi.send(OPEN_OR_STEP_IN);
    assert_eq!(forest(&mut bdi, &TALL), THE_COPY_OPEN);
    assert_eq!(selected(&mut bdi, &TALL), "ark-1.1.1");

    bdi.send(SHUT_OR_STEP_OUT);
    assert_eq!(forest(&mut bdi, &TALL), THE_COPY_OPEN);
    assert_eq!(selected(&mut bdi, &TALL), "ark-1.1");

    bdi.send(SHUT_OR_STEP_OUT);
    assert_eq!(forest(&mut bdi, &TALL), AT_REST);

    bdi.send(TOGGLE);
    assert_eq!(forest(&mut bdi, &TALL), THE_COPY_OPEN);

    bdi.send(TOGGLE);
    assert_eq!(forest(&mut bdi, &TALL), AT_REST);
}

/// The motions walk the opened forest row by row, and the view follows the
/// selection by the least it can: down to the end, back up by half a band
/// until the selection is above the view and the view moves to it, and back
/// to the top.
#[test]
fn the_motions_walk_the_opened_forest_and_scroll_the_view_after_it() {
    let (mut bdi, _fixture) = over_every_fold_kind("motions", &SHORT);

    bdi.send(EXPAND_THE_FOREST);
    assert_eq!(forest(&mut bdi, &SHORT), &EVERYTHING_OPEN[..7]);

    bdi.send(DOWN);
    assert_eq!(forest(&mut bdi, &SHORT), &EVERYTHING_OPEN[..7]);
    assert_eq!(selected(&mut bdi, &SHORT), "ark-1");

    bdi.send(LAST_ROW);
    assert_eq!(forest(&mut bdi, &SHORT), &EVERYTHING_OPEN[10..]);
    assert_eq!(selected(&mut bdi, &SHORT), "ark-2.1");

    bdi.send(UP);
    assert_eq!(forest(&mut bdi, &SHORT), &EVERYTHING_OPEN[10..]);
    assert_eq!(selected(&mut bdi, &SHORT), "ark-2");

    bdi.send(HALF_SCREEN_UP);
    assert_eq!(forest(&mut bdi, &SHORT), &EVERYTHING_OPEN[10..]);
    assert_eq!(selected(&mut bdi, &SHORT), "ark-1.5");

    bdi.send(HALF_SCREEN_UP);
    assert_eq!(forest(&mut bdi, &SHORT), &EVERYTHING_OPEN[9..16]);
    assert_eq!(selected(&mut bdi, &SHORT), "ark-1.1.1");

    bdi.send(HALF_SCREEN_DOWN);
    assert_eq!(forest(&mut bdi, &SHORT), &EVERYTHING_OPEN[9..16]);
    assert_eq!(selected(&mut bdi, &SHORT), "ark-1.5");

    // The first row is the project's, and a project has no window to name
    // it, so the row below says where `g` went.
    bdi.send(FIRST_ROW);
    assert_eq!(forest(&mut bdi, &SHORT), &EVERYTHING_OPEN[..7]);
    bdi.send(DOWN);
    assert_eq!(selected(&mut bdi, &SHORT), "ark-1");
}

/// `a` draws the tree the filter was holding back among the others, resting
/// shut, and takes the line that stood for it away; the fold the reader set
/// by hand stays where they set it, through the change and back.
#[test]
fn a_moves_the_hidden_tree_and_leaves_the_folds_alone() {
    let (mut bdi, _fixture) = over_every_fold_kind("filter", &TALL);
    bdi.send(&clicked_on(GLAZE_THE_LANTERN));
    bdi.send(COLLAPSE_THE_SUBTREE);
    assert_eq!(forest(&mut bdi, &TALL), GLAZE_THE_LANTERN_SHUT);

    bdi.send(SHOW_EVERY_TREE);
    assert_eq!(
        forest(&mut bdi, &TALL),
        &[
            "▾ arkham  ✓ <age> ago  3/10  1 agent",
            "  ├── ○ ark-1  raise the beacon  3/8",
            "  │   ├── ○ .1  cast the bracket  0/2",
            "  │   │   └── ◐ .1  pour the iron  ◍ wT:p2 · working",
            "  │   ├─▸ ○ .2  glaze the lantern  0/3  ◍ 1 agent beneath",
            "  │   ├── ○ .3  mount the lens  0/3",
            "  │   │   └┄▸ ○ ark-1.1  cast the bracket  0/2  ◍ 1 agent beneath",
            "  │   └─▸ ✓ 3 more beads · finished, and nobody on them",
            "  └─▸ ○ ark-2  dredge the harbour  0/2",
        ]
    );

    bdi.send(SHOW_EVERY_TREE);
    assert_eq!(forest(&mut bdi, &TALL), GLAZE_THE_LANTERN_SHUT);
}

/// A collection that finds the same work under a fold the reader shut leaves
/// it shut.
#[test]
fn a_refresh_keeps_a_fold_shut_over_work_that_has_not_changed() {
    let (mut bdi, fixture) = over_every_fold_kind("refreshed", &TALL);
    bdi.send(&clicked_on(GLAZE_THE_LANTERN));
    bdi.send(COLLAPSE_THE_SUBTREE);
    assert_eq!(forest(&mut bdi, &TALL), GLAZE_THE_LANTERN_SHUT);
    let read_before = fixture.tracker.calls().len();

    bdi.send(REFRESH);
    bdi.settle(A_SILENCE, GIVING_UP);
    assert!(
        fixture.tracker.calls().len() > read_before,
        "the refresh asked bd nothing, so the frame after it pins nothing: {:?}",
        fixture.tracker.calls()
    );
    assert_eq!(forest(&mut bdi, &TALL), GLAZE_THE_LANTERN_SHUT);
}

/// A collection that finds live work under a fold the reader shut, that was
/// not there when they shut it, lets the fold go: the node rests open again,
/// onto the new work.
#[test]
fn a_refresh_lets_a_fold_go_when_live_work_arrives_under_it() {
    let (mut bdi, fixture) = over_every_fold_kind("new-work", &TALL);
    bdi.send(&clicked_on(MOUNT_THE_LENS));
    bdi.send(COLLAPSE_THE_SUBTREE);
    assert_eq!(
        forest(&mut bdi, &TALL),
        &[
            "▾ arkham  ✓ <age> ago  3/10  1 agent",
            "  ├── ○ ark-1  raise the beacon  3/8",
            "  │   ├── ○ .1  cast the bracket  0/2",
            "  │   │   └── ◐ .1  pour the iron  ◍ wT:p2 · working",
            "  │   ├── ○ .2  glaze the lantern  0/3",
            "  │   │   └┄▸ ○ ark-1.1  cast the bracket  0/2  ◍ 1 agent beneath",
            "  │   ├─▸ ○ .3  mount the lens  0/3  ◍ 1 agent beneath",
            "  │   └─▸ ✓ 3 more beads · finished, and nobody on them",
            "  └─▸ 1 tree with no live agent",
        ]
    );

    fixture.tracker.holds(THE_TRACKER_WITH_NEW_WORK);
    fixture
        .herdr
        .lists(&fixture.panes(&[THE_PANE, THE_NEW_PANE]));
    bdi.send(REFRESH);
    bdi.settle(A_SILENCE, GIVING_UP);
    assert_eq!(
        forest(&mut bdi, &TALL),
        &[
            "▾ arkham  ✓ <age> ago  3/11  2 agents",
            "  ├── ○ ark-1  raise the beacon  3/9",
            "  │   ├── ○ .1  cast the bracket  0/2",
            "  │   │   └── ◐ .1  pour the iron  ◍ wT:p2 · working",
            "  │   ├── ○ .2  glaze the lantern  0/3",
            "  │   │   └┄▸ ○ ark-1.1  cast the bracket  0/2  ◍ 1 agent beneath",
            "  │   ├── ○ .3  mount the lens  0/4",
            "  │   │   ├── ◐ .1  grind the glass  ◍ wT:p3 · working",
            "  │   │   └┄▸ ○ ark-1.1  cast the bracket  0/2  ◍ 1 agent beneath",
            "  │   └─▸ ✓ 3 more beads · finished, and nobody on them",
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
    let (mut bdi, fixture) = over_every_fold_kind("finished-work", &TALL);
    bdi.send(SHOW_EVERY_TREE);
    bdi.send(&clicked_on(RAISE_THE_KADATH));
    bdi.send(EXPAND_THE_SUBTREE);
    assert_eq!(
        forest(&mut bdi, &TALL),
        &[
            "▾ arkham  ✓ <age> ago  3/10  1 agent",
            "  ├── ○ ark-1  raise the beacon  3/8",
            "  │   ├── ○ .1  cast the bracket  0/2",
            "  │   │   └── ◐ .1  pour the iron  ◍ wT:p2 · working",
            "  │   ├── ○ .2  glaze the lantern  0/3",
            "  │   │   └┄┄ ○ ark-1.1  cast the bracket  0/2",
            "  │   │       └── ◐ .1  pour the iron  ◍ wT:p2 · working",
            "  │   ├── ○ .3  mount the lens  0/3",
            "  │   │   └┄┄ ○ ark-1.1  cast the bracket  0/2",
            "  │   │       └── ◐ .1  pour the iron  ◍ wT:p2 · working",
            "  │   └── ✓ 3 more beads · finished, and nobody on them",
            "  │       ├── ✓ .4  survey the headland",
            "  │       ├── ✓ .5  draw up the plans",
            "  │       └── ✓ .6  clear the site",
            "  └─▸ ○ ark-2  dredge the harbour  0/2",
        ]
    );

    fixture.tracker.holds(THE_TRACKER_WITH_FINISHED_WORK);
    fixture.herdr.lists(&fixture.panes(&[]));
    bdi.send(REFRESH);
    bdi.settle(A_SILENCE, GIVING_UP);
    assert_eq!(
        forest(&mut bdi, &TALL),
        &[
            "▾ arkham  ✓ <age> ago  4/10",
            "  ├── ○ ark-1  raise the beacon  4/8",
            "  │   ├── ○ .1  cast the bracket  1/2",
            "  │   │   └── ✓ .1  pour the iron",
            "  │   ├── ○ .2  glaze the lantern  1/3",
            "  │   │   └┄┄ ○ ark-1.1  cast the bracket  1/2",
            "  │   │       └── ✓ .1  pour the iron",
            "  │   ├── ○ .3  mount the lens  1/3",
            "  │   │   └┄┄ ○ ark-1.1  cast the bracket  1/2",
            "  │   │       └── ✓ .1  pour the iron",
            "  │   └── ✓ 3 more beads · finished, and nobody on them",
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
    let (mut bdi, _fixture) = over_every_fold_kind("searched", &TALL);
    bdi.send(COLLAPSE_THE_FOREST);
    assert_eq!(forest(&mut bdi, &TALL), EVERYTHING_SHUT);

    bdi.send(b"/ark-1.1.1\r");
    assert_eq!(
        forest(&mut bdi, &TALL),
        &[
            "▾ arkham  ✓ <age> ago  3/10  1 agent",
            "  ├── ○ ark-1  raise the beacon  3/8",
            "  │   ├── ○ .1  cast the bracket  0/2",
            "  │   │   └── ◐ .1  pour the iron  ◍ wT:p2 · working",
            "  │   ├─▸ ○ .2  glaze the lantern  0/3  ◍ 1 agent beneath",
            "  │   ├─▸ ○ .3  mount the lens  0/3  ◍ 1 agent beneath",
            "  │   └─▸ ✓ 3 more beads · finished, and nobody on them",
            "  └─▸ 1 tree with no live agent",
        ]
    );
    assert_eq!(selected(&mut bdi, &TALL), "ark-1.1.1");
}

/// A click on a row of the opened forest selects the line drawn there, which
/// after `E` is a line the first screen never drew.
#[test]
fn a_click_after_e_selects_the_row_it_lands_on() {
    let (mut bdi, _fixture) = over_every_fold_kind("clicked", &TALL);
    bdi.send(EXPAND_THE_FOREST);
    assert_eq!(forest(&mut bdi, &TALL), EVERYTHING_OPEN);

    let survey_the_silt = 16;
    bdi.send(&clicked_on(survey_the_silt));
    assert_eq!(forest(&mut bdi, &TALL), EVERYTHING_OPEN);
    assert_eq!(selected(&mut bdi, &TALL), "ark-2.1");
}

/// The shims a run reads, kept so a test can change what the next collection
/// finds.
struct Fixture {
    tracker: ShimmedTracker,
    herdr: ShimmedHerdr,
    arkham: PathBuf,
}

impl Fixture {
    /// What `herdr agent list` answers: these panes, every one working in the
    /// project's directory.
    fn panes(&self, panes: &[&str]) -> String {
        let agents: Vec<String> = panes
            .iter()
            .map(|pane| {
                format!(
                    r#"{{"pane_id":"{pane}","cwd":"{}","agent_status":"working"}}"#,
                    self.arkham.display()
                )
            })
            .collect();
        format!(r#"{{"result":{{"agents":[{}]}}}}"#, agents.join(","))
    }
}

/// A `bdi` on a pty of this height over the tracker at the top of this
/// file, drawn and settled, with the selection where a run leaves it: on
/// the project's line.
///
/// `bdi` is started in `HOME` itself, which is not the project's directory,
/// so the directory chooses no scope.
fn over_every_fold_kind(named: &str, screen: &Screen) -> (Driven, Fixture) {
    let home = std::env::temp_dir().join(format!("bdi-{named}-{}", std::process::id()));
    let arkham = home.join("arkham");
    std::fs::create_dir_all(&arkham).expect("the directory is ours to make");
    std::fs::create_dir_all(home.join(".config/beady-eye")).expect("the directory is ours to make");
    std::fs::write(
        home.join(".config/beady-eye/config.toml"),
        format!(
            "[[projects]]\nname = \"arkham\"\npath = \"{}\"\n",
            arkham.display()
        ),
    )
    .expect("the config is ours to write");

    let fixture = Fixture {
        tracker: ShimmedTracker::beside(&home),
        herdr: ShimmedHerdr::beside(&home),
        arkham,
    };
    fixture.tracker.holds(THE_TRACKER);
    fixture.herdr.lists(&fixture.panes(&[THE_PANE]));
    let mut environment = fixture.tracker.environment();
    environment.extend(fixture.herdr.environment());
    environment.push(a_socket_of_its_own(&home));

    let mut bdi = Driven::bdi(screen.rows, COLS, home, &environment);
    bdi.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);
    assert_eq!(
        fixture.tracker.unanswered(),
        Vec::<String>::new(),
        "bd was asked something the shim had no answer for"
    );
    (bdi, fixture)
}

/// The forest band as it stands, one string per row, in the words the
/// assertions use: a row's blanks squeezed to one gap, the project's clock
/// masked, and the rows below the last drawn one dropped.
///
/// Read off a frame `bdi` was made to repaint whole, for the reason `row_of`
/// gives. The screen is grown a row and shrunk back, and the second answer
/// is the frame: the same height as every other frame this test reads, so a
/// row is the same row whichever read it came from.
#[track_caller]
fn forest(bdi: &mut Driven, screen: &Screen) -> Vec<String> {
    let mut rows: Vec<String> = rows_drawn(&repainted(bdi, screen))
        .into_iter()
        .take(screen.band)
        .map(|row| without_the_clock(&squeezed(&row)))
        .collect();
    while rows.last().is_some_and(String::is_empty) {
        rows.pop();
    }
    rows
}

/// The id of the bead the selection is on, read off the window Enter opens
/// over it. The window is taken down again before this returns.
#[track_caller]
fn selected(bdi: &mut Driven, screen: &Screen) -> String {
    bdi.send(SHOW_IT);
    let screen = repainted(bdi, screen);
    let title = window_over(&screen).unwrap_or_else(|| {
        panic!(
            "no bead window is up. The screen: {:#?}",
            rows_drawn(&screen)
        )
    });
    bdi.send(BACK);
    bdi.settle(A_SILENCE, GIVING_UP);
    title
}

/// The whole screen, drawn again from nothing.
fn repainted(bdi: &mut Driven, screen: &Screen) -> Vec<u8> {
    bdi.settle(A_SILENCE, GIVING_UP);
    let grown = bdi.resize(screen.rows + 1, COLS);
    bdi.answer_to(grown, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);
    let back = bdi.resize(screen.rows, COLS);
    bdi.answer_to(back, GIVING_UP)
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

/// The project's line with how long ago it was read masked, since that is a
/// clock and not a fold.
fn without_the_clock(row: &str) -> String {
    let Some((before, after)) = row.split_once("✓ ") else {
        return row.to_string();
    };
    match after.split_once(" ago") {
        Some((_, rest)) => format!("{before}✓ <age> ago{rest}"),
        None => row.to_string(),
    }
}
