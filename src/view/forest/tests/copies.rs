use super::*;
use pretty_assertions::assert_eq;

/// A bead reachable from two roots is drawn in both their trees, and the
/// selection has to be able to sit on either copy. `find_cursor` took the
/// first line carrying the handle, so the redraw that follows every
/// action pulled a step onto the lower copy back up to the upper one, and
/// the list below it could not be walked into at all.
#[test]
fn stepping_down_past_a_bead_drawn_twice_reaches_the_bottom_of_the_list() {
    let mut forest = flatten(overlapping(&panes_on(&["qua-1.2", "wha-2.1"])));
    assert_eq!(
        lines_of(&forest, "qua-1.2").len(),
        2,
        "{:#?}",
        sketch(&forest)
    );
    let drawn = forest.lines().len();

    assert_eq!(
        walk_down(&mut forest),
        (0..drawn).collect::<Vec<usize>>(),
        "{:#?}",
        sketch(&forest)
    );
}

/// The forest holds the trees it was handed, not copies of them. A
/// collection landing while the reader moves is a stall on top of the
/// keystroke it arrives beside, and copying every tree into the forest
/// was most of that stall.
#[test]
fn the_forest_holds_the_trees_it_was_handed_rather_than_copies() {
    let handed = snapshot();
    let trees = handed.trees.clone();
    let mut forest = flatten(handed);
    assert_held_exactly(&forest, &trees);

    let again = snapshot();
    let trees = again.trees.clone();
    forest.refresh(again);
    assert_held_exactly(&forest, &trees);
}

fn assert_held_exactly(forest: &Forest, trees: &[Arc<Tree>]) {
    assert_eq!(forest.snapshot().trees.len(), trees.len());
    for (held, was) in forest.snapshot().trees.iter().zip(trees) {
        assert!(Arc::ptr_eq(held, was), "{} was copied", was.root);
        assert_eq!(
            Arc::strong_count(was),
            4,
            "{} is held by collected, trees, the lines drawn from it and this test, and nothing else",
            was.root
        );
    }
}

/// Each copy of a bead drawn twice folds over a list of its own, so
/// shutting one says nothing about the other. Both carried the same
/// handle, so one keystroke shut them both and the reader lost a list
/// they had never been looking at.
#[test]
fn folding_one_copy_of_a_bead_drawn_twice_leaves_the_other_open() {
    let mut forest = flatten(overlapping(&panes_on(&["qua-1.2", "wha-2.1"])));
    let [upper, lower] = copies_of(&forest, "qua-1.2");

    step_onto(&mut forest, lower);
    forest.apply(Action::ToggleFold);

    assert_eq!(
        forest.lines()[upper].folded,
        Some(true),
        "{:#?}",
        sketch(&forest)
    );
}

/// The same bead under two parents in one tree, which is what a `blocks`
/// edge drawn as nesting gives: the copies share a root as well as a key,
/// so nothing but the way down to them tells them apart.
///
/// Each copy is opened against the way the other one rests, so a fold
/// remembered against the bead rather than against the line would have to
/// give one of them the other's answer.
#[test]
fn folding_one_copy_of_a_bead_drawn_twice_in_one_tree_leaves_the_other_alone() {
    let mut forest = under_every_copy(drawn_twice_in_one_tree());
    let [_, lower] = copies_of(&forest, "dun-9");

    step_onto(&mut forest, lower);
    forest.apply(Action::ToggleFold);
    let [upper, _] = copies_of(&forest, "dun-9");
    step_onto(&mut forest, upper);
    forest.apply(Action::ToggleFold);

    let [upper, lower] = copies_of(&forest, "dun-9");
    assert_eq!(
        (forest.lines()[upper].folded, forest.lines()[lower].folded),
        (Some(false), Some(true)),
        "{:#?}",
        sketch(&forest)
    );
}

/// A bead reached more than one way down is one piece of work with a line
/// each, and the first line is the one that stands for it. The rest are
/// shut, so the subtree is drawn once however many ways there are into it.
#[test]
fn a_bead_drawn_twice_in_one_tree_rests_open_on_the_first_line_and_shut_on_the_second() {
    let forest = under_every_copy(drawn_twice_in_one_tree());
    let [upper, lower] = copies_of(&forest, "dun-9");

    assert_eq!(
        (forest.lines()[upper].folded, forest.lines()[lower].folded),
        (Some(true), Some(false)),
        "{:#?}",
        sketch(&forest)
    );
}

/// Shut, never absent: the later line is a way into the same subtree, and
/// opening it draws the subtree there too.
#[test]
fn the_second_line_of_a_bead_drawn_twice_opens_onto_the_same_subtree() {
    let mut forest = under_every_copy(drawn_twice_in_one_tree());
    let [_, lower] = copies_of(&forest, "dun-9");
    assert_eq!(
        lines_of(&forest, "dun-9.1").len(),
        1,
        "{:#?}",
        sketch(&forest)
    );

    step_onto(&mut forest, lower);
    forest.apply(Action::ToggleFold);

    assert_eq!(
        lines_of(&forest, "dun-9.1").len(),
        2,
        "{:#?}",
        sketch(&forest)
    );
}

/// `D` lets go of every fold set by hand, so a reader who opened a later
/// line gets it back the way `bdi` would have drawn it.
#[test]
fn letting_go_of_the_folds_shuts_a_second_line_a_reader_opened() {
    let mut forest = under_every_copy(drawn_twice_in_one_tree());
    let [_, lower] = copies_of(&forest, "dun-9");
    step_onto(&mut forest, lower);
    forest.apply(Action::ToggleFold);

    forest.apply(Action::RestoreDefault);

    let [upper, lower] = copies_of(&forest, "dun-9");
    assert_eq!(
        (forest.lines()[upper].folded, forest.lines()[lower].folded),
        (Some(true), Some(false)),
        "{:#?}",
        sketch(&forest)
    );
}

/// The rule that opens the spine begins afresh at the node a scope is set
/// on. Under every copy, a second copy of a bead rests shut
/// because the way down to it is not the first; a scope set on that copy
/// makes it the first the rule has seen, and it rests open as the upper
/// one does.
#[test]
fn a_rule_scoped_to_a_second_copy_of_a_bead_begins_afresh_there() {
    let mut forest = under_every_copy(drawn_twice_in_one_tree());
    let [_, lower] = copies_of(&forest, "dun-9");
    let scoped = forest.lines()[lower]
        .place
        .clone()
        .expect("a bead line stands on a place");

    forest.spines.insert(Handle::Bead(scoped), Spine::EveryCopy);
    forest.answer();
    forest.lay_out();

    let [upper, lower] = copies_of(&forest, "dun-9");
    assert_eq!(
        (forest.lines()[upper].folded, forest.lines()[lower].folded),
        (Some(true), Some(true)),
        "{:#?}",
        sketch(&forest)
    );
    assert_eq!(
        lines_of(&forest, "dun-9.1").len(),
        2,
        "{:#?}",
        sketch(&forest)
    );
}

/// Every copy puts every copy of a bead on the spine, so a bead both halves of an epic wait on is drawn under each of
/// them and the work beneath it is drawn under the first.
#[test]
fn every_copy_of_a_bead_two_siblings_wait_on_is_opened_to() {
    let forest = under_every_copy(a_blocker_both_halves_wait_on("dun-1.1"));

    assert_eq!(
        lines_of(&forest, "dun-1").len(),
        2,
        "{:#?}",
        sketch(&forest)
    );
    for id in ["dun-2.1", "dun-2.2"] {
        assert_eq!(
            forest.lines()[lines_of(&forest, id)[0]].folded,
            Some(true),
            "{id} rests open: {:#?}",
            sketch(&forest)
        );
    }
}

/// A one-copy rule opens each bead that earns a fold on one way down, so
/// the same tree draws that bead once. The half off the way chosen stands
/// over the same agent and says so, which is what keeps the rule from
/// hiding one.
#[test]
fn a_one_copy_rule_opens_one_way_down_and_the_other_half_says_what_it_is_shut_over() {
    let mut forest = flatten(a_blocker_both_halves_wait_on("dun-1.1"));

    put_in_force(&mut forest, Spine::Deepest, Action::CycleSpineForest);

    assert_eq!(
        lines_of(&forest, "dun-1").len(),
        1,
        "{:#?}",
        sketch(&forest)
    );
    assert_eq!(
        forest.lines()[lines_of(&forest, "dun-2.1")[0]].folded,
        Some(true),
        "the deepest way down to dun-1 goes through dun-2.1: {:#?}",
        sketch(&forest)
    );
    assert_eq!(
        forest.lines()[lines_of(&forest, "dun-2.2")[0]].folded,
        Some(false),
        "{:#?}",
        sketch(&forest)
    );
    assert_eq!(
        row_of(&forest, "dun-2.2")
            .shut_over
            .as_ref()
            .map(|beneath| beneath.live_agents),
        Some(1),
        "{:#?}",
        sketch(&forest)
    );
}

/// Each rule chooses its own way down to the same bead, and the screen is
/// the way it chose: the bead that waits on it in fewest steps under
/// shallowest, the earlier of the two siblings under first reached, its
/// own parent under parent-child, and the bead that waits on it furthest
/// down under deepest. Every copy opens all four at once, which is the
/// screen the one-copy rules were written against.
#[test]
fn each_rule_opens_the_way_down_it_chose_and_rests_the_others_shut() {
    for (rule, open) in [
        (
            Spine::EveryCopy,
            &[
                "bel-1",
                "bel-1.1",
                "bel-1.1.1",
                "bel-1.1.2",
                "bel-1.1.3",
                "bel-1.1.3.1",
            ][..],
        ),
        (Spine::FirstReached, &["bel-1", "bel-1.1", "bel-1.1.1"]),
        (Spine::Shallowest, &["bel-1", "bel-1.1"]),
        (Spine::ParentChild, &["bel-1", "bel-1.1", "bel-1.1.2"]),
        (
            Spine::Deepest,
            &["bel-1", "bel-1.1", "bel-1.1.3", "bel-1.1.3.1"],
        ),
    ] {
        let mut forest = flatten(four_ways_to_the_agent());

        put_in_force(&mut forest, rule, Action::CycleSpineForest);

        assert_eq!(
            resting_open(&forest),
            open,
            "under {rule:?}: {:#?}",
            sketch(&forest)
        );
    }
}

/// A fresh forest is under deepest before the reader presses anything,
/// so the one copy of a bead it opens is the one furthest down.
#[test]
fn a_fresh_forest_opens_each_beads_deepest_copy() {
    let forest = flatten(four_ways_to_the_agent());

    assert_eq!(forest.spine(), Spine::Deepest);
    assert_eq!(
        resting_open(&forest),
        ["bel-1", "bel-1.1", "bel-1.1.3", "bel-1.1.3.1"],
        "{:#?}",
        sketch(&forest)
    );
}

/// Starting under deepest takes no rule out of the reader's reach: `s`
/// comes round to every one of them and back to deepest.
#[test]
fn cycling_from_the_rule_a_forest_starts_under_reaches_every_rule() {
    let mut forest = flatten(four_ways_to_the_agent());
    let mut reached = Vec::new();

    for _ in Spine::EVERY {
        forest.apply(Action::CycleSpineForest);
        reached.push(forest.spine());
    }

    assert_eq!(reached.last(), Some(&Spine::Deepest));
    reached.sort_by_key(|rule| Spine::EVERY.iter().position(|each| each == rule));
    assert_eq!(reached, Spine::EVERY);
}

/// Parent-child falls back to the way the walk placed a bead on where the
/// rule was set below that bead's parent. The bead is still work a reader
/// needs, so the rule opens the one way down to it there is from here —
/// the way that under the same rule over the whole forest rests shut.
#[test]
fn parent_child_set_below_a_beads_parent_opens_the_way_the_walk_placed_it_on() {
    let mut forest = flatten(four_ways_to_the_agent());
    let at = lines_of(&forest, "bel-1.1.1")[0];
    step_onto(&mut forest, at);

    put_in_force(&mut forest, Spine::ParentChild, Action::CycleSpine);

    assert_eq!(
        forest.lines()[lines_of(&forest, "bel-1.1.1")[0]].folded,
        Some(true),
        "{:#?}",
        sketch(&forest)
    );
}

/// Every rule leaves a way down to the agent open, even where a bead's
/// own parent hangs beneath it. Parent-child keeps the way the walk
/// placed such a bead on: hanging it back under its parent would ring the
/// two of them off from everything above, and the agent would sit behind
/// a fold on a line that is drawn wherever the reader looks.
#[test]
fn a_bead_whose_parent_hangs_beneath_it_is_still_opened_to() {
    for rule in Spine::EVERY.iter().copied() {
        let mut forest = flatten(alone("dunwich", PARENT_BENEATH, &panes_on(&["cyc-2.3"])));

        put_in_force(&mut forest, rule, Action::CycleSpineForest);

        assert!(
            !lines_of(&forest, "cyc-2.3").is_empty(),
            "under {rule:?} the agent's bead is drawn nowhere: {:#?}",
            sketch(&forest)
        );
    }
}

/// The key cycles, so pressing it once per rule comes back to the screen
/// it started on.
#[test]
fn cycling_the_rule_through_every_one_puts_the_screen_back() {
    let mut forest = flatten(a_blocker_both_halves_wait_on("dun-1.1"));
    let was = sketch(&forest);

    for _ in Spine::EVERY {
        forest.apply(Action::CycleSpineForest);
    }

    assert_eq!(sketch(&forest), was);
}

/// `s` puts the rule in force under the selected node alone. A tree it
/// was not pressed in is drawn exactly as it was, which is what tells it
/// from `S`.
#[test]
fn a_rule_set_under_one_node_leaves_the_rest_of_the_forest_alone() {
    let two_roots = || {
        together(
            "dunwich",
            &[BLOCKS_BOTH, TWICE],
            &panes_on(&["dun-1.1", "dun-9.1"]),
        )
    };
    // The second root and everything under it, which is every line from
    // its own down: the rule is set in the first, so nothing here is in
    // the subtree it was set on.
    let other = |forest: &Forest| sketch(forest)[lines_of(forest, "dun-8")[0]..].to_vec();
    let mut forest = under_every_copy(two_roots());
    let elsewhere = other(&forest);

    let at = lines_of(&forest, "dun-2")[0];
    step_onto(&mut forest, at);
    assert!(forest.apply(Action::CycleSpine));

    assert_eq!(
        lines_of(&forest, "dun-1").len(),
        1,
        "{:#?}",
        sketch(&forest)
    );
    assert_eq!(
        other(&forest),
        elsewhere,
        "the other tree moved: {:#?}",
        sketch(&forest)
    );
}

/// The rule is derived from the snapshot on every refresh rather than
/// written down as folds, so the way it opens follows the agents as they
/// move. Here the agent leaves the bead both halves wait on for one under
/// the half that was resting shut, and the two halves change places.
#[test]
fn the_way_chosen_opens_to_the_agent_wherever_a_refresh_puts_it() {
    let mut forest = flatten(a_blocker_both_halves_wait_on("dun-1.1"));
    assert!(forest.apply(Action::CycleSpineForest));

    forest.refresh(a_blocker_both_halves_wait_on("dun-2.2.1"));

    assert_eq!(
        (
            forest.lines()[lines_of(&forest, "dun-2.1")[0]].folded,
            forest.lines()[lines_of(&forest, "dun-2.2")[0]].folded,
        ),
        (Some(false), Some(true)),
        "{:#?}",
        sketch(&forest)
    );
}

/// A looped tree is drawn under every rule. The ways a one-copy rule
/// chooses are the longest that skip a link back onto the way down, which
/// is the same cut every walk here makes, so a loop costs a degraded
/// answer rather than no answer.
#[test]
fn a_looped_tree_draws_under_every_rule() {
    let mut forest = flatten(alone("dunwich", LOOPED, &panes_on(&["cyc-1.1"])));

    for _ in Spine::EVERY {
        assert!(!sketch(&forest).is_empty());
        assert!(
            lines_of(&forest, "cyc-1.1").len() == 1,
            "{:#?}",
            sketch(&forest)
        );
        forest.apply(Action::CycleSpineForest);
    }
}

/// The rule and the folds the reader set by hand are separate things.
/// With every fold pointed shut, the rule moves no row; `D` lets go of
/// the folds and what the forest rests as is the new rule's.
#[test]
fn cycling_the_rule_moves_no_fold_the_reader_set_by_hand() {
    let mut forest = under_every_copy(a_blocker_both_halves_wait_on("dun-1.1"));
    forest.apply(Action::CollapseForest);
    let shut = sketch(&forest);

    forest.apply(Action::CycleSpineForest);
    assert_eq!(sketch(&forest), shut);

    assert!(forest.apply(Action::RestoreDefault));
    assert_eq!(
        lines_of(&forest, "dun-1").len(),
        1,
        "{:#?}",
        sketch(&forest)
    );
}

/// The rule is on the screen as well as in the rows, so a press that
/// moves it asks for the screen back even where every row is where it
/// was. Under a forest folded shut by hand the rows cannot move, and the
/// foot would have gone on naming the rule the reader had just left.
#[test]
fn cycling_the_rule_asks_for_the_screen_back_where_no_row_moves() {
    let mut forest = under_every_copy(a_blocker_both_halves_wait_on("dun-1.1"));
    forest.apply(Action::CollapseForest);
    let shut = sketch(&forest);

    assert!(forest.apply(Action::CycleSpineForest));

    assert_eq!(sketch(&forest), shut);
    assert_eq!(forest.spine(), Spine::EveryCopy.next());
}

/// The screen says which rule is in force at the selection, and the
/// selection is what it is said at: a node a reader has put a rule in
/// force under says that rule, and a node outside it says the forest's.
#[test]
fn the_rule_in_force_is_the_one_set_on_the_nearest_line_at_or_above_the_selection() {
    let mut forest = under_every_copy(a_blocker_both_halves_wait_on("dun-1.1"));
    let at = lines_of(&forest, "dun-2.1")[0];
    step_onto(&mut forest, at);
    forest.apply(Action::CycleSpine);

    assert_eq!(forest.spine(), Spine::EveryCopy.next());

    let at = lines_of(&forest, "dun-2.2")[0];
    step_onto(&mut forest, at);
    assert_eq!(forest.spine(), Spine::EveryCopy);
}

/// A shut line says what it is shut over on every copy of its bead. The
/// copy a reader is looking at is often not the one the walk reached first,
/// and a shut line saying nothing reads as a fold hiding nothing.
#[test]
fn every_line_of_a_bead_drawn_twice_says_what_it_is_shut_over() {
    let forest = flatten(closed_bead_drawn_twice_in_one_tree());
    let [upper, lower] = copies_of(&forest, "dun-4");

    assert_eq!(
        (notes_at(&forest, upper), notes_at(&forest, lower)),
        (
            vec![phrase::unfinished_beneath(1)],
            vec![phrase::unfinished_beneath(1)]
        ),
        "{:#?}",
        sketch(&forest)
    );
}

/// What a bead row says beyond its own fields, on one line.
fn notes_at(forest: &Forest, at: usize) -> Vec<String> {
    match &forest.lines()[at].content {
        Content::Bead(row) => row.notes.clone(),
        content => panic!("line {at} is not a bead row: {content:?}"),
    }
}

/// The tracker is written while the list is being read, so a refresh can
/// land between any two keystrokes. It re-derives the selection from the
/// handle it holds, and must settle on the copy the selection was on
/// rather than on that copy's twin higher up the list.
#[test]
fn a_refresh_between_steps_does_not_pull_the_selection_back_to_a_twin() {
    let panes = panes_on(&["qua-1.2", "wha-2.1"]);
    let mut forest = flatten(overlapping(&panes));
    let drawn = forest.lines().len();

    forest.apply(Action::Move(Motion::FirstRow));
    let mut visited = vec![forest.selected_line()];
    for _ in 1..drawn {
        forest.apply(Action::Move(Motion::NextRow));
        forest.refresh(overlapping(&panes));
        visited.push(forest.selected_line());
    }

    assert_eq!(
        visited,
        (0..drawn).collect::<Vec<usize>>(),
        "{:#?}",
        sketch(&forest)
    );
}
