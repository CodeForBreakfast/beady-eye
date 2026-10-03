use super::*;
use pretty_assertions::assert_eq;

/// The forest can say where the selection is, as a place: a bead drawn
/// under two parents is drawn twice, and a caller keeping this to come
/// back to is keeping the copy the reader was looking at.
#[test]
fn the_forest_says_where_the_selection_is() {
    let forest = flatten(snapshot());

    assert_eq!(
        forest.place().map(|place| place.key().clone()),
        Some(key("dunwich", "dun-7"))
    );
}

/// Nothing where the selection is not on a bead at all, which is a line
/// with nowhere to come back to rather than a line with no name.
#[test]
fn a_line_that_is_not_a_bead_is_nowhere_to_come_back_to() {
    let mut forest = flatten(snapshot());
    forest.apply(Action::Move(Motion::FirstRow));

    assert_eq!(forest.place(), None, "{:#?}", sketch(&forest));
}

/// A bead the trees hold is one the forest can go to, whether or not it
/// is drawn this instant; one no tree holds is not. The question is asked
/// of the snapshot, so it goes on being the same question when roots are
/// found another way.
#[test]
fn the_forest_draws_the_beads_its_trees_hold_and_no_others() {
    let forest = flatten(snapshot());

    assert!(forest.draws(&key("dunwich", "dun-7.1.1")));
    assert!(!forest.draws(&key("dunwich", "dun-404")));
    assert!(
        !forest.draws(&key("ferry", "dun-7")),
        "a bead is (project, id), so one project's id is not another's"
    );
}

/// The whole point: `dun-7.1` is drawn shut, so `dun-7.1.1` is on no line
/// at all. Going to it opens what is over it and lands on it.
#[test]
fn going_to_a_bead_under_a_shut_fold_opens_it_and_lands_there() {
    let mut forest = flatten(snapshot());
    assert!(
        !drawn_here(&forest, "true the mount"),
        "the bead is drawn already, so this would test nothing: {:#?}",
        sketch(&forest)
    );

    assert!(forest.go_to(&key("dunwich", "dun-7.1.1")));

    assert_eq!(cursor(&forest), Some(&key("dunwich", "dun-7.1.1")));
    assert!(
        drawn_here(&forest, "true the mount"),
        "{:#?}",
        sketch(&forest)
    );
}

/// A tree the filter took out of the forest is drawn in the group below
/// its project, and that group has a fold of its own. Both open, and the
/// project's with them.
#[test]
fn going_to_a_bead_in_a_tree_the_filter_hid_opens_the_group_over_it() {
    let mut forest = flatten(snapshot());

    assert!(forest.go_to(&key("harbour", "hbr-3.1")));

    assert_eq!(cursor(&forest), Some(&key("harbour", "hbr-3.1")));
}

/// A fold the reader shut by hand stays shut until something asks
/// otherwise, and asking to be taken to a bead underneath it is asking.
#[test]
fn going_to_a_bead_opens_a_fold_the_reader_shut_by_hand() {
    let mut forest = flatten(snapshot());
    forest.go_to(&key("dunwich", "dun-7.1"));
    forest.apply(Action::CollapseSubtree);
    assert!(
        !drawn_here(&forest, "true the mount"),
        "{:#?}",
        sketch(&forest)
    );

    assert!(forest.go_to(&key("dunwich", "dun-7.1.1")));

    assert_eq!(cursor(&forest), Some(&key("dunwich", "dun-7.1.1")));
}

/// `dun-7.1.1` hangs under `dun-7` by way of `dun-7.1`, not by way of the
/// run beside it, so landing on it leaves that run counted.
#[test]
fn a_search_leaves_shut_a_run_the_way_down_does_not_go_through() {
    let mut forest = flatten(snapshot());
    forest.apply(Action::CollapseForest);

    assert_eq!(
        forest.seek_here("dun-7.1.1"),
        went_to("dunwich", "dun-7.1.1", 1, 1)
    );

    assert!(
        drawn_here(&forest, "└─▸ … 3 more"),
        "{:#?}",
        sketch(&forest)
    );
    assert!(
        !drawn_here(&forest, "survey the mast"),
        "{:#?}",
        sketch(&forest)
    );
}

#[test]
fn a_search_opens_the_run_its_bead_is_counted_in() {
    let mut forest = flatten(snapshot());
    forest.apply(Action::CollapseForest);

    assert_eq!(
        forest.seek_here("dun-7.2"),
        went_to("dunwich", "dun-7.2", 1, 1)
    );

    assert!(
        drawn_here(&forest, "└── … 3 more"),
        "{:#?}",
        sketch(&forest)
    );
}

/// A bead no tree holds is nowhere to go, and the forest is left exactly
/// as it was rather than half-opened on the way to nothing.
#[test]
fn going_to_a_bead_no_tree_holds_moves_nothing() {
    let mut forest = flatten(snapshot());
    let was = sketch(&forest);
    let selected = forest.selected_line();

    assert!(!forest.go_to(&key("dunwich", "dun-404")));

    assert_eq!(sketch(&forest), was);
    assert_eq!(forest.selected_line(), selected);
}

/// A bead drawn under two parents is two lines, and coming back means the
/// one the reader was on. A key could not say which; a place does.
#[test]
fn coming_back_to_a_place_lands_on_the_copy_that_was_left() {
    let mut forest = flatten(drawn_twice_in_one_tree());
    forest.apply(Action::ExpandSubtree);
    let twice: Vec<Place> = forest
        .lines()
        .iter()
        .filter_map(|line| line.place.clone())
        .filter(|place| *place.key() == key("dunwich", "dun-9.1"))
        .collect();
    assert_eq!(
        twice.len(),
        2,
        "the fixture draws it twice: {:#?}",
        sketch(&forest)
    );

    forest.apply(Action::Move(Motion::FirstRow));
    assert!(forest.go_to_place(&twice[1]));

    assert_eq!(forest.place(), Some(&twice[1]));
    assert_ne!(forest.place(), Some(&twice[0]), "the other copy of it");
}
