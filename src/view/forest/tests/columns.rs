//! The column a line's content starts in.

use super::*;
use pretty_assertions::assert_eq;

/// How wide a prefix is on screen. Every glyph a prefix is drawn from is
/// one column, so counting them is the column its content starts in.
fn columns(prefix: &str) -> usize {
    prefix.chars().count()
}

/// `sdg-4.3` rests shut over work of its own and `sdg-4.1` rests open
/// beside it, both children of the root. A reader runs down the column
/// the ids are in, and a line pushed right of its siblings is out of the
/// column that was drawn to be read.
///
/// Asked in columns rather than of a substring: every prefix here holds
/// the elbow the other one does, so a test that looked for one found it
/// on both and said nothing about where they started.
#[test]
fn a_shut_node_starts_in_the_same_column_as_an_open_sibling() {
    let forest = flatten(ready_alone(
        "dunwich",
        SIDING,
        &panes_on(&["sdg-4.3"]),
        &["sdg-4.1.2"],
    ));
    let shut = line_of(&forest, "sdg-4.3");
    let open = line_of(&forest, "sdg-4.1");

    assert_eq!(shut.folded, Some(false), "sdg-4.3 is the one resting shut");
    assert_eq!(open.folded, Some(true), "sdg-4.1 is the one resting open");
    assert_eq!(shut.depth, open.depth, "they are siblings");
    assert_eq!(
        columns(&shut.prefix),
        columns(&open.prefix),
        "a shut node and an open sibling start in different columns:\n{}",
        sketch(&forest).join("\n")
    );
}

/// A bead drawn under one it blocks is drawn on a dashed arm, and under
/// its parent on the solid one every child gets. Otherwise the two copies
/// are the same row twice, and a reader takes the tree for having
/// duplicated it.
///
/// The arm is the elbow's, so it holds whether the line rests shut or
/// open: the fold marker takes the arm's last column exactly as it does
/// on a child, and the width is the four columns a level every line has.
#[test]
fn a_bead_drawn_under_one_it_blocks_hangs_on_a_dashed_arm() {
    let mut forest = flatten(alone("dunwich", SLUICE, &panes_on(&["slu-1.1"])));

    assert_eq!(
        sketch(&forest),
        vec![
            "▾ dunwich",
            "  └── ◐ slu-1 rehang the sluice",
            "      ├─▸ ◐ .1 forge the new pintles",
            "      └── ○ .2 hang the gate",
            "          └┄▸ ◐ slu-1.1 forge the new pintles",
        ]
    );

    forest.apply(Action::ExpandSubtree);

    assert_eq!(
        sketch(&forest),
        vec![
            "▾ dunwich",
            "  └── ◐ slu-1 rehang the sluice",
            "      ├── ◐ .1 forge the new pintles",
            "      │   └── ○ .1 cast the pintle blanks",
            "      └── ○ .2 hang the gate",
            "          └┄┄ ◐ slu-1.1 forge the new pintles",
            "              └── ○ .1 cast the pintle blanks",
        ]
    );
}

/// A column of short ids is only readable if a reader can rebuild the
/// whole one from it: they put what the row above says in front of what
/// this row says, and stop at a row that reads whole because there is
/// nothing left to put in front of it. Over every bead these fixtures
/// draw, that walk gives back the id `bd` holds it under.
#[test]
fn walking_up_the_rows_and_joining_the_ids_gives_a_beads_whole_id_back() {
    let fixtures = [
        flatten(snapshot()),
        flatten(tower_staffed(&["tow-1.1.1.1"])),
        flatten(alone("dunwich", SLUICE, &panes_on(&["slu-1.1"]))),
        flatten(alone("dunwich", RELAY, &two_panes())),
        flatten(alone("dunwich", SIDING, &panes_on(&["sdg-4.3"]))),
    ];

    let mut shortened = 0;
    for mut forest in fixtures {
        forest.apply(Action::ExpandSubtree);
        let mut above: Vec<String> = Vec::new();
        for line in forest.lines() {
            let (Some(place), Content::Bead(row)) = (&line.place, &line.content) else {
                continue;
            };
            above.truncate(place.steps.len());
            let rebuilt = match above.last() {
                Some(parent) if row.id.starts_with('.') => format!("{parent}{}", row.id),
                _ => row.id.clone(),
            };

            assert_eq!(rebuilt, place.key().id, "{:#?}", sketch(&forest));

            shortened += usize::from(row.id.starts_with('.'));
            above.push(rebuilt);
        }
    }

    assert!(shortened > 0, "no row was drawn short to walk up from");
}

/// Four columns a level, every line in a forest with trees in it,
/// whatever that line is doing.
///
/// Stated once over whole screens rather than shape by shape. What went
/// wrong was a span appended to the prefix of one kind of line, and the
/// next such span will be appended by someone reading a rule about the
/// kind of line they happen to be drawing.
///
/// The one line outside the rule is the forest with nothing in it, which
/// stands for the whole screen rather than for a place in a tree and has
/// no prefix at all.
#[test]
fn every_prefix_is_four_columns_a_level() {
    for json in [DUNWICH, DEPOT, RELAY, SIDING, TOWER, KADATH, SLUICE] {
        let staffed = panes_on(&[
            "dun-7.1", "dep-1.1", "rly-2.1", "sdg-4.3", "tow-1.1", "bcn-6", "slu-1.1",
        ]);
        let mut forest = flatten(alone("dunwich", json, &staffed));
        four_columns_a_level(&forest);
        forest.apply(Action::ExpandSubtree);
        four_columns_a_level(&forest);
    }
}

fn four_columns_a_level(forest: &Forest) {
    for line in forest.lines() {
        assert_eq!(
            columns(&line.prefix),
            2 + 4 * line.depth as usize,
            "a prefix at depth {} is not four columns a level:\n{}",
            line.depth,
            sketch(forest).join("\n")
        );
    }
}

/// A root that would not read has nothing under it, so it draws no marker
/// and holds no fold state either. The two say the same thing about the
/// same line, and a line offering a fold nothing could act on is how the
/// marker got there in the first place.
#[test]
fn an_unread_root_holds_no_fold_to_set() {
    let forest = flatten(snapshot());
    let header = forest
        .lines()
        .iter()
        .find(|line| matches!(&line.content, Content::Unread(unread) if unread.root == "fer-2"))
        .expect("the shared snapshot draws a tree whose tracker refused");

    assert_eq!(header.folded, None, "{:#?}", sketch(&forest));
    assert_eq!(
        columns(&header.prefix),
        columns(&prefix(&[], true, false, None))
    );
}

/// A root that read fine and has nothing under it holds no fold either.
/// The unread root above takes `draw_tree`'s arm for a tree with no nodes
/// and never reaches the fold, so this is the only place a bead that is
/// drawn and has no children is asked whether it offers one.
///
/// The `folded` assertion is the one carrying the weight. A marker is
/// drawn off `!kids.is_empty() && !open`, which stays false here however
/// the fold state is decided, so the sketch reads the same whether this
/// line holds no fold or holds one pointing shut — and a line that holds
/// one pointing shut is a fold every walk over the forest keeps trying to
/// open. The screen is where that is invisible, which is why it is asked
/// of the state instead.
#[test]
fn a_root_with_no_children_holds_no_fold_to_set() {
    let forest = flatten(alone("dunwich", KADATH, &panes_on(&["bcn-6"])));

    assert_eq!(
        sketch(&forest),
        vec!["▾ dunwich", "  └── ◐ bcn-6 re-lamp the kadath"]
    );

    let root = forest
        .lines()
        .iter()
        .find(|line| matches!(&line.content, Content::Bead(row) if row.id == "bcn-6"))
        .expect("the fixture draws its root");

    assert_eq!(root.folded, None, "{:#?}", sketch(&forest));
    assert_eq!(
        columns(&root.prefix),
        columns(&prefix(&[], true, false, None))
    );
}
