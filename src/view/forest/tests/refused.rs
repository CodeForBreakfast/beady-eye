use super::*;
use pretty_assertions::assert_eq;

/// A tracker that refused keeps its root and reports no nodes, so there
/// is no way down from its header to walk. The header is a line all the
/// same, and holding the selection on it across a refresh is what says
/// the forest knows a tree by its root rather than by a node.
#[test]
fn the_selection_holds_the_line_of_a_root_whose_tracker_refused() {
    let mut forest = flatten(snapshot());
    let header = forest
        .lines()
        .iter()
        .position(|line| matches!(&line.content, Content::Unread(unread) if unread.root == "fer-2"))
        .expect("the shared snapshot draws a tree whose tracker refused");

    step_onto(&mut forest, header);
    forest.refresh(snapshot());

    assert_eq!(forest.selected_line(), header, "{:#?}", sketch(&forest));
}

/// A root whose tracker refused has no way down for a rule to be set on,
/// so `s` pressed on its header puts nothing in force there.
#[test]
fn a_root_whose_tracker_refused_takes_no_rule() {
    let mut forest = flatten(snapshot());
    select_bead(&mut forest, "fer-2");
    let before = forest.spine();

    forest.apply(Action::CycleSpine);

    assert_eq!(forest.spine(), before, "{:#?}", sketch(&forest));
}

/// A cursor whose tree and project have both gone falls back to the first
/// tree drawn, and where that is a root whose tracker refused, it lands
/// on its header.
#[test]
fn a_lost_cursor_falls_back_onto_the_header_of_a_root_whose_tracker_refused() {
    let mut forest = flatten(built(Filter::All));
    select_bead(&mut forest, "hbr-3");

    forest.refresh(gather(
        vec![Tree::tracker_unreachable(
            "ferry",
            "fer-2",
            TrackerFailure::Auth,
        )],
        Vec::new(),
        Filter::All,
    ));

    assert_eq!(
        forest.place(),
        Some(&Place::root(key("ferry", "fer-2"))),
        "{:#?}",
        sketch(&forest)
    );
}

/// A selected root whose tracker stops reading keeps the selection on
/// its header, however many rows the trees above it lost meanwhile.
#[test]
fn a_selected_root_keeps_the_selection_when_its_tracker_stops_reading() {
    let mut forest = flatten(built(Filter::All));
    select_bead(&mut forest, "hbr-3");

    forest.refresh(gather(
        vec![
            Tree::tracker_unreachable("dunwich", "dun-7", TrackerFailure::Auth),
            Tree::tracker_unreachable("ferry", "fer-2", TrackerFailure::Auth),
            Tree::tracker_unreachable("harbour", "hbr-3", TrackerFailure::Auth),
        ],
        Vec::new(),
        Filter::All,
    ));

    assert_eq!(
        forest.place(),
        Some(&Place::root(key("harbour", "hbr-3"))),
        "{:#?}",
        sketch(&forest)
    );
}

/// Going back to the header of a root whose tracker refused lands on it,
/// as going back to any line still drawn does.
#[test]
fn going_back_to_the_header_of_a_root_whose_tracker_refused_lands_on_it() {
    let mut forest = flatten(snapshot());
    let header = Place::root(key("ferry", "fer-2"));

    assert!(forest.go_to_place(&header), "{:#?}", sketch(&forest));
    assert_eq!(forest.place(), Some(&header), "{:#?}", sketch(&forest));
}
