use super::*;
use pretty_assertions::assert_eq;

/// Every bead on screen, by id, in render order. A bead reachable more
/// than once is here once per copy drawn.
fn drawn_beads(forest: &Forest) -> Vec<String> {
    forest
        .lines()
        .iter()
        .filter_map(|line| line.bead().map(|key| key.id.clone()))
        .collect()
}

/// Opening a node draws children that were not there to be enumerated
/// when the key was pressed, so one pass over the lines stops at the
/// first level it opened. Tower is a spine four deep with nothing live
/// in it, so every level below the header is a fold no reader could see.
#[test]
fn expanding_reaches_a_fold_that_was_not_drawn_when_it_was_pressed() {
    let mut forest = flatten(tower_staffed(&[]));
    assert_eq!(drawn_beads(&forest), ["tow-1"], "{:#?}", sketch(&forest));

    forest.apply(Action::ExpandSubtree);

    assert_eq!(
        drawn_beads(&forest),
        [
            "tow-1",
            "tow-1.1",
            "tow-1.1.1",
            "tow-1.1.1.1",
            "tow-1.2",
            "tow-1.2.1"
        ],
        "{:#?}",
        sketch(&forest)
    );
}

/// A fold beneath a shut fold is on no line the screen draws, and a walk
/// that pointed only what was drawn cost a draw per level to reach it.
/// Tower is a spine four deep with nothing live in it, so every fold
/// under the header is one the reader could not see, and one walk
/// reaches the deepest of them in one draw.
#[test]
fn one_walk_reaches_a_fold_nested_several_shut_folds_deep_in_one_draw() {
    let mut forest = flatten(tower_staffed(&[]));
    assert_eq!(drawn_beads(&forest), ["tow-1"], "{:#?}", sketch(&forest));
    let scope = forest.handle_at(forest.selected).expect("a selected line");
    let before = layout::draws_so_far();

    forest.fold_in(Some(&scope), true);

    assert_eq!(layout::draws_so_far() - before, 1);
    forest.lay_out();
    assert_eq!(
        drawn_beads(&forest),
        [
            "tow-1",
            "tow-1.1",
            "tow-1.1.1",
            "tow-1.1.1.1",
            "tow-1.2",
            "tow-1.2.1"
        ],
        "{:#?}",
        sketch(&forest)
    );
}

/// `e` holds every fold beneath it open as the key set it, the ones it
/// found resting open included, so a fold that was open because of the
/// agent beneath it stays open when that agent moves. The spine to
/// `tow-1.1.1.1` rested open while the agent was there, and is still
/// open once it has moved to `tow-1.2.1`.
#[test]
fn expanding_holds_a_fold_it_found_resting_open_across_a_refresh() {
    let mut forest = flatten(tower_staffed(&["tow-1.1.1.1"]));
    forest.apply(Action::ExpandSubtree);

    forest.refresh(tower_staffed(&["tow-1.2.1"]));

    assert_eq!(
        drawn_beads(&forest),
        [
            "tow-1",
            "tow-1.1",
            "tow-1.1.1",
            "tow-1.1.1.1",
            "tow-1.2",
            "tow-1.2.1"
        ],
        "{:#?}",
        sketch(&forest)
    );
}

/// The place a bead is drawn on, taken off the screen so the test does
/// not spell the way down to it by hand.
fn place_of_line(forest: &Forest, id: &str) -> Place {
    line_of(forest, id)
        .place
        .clone()
        .expect("a bead line stands on a place")
}

/// `e` writes one entry, on the line it was pressed on, saying everything
/// beneath it opens. Nothing is written per line: the spine to
/// `tow-1.1.1.1` rests open and `tow-1.2` rested shut, and neither is
/// named.
#[test]
fn expanding_a_subtree_holds_one_entry() {
    let mut forest = flatten(tower_staffed(&["tow-1.1.1.1"]));
    forest.apply(Action::ExpandSubtree);

    let root = Handle::Bead(place_of_line(&forest, "tow-1"));
    assert_eq!(
        forest.folds.entries().collect::<Vec<_>>(),
        [(
            &root,
            &Fold {
                line: None,
                scope: Some(handle::Scope::Points { open: true }),
            }
        )],
        "{:#?}",
        sketch(&forest)
    );
}

/// `E` writes one entry per line at the top of the forest — a project or
/// a group below the trees — and nothing under any of them, so the map
/// is the width of the forest and not the number of lines it opened.
#[test]
fn expanding_the_forest_holds_one_entry_per_line_at_the_top() {
    let mut forest = flatten(built(Filter::All));

    forest.apply(Action::ExpandForest);

    let top: BTreeSet<Handle> = forest
        .lines()
        .iter()
        .filter(|line| line.depth == 0 && line.folded.is_some())
        .filter_map(handle_of)
        .collect();
    assert!(top.len() < forest.lines().len(), "{:#?}", sketch(&forest));
    assert_eq!(
        forest
            .folds
            .entries()
            .map(|(handle, _)| handle.clone())
            .collect::<BTreeSet<_>>(),
        top,
        "{:#?}",
        sketch(&forest)
    );
}

/// A shut scope spent by live work arriving beneath it lets go of the way
/// down to that work and nothing else: `tow-1.2` was folded away and
/// nothing new is under it, so it stays shut while `tow-1.1` opens onto
/// the agent that arrived on `tow-1.1.1`. `tow-1.1.1` itself has nothing
/// new beneath it and stays shut over `tow-1.1.1.1`.
#[test]
fn a_shut_scope_is_spent_only_along_the_way_down_to_what_arrived() {
    let mut forest = flatten(tower_staffed(&["tow-1.1.1.1", "tow-1.2.1"]));
    forest.apply(Action::CollapseSubtree);
    assert_eq!(drawn_beads(&forest), ["tow-1"], "{:#?}", sketch(&forest));

    forest.refresh(tower_staffed(&["tow-1.1.1", "tow-1.1.1.1", "tow-1.2.1"]));

    assert_eq!(
        drawn_beads(&forest),
        ["tow-1", "tow-1.1", "tow-1.1.1", "tow-1.2"],
        "{:#?}",
        sketch(&forest)
    );
}

/// `d` under a scope puts the node and everything beneath it back to
/// resting, and leaves the scope standing over its siblings: `tow-1.2`
/// stays shut by the `c` on the root, while `tow-1.1` rests open onto
/// the agent under it.
#[test]
fn restoring_the_default_under_a_scope_leaves_the_scope_over_the_siblings() {
    let mut forest = flatten(tower_staffed(&["tow-1.1.1.1"]));
    forest.apply(Action::CollapseSubtree);
    forest.apply(Action::ToggleFold);
    assert_eq!(
        drawn_beads(&forest),
        ["tow-1", "tow-1.1", "tow-1.2"],
        "{:#?}",
        sketch(&forest)
    );

    select_bead(&mut forest, "tow-1.1");
    forest.apply(Action::RestoreSubtree);

    assert_eq!(
        drawn_beads(&forest),
        ["tow-1", "tow-1.1", "tow-1.1.1", "tow-1.1.1.1", "tow-1.2"],
        "{:#?}",
        sketch(&forest)
    );
}

/// A scope that shut is still spent by what arrives under it when the
/// reader has opened the line it was set on by hand: the line stays as
/// they opened it, the way down to the agent that arrived on `tow-1.1.1`
/// opens, and `tow-1.2` stays shut beside it.
#[test]
fn a_shut_scope_under_a_line_opened_by_hand_is_still_spent_beneath_it() {
    let mut forest = flatten(tower_staffed(&["tow-1.1.1.1", "tow-1.2.1"]));
    forest.apply(Action::CollapseSubtree);
    forest.apply(Action::ToggleFold);
    assert_eq!(
        drawn_beads(&forest),
        ["tow-1", "tow-1.1", "tow-1.2"],
        "{:#?}",
        sketch(&forest)
    );

    forest.refresh(tower_staffed(&["tow-1.1.1", "tow-1.1.1.1", "tow-1.2.1"]));

    assert_eq!(
        drawn_beads(&forest),
        ["tow-1", "tow-1.1", "tow-1.1.1", "tow-1.2"],
        "{:#?}",
        sketch(&forest)
    );
}

/// A fold the scope pointed open and the reader then shut by hand, once
/// spent, follows the default rather than the scope: `tow-1.2` opens
/// onto the agent that arrives on `tow-1.2.1` and shuts again once that
/// agent has gone, as a fold the reader never touched would.
#[test]
fn a_fold_shut_by_hand_under_an_open_scope_rests_once_it_is_spent() {
    let mut forest = flatten(tower_staffed(&["tow-1.1.1.1"]));
    forest.apply(Action::ExpandSubtree);
    toggle_fold_of(&mut forest, "tow-1.2");
    assert_eq!(
        drawn_beads(&forest),
        ["tow-1", "tow-1.1", "tow-1.1.1", "tow-1.1.1.1", "tow-1.2"],
        "{:#?}",
        sketch(&forest)
    );

    forest.refresh(tower_staffed(&["tow-1.1.1.1", "tow-1.2.1"]));
    assert_eq!(
        drawn_beads(&forest),
        [
            "tow-1",
            "tow-1.1",
            "tow-1.1.1",
            "tow-1.1.1.1",
            "tow-1.2",
            "tow-1.2.1"
        ],
        "{:#?}",
        sketch(&forest)
    );

    forest.refresh(tower_staffed(&["tow-1.1.1.1"]));
    assert_eq!(
        drawn_beads(&forest),
        ["tow-1", "tow-1.1", "tow-1.1.1", "tow-1.1.1.1", "tow-1.2"],
        "{:#?}",
        sketch(&forest)
    );
}

/// A run is the one line whose scope reaches beads its handle does not
/// name the way down to, so `d` inside it is where a scope set on it
/// could be missed: `dep-1.2` was opened by `e` on the run and rests
/// shut again when `d` is pressed on it, while the run stays open.
#[test]
fn restoring_the_default_inside_an_expanded_run_shuts_the_branch_again() {
    let mut forest = flatten(depot());
    select_run(&mut forest);
    forest.apply(Action::ExpandSubtree);
    assert!(
        drawn_beads(&forest).contains(&"dep-1.2.1".to_string()),
        "{:#?}",
        sketch(&forest)
    );

    select_bead(&mut forest, "dep-1.2");
    forest.apply(Action::RestoreSubtree);

    let drawn = drawn_beads(&forest);
    assert!(
        drawn.contains(&"dep-1.2".to_string()),
        "{:#?}",
        sketch(&forest)
    );
    assert!(
        !drawn.contains(&"dep-1.2.1".to_string()),
        "{:#?}",
        sketch(&forest)
    );
}

/// Tower with `tow-1.1.1.1` gone, so `tow-1.1.1` is a leaf with no fold.
fn tower_without_cables() -> Snapshot {
    alone("dunwich", &without(TOWER, "tow-1.1.1.1"), &[])
}

/// A run goes when its finished siblings drop below three, and what `e`
/// on it opened stays open outside it: `dep-1.2` was opened by `e` on
/// the run and is still open onto `dep-1.2.1` once `dep-1.4` has gone
/// and the run with it.
#[test]
fn a_branch_expanded_from_a_run_stays_open_once_the_run_has_gone() {
    let mut forest = flatten(depot());
    select_run(&mut forest);
    forest.apply(Action::ExpandSubtree);
    assert!(
        drawn_beads(&forest).contains(&"dep-1.2.1".to_string()),
        "{:#?}",
        sketch(&forest)
    );

    forest.refresh(alone(
        "dunwich",
        &without(DEPOT, "dep-1.4"),
        &panes_on(&["dep-1.1"]),
    ));

    let under_the_root = place_of_line(&forest, "dep-1");
    assert!(
        !forest.lines().iter().any(
            |line| matches!(&line.content, Content::Elided { under, .. } if *under == under_the_root)
        ),
        "the run has gone: {:#?}",
        sketch(&forest)
    );
    assert!(
        drawn_beads(&forest).contains(&"dep-1.2.1".to_string()),
        "{:#?}",
        sketch(&forest)
    );
}

/// A fold set on a line whose children have since gone is still a fold,
/// so `e` on a line above it takes it with everything else beneath: when
/// the children come back, the line answers from the scope and not from
/// the fold the reader shut before they went.
#[test]
fn expanding_takes_a_fold_whose_line_has_no_children_today() {
    let mut forest = flatten(tower_staffed(&["tow-1.1.1.1"]));
    toggle_fold_of(&mut forest, "tow-1.1.1");
    forest.refresh(tower_without_cables());
    assert_eq!(drawn_beads(&forest), ["tow-1"], "{:#?}", sketch(&forest));

    select_bead(&mut forest, "tow-1");
    forest.apply(Action::ExpandSubtree);
    forest.refresh(tower_staffed(&["tow-1.1.1.1"]));

    assert!(
        drawn_beads(&forest).contains(&"tow-1.1.1.1".to_string()),
        "{:#?}",
        sketch(&forest)
    );
}

/// A scope that shut is spent along the way down to what arrived while
/// the forest is rooted at a bead beneath it, when the mode draws the
/// scope's own line behind the rest, shut in the group of roots it is
/// holding back, and the rooted bead's branch apart from it: `tow-1.1` opens onto the agent that arrived on `tow-1.1.1`,
/// rooted and put back alike, and `tow-1.2` stays shut.
#[test]
fn a_shut_scope_is_spent_beneath_the_bead_the_forest_is_rooted_at() {
    let mut forest = flatten(tower_staffed(&["tow-1.1.1.1", "tow-1.2.1"]));
    forest.apply(Action::CollapseSubtree);
    forest.apply(Action::ToggleFold);
    focus_on(&mut forest, "tow-1.1");
    assert_eq!(drawn_beads(&forest), ["tow-1.1"], "{:#?}", sketch(&forest));

    forest.refresh(tower_staffed(&["tow-1.1.1", "tow-1.1.1.1", "tow-1.2.1"]));
    assert_eq!(
        drawn_beads(&forest),
        ["tow-1.1", "tow-1.1.1"],
        "{:#?}",
        sketch(&forest)
    );

    forest.apply(Action::FocusForest);
    assert_eq!(
        drawn_beads(&forest),
        ["tow-1", "tow-1.1", "tow-1.1.1", "tow-1.2"],
        "{:#?}",
        sketch(&forest)
    );
}

/// What `e` on a run opened stays open when the forest is rooted at a
/// bead inside that run: `dep-1.2` was opened by `e` on the run it hangs
/// in, and stays open onto `dep-1.2.1` when the forest is rooted at it.
#[test]
fn rooting_the_forest_inside_an_expanded_run_keeps_it_open() {
    let mut forest = flatten(depot());
    select_run(&mut forest);
    forest.apply(Action::ExpandSubtree);
    assert!(
        drawn_beads(&forest).contains(&"dep-1.2.1".to_string()),
        "{:#?}",
        sketch(&forest)
    );

    focus_on(&mut forest, "dep-1.2");

    assert!(
        drawn_beads(&forest).contains(&"dep-1.2.1".to_string()),
        "{:#?}",
        sketch(&forest)
    );
}

/// Spending a scope that shut lets go of the shut folds on the way down
/// to what arrived, and a fold a nearer scope opened is not one of them:
/// it holds nothing back. `tow-1.1` and `tow-1.1.1` were opened by `e`
/// under the `c` on the root, and stay open after an agent has come and
/// gone beneath them.
#[test]
fn spending_a_shut_scope_leaves_a_fold_a_nearer_scope_opened_open() {
    let mut forest = flatten(tower_staffed(&["tow-1.2.1"]));
    forest.apply(Action::CollapseSubtree);
    forest.apply(Action::ToggleFold);
    select_bead(&mut forest, "tow-1.1");
    forest.apply(Action::ExpandSubtree);
    let expanded = drawn_beads(&forest);
    assert_eq!(
        expanded,
        ["tow-1", "tow-1.1", "tow-1.1.1", "tow-1.1.1.1", "tow-1.2"],
        "{:#?}",
        sketch(&forest)
    );

    forest.refresh(tower_staffed(&["tow-1.1.1.1", "tow-1.2.1"]));
    forest.refresh(tower_staffed(&["tow-1.2.1"]));

    assert_eq!(drawn_beads(&forest), expanded, "{:#?}", sketch(&forest));
}

/// Put the selection on the line over the roots the mode is holding back.
fn select_out_of_the_way(forest: &mut Forest) {
    let at = forest
        .lines()
        .iter()
        .position(|line| {
            matches!(&line.content, Content::Group(group) if group.kind == GroupKind::OutOfTheWay)
        })
        .unwrap_or_else(|| panic!("no roots are held back: {:#?}", sketch(forest)));
    step_onto(forest, at);
}

/// The line over the roots the mode is holding back is drawn only while
/// the forest is rooted, and `c` on it shuts folds that are spent by what
/// arrives under them as any other is: `tow-1.1` opens onto the agent
/// that arrived on `tow-1.1.1`, under the root the reader opened again.
#[test]
fn a_fold_shut_from_the_held_back_roots_line_is_spent_by_what_arrives_under_it() {
    let mut forest = flatten(tower_staffed(&["tow-1.1.1.1", "tow-1.2.1"]));
    focus_on(&mut forest, "tow-1.2");
    select_out_of_the_way(&mut forest);
    forest.apply(Action::CollapseSubtree);
    forest.apply(Action::ToggleFold);
    toggle_fold_of(&mut forest, "tow-1");
    assert_eq!(
        drawn_beads(&forest),
        ["tow-1.2", "tow-1.2.1", "tow-1", "tow-1.1"],
        "{:#?}",
        sketch(&forest)
    );

    forest.refresh(tower_staffed(&["tow-1.1.1", "tow-1.1.1.1", "tow-1.2.1"]));

    assert_eq!(
        drawn_beads(&forest),
        ["tow-1.2", "tow-1.2.1", "tow-1", "tow-1.1", "tow-1.1.1"],
        "{:#?}",
        sketch(&forest)
    );
}

/// Nothing stands under the line over the held-back roots once the
/// forest is put back, so what `c` on it shut has to be held by the
/// folds themselves: `tow-1.1` stays shut over the agent beneath it.
/// `tow-1` is opened by the key that puts the forest back, which leaves
/// the selection on the bead it was rooted at.
#[test]
fn a_fold_shut_from_the_held_back_roots_line_outlives_putting_the_forest_back() {
    let mut forest = flatten(tower_staffed(&["tow-1.1.1.1", "tow-1.2.1"]));
    focus_on(&mut forest, "tow-1.2");
    select_out_of_the_way(&mut forest);
    forest.apply(Action::CollapseSubtree);

    forest.apply(Action::FocusForest);

    assert_eq!(
        drawn_beads(&forest),
        ["tow-1", "tow-1.1", "tow-1.2", "tow-1.2.1"],
        "{:#?}",
        sketch(&forest)
    );
}

/// The bead the forest is rooted at is drawn apart from the root the
/// mode holds back, so `c` on that root reaches what is drawn beneath
/// it there and not the rooted bead: `tow-1.2` stays open onto
/// `tow-1.2.1` after `c` on `tow-1` behind the held-back roots line.
#[test]
fn shutting_a_held_back_root_leaves_the_bead_the_forest_is_rooted_at_open() {
    let mut forest = flatten(tower_staffed(&["tow-1.1.1.1", "tow-1.2.1"]));
    focus_on(&mut forest, "tow-1.2");
    select_out_of_the_way(&mut forest);
    forest.apply(Action::ToggleFold);
    select_bead(&mut forest, "tow-1");
    forest.apply(Action::CollapseSubtree);

    assert_eq!(
        drawn_beads(&forest),
        ["tow-1.2", "tow-1.2.1", "tow-1"],
        "{:#?}",
        sketch(&forest)
    );
}

/// `d` on that root puts back what is drawn beneath it there and no
/// more: `tow-1.2`, shut by `c` on `tow-1` before the forest was rooted
/// at it, stays shut after `d` on `tow-1` behind the held-back roots
/// line, where `tow-1` itself rests shut as every root there does.
#[test]
fn restoring_a_held_back_root_leaves_the_bead_the_forest_is_rooted_at_as_it_was() {
    let mut forest = flatten(tower_staffed(&["tow-1.1.1.1", "tow-1.2.1"]));
    forest.apply(Action::CollapseSubtree);
    forest.apply(Action::ToggleFold);
    focus_on(&mut forest, "tow-1.2");
    assert_eq!(drawn_beads(&forest), ["tow-1.2"], "{:#?}", sketch(&forest));
    select_out_of_the_way(&mut forest);
    forest.apply(Action::ToggleFold);
    select_bead(&mut forest, "tow-1");
    forest.apply(Action::RestoreSubtree);

    assert_eq!(
        drawn_beads(&forest),
        ["tow-1.2", "tow-1"],
        "{:#?}",
        sketch(&forest)
    );
}

/// Rooting the forest at a bead in a tree the filter is holding back
/// draws that tree under its project, where its group would have been,
/// and what `e` on the group opened stays open there: `hbr-3` stays
/// open onto `hbr-3.1` when the forest is rooted at it.
#[test]
fn rooting_the_forest_in_an_expanded_hidden_tree_keeps_it_open() {
    let mut forest = flatten(snapshot());
    let group = forest
        .lines()
        .iter()
        .position(|line| {
            matches!(&line.content, Content::Group(group) if group.kind == GroupKind::HiddenTrees)
        })
        .expect("the filter hid a tree");
    step_onto(&mut forest, group);
    forest.apply(Action::ExpandSubtree);
    assert!(
        drawn_beads(&forest).contains(&"hbr-3.1".to_string()),
        "{:#?}",
        sketch(&forest)
    );

    focus_on(&mut forest, "hbr-3");

    assert!(
        drawn_beads(&forest).contains(&"hbr-3.1".to_string()),
        "{:#?}",
        sketch(&forest)
    );
}

/// Showing every tree takes the trees the filter was holding back out
/// of their group and draws them under the project, and what `e` on the
/// group opened stays open there: `hbr-3` is still open onto `hbr-3.1`.
#[test]
fn showing_every_tree_keeps_what_was_expanded_from_the_hidden_trees_group_open() {
    let mut forest = flatten(snapshot());
    let group = forest
        .lines()
        .iter()
        .position(|line| {
            matches!(&line.content, Content::Group(group) if group.kind == GroupKind::HiddenTrees)
        })
        .expect("the filter hid a tree");
    step_onto(&mut forest, group);
    forest.apply(Action::ExpandSubtree);

    forest.apply(Action::ToggleFilter);

    assert!(
        drawn_beads(&forest).contains(&"hbr-3.1".to_string()),
        "{:#?}",
        sketch(&forest)
    );
}

/// `d` on the group of trees the filter is holding back puts every fold
/// beneath it back, and they stay put back once the filter shows every
/// tree: `hbr-3`, opened by `E`, rests shut under its project after `d`
/// on the group and `a`.
#[test]
fn showing_every_tree_keeps_what_was_restored_from_the_hidden_trees_group_resting() {
    let mut forest = flatten(snapshot());
    forest.apply(Action::ToggleFilter);
    assert!(
        !drawn_beads(&forest).contains(&"hbr-3.1".to_string()),
        "hbr-3 rests shut under its project: {:#?}",
        sketch(&forest)
    );
    forest.apply(Action::ToggleFilter);

    forest.apply(Action::ExpandForest);
    let group = forest
        .lines()
        .iter()
        .position(|line| {
            matches!(&line.content, Content::Group(group) if group.kind == GroupKind::HiddenTrees)
        })
        .expect("the filter hid a tree");
    step_onto(&mut forest, group);
    forest.apply(Action::RestoreSubtree);
    forest.apply(Action::ToggleFilter);

    assert!(
        !drawn_beads(&forest).contains(&"hbr-3.1".to_string()),
        "{:#?}",
        sketch(&forest)
    );
}

/// The lines the selection stands over, and itself: everything from it to
/// the first line drawn at its own depth or shallower.
///
/// Read off the screen rather than asked of the forest, so a walk that
/// pointed the folds of the wrong lines is answered by the drawing and
/// not by the same reckoning that misplaced them.
fn from_the_selection_down(forest: &Forest) -> impl Iterator<Item = &Line> {
    let at = forest.selected_line();
    let depth = forest.lines()[at].depth;
    forest.lines().iter().skip(at).take(1).chain(
        forest
            .lines()
            .iter()
            .skip(at + 1)
            .take_while(move |line| line.depth > depth),
    )
}

/// Six shapes of tree, each of them the one tree of its project, so the
/// root the selection opens on stands over every fold that tree has.
#[test]
fn expanding_from_a_root_leaves_no_fold_shut_under_it() {
    for json in [DUNWICH, DEPOT, RELAY, SIDING, TOWER, KADATH] {
        let mut forest = flatten(alone("dunwich", json, &two_panes()));
        forest.apply(Action::ExpandSubtree);

        assert!(
            from_the_selection_down(&forest).all(|line| line.folded != Some(false)),
            "{:#?}",
            sketch(&forest)
        );
    }
}

/// The mirror, and the project's own line is what says where the scope
/// stopped: `c` shuts the root it was pressed on and everything that root
/// stands over, and leaves the project above it as the reader had it.
#[test]
fn collapsing_from_a_root_shuts_it_and_leaves_the_project_above_it_open() {
    for json in [DUNWICH, DEPOT, RELAY, SIDING, TOWER, KADATH] {
        let mut forest = flatten(alone("dunwich", json, &two_panes()));
        forest.apply(Action::CollapseSubtree);

        assert!(
            from_the_selection_down(&forest).all(|line| line.folded != Some(true)),
            "{:#?}",
            sketch(&forest)
        );
        assert_eq!(
            forest.lines()[0].folded,
            Some(true),
            "{:#?}",
            sketch(&forest)
        );
    }
}

/// `E` and `C` are the same walks over the whole forest, so a reader on
/// one project's line reaches the others' folds too — the assertion that
/// tells the forest-wide key from the scoped one under it.
#[test]
fn expanding_the_forest_leaves_no_fold_shut_anywhere() {
    let mut forest = flatten(built(Filter::All));
    select_project(&mut forest, "dunwich");

    forest.apply(Action::ExpandForest);

    assert!(
        forest.lines().iter().all(|line| line.folded != Some(false)),
        "{:#?}",
        sketch(&forest)
    );
}

#[test]
fn collapsing_the_forest_leaves_no_fold_open_anywhere() {
    let mut forest = flatten(built(Filter::All));
    forest.apply(Action::ExpandForest);
    select_project(&mut forest, "harbour");

    forest.apply(Action::CollapseForest);

    assert!(
        forest.lines().iter().all(|line| line.folded != Some(true)),
        "{:#?}",
        sketch(&forest)
    );
}

/// `d` spends the hand folds on the selected node and its descendants
/// and no other, so a sibling the reader folded keeps that fold. Two
/// presses on one forest: the first puts a hand-opened node back to
/// resting shut while its hand-shut sibling stays shut, which `D` and
/// `e` would not do; the second puts a hand-shut node back to resting
/// open, which `c` would not do.
#[test]
fn restoring_the_default_under_a_node_leaves_a_sibling_where_the_reader_put_it() {
    let mut forest = flatten(tower_staffed(&["tow-1.1.1.1"]));
    toggle_fold_of(&mut forest, "tow-1.1");
    toggle_fold_of(&mut forest, "tow-1.2");

    select_bead(&mut forest, "tow-1.2");
    forest.apply(Action::RestoreSubtree);
    assert_eq!(
        drawn_beads(&forest),
        ["tow-1", "tow-1.1", "tow-1.2"],
        "{:#?}",
        sketch(&forest)
    );

    select_bead(&mut forest, "tow-1.1");
    forest.apply(Action::RestoreSubtree);
    assert_eq!(
        drawn_beads(&forest),
        ["tow-1", "tow-1.1", "tow-1.1.1", "tow-1.1.1.1", "tow-1.2"],
        "{:#?}",
        sketch(&forest)
    );
}

/// A hand fold under a hand-shut node is out of sight and still a hand
/// fold, so `d` has to reach it there or the node it put back would
/// spring open onto a subtree still shut by hand.
#[test]
fn restoring_the_default_under_a_node_spends_a_fold_the_reader_cannot_see() {
    let mut forest = flatten(tower_staffed(&["tow-1.1.1.1"]));
    toggle_fold_of(&mut forest, "tow-1.1.1");
    toggle_fold_of(&mut forest, "tow-1.1");

    select_bead(&mut forest, "tow-1.1");
    forest.apply(Action::RestoreSubtree);

    assert_eq!(
        drawn_beads(&forest),
        ["tow-1", "tow-1.1", "tow-1.1.1", "tow-1.1.1.1", "tow-1.2"],
        "{:#?}",
        sketch(&forest)
    );
}

/// A run is the one line whose fold draws lines that are not its own
/// children by any other reckoning, so it is the one where opening the
/// scope could plausibly lose it. It does not: the run keeps its line
/// when it opens, and the beads it stood for hang a level under it.
#[test]
fn a_run_keeps_its_scope_through_being_opened_and_shut_again() {
    let mut forest = flatten(snapshot());
    select_run(&mut forest);
    let shut = drawn_beads(&forest);

    assert!(
        forest.apply(Action::ExpandSubtree),
        "{:#?}",
        sketch(&forest)
    );
    let opened = drawn_beads(&forest);
    assert!(
        opened.len() > shut.len(),
        "the run drew nothing when it opened: {:#?}",
        sketch(&forest)
    );
    assert!(
        matches!(
            forest.lines()[forest.selected_line()].content,
            Content::Elided { .. }
        ),
        "the selection came off the run: {:#?}",
        sketch(&forest)
    );

    assert!(
        forest.apply(Action::CollapseSubtree),
        "{:#?}",
        sketch(&forest)
    );

    assert_eq!(drawn_beads(&forest), shut, "{:#?}", sketch(&forest));
}

/// A project's line takes the scope as a bead's does, and a project is
/// the widest thing a reader can press these keys on. The others keep
/// what they had, which is what a reader who navigated to one project is
/// asking for.
#[test]
fn collapsing_from_a_project_leaves_the_other_projects_where_they_were() {
    let mut forest = flatten(built(Filter::All));
    forest.apply(Action::ExpandSubtree);
    select_project(&mut forest, "dunwich");
    let elsewhere: Vec<String> = drawn_beads(&forest)
        .into_iter()
        .filter(|id| !id.starts_with("dun-"))
        .collect();
    assert!(
        !elsewhere.is_empty(),
        "nothing outside dunwich to be left alone: {:#?}",
        sketch(&forest)
    );

    forest.apply(Action::CollapseSubtree);

    assert_eq!(
        drawn_beads(&forest)
            .into_iter()
            .filter(|id| !id.starts_with("dun-"))
            .collect::<Vec<_>>(),
        elsewhere,
        "{:#?}",
        sketch(&forest)
    );
    assert!(
        !drawn_beads(&forest).iter().any(|id| id.starts_with("dun-")),
        "{:#?}",
        sketch(&forest)
    );
}

/// `e` folds from the selected node, so a sibling subtree keeps whatever
/// the reader left it at. `tow-1.2` is shut here and stays shut, which is
/// the assertion that tells a scoped fold from a global one.
///
/// The scope is a `Handle`, and a handle names the way down to a line
/// rather than the bead standing on it, so exactly one line carries it —
/// which is what stops a fold set inside the window from also reaching a
/// second copy of that bead outside it.
#[test]
fn expanding_from_a_node_leaves_a_sibling_subtree_where_it_was() {
    let mut forest = flatten(tower_staffed(&[]));
    toggle_fold_of(&mut forest, "tow-1");
    select_bead(&mut forest, "tow-1.1");

    forest.apply(Action::ExpandSubtree);

    assert_eq!(
        drawn_beads(&forest),
        ["tow-1", "tow-1.1", "tow-1.1.1", "tow-1.1.1.1", "tow-1.2"],
        "{:#?}",
        sketch(&forest)
    );
}

/// `c` the same way round: `tow-1.1` shuts over its subtree, `tow-1.2`
/// keeps the one the reader opened, and the node the selection sits on
/// is shut rather than left open over shut children.
#[test]
fn collapsing_from_a_node_leaves_a_sibling_subtree_where_it_was() {
    let mut forest = flatten(tower_staffed(&[]));
    forest.apply(Action::ExpandSubtree);
    select_bead(&mut forest, "tow-1.1");

    forest.apply(Action::CollapseSubtree);

    assert_eq!(
        drawn_beads(&forest),
        ["tow-1", "tow-1.1", "tow-1.2", "tow-1.2.1"],
        "{:#?}",
        sketch(&forest)
    );
}

/// Collapsing opens the subtree first, so that every fold in it lands on
/// a drawn line and one pass can shut them all. That opening walk is
/// scoped too: a sibling the reader left shut is not thrown open on the
/// way past and shut again a frame later.
///
/// `tow-1.2` rests shut here and is never selected, so a walk that opened
/// the forest and narrowed only the shutting pass would leave `tow-1.2.1`
/// on screen.
#[test]
fn collapsing_from_a_node_does_not_open_a_sibling_on_the_way() {
    let mut forest = flatten(tower_staffed(&[]));
    toggle_fold_of(&mut forest, "tow-1");
    toggle_fold_of(&mut forest, "tow-1.1");
    select_bead(&mut forest, "tow-1.1");

    forest.apply(Action::CollapseSubtree);

    assert_eq!(
        drawn_beads(&forest),
        ["tow-1", "tow-1.1", "tow-1.2"],
        "{:#?}",
        sketch(&forest)
    );
}

/// `e` on a shut node opens the node itself and not only what hangs
/// beneath it, which is the ordinary case for `e`: a key that opened only
/// the children of a node the reader cannot see inside would leave the
/// screen exactly as it found it.
#[test]
fn expanding_from_a_shut_node_opens_that_node_too() {
    let mut forest = flatten(tower_staffed(&[]));
    toggle_fold_of(&mut forest, "tow-1");
    select_bead(&mut forest, "tow-1.2");
    assert_eq!(
        forest.fold_at(forest.selected_line()),
        Some(false),
        "the node this is about has to start shut: {:#?}",
        sketch(&forest)
    );

    assert!(
        forest.apply(Action::ExpandSubtree),
        "{:#?}",
        sketch(&forest)
    );

    assert_eq!(
        drawn_beads(&forest),
        ["tow-1", "tow-1.1", "tow-1.2", "tow-1.2.1"],
        "{:#?}",
        sketch(&forest)
    );
}

/// A row carrying no fold has nothing drawn beneath it — a note, an
/// unread root, a thing in a group and a leaf bead are all of them — so
/// the scope taken from such a row is the row alone, and both keys find
/// nothing to point. The screen does not move and nothing is said.
///
/// The implication runs one way only. A fold with nothing under it is
/// ordinary: a project waiting on its first collection draws a header
/// that folds over no trees at all.
#[test]
fn the_fold_keys_on_a_row_with_no_fold_leave_the_screen_where_it_was() {
    for action in [Action::ExpandSubtree, Action::CollapseSubtree] {
        let mut forest = flatten(tower_staffed(&[]));
        forest.apply(Action::ExpandSubtree);
        select_bead(&mut forest, "tow-1.1.1.1");
        assert_eq!(
            forest.fold_at(forest.selected_line()),
            None,
            "the row this is about has to carry no fold: {:#?}",
            sketch(&forest)
        );
        let before = sketch(&forest);

        assert!(!forest.apply(action), "{action:?}: {:#?}", sketch(&forest));

        assert_eq!(sketch(&forest), before, "{action:?}");
    }
}

/// A fold shut over another fold hides it without settling it, so
/// shutting only what is on screen leaves that one resting open. The
/// reader then opens their way back down and a subtree springs at them
/// from a forest they were told was collapsed.
///
/// `tow-1.1` is shut by hand first, which puts `tow-1.1.1` out of sight
/// still resting open over the agent beneath it. `c` from the root above
/// has to reach it there.
#[test]
fn collapsing_shuts_a_fold_the_reader_cannot_see() {
    let mut forest = flatten(tower_staffed(&["tow-1.1.1.1"]));
    toggle_fold_of(&mut forest, "tow-1.1");
    select_bead(&mut forest, "tow-1");

    forest.apply(Action::CollapseSubtree);
    toggle_fold_of(&mut forest, "tow-1");
    toggle_fold_of(&mut forest, "tow-1.1");

    assert_eq!(
        drawn_beads(&forest),
        ["tow-1", "tow-1.1", "tow-1.1.1", "tow-1.2"],
        "{:#?}",
        sketch(&forest)
    );
}

/// The default is derived from what is live rather than stored, so
/// restoring it after an agent has gone home opens the spine to the work
/// that is left and not to where the work was.
///
/// The agent on `tow-1.1.1.1` goes and the one on `tow-1.2.1` stays, so
/// nothing new arrives under any fold and every fold collapse-all set
/// survives the refresh. What is restored is therefore the whole of what
/// the key did, and not something the refresh had already undone.
#[test]
fn restoring_the_default_recomputes_it_rather_than_replaying_the_old_one() {
    let mut forest = flatten(tower_staffed(&["tow-1.1.1.1", "tow-1.2.1"]));
    forest.apply(Action::CollapseSubtree);
    forest.refresh(tower_staffed(&["tow-1.2.1"]));

    forest.apply(Action::RestoreDefault);

    assert_eq!(
        sketch(&forest),
        sketch(&flatten(tower_staffed(&["tow-1.2.1"])))
    );
}

/// A fold these keys set is a fold the user set, so it keeps the standing
/// rule: it survives a refresh that brings nothing new beneath it.
#[test]
fn the_folds_these_keys_set_survive_a_refresh() {
    for action in [Action::ExpandSubtree, Action::CollapseSubtree] {
        let mut forest = flatten(tower_staffed(&["tow-1.1.1.1"]));
        forest.apply(action);
        let before = sketch(&forest);

        forest.refresh(tower_staffed(&["tow-1.1.1.1"]));

        assert_eq!(sketch(&forest), before, "{action:?}");
    }
}

/// No fold `bdi` chooses hides a live agent. `c` is the one place a
/// reader may override that, because they asked for it by name — and
/// restoring the default is how they get the agent back.
#[test]
fn collapsing_may_shut_a_fold_over_a_live_agent_because_the_reader_asked() {
    let mut forest = flatten(tower_staffed(&["tow-1.1.1.1"]));
    let staffed = "tow-1.1.1.1".to_string();
    assert!(
        drawn_beads(&forest).contains(&staffed),
        "{:#?}",
        sketch(&forest)
    );

    forest.apply(Action::CollapseSubtree);
    assert!(
        !drawn_beads(&forest).contains(&staffed),
        "{:#?}",
        sketch(&forest)
    );

    forest.apply(Action::RestoreDefault);
    assert!(
        drawn_beads(&forest).contains(&staffed),
        "{:#?}",
        sketch(&forest)
    );
}

/// These six keys are about folds. Which trees are drawn at all is the
/// filter's, with its own key and its own word for what it does, so a
/// reader who pressed `a` deliberately does not lose it to a fold key.
#[test]
fn the_fold_keys_leave_the_filter_where_the_reader_put_it() {
    for action in [
        Action::ExpandSubtree,
        Action::CollapseSubtree,
        Action::RestoreSubtree,
        Action::ExpandForest,
        Action::CollapseForest,
        Action::RestoreDefault,
    ] {
        let mut forest = flatten(built(Filter::LiveAgents));
        forest.apply(Action::ToggleFilter);

        forest.apply(action);

        assert_eq!(forest.snapshot().filter, Filter::All, "{action:?}");
    }
}

/// The filter is the reader's too, and a collection landing under them is
/// not them changing their mind. The one line that offers the key is the
/// hidden-trees group header, so a refresh that puts the filter back reads
/// on screen as that group shutting itself.
#[test]
fn a_refresh_leaves_the_filter_where_the_reader_put_it() {
    let mut forest = flatten(built(Filter::LiveAgents));
    forest.apply(Action::ToggleFilter);
    let before = sketch(&forest);

    forest.refresh(built(Filter::LiveAgents));

    assert_eq!(forest.snapshot().filter, Filter::All);
    assert_eq!(sketch(&forest), before);
}

/// `apply` reports whether the screen moved, and a subtree already open
/// has nowhere to go. The loop redraws on that answer.
#[test]
fn expanding_from_a_node_already_open_moves_nothing() {
    let mut forest = flatten(tower_staffed(&[]));

    assert!(
        forest.apply(Action::ExpandSubtree),
        "{:#?}",
        sketch(&forest)
    );
    assert!(
        !forest.apply(Action::ExpandSubtree),
        "{:#?}",
        sketch(&forest)
    );
}
