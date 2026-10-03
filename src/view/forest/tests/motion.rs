use super::*;
use pretty_assertions::assert_eq;

#[test]
fn the_selection_survives_a_refresh_that_reorders_the_nodes() {
    let mut forest = flatten(snapshot());
    open(&mut forest, &key("dunwich", "dun-7.1"));
    select(&mut forest, &key("dunwich", "dun-7.1.2"));
    let was = forest.selected_line();

    let reordered = edited(DUNWICH, r#""priority":3"#, r#""priority":1"#);
    forest.refresh(gather(
        vec![tree_of("dunwich", &reordered)],
        Vec::new(),
        Filter::LiveAgents,
    ));

    assert_eq!(cursor(&forest), Some(&key("dunwich", "dun-7.1.2")));
    assert_ne!(forest.selected_line(), was);
}

/// Closing a bead under the cursor is the ordinary way for one to go, and
/// the tree it was in is still on screen. The cursor stays in that tree,
/// on the parent, rather than going back to the top of the forest.
#[test]
fn a_refresh_that_drops_the_selected_bead_falls_back_to_its_parent() {
    let mut forest = flatten(snapshot());
    open(&mut forest, &key("dunwich", "dun-7.1"));
    select(&mut forest, &key("dunwich", "dun-7.1.2"));

    let without = edited(
        DUNWICH,
        r#"{"id":"dun-7.1.2","title":"seal the feed horn","status":"open",
   "dependencies":[{"depends_on_id":"dun-7.1","type":"parent-child"}],
   "priority":3,"issue_type":"task"},"#,
        "",
    );
    forest.refresh(gather(
        vec![tree_of("dunwich", &without)],
        Vec::new(),
        Filter::LiveAgents,
    ));

    assert_eq!(cursor(&forest), Some(&key("dunwich", "dun-7.1")));
}

/// The bead the selection was on is gone with its whole tree, and the
/// nearest of its forebears the new snapshot still draws is its
/// project's line: dunwich still has panes working in it that no bead
/// claims, so the line is still there to fall back to.
#[test]
fn a_refresh_that_drops_the_selected_bead_leaves_the_selection_somewhere_real() {
    let mut forest = flatten(snapshot());
    open(&mut forest, &key("dunwich", "dun-7.1"));
    select(&mut forest, &key("dunwich", "dun-7.1.2"));
    // Harbour has no live agent, so it is a tree only a reader showing
    // every tree can be left standing on.
    forest.apply(Action::ToggleFilter);

    forest.refresh(gather(
        vec![tree_of("harbour", HARBOUR)],
        Vec::new(),
        Filter::LiveAgents,
    ));

    assert!(
        matches!(
            &forest.lines()[forest.selected_line()].content,
            Content::Project(line) if line.project == "dunwich"
        ),
        "{:#?}",
        sketch(&forest)
    );
}

/// `a` is a display choice over what was already collected, so what it
/// renders is the answer the trackers already gave.
#[test]
fn dropping_the_filter_re_renders_rather_than_re_collecting() {
    let mut forest = flatten(snapshot());
    let generated_at = forest.snapshot().generated_at;

    assert!(forest.apply(Action::ToggleFilter));

    assert_eq!(forest.snapshot().filter, Filter::All);
    assert_eq!(forest.snapshot().generated_at, generated_at);
    assert!(sketch(&forest).iter().any(|line| line == "▾ harbour"));
    assert!(sketch(&forest).iter().any(|line| line.contains("hbr-3")));
    assert!(!sketch(&forest)
        .iter()
        .any(|line| line.contains("[HiddenTrees]")));
}

#[test]
fn collapsing_an_expanded_node_and_then_collapsing_again_moves_to_its_parent() {
    let mut forest = flatten(snapshot());
    open(&mut forest, &key("dunwich", "dun-7.1"));

    assert!(forest.apply(Action::CollapseOrParent));
    assert_eq!(cursor(&forest), Some(&key("dunwich", "dun-7.1")));
    assert!(!sketch(&forest).iter().any(|line| line.contains(".1.1")));

    assert!(forest.apply(Action::CollapseOrParent));
    assert_eq!(cursor(&forest), Some(&key("dunwich", "dun-7")));
}

#[test]
fn expanding_a_collapsed_node_and_then_expanding_again_moves_to_its_first_child() {
    let mut forest = flatten(snapshot());
    open(&mut forest, &key("dunwich", "dun-7.1"));
    forest.apply(Action::CollapseOrParent);

    assert!(forest.apply(Action::ExpandOrChild));
    assert_eq!(cursor(&forest), Some(&key("dunwich", "dun-7.1")));

    assert!(forest.apply(Action::ExpandOrChild));
    assert_eq!(cursor(&forest), Some(&key("dunwich", "dun-7.1.1")));
}

#[test]
fn a_leaf_has_no_child_to_move_to_and_no_fold_to_collapse() {
    let mut forest = flatten(snapshot());
    open(&mut forest, &key("dunwich", "dun-7.1"));
    select(&mut forest, &key("dunwich", "dun-7.1.1"));

    assert!(!forest.apply(Action::ExpandOrChild));
    assert!(forest.apply(Action::CollapseOrParent));
    assert_eq!(cursor(&forest), Some(&key("dunwich", "dun-7.1")));
}

#[test]
fn moving_stops_at_the_ends() {
    let mut forest = flatten(snapshot());
    forest.apply(Action::Move(Motion::FirstRow));

    assert!(!forest.apply(Action::Move(Motion::PreviousRow)));
    assert_eq!(forest.selected_line(), 0);

    forest.apply(Action::Move(Motion::LastRow));
    let last = forest.selected_line();

    assert!(!forest.apply(Action::Move(Motion::NextRow)));
    assert_eq!(forest.selected_line(), last);
}

/// `^D` and `^U` move by half the band the trees are in, which the
/// renderer measured for the last frame — not half a screen the key bar
/// and the tail also sit in.
#[test]
fn a_half_screen_moves_as_far_as_the_renderer_says_it_should() {
    let mut forest = flatten(snapshot());
    forest.fit(6);

    forest.apply(Action::Move(Motion::HalfScreenDown));

    assert_eq!(forest.selected_line(), 4);

    forest.apply(Action::Move(Motion::HalfScreenUp));

    assert_eq!(forest.selected_line(), 1);
}

/// A band with no room to halve still moves the selection, so `^D` on the
/// shortest screen there is does something rather than nothing.
#[test]
fn a_half_screen_of_a_band_too_short_to_halve_is_one_row() {
    let mut forest = flatten(snapshot());
    forest.fit(1);
    let mut stepped = flatten(snapshot());
    stepped.fit(1);

    assert!(forest.apply(Action::Move(Motion::HalfScreenDown)));
    stepped.apply(Action::Move(Motion::NextRow));

    assert_eq!(forest.selected_line(), stepped.selected_line());
}
