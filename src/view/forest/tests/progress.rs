//! What a bead says of its subtree's progress, and what a keystroke walks to say it.

use super::*;
use pretty_assertions::assert_eq;

/// A line that stands for more than itself says how much of that is done,
/// which is the question a root's `35/51` answers — asked here one level
/// down. `dun-7.1` is open and holds two open children, so its subtree is
/// three beads with none of them closed.
///
/// Counted including the bead's own line, because that is what a root
/// already does: `snapshot` sets `total` to `nodes.len()`, and the root is
/// one of those nodes. One rule at every depth.
#[test]
fn a_bead_with_children_says_how_much_of_its_own_subtree_is_done() {
    let forest = flatten(snapshot());

    assert_eq!(
        row_of(&forest, "dun-7.1").progress,
        Some(Progress {
            finished: 0,
            total: 3
        })
    );
}

/// Both halves of the fraction have to count. `dep-1.2` is closed with two
/// closed beads under it, so a subtree that is finished says so — an epic
/// stuck at `0/3` whatever its children did would be worse than no count.
#[test]
fn a_finished_subtree_counts_its_closed_beads_and_not_only_its_size() {
    let tree = tree_of("dunwich", DEPOT);
    let (at, above) = way_to(&tree, "dep-1.2");

    assert_eq!(
        progress_of(&tree, at, &above),
        Some(Progress {
            finished: 4,
            total: 4
        })
    );
}

/// bd keeps a pinned bead indefinitely and never counts it as work: not
/// ready, not blocking, out of its default list. So it is finished here,
/// counted on the done side of a fraction and swept into a run beside the
/// closed beads it sits with.
#[test]
fn a_pinned_bead_folds_and_counts_as_finished_work() {
    const PINNED: &str = r#"[
      {"id":"pin-1","title":"stock the depot","status":"open"},
      {"id":"pin-1.1","title":"count the crates","status":"in_progress",
       "dependencies":[{"depends_on_id":"pin-1","type":"parent-child"}]},
      {"id":"pin-1.2","title":"sweep the floor","status":"closed",
       "dependencies":[{"depends_on_id":"pin-1","type":"parent-child"}]},
      {"id":"pin-1.3","title":"the loading rota","status":"pinned",
       "dependencies":[{"depends_on_id":"pin-1","type":"parent-child"}]},
      {"id":"pin-1.4","title":"oil the doors","status":"closed",
       "dependencies":[{"depends_on_id":"pin-1","type":"parent-child"}]}
    ]"#;
    let tree = tree_of("dunwich", PINNED);

    assert_eq!(
        progress_of(&tree, 0, &[]),
        Some(Progress {
            finished: 3,
            total: 5
        })
    );
    assert_eq!(
        sketch(&flatten(alone("dunwich", PINNED, &[]))),
        vec![
            "▾ dunwich",
            "  └── ○ pin-1 stock the depot",
            "      ├── ◐ .1 count the crates",
            "      └─▸ … 3 more",
        ]
    );
}

/// A fraction says how much of what a bead is waiting on is done, and
/// that is a count of beads. A blocker two of its descendants share is one
/// piece of work whether it is drawn once or twice.
#[test]
fn a_fraction_counts_beads_rather_than_the_rows_they_are_drawn_on() {
    const SHARED: &str = r#"[
      {"id":"shr-1","title":"root","status":"open"},
      {"id":"shr-1.1","title":"one","status":"open",
       "dependencies":[{"depends_on_id":"shr-1","type":"parent-child"},
                       {"depends_on_id":"shr-1.9","type":"blocks"}]},
      {"id":"shr-1.2","title":"two","status":"open",
       "dependencies":[{"depends_on_id":"shr-1","type":"parent-child"},
                       {"depends_on_id":"shr-1.9","type":"blocks"}]},
      {"id":"shr-1.9","title":"what both wait on","status":"closed",
       "dependencies":[{"depends_on_id":"shr-1","type":"parent-child"}]}
    ]"#;
    let tree = tree_of("dunwich", SHARED);

    assert_eq!(
        progress_of(&tree, 0, &[]),
        Some(Progress {
            finished: 1,
            total: 4
        })
    );
}

/// A closed bead's descendants are what had to finish before it, so beads
/// that merely waited on it are not among them and cannot be counted into
/// its fraction.
#[test]
fn a_closed_blocker_reports_no_fraction_over_the_beads_that_waited_on_it() {
    const WAITED: &str = r#"[
      {"id":"wtd-1","title":"root","status":"open"},
      {"id":"wtd-1.1","title":"waiting","status":"open",
       "dependencies":[{"depends_on_id":"wtd-1","type":"parent-child"},
                       {"depends_on_id":"wtd-1.9","type":"blocks"}]},
      {"id":"wtd-1.9","title":"done","status":"closed",
       "dependencies":[{"depends_on_id":"wtd-1","type":"parent-child"}]}
    ]"#;
    let tree = tree_of("dunwich", WAITED);
    let (at, above) = way_to(&tree, "wtd-1.9");

    assert_eq!(progress_of(&tree, at, &above), None);
}

/// A leaf stands for itself alone, so there is nothing to be part-way
/// through and a fraction over one bead would only repeat its glyph.
#[test]
fn a_bead_with_no_children_has_no_progress_to_report() {
    let forest = flatten(snapshot());

    assert_eq!(row_of(&forest, "dun-7.7").progress, None);
}

/// Everything a line says of the tree beneath it — how far along it is,
/// what it is shut over, whether it rests open, what a run stands for —
/// depends on the snapshot alone. So it is answered once, when the forest
/// takes the snapshot, and a keystroke asks nothing of the tree.
#[test]
fn a_keystroke_walks_no_subtree() {
    let mut forest = flatten(built(Filter::All));
    assert!(
        forest
            .lines()
            .iter()
            .any(|line| matches!(line.content, Content::Elided { .. })),
        "the fixture draws no run, so a keystroke had no run to size"
    );
    let before = walks_on_this_thread();

    forest.apply(Action::Move(Motion::NextRow));

    assert_eq!(walks_on_this_thread() - before, 0);
}

/// A collection lands every time the watcher speaks, and most of them find
/// every tree as it was. A tree that did not move keeps what was answered
/// of it, so such a collection asks nothing of any tree.
#[test]
fn a_collection_that_moved_no_tree_walks_no_subtree() {
    let mut forest = flatten(built(Filter::All));
    let before = walks_on_this_thread();

    forest.refresh(built(Filter::All));

    assert_eq!(walks_on_this_thread() - before, 0);
}

/// A tree that did move is answered again: a child closing under
/// `dun-7.1` is a bead more of its subtree done.
#[test]
fn a_collection_that_moved_a_tree_answers_it_again() {
    let mut forest = flatten(snapshot());

    let finished = edited(
        DUNWICH,
        r#""id":"dun-7.1.1","title":"true the mount","status":"open""#,
        r#""id":"dun-7.1.1","title":"true the mount","status":"closed""#,
    );
    forest.refresh(gather(
        vec![tree_of("dunwich", &finished)],
        Vec::new(),
        Filter::LiveAgents,
    ));

    assert_eq!(
        row_of(&forest, "dun-7.1").progress,
        Some(Progress {
            finished: 1,
            total: 3
        })
    );
}

/// A subtree the folds name nothing at or beneath is counted from its
/// tree rather than drawn, and drawn from it only where a reader reaches
/// in. So a key that lays the forest out again costs the count and the
/// lines the folds name, which on a large forest fully opened is a few
/// thousand lines of hundreds of thousands. What is drawn on reaching in
/// is exactly what was counted.
#[test]
fn a_subtree_the_folds_do_not_name_is_counted_rather_than_drawn() {
    let mut forest = flatten(built(Filter::All));
    forest.apply(Action::ExpandForest);
    let drawn = forest.lines();
    let mut lines = 0;
    let mut counted = 0;
    for top in drawn.top() {
        drawn.visit(top, &mut |node| {
            lines += 1;
            counted += usize::from(!matches!(node.beneath, Beneath::Nothing));
            true
        });
    }
    assert!(counted > 0, "{:#?}", sketch(&forest));
    assert_eq!(lines, drawn.len());
    assert_eq!(drawn.iter().count(), drawn.len());
}

/// The width table covers the lines a reader has not looked at yet: a
/// subtree counted rather than drawn is measured into the same table, so
/// the id column does not shift when a scroll first draws it. Once every
/// line is drawn, the lines measure the table they were given.
#[test]
fn the_widths_measure_the_lines_a_counted_subtree_has_not_drawn() {
    let mut forest = flatten(alone("dunwich", WIDE_BELOW, &[]));
    forest.apply(Action::ExpandForest);
    let drawn = forest.lines();
    let widths_of = |lines: &mut dyn Iterator<Item = &Line>| {
        let mut widths = Widths::default();
        for line in lines {
            if let Content::Bead(row) = &line.content {
                widths.merge(&identity_widths(row, &forest.layout));
            }
        }
        widths
    };

    let mut so_far = Vec::new();
    for top in drawn.top() {
        drawn.visit(top, &mut |node| {
            so_far.push(&node.line);
            matches!(node.beneath, Beneath::Nothing)
        });
    }
    let before = widths_of(&mut so_far.into_iter());
    assert!(
        before.of(&Cell::Id) < ".1000000".len(),
        "the widest id is drawn before anything reaches it: {:#?}",
        sketch(&forest)
    );
    assert_eq!(drawn.widths().of(&Cell::Id), ".1000000".len());

    let every = widths_of(&mut drawn.iter());
    assert_eq!(drawn.widths(), &every);
}

/// The lines a forest draws depend on its folds, its root and its
/// filter, and a key that moves only the selection changes none of
/// them. So the lines it held are the lines it would draw, and a motion
/// draws nothing — on a large forest fully opened, a draw is hundreds of
/// thousands of lines and a keystroke that made one did not answer at
/// once.
///
/// `h` and `l` stepping out of and into a node move only the selection
/// too. A fold is the control: it changes what is drawn and draws once.
#[test]
fn a_key_that_moves_only_the_selection_draws_nothing() {
    let mut forest = flatten(built(Filter::All));
    let before = layout::draws_so_far();

    forest.apply(Action::Move(Motion::NextRow));
    forest.apply(Action::Move(Motion::HalfScreenDown));
    forest.apply(Action::Move(Motion::LastRow));
    forest.apply(Action::Move(Motion::FirstRow));
    // A leaf has no fold for `h` to shut, so it steps out to the parent,
    // which is open because the leaf is drawn, so `l` steps back in.
    let leaf = forest
        .lines()
        .iter()
        .position(|line| line.folded.is_none() && selectable(line))
        .expect("the fixture draws a leaf");
    forest.select_line(leaf);
    forest.apply(Action::CollapseOrParent);
    forest.apply(Action::ExpandOrChild);
    assert_eq!(layout::draws_so_far() - before, 0);

    forest.apply(Action::ToggleFold);
    assert_eq!(layout::draws_so_far() - before, 1);
}

/// What a key reports is what the loop redraws on, and every key
/// reports the same thing: the selection moved, a line changed, or the
/// view scrolled. Each is asked on a fresh forest and again with the
/// first root shut by hand, so each key changes something at least
/// once, and a key reporting a change it did not make, or none it did,
/// says so. So does a key that left the screen behind what it changed,
/// since a stale screen changes nothing and a skipped draw is only
/// right where there was nothing to draw.
#[test]
fn every_key_reports_exactly_what_it_changed() {
    let keys = [
        Action::Move(Motion::NextRow),
        Action::CollapseOrParent,
        Action::ExpandOrChild,
        Action::ToggleFold,
        Action::ExpandSubtree,
        Action::CollapseSubtree,
        Action::RestoreSubtree,
        Action::ExpandForest,
        Action::CollapseForest,
        Action::RestoreDefault,
        Action::ToggleFilter,
        Action::FocusForest,
    ];
    for key in keys {
        let mut changed_once = false;
        for shut_by_hand in [false, true] {
            let mut forest = flatten(built(Filter::All));
            forest.fit(4);
            if shut_by_hand {
                forest.apply(Action::CollapseSubtree);
            }
            let (selected, from) = (forest.selected_line(), forest.from());
            let lines = forest.lines().clone();

            let reported = forest.apply(key);

            let drawn = layout::draw(
                &forest.snapshot,
                &forest.facts,
                &forest.folds,
                &forest.rooted(),
            );
            assert_eq!(
                forest.lines(),
                &Drawn::new(drawn),
                "{key:?} left the screen stale"
            );
            let changed = forest.selected_line() != selected
                || forest.from() != from
                || forest.lines() != &lines;
            assert_eq!(reported, changed, "{key:?}, shut by hand: {shut_by_hand}");
            changed_once |= changed;
        }
        assert!(changed_once, "{key:?} changed nothing either way");
    }
}

/// What `cyc-1.1` stands over is `cyc-1.2` alone: its forebear is above
/// it, not beneath, and only the way down to it can say so. A bead the
/// tree holds once can be answered once only where no way down is cut.
#[test]
fn a_bead_on_a_loop_counts_what_the_way_down_leaves_beneath_it() {
    let forest = flatten(alone("dunwich", LOOPED, &panes_on(&["cyc-1.1"])));

    assert_eq!(
        row_of(&forest, "cyc-1.1").progress,
        Some(Progress {
            finished: 1,
            total: 2
        })
    );
}

/// Where no loop is cut, nothing beneath a bead can be above it, so the
/// way down changes no answer and every copy of a bead gets the one the
/// tree keeps for it.
#[test]
fn where_no_loop_is_cut_every_way_down_to_a_bead_gets_the_same_answer() {
    let fixtures = [
        DUNWICH,
        DEPOT,
        RELAY,
        SIDING,
        TOWER,
        KADATH,
        SLUICE,
        TWICE,
        CLOSED_TWICE,
        SHARED_IN_A_RUN,
    ];
    let staffed = panes_on(&[
        "dun-7.1", "dep-1.1", "rly-2.1", "sdg-4.3", "tow-1.1", "bcn-6", "slu-1.1",
    ]);
    for json in fixtures {
        let tree = alone("dunwich", json, &staffed).collected.remove(0);
        assert!(tree.cycles.is_empty(), "{} has a loop", tree.root);
        let facts = TreeFacts::of(&tree);

        for (at, above) in every_way_down(&tree) {
            assert_eq!(
                facts.bead(&tree, at, &above),
                facts_of(&tree, at, &above),
                "{} reached by {above:?}",
                tree.beads[at].id
            );
        }
    }
}

/// Every way down the walk takes, as the bead it lands on and the beads
/// above it.
fn every_way_down(tree: &Tree) -> Vec<(usize, Vec<usize>)> {
    let mut ways = Vec::new();
    let mut going = vec![(0, Vec::new())];
    while let Some((at, above)) = going.pop() {
        let below = way_below(&above, at);
        going.extend(
            links_below(tree, at, &above)
                .into_iter()
                .map(|link| (link.bead, below.clone())),
        );
        ways.push((at, above));
    }
    ways
}
