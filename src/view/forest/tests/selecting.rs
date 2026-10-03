use super::*;
use pretty_assertions::assert_eq;

#[test]
fn selecting_a_line_puts_the_selection_on_it() {
    let mut forest = flatten(snapshot());
    let reachable = walk_down(&mut forest);

    for at in reachable {
        assert!(
            forest.select_line(at) || forest.selected_line() == at,
            "line {at} would not take the selection: {:#?}",
            sketch(&forest)
        );
        assert_eq!(forest.selected_line(), at, "{:#?}", sketch(&forest));
    }
}

/// The keyboard never rests on a note or an elided run, because every
/// motion filters through the same predicate. A pointer names one line
/// and no other, so a line the keyboard cannot rest on takes no selection
/// from a click either — sliding to the neighbour would select something
/// the reader did not point at.
#[test]
fn selecting_a_line_the_selection_cannot_rest_on_leaves_it_where_it_was() {
    let mut forest = flatten(snapshot());
    let unreachable: Vec<usize> = (0..forest.lines().len())
        .filter(|at| !selectable(&forest.lines()[*at]))
        .collect();

    assert!(!unreachable.is_empty(), "{:#?}", sketch(&forest));
    for at in unreachable {
        forest.apply(Action::Move(Motion::FirstRow));
        let was = forest.selected_line();

        assert!(
            !forest.select_line(at),
            "line {at} took the selection: {:#?}",
            sketch(&forest)
        );
        assert_eq!(forest.selected_line(), was);
    }
}

#[test]
fn selecting_a_line_that_is_not_drawn_leaves_the_selection_where_it_was() {
    let mut forest = flatten(snapshot());
    forest.apply(Action::Move(Motion::FirstRow));

    assert!(!forest.select_line(forest.lines().len()));
    assert!(!forest.select_line(usize::MAX));
    assert_eq!(forest.selected_line(), 0);
}

#[test]
fn selecting_the_line_already_selected_changes_nothing() {
    let mut forest = flatten(snapshot());
    forest.apply(Action::Move(Motion::FirstRow));
    let at = forest.selected_line();

    assert!(!forest.select_line(at));
    assert_eq!(forest.selected_line(), at);
}

/// A bead reachable from two roots is drawn under each, so a click names
/// one of two rows carrying the same handle. The refresh that follows
/// re-derives the selection from that handle, and has to settle on the
/// copy the pointer landed on rather than on its twin higher up.
#[test]
fn selecting_the_lower_copy_of_a_twin_keeps_the_selection_on_it_across_a_refresh() {
    let panes = panes_on(&["qua-1.2", "wha-2.1"]);
    let mut forest = flatten(overlapping(&panes));
    let copies = lines_of(&forest, "qua-1.2");

    assert_eq!(copies.len(), 2, "{:#?}", sketch(&forest));
    let lower = copies[1];

    assert!(forest.select_line(lower), "{:#?}", sketch(&forest));
    forest.refresh(overlapping(&panes));

    assert_eq!(forest.selected_line(), lower, "{:#?}", sketch(&forest));
}
