//! Rooting the forest at a bead, named at the start or focused on the way.

use super::*;
use pretty_assertions::assert_eq;

/// A bead named on the command line starts the forest exactly where
/// Shift+F on it would have put it: a root, a bead under one, and a root
/// the filter is holding back.
#[test]
fn naming_a_bead_starts_the_forest_as_focusing_it_does() {
    for (project, id) in [
        ("dunwich", "dun-7"),
        ("dunwich", "dun-7.1"),
        ("harbour", "hbr-3"),
    ] {
        let mut focused = flatten(snapshot());
        if project == "harbour" {
            select_hidden_tree(&mut focused);
        } else {
            select_bead(&mut focused, id);
        }
        assert!(focused.apply(Action::FocusForest));

        let named = named_on_the_command_line(&[(project, id)]);

        assert_eq!(sketch(&named), sketch(&focused), "named {id}");
        assert_eq!(cursor(&named), cursor(&focused), "named {id}");
    }
}

/// Several named are each drawn as a root, and everything else goes
/// behind the lines focus draws.
#[test]
fn naming_several_beads_draws_each_as_a_root() {
    let forest = named_on_the_command_line(&[("dunwich", "dun-7.1"), ("harbour", "hbr-3")]);

    assert_eq!(
        sketch(&forest)
            .into_iter()
            .filter(|row| !row.contains("── - "))
            .collect::<Vec<String>>(),
        vec![
            "▾ dunwich",
            "  ├─▸ ○ dun-7.1 re-point the dish",
            "  │   └── ! OrphanedDependencies(1)",
            "  ├─▸ [OutOfTheWay dunwich] 1",
            "  └── [Unattributed dunwich] 2",
            "▾ ferry",
            "  ├─▸ [OutOfTheWay ferry] 1",
            "  └── [Unattributed ferry] 1",
            "▾ harbour",
            "  └─▸ ○ hbr-3 dredge the channel",
            "▸ [FailedProjects] 1",
            "▾ [Unconfigured] 1",
            "▾ [Conflicts] 1",
        ]
    );
}

/// Two named in one tree are both drawn as roots, and the root above them
/// goes behind the line without either.
#[test]
fn naming_two_beads_in_one_tree_draws_both_and_holds_back_the_rest() {
    let mut forest = named_on_the_command_line(&[("dunwich", "dun-7.1"), ("dunwich", "dun-7.7")]);

    open_the_line_holding_roots_back(&mut forest, "dunwich");

    assert_eq!(
        sketch(&forest)
            .into_iter()
            .filter(|row| !row.contains("── - "))
            .collect::<Vec<String>>(),
        vec![
            "▾ dunwich",
            "  ├─▸ ○ dun-7.1 re-point the dish",
            "  │   └── ! OrphanedDependencies(1)",
            "  ├── ○ dun-7.7 log the survey marks",
            "  │   └── ! OrphanedDependencies(1)",
            "  ├── [OutOfTheWay dunwich] 1",
            "  │   └─▸ ◐ dun-7 lift the ground station",
            "  │       └── ! OrphanedDependencies(1)",
            "  └── [Unattributed dunwich] 2",
            "▾ ferry",
            "  ├─▸ [OutOfTheWay ferry] 1",
            "  └── [Unattributed ferry] 1",
            "▾ harbour",
            "  └─▸ [OutOfTheWay harbour] 1",
            "▸ [FailedProjects] 1",
            "▾ [Unconfigured] 1",
            "▾ [Conflicts] 1",
        ]
    );

    let behind = *lines_of(&forest, "dun-7")
        .first()
        .expect("dun-7 is behind the line");
    step_onto(&mut forest, behind);
    assert!(forest.apply(Action::ExpandOrChild));
    let copies_of = |id: &str| {
        forest
            .lines()
            .iter()
            .filter(|line| {
                line.place
                    .as_ref()
                    .is_some_and(|place| place.key().id == id)
            })
            .count()
    };
    assert_eq!(
        copies_of("dun-7.4"),
        1,
        "dun-7 opened: {:#?}",
        sketch(&forest)
    );
    assert_eq!(
        (copies_of("dun-7.1"), copies_of("dun-7.7")),
        (1, 1),
        "the root behind the line draws neither named bead again: {:#?}",
        sketch(&forest)
    );
}

/// A bead named beneath another named bead is already drawn under it.
#[test]
fn a_bead_named_beneath_another_named_bead_adds_no_root() {
    let both = named_on_the_command_line(&[("dunwich", "dun-7.1"), ("dunwich", "dun-7")]);

    let one = named_on_the_command_line(&[("dunwich", "dun-7")]);

    assert_eq!(sketch(&both), sketch(&one));
}

/// After a start with beads named, Shift+F is as it is after any other
/// focus: the mode goes, and the forest is the one an unnamed start draws.
#[test]
fn shift_f_after_a_named_start_draws_the_unnamed_forest() {
    let unnamed = sketch(&flatten(snapshot()));
    for named in [
        vec![("dunwich", "dun-7.1")],
        vec![("dunwich", "dun-7.1"), ("harbour", "hbr-3")],
    ] {
        let mut forest = named_on_the_command_line(&named);

        assert!(forest.apply(Action::FocusForest));

        assert_eq!(sketch(&forest), unnamed, "named {named:?}");
    }
}

/// Shift+F after a start with several beads named comes back to the one
/// the selection is under, not the first named.
#[test]
fn shift_f_after_several_named_keeps_the_bead_the_selection_is_under() {
    let mut forest = named_on_the_command_line(&[("dunwich", "dun-7.1"), ("harbour", "hbr-3")]);
    select_bead(&mut forest, "hbr-3");

    assert!(forest.apply(Action::FocusForest));

    assert_eq!(cursor(&forest), Some(&key("harbour", "hbr-3")));
}

/// The first frame is drawn before any tracker answers, so a bead named
/// then is focused when the collection that draws it lands.
#[test]
fn a_bead_named_before_anything_is_read_is_focused_when_it_is_drawn() {
    let mut named = flatten(Snapshot::awaiting(
        vec![
            "dunwich".to_string(),
            "ferry".to_string(),
            "harbour".to_string(),
        ],
        Vec::new(),
        A_PROVIDER,
        Scope::Everything,
        Filter::LiveAgents,
        now(),
    ));
    named.focus_when_drawn(vec![key("dunwich", "dun-7.1")]);

    named.refresh(snapshot());

    assert_eq!(
        sketch(&named),
        sketch(&named_on_the_command_line(&[("dunwich", "dun-7.1")]))
    );
    assert_eq!(cursor(&named), Some(&key("dunwich", "dun-7.1")));
}

/// A tracker that failed to answer has not said the named bead is not
/// there, so it is focused once a later collection draws it.
#[test]
fn a_bead_named_while_its_tracker_fails_is_focused_once_it_answers() {
    let mut forest = flatten(gather(
        vec![
            Tree::tracker_unreachable("dunwich", "dun-7", TrackerFailure::Auth),
            tree_of("harbour", HARBOUR),
        ],
        Vec::new(),
        Filter::LiveAgents,
    ));
    forest.focus_when_drawn(vec![key("dunwich", "dun-7.1")]);

    forest.refresh(snapshot());

    assert_eq!(
        sketch(&forest),
        sketch(&named_on_the_command_line(&[("dunwich", "dun-7.1")]))
    );
}

/// A bead its tracker reports missing is not waited for: the forest is
/// the unnamed one, with the missing root reported where it would be.
#[test]
fn a_named_bead_its_tracker_reports_missing_is_let_go() {
    let missing = || {
        gather(
            vec![
                tree_of("dunwich", DUNWICH),
                Tree::unread("dunwich", "dun-404", TrackerState::RootNotFound),
                tree_of("harbour", HARBOUR),
            ],
            Vec::new(),
            Filter::LiveAgents,
        )
    };
    let mut forest = flatten(missing());

    forest.focus_when_drawn(vec![key("dunwich", "dun-404")]);

    assert_eq!(sketch(&forest), sketch(&flatten(missing())));
    assert!(forest.named.is_empty(), "still waiting: {:?}", forest.named);
}

/// Every root but the one focused goes, and so does every other project's
/// tree. The project lines stay, because what is holding their roots back
/// hangs under them.
#[test]
fn focusing_a_root_leaves_it_the_only_one_drawn() {
    let mut forest = flatten(snapshot());
    focus_on(&mut forest, "dun-7");

    assert_eq!(
        // Without the things in the groups, which say nothing about where
        // the trees went and are long enough to bury the rows that do.
        sketch(&forest)
            .into_iter()
            .filter(|row| !row.contains("── - "))
            .collect::<Vec<String>>(),
        vec![
            "▾ dunwich",
            "  ├── ◐ dun-7 lift the ground station",
            "  │   ├── ! OrphanedDependencies(1)",
            "  │   ├─▸ ○ .1 re-point the dish",
            "  │   ├── ○ .7 log the survey marks",
            "  │   ├── ✓ .4 clear the access road",
            "  │   └─▸ … 3 more",
            "  └── [Unattributed dunwich] 2",
            "▾ ferry",
            "  ├─▸ [OutOfTheWay ferry] 1",
            "  └── [Unattributed ferry] 1",
            "▾ harbour",
            "  └─▸ [OutOfTheWay harbour] 1",
            "▸ [FailedProjects] 1",
            "▾ [Unconfigured] 1",
            "▾ [Conflicts] 1",
        ]
    );
}

/// Every other root is one line away rather than gone, under the project
/// it belongs to. Dunwich's own root is behind that line too, for the part
/// of it the mode stopped drawing: the beads above the focused bead.
#[test]
fn the_roots_the_mode_stops_drawing_go_behind_one_line_per_project() {
    let mut forest = flatten(snapshot());
    focus_on(&mut forest, "dun-7.1");

    assert_eq!(
        sketch(&forest)
            .into_iter()
            .filter(|row| !row.contains("── - "))
            .collect::<Vec<String>>(),
        vec![
            "▾ dunwich",
            "  ├─▸ ○ dun-7.1 re-point the dish",
            "  │   └── ! OrphanedDependencies(1)",
            "  ├─▸ [OutOfTheWay dunwich] 1",
            "  └── [Unattributed dunwich] 2",
            "▾ ferry",
            "  ├─▸ [OutOfTheWay ferry] 1",
            "  └── [Unattributed ferry] 1",
            "▾ harbour",
            "  └─▸ [OutOfTheWay harbour] 1",
            "▸ [FailedProjects] 1",
            "▾ [Unconfigured] 1",
            "▾ [Conflicts] 1",
        ]
    );
}

/// One line away means the line opens onto them. They rest shut inside it,
/// as the trees the filter is holding back do.
///
/// Ferry's root was on the screen and harbour's was behind the filter, and
/// the line opens onto either, because one line stands for both sets.
#[test]
fn opening_the_line_over_the_held_back_roots_draws_them() {
    for (project, root) in [
        ("ferry", "fer-2 unread"),
        ("harbour", "hbr-3 dredge the channel"),
    ] {
        let mut forest = flatten(snapshot());
        focus_on(&mut forest, "dun-7.1");

        open_the_line_holding_roots_back(&mut forest, project);

        assert!(drawn_here(&forest, root), "{:#?}", sketch(&forest));
    }
}

/// Which row one project's held-back roots are behind.
fn the_line_holding_roots_back(forest: &Forest, project: &str) -> usize {
    forest
        .lines()
        .iter()
        .position(|line| {
            matches!(&line.content, Content::Group(group)
            if group.kind == GroupKind::OutOfTheWay
                && group.project.as_deref() == Some(project))
        })
        .unwrap_or_else(|| panic!("no line holds {project} back: {:#?}", sketch(forest)))
}

/// Step onto that line and open it, with the keys a reader has.
fn open_the_line_holding_roots_back(forest: &mut Forest, project: &str) {
    let at = the_line_holding_roots_back(forest, project);
    step_onto(forest, at);
    assert!(forest.apply(Action::ExpandOrChild));
}

/// What the line holding one project's roots back says it stands over.
fn held_by_the_line(forest: &Forest, project: &str) -> Counts {
    forest
        .lines()
        .iter()
        .find_map(|line| match &line.content {
            Content::Group(group)
                if group.kind == GroupKind::OutOfTheWay
                    && group.project.as_deref() == Some(project) =>
            {
                group.held.clone()
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("no counts on {project}'s line: {:#?}", sketch(forest)))
}

/// The line stands over open work with seats and anomalies on it, which is
/// the whole reason the reader pressed the key, so it counts them rather
/// than saying a number of roots and leaving them unsaid.
#[test]
fn the_line_over_the_held_back_roots_counts_the_seats_and_anomalies_in_them() {
    let mut forest = flatten(snapshot());
    select_hidden_tree(&mut forest);
    assert!(forest.apply(Action::FocusForest));

    let held = held_by_the_line(&forest, "dunwich");

    assert_eq!(held, counts_of(&forest, "dunwich", "dun-7"));
    assert!(held.live_agents > 0, "dunwich is where the agents are");
}

/// What one of a project's trees adds up to, by its root.
fn counts_of(forest: &Forest, project: &str, root: &str) -> Counts {
    forest
        .snapshot()
        .tree(&key(project, root))
        .unwrap_or_else(|| panic!("{project} has no tree at {root}"))
        .counts
        .clone()
}

/// The reader asked to finish the bead they pressed the key on, so that is
/// the bead the forest they come back to is standing on, wherever they had
/// walked to underneath it.
#[test]
fn putting_the_forest_back_leaves_the_selection_on_the_bead_it_was_rooted_at() {
    let mut forest = flatten(snapshot());
    let was = sketch(&forest);
    focus_on(&mut forest, "dun-7.1");
    forest.apply(Action::Move(Motion::LastRow));

    assert!(forest.apply(Action::FocusForest));

    assert_eq!(sketch(&forest), was);
    assert_eq!(cursor(&forest), Some(&key("dunwich", "dun-7.1")));
}

/// `bdi` runs on the live-agent filter unless told otherwise, and the
/// group of trees it is holding back is where a reader meets a quiet one.
/// Rooting the forest at a bead in one of those draws it like any other,
/// rather than finding no tree to root at.
#[test]
fn rooting_the_forest_at_a_bead_the_filter_holds_back_draws_its_tree() {
    let mut forest = flatten(snapshot());
    select_hidden_tree(&mut forest);
    assert_eq!(cursor(&forest), Some(&key("harbour", "hbr-3")));

    assert!(forest.apply(Action::FocusForest));

    let hbr = forest
        .lines()
        .iter()
        .find(|line| line.bead().is_some_and(|bead| bead.id == "hbr-3"))
        .unwrap_or_else(|| panic!("hbr-3 is not drawn: {:#?}", sketch(&forest)));
    assert_eq!(hbr.depth, 1, "drawn where a root is drawn");
    assert!(
        !drawn_here(&forest, "HiddenTrees"),
        "the group draws whole trees: {:#?}",
        sketch(&forest)
    );
}

/// The mode stands on the bead named at the keystroke, so walking about
/// under it does not move it.
#[test]
fn moving_the_selection_leaves_the_forest_rooted_where_it_was() {
    let mut forest = flatten(snapshot());
    toggle_fold_of(&mut forest, "dun-7.1");
    focus_on(&mut forest, "dun-7.1");
    let rooted = sketch(&forest);

    forest.apply(Action::Move(Motion::LastRow));
    forest.apply(Action::Move(Motion::FirstRow));

    assert_eq!(sketch(&forest), rooted);
}

/// The bead going out of the collection is the one thing that ends the
/// mode on its own: there is nothing left to root the forest at.
#[test]
fn a_collection_that_has_lost_the_focused_bead_puts_the_forest_back() {
    let mut forest = flatten(snapshot());
    toggle_fold_of(&mut forest, "dun-7.1");
    focus_on(&mut forest, "dun-7.1.1");
    assert!(!drawn_here(&forest, "fer-2"), "rooted at one bead");

    let renamed = edited(DUNWICH, r#""id":"dun-7.1.1""#, r#""id":"dun-7.1.9""#);
    forest.refresh(gather(
        vec![
            tree_of("dunwich", &renamed),
            Tree::tracker_unreachable("ferry", "fer-2", TrackerFailure::Auth),
            tree_of("harbour", HARBOUR),
        ],
        Vec::new(),
        Filter::LiveAgents,
    ));

    assert!(
        drawn_here(&forest, "fer-2"),
        "every root is back: {:#?}",
        sketch(&forest)
    );
}

/// And the next press roots the forest at the bead the reader is on,
/// rather than being spent putting back a forest that is already back.
#[test]
fn the_key_roots_the_forest_afresh_once_the_focused_bead_has_gone() {
    let mut forest = flatten(snapshot());
    toggle_fold_of(&mut forest, "dun-7.1");
    focus_on(&mut forest, "dun-7.1.1");
    let renamed = edited(DUNWICH, r#""id":"dun-7.1.1""#, r#""id":"dun-7.1.9""#);
    forest.refresh(gather(
        vec![
            tree_of("dunwich", &renamed),
            Tree::tracker_unreachable("ferry", "fer-2", TrackerFailure::Auth),
            tree_of("harbour", HARBOUR),
        ],
        Vec::new(),
        Filter::LiveAgents,
    ));

    focus_on(&mut forest, "dun-7.1");

    assert!(
        !drawn_here(&forest, "fer-2"),
        "rooted at the bead just asked for: {:#?}",
        sketch(&forest)
    );
}

/// A tracker that did not answer has not said the focused bead is gone, so
/// the mode holds through it: still focused, the selection still on the
/// bead, and every other project still behind its line.
#[test]
fn a_read_of_the_focused_beads_tracker_that_failed_leaves_the_mode_as_it_was() {
    let mut forest = flatten(snapshot());
    focus_on(&mut forest, "dun-7.1");
    let was = behind_the_line_elsewhere(&forest);

    forest.refresh(dunwich_failed());

    assert!(forest.is_focused(), "the mode ended: {:#?}", sketch(&forest));
    assert_eq!(cursor(&forest), Some(&key("dunwich", "dun-7.1")));
    assert_eq!(behind_the_line_elsewhere(&forest), was);
    assert!(
        drawn_here(&forest, "⚠ dun-7.1 unread"),
        "nothing stands where the bead was: {:#?}",
        sketch(&forest)
    );
}

/// The same holds for a bead named on the command line.
#[test]
fn a_read_of_a_named_beads_tracker_that_failed_leaves_the_mode_as_it_was() {
    let mut forest = named_on_the_command_line(&[("dunwich", "dun-7.1")]);
    let was = behind_the_line_elsewhere(&forest);

    forest.refresh(dunwich_failed());

    assert!(forest.is_focused(), "the mode ended: {:#?}", sketch(&forest));
    assert_eq!(cursor(&forest), Some(&key("dunwich", "dun-7.1")));
    assert_eq!(behind_the_line_elsewhere(&forest), was);
}

/// A focused root its tracker would not read this time is not known to
/// have gone either. The forest stays rooted at it, and is drawn as it was
/// once the tracker answers again.
#[test]
fn a_focused_root_that_stopped_reading_stays_focused_until_it_reads_again() {
    let mut forest = flatten(built(Filter::All));
    focus_on(&mut forest, "dun-7");
    let rooted = sketch(&forest);

    forest.refresh(gather(
        vec![
            Tree::tracker_unreachable("dunwich", "dun-7", TrackerFailure::Auth),
            tree_of("harbour", HARBOUR),
        ],
        Vec::new(),
        Filter::All,
    ));
    assert!(
        !drawn_here(&forest, "hbr-3 dredge the channel"),
        "every root is back: {:#?}",
        sketch(&forest)
    );
    assert!(
        !drawn_here(&forest, "[OutOfTheWay dunwich]"),
        "the focused root went behind the line: {:#?}",
        sketch(&forest)
    );
    assert_eq!(cursor(&forest), Some(&key("dunwich", "dun-7")));

    forest.refresh(built(Filter::All));

    assert_eq!(sketch(&forest), rooted);
}

/// The view was started to show one bead, so that bead leaving does not
/// open the whole forest: the line where it stood says it is gone.
#[test]
fn a_named_bead_its_tracker_no_longer_holds_is_said_to_be_gone_where_it_stood() {
    let mut forest = named_on_the_command_line(&[("dunwich", "dun-7.1.1")]);

    let renamed = edited(DUNWICH, r#""id":"dun-7.1.1""#, r#""id":"dun-7.1.9""#);
    forest.refresh(gather(
        vec![
            tree_of("dunwich", &renamed),
            Tree::tracker_unreachable("ferry", "fer-2", TrackerFailure::Auth),
            tree_of("harbour", HARBOUR),
        ],
        Vec::new(),
        Filter::LiveAgents,
    ));

    assert!(forest.is_focused(), "the mode ended: {:#?}", sketch(&forest));
    assert!(
        !drawn_here(&forest, "fer-2"),
        "still rooted at one bead: {:#?}",
        sketch(&forest)
    );
    assert_eq!(cursor(&forest), Some(&key("dunwich", "dun-7.1.1")));
    assert!(
        drawn_here(&forest, "⚠ dun-7.1.1 gone"),
        "nothing says the bead is gone: {:#?}",
        sketch(&forest)
    );
}

/// Dunwich failing to answer, as a collection says it: no trees of its own,
/// and its failure among the failed projects.
fn dunwich_failed() -> Snapshot {
    gather(
        vec![
            Tree::tracker_unreachable("ferry", "fer-2", TrackerFailure::Auth),
            tree_of("harbour", HARBOUR),
        ],
        vec![FailedProject {
            project: "dunwich".into(),
            tracker: TrackerFailure::Unstartable,
        }],
        Filter::LiveAgents,
    )
}

/// The lines the mode holds the other projects' roots behind, as drawn.
fn behind_the_line_elsewhere(forest: &Forest) -> Vec<String> {
    sketch(forest)
        .into_iter()
        .filter(|row| row.contains("[OutOfTheWay ferry]") || row.contains("[OutOfTheWay harbour]"))
        .collect()
}

/// Going to a bead opens what is shut over it, and under this mode what is
/// shut over every other root is the line the mode put them behind. A
/// search that found a bead it cannot reach is a search that failed.
#[test]
fn going_to_a_held_back_bead_opens_the_line_holding_its_root() {
    let mut forest = flatten(snapshot());
    focus_on(&mut forest, "dun-7.1");

    assert!(
        forest.go_to(&key("harbour", "hbr-3")),
        "cannot reach harbour: {:#?}",
        sketch(&forest)
    );
    assert_eq!(cursor(&forest), Some(&key("harbour", "hbr-3")));
}

/// A root the filter was showing as much as one it was not. The mode holds
/// both back behind the one line, so that line is what is shut over either.
#[test]
fn going_to_a_bead_in_a_root_the_filter_was_showing_opens_it_as_well() {
    let mut forest = flatten(snapshot());
    select_hidden_tree(&mut forest);
    assert!(forest.apply(Action::FocusForest));

    assert!(
        forest.go_to(&key("dunwich", "dun-7.1")),
        "cannot reach dunwich: {:#?}",
        sketch(&forest)
    );
    assert_eq!(cursor(&forest), Some(&key("dunwich", "dun-7.1")));
}

/// The reader is put back on the bead they were finishing even where they
/// shut something over it while they were down there. A forest that comes
/// back with the selection somewhere else has not put them back.
#[test]
fn putting_the_forest_back_opens_what_has_been_shut_over_the_bead() {
    let mut forest = flatten(snapshot());
    focus_on(&mut forest, "dun-7.1");
    forest.folds.set(Handle::Project("dunwich".into()), false);

    assert!(forest.apply(Action::FocusForest));

    assert_eq!(cursor(&forest), Some(&key("dunwich", "dun-7.1")));
}

/// A bead that moved is still the bead. What ends the mode is the bead
/// going out of the collection, and a tracker that reparented it has done
/// nothing of the kind.
#[test]
fn a_collection_that_moved_the_focused_bead_stays_rooted_at_it() {
    let mut forest = flatten(snapshot());
    toggle_fold_of(&mut forest, "dun-7.1");
    focus_on(&mut forest, "dun-7.1.2");
    let moved = edited(
        DUNWICH,
        r#""seal the feed horn","status":"open",
   "dependencies":[{"depends_on_id":"dun-7.1""#,
        r#""seal the feed horn","status":"open",
   "dependencies":[{"depends_on_id":"dun-7""#,
    );

    forest.refresh(gather(
        vec![
            tree_of("dunwich", &moved),
            Tree::tracker_unreachable("ferry", "fer-2", TrackerFailure::Auth),
            tree_of("harbour", HARBOUR),
        ],
        Vec::new(),
        Filter::LiveAgents,
    ));

    assert!(
        !drawn_here(&forest, "fer-2"),
        "still rooted at one bead: {:#?}",
        sketch(&forest)
    );
    assert!(drawn_here(&forest, "seal the feed horn"));
}

/// The rule begins afresh where the moved bead now stands, so the forest
/// draws what rooting there after the move would have drawn.
#[test]
fn a_collection_that_moved_the_focused_bead_roots_the_forest_where_it_moved_to() {
    let moved = || {
        let moved = edited(
            DUNWICH,
            r#""seal the feed horn","status":"open",
   "dependencies":[{"depends_on_id":"dun-7.1""#,
            r#""seal the feed horn","status":"open",
   "dependencies":[{"depends_on_id":"dun-7""#,
        );
        gather(
            vec![
                tree_of("dunwich", &moved),
                Tree::tracker_unreachable("ferry", "fer-2", TrackerFailure::Auth),
                tree_of("harbour", HARBOUR),
            ],
            Vec::new(),
            Filter::LiveAgents,
        )
    };
    let mut followed = flatten(snapshot());
    toggle_fold_of(&mut followed, "dun-7.1");
    focus_on(&mut followed, "dun-7.1.2");
    followed.refresh(moved());

    let mut rooted_there = flatten(moved());
    focus_on(&mut rooted_there, "dun-7.1.2");

    assert_eq!(sketch(&followed), sketch(&rooted_there));
}

/// Pressing the key to come back out changes what is on the screen, so the
/// press says so. A press the loop reads as changing nothing leaves the
/// rooted forest drawn over a forest that has been put back.
#[test]
fn putting_the_forest_back_says_the_screen_changed() {
    let mut forest = flatten(snapshot());
    focus_on(&mut forest, "dun-7");

    assert!(forest.apply(Action::FocusForest));
}

/// A search counts the matches it can take the reader to. Under this mode
/// the beads above the focused one are drawn nowhere, so a search that
/// counted them would step onto one and report it as nothing found.
#[test]
fn every_match_a_search_counts_under_the_mode_can_be_landed_on() {
    let mut forest = flatten(snapshot());
    focus_on(&mut forest, "dun-7.1");

    let first = forest.seek_here("the");
    let Landed::On { of, .. } = first else {
        panic!("nothing matched: {first:?}")
    };
    for step in 1..=of {
        let landed = forest.next_match(true);
        assert!(
            matches!(landed, Some(Landed::On { .. })),
            "match {step} of {of} cannot be landed on: {landed:#?}",
        );
    }
}

/// A bead closing is not the bead leaving. `bdi` reads every bead a
/// tracker holds, so a closed one stays in the collection and the forest
/// stays rooted at it.
#[test]
fn the_focused_bead_closing_leaves_the_forest_rooted_at_it() {
    let mut forest = flatten(snapshot());
    focus_on(&mut forest, "dun-7.1");

    let closed = edited(
        DUNWICH,
        r#"{"id":"dun-7.1","title":"re-point the dish","status":"open"#,
        r#"{"id":"dun-7.1","title":"re-point the dish","status":"closed"#,
    );
    forest.refresh(gather(
        vec![
            tree_of("dunwich", &closed),
            Tree::tracker_unreachable("ferry", "fer-2", TrackerFailure::Auth),
            tree_of("harbour", HARBOUR),
        ],
        Vec::new(),
        Filter::LiveAgents,
    ));

    assert!(
        !drawn_here(&forest, "fer-2"),
        "still rooted at one bead: {:#?}",
        sketch(&forest)
    );
    assert!(drawn_here(&forest, "re-point the dish"));
}

/// A project, a line standing over a stretch of finished beads, and a root
/// whose tracker refused are all lines a reader can sit on and none of
/// them is a bead with a tree under it.
#[test]
fn asking_to_root_the_forest_at_a_line_that_is_not_a_bead_does_nothing() {
    let mut forest = flatten(snapshot());
    let was = sketch(&forest);

    select_project(&mut forest, "dunwich");
    assert!(!forest.apply(Action::FocusForest));
    assert_eq!(sketch(&forest), was);

    select_run(&mut forest);
    assert!(!forest.apply(Action::FocusForest));
    assert_eq!(sketch(&forest), was);

    select_bead(&mut forest, "fer-2");
    assert!(!forest.apply(Action::FocusForest));
    assert_eq!(sketch(&forest), was);
}

/// A bead deep in a tree is drawn where a root is drawn, and what hangs
/// under it is what hangs under it anywhere else.
///
/// It reads whole the way a root does, and the beads under it read against
/// it. A column of ids is read by putting the drawn root in front of each
/// one, so a suffix cut against a root that is behind the line names a
/// bead that is not there.
#[test]
fn focusing_a_bead_under_a_root_draws_it_where_that_root_was() {
    let mut forest = flatten(snapshot());
    toggle_fold_of(&mut forest, "dun-7.1");
    focus_on(&mut forest, "dun-7.1");

    assert_eq!(
        sketch(&forest)
            .into_iter()
            .take_while(|row| !row.contains("Unattributed"))
            .collect::<Vec<String>>(),
        vec![
            "▾ dunwich",
            "  ├── ○ dun-7.1 re-point the dish",
            "  │   ├── ! OrphanedDependencies(1)",
            "  │   ├── ○ .1 true the mount",
            "  │   └── ○ .2 seal the feed horn",
            "  ├─▸ [OutOfTheWay dunwich] 1",
        ]
    );
}

/// The mode stops drawing the beads above the focused one, and *degrade,
/// never disappear* binds over them as it does over a whole root: they go
/// behind the same line, under the project they are in, and the root they
/// hang from rests shut there as a held-back root does.
#[test]
fn the_beads_above_the_focused_bead_go_behind_the_line_as_a_root_does() {
    let mut forest = flatten(snapshot());
    focus_on(&mut forest, "dun-7.1");

    open_the_line_holding_roots_back(&mut forest, "dunwich");

    assert_eq!(
        sketch(&forest)
            .into_iter()
            .take_while(|row| !row.contains("Unattributed"))
            .collect::<Vec<String>>(),
        vec![
            "▾ dunwich",
            "  ├─▸ ○ dun-7.1 re-point the dish",
            "  │   └── ! OrphanedDependencies(1)",
            "  ├── [OutOfTheWay dunwich] 1",
            "  │   └─▸ ◐ dun-7 lift the ground station",
            "  │       └── ! OrphanedDependencies(1)",
        ]
    );
}

/// What is behind the line reads against the root it is drawn under, as it
/// does with the mode off. Only the drawing rooted at one bead shortens
/// against that bead, and the roots behind the line are not that drawing.
#[test]
fn a_bead_behind_the_line_reads_against_the_root_it_hangs_under() {
    let mut forest = flatten(snapshot());
    focus_on(&mut forest, "dun-7.1");
    open_the_line_holding_roots_back(&mut forest, "dunwich");

    toggle_fold_of(&mut forest, "dun-7");

    assert!(
        drawn_here(&forest, "○ .7 log the survey marks"),
        "{:#?}",
        sketch(&forest)
    );
}

/// The focused bead is drawn where a root is drawn, so the line the rest
/// of its root is behind leaves it there rather than drawing it twice.
#[test]
fn the_line_leaves_the_focused_bead_to_the_root_of_the_forest() {
    let mut forest = flatten(snapshot());
    focus_on(&mut forest, "dun-7.1");

    open_the_line_holding_roots_back(&mut forest, "dunwich");

    assert_eq!(
        sketch(&forest)
            .iter()
            .filter(|row| row.contains("re-point the dish"))
            .count(),
        1,
        "{:#?}",
        sketch(&forest)
    );
}

/// A seat on a bead above the focused one is a seat nothing else on the
/// screen says is there, so the line standing over that bead counts it.
#[test]
fn the_line_counts_the_seats_above_the_focused_bead() {
    let mut forest = flatten(snapshot());
    focus_on(&mut forest, "dun-7.1");

    let held = held_by_the_line(&forest, "dunwich");

    assert_eq!(
        held.live_agents,
        counts_of(&forest, "dunwich", "dun-7").live_agents,
        "every seat in dunwich is on a bead the mode stopped drawing"
    );
    assert!(held.live_agents > 0, "dunwich is where the agents are");
}

/// It counts what it is standing over rather than the whole root the beads
/// came from. The focused bead and what hangs beneath it are on the screen,
/// and a line counting those sends a reader looking for rows they are
/// already reading.
#[test]
fn the_line_counts_only_the_beads_the_mode_stopped_drawing() {
    let mut forest = flatten(snapshot());
    focus_on(&mut forest, "dun-7.1");

    let held = held_by_the_line(&forest, "dunwich");

    assert_eq!(
        held.total,
        counts_of(&forest, "dunwich", "dun-7").total - 3,
        "dun-7.1 and the two beads beneath it are drawn at the root"
    );
}

/// Going to a bead opens what is shut over it, and what is shut over a
/// bead above the focused one is the line the rest of its root is behind.
#[test]
fn going_to_a_bead_above_the_focused_one_opens_the_line_it_is_behind() {
    let mut forest = flatten(snapshot());
    focus_on(&mut forest, "dun-7.1");

    assert!(
        forest.go_to(&key("dunwich", "dun-7")),
        "cannot reach the bead above: {:#?}",
        sketch(&forest)
    );
    assert_eq!(cursor(&forest), Some(&key("dunwich", "dun-7")));
}

/// A run says what opening it would draw. Where the bead the forest is
/// rooted at came out of one, the run behind the line stands for the
/// siblings it left there rather than counting a bead drawn at the root of
/// the forest.
#[test]
fn a_run_behind_the_line_leaves_the_focused_bead_out_of_its_count() {
    let mut forest = flatten(snapshot());
    assert!(forest.go_to(&key("dunwich", "dun-7.2")), "no such bead");
    assert!(forest.apply(Action::FocusForest));

    assert!(
        forest.go_to(&key("dunwich", "dun-7.3")),
        "cannot reach a bead in the run: {:#?}",
        sketch(&forest)
    );

    assert!(
        drawn_here(&forest, "… 2 more"),
        "the run stands for the siblings left in it: {:#?}",
        sketch(&forest)
    );
}

/// And a run with the focused bead somewhere beneath one of its members
/// stands for one bead fewer for the same reason, rather than for what the
/// whole tree would have put behind it.
#[test]
fn a_run_behind_the_line_leaves_out_a_focused_bead_beneath_a_member() {
    let mut forest = flatten(depot());
    assert!(forest.go_to(&key("dunwich", "dep-1.2.1")), "no such bead");
    assert!(forest.apply(Action::FocusForest));

    assert!(
        forest.go_to(&key("dunwich", "dep-1.3")),
        "cannot reach the run: {:#?}",
        sketch(&forest)
    );

    assert!(drawn_here(&forest, "… 5 more"), "{:#?}", sketch(&forest));
}

/// A bead reachable more than once is drawn once for every way down to it,
/// and the key roots the forest at the copy the reader pressed it on. The
/// mode draws none of the others, so going to that bead goes to the copy
/// it is drawn at rather than to the first way down to it.
#[test]
fn going_to_the_focused_bead_reaches_the_copy_it_is_rooted_at() {
    let mut forest = under_every_copy(drawn_twice_in_one_tree());
    let [_, lower] = copies_of(&forest, "dun-9");
    step_onto(&mut forest, lower);
    assert!(forest.apply(Action::FocusForest));

    assert!(
        forest.go_to(&key("dunwich", "dun-9")),
        "cannot reach the bead the forest is rooted at: {:#?}",
        sketch(&forest)
    );
}

/// Rooting the forest at a copy begins the rule afresh there, under
/// whichever rule is in force: a reader cannot tell a first copy from a
/// later one, so rooting at either draws the same forest.
#[test]
fn rooting_the_forest_at_a_later_copy_draws_what_rooting_at_the_first_does() {
    for rule in Spine::EVERY {
        let rooted_at = |copy: usize| {
            let mut forest = flatten(deep_bead_drawn_twice_in_one_tree());
            put_in_force(&mut forest, *rule, Action::CycleSpineForest);
            // A one-copy rule rests the later copy's way shut.
            let guying = lines_of(&forest, "dun-3.2")[0];
            step_onto(&mut forest, guying);
            forest.apply(Action::ExpandOrChild);
            let at = copies_of(&forest, "dun-6")[copy];
            step_onto(&mut forest, at);
            assert!(forest.apply(Action::FocusForest));
            forest
                .lines()
                .iter()
                .map(|line| format!("{}{} {:?}", line.prefix, said(&line.content), line.folded))
                .collect::<Vec<_>>()
        };

        let first = rooted_at(0);
        assert!(
            first
                .iter()
                .any(|line| line.contains("bore the bolt holes")),
            "{rule:?}: the first copy opens onto the work: {first:#?}"
        );
        assert_eq!(rooted_at(1), first, "{rule:?}");
    }
}

/// And everything only that copy reaches. The first way down to one of
/// those beads goes through the copy the mode draws nowhere, so a search
/// answering with it would count a bead it could not land on.
#[test]
fn going_to_a_bead_under_the_focused_copy_reaches_it() {
    let mut forest = under_every_copy(drawn_twice_in_one_tree());
    let [_, lower] = copies_of(&forest, "dun-9");
    step_onto(&mut forest, lower);
    assert!(forest.apply(Action::FocusForest));

    assert!(
        forest.go_to(&key("dunwich", "dun-9.1")),
        "cannot reach the bead beneath it: {:#?}",
        sketch(&forest)
    );
    assert_eq!(cursor(&forest), Some(&key("dunwich", "dun-9.1")));
}

/// A root the tracker files under another root is still the bead the
/// reader was finishing. Its place stops being a root's, so the mode goes
/// looking for the bead, exactly as it does for a bead moved further down.
#[test]
fn a_collection_that_filed_the_focused_root_under_another_stays_rooted_at_it() {
    let staffed = panes_on(&["hbr-9.1"]);
    let mut forest = flatten(together("dunwich", &[HARBOUR, SLIPWAY], &staffed));
    focus_on(&mut forest, "hbr-9");
    assert!(
        !drawn_here(&forest, "dredge the channel"),
        "rooted at one bead: {:#?}",
        sketch(&forest)
    );

    forest.refresh(together("dunwich", &[SLIPWAY_UNDER_HARBOUR], &staffed));

    assert!(
        drawn_here(&forest, "re-deck the slipway"),
        "the bead is still where a root is drawn: {:#?}",
        sketch(&forest)
    );
    assert!(
        drawn_here(&forest, "[OutOfTheWay dunwich]"),
        "still rooted at one bead: {:#?}",
        sketch(&forest)
    );
}

/// A search counts in the order the rows are drawn, and the bead the
/// forest is rooted at is the first row on the screen however its tree
/// came in its project's own order.
#[test]
fn a_search_counts_from_the_bead_the_forest_is_rooted_at() {
    let mut forest = flatten(together(
        "dunwich",
        &[HARBOUR, SLIPWAY],
        &panes_on(&["hbr-9.1"]),
    ));
    assert!(forest.go_to(&key("dunwich", "hbr-3.1")), "no such bead");
    assert!(forest.apply(Action::FocusForest));
    forest.apply(Action::Move(Motion::FirstRow));

    let landed = forest.seek_here("the");

    let Landed::On { key: found, at, .. } = landed else {
        panic!("nothing matched: {landed:?}")
    };
    assert_eq!((found, at), (key("dunwich", "hbr-3.1"), 1));
}

/// Stepping through matches from a row that is not a bead carries on from
/// where the reader is standing, and a shut line is asked what it holds
/// rather than read past. This line holds beads exactly as the filter's
/// does, so a reader standing on it steps into what is behind it.
#[test]
fn stepping_from_the_shut_line_carries_on_into_the_roots_behind_it() {
    let mut forest = flatten(snapshot());
    focus_on(&mut forest, "dun-7.1");
    forest.seek_here("the");
    let at = the_line_holding_roots_back(&forest, "dunwich");
    step_onto(&mut forest, at);
    assert_eq!(
        forest.lines()[at].folded,
        Some(false),
        "the line rests shut"
    );

    let landed = forest.next_match(true);

    let Some(Landed::On { key: found, .. }) = landed else {
        panic!("nothing matched: {landed:?}")
    };
    assert_eq!(found, key("dunwich", "dun-7"));
}

/// A root whose tracker refused leads its project whatever the filter
/// says, and it holds no bead. The line has to look past it for the bead
/// it stands on, or a reader stepping off it walks past everything it
/// holds and wraps round to the top of the screen.
#[test]
fn stepping_from_the_shut_line_reaches_past_a_root_holding_no_bead() {
    let mut forest = flatten(gather(
        vec![
            tree_of("dunwich", DUNWICH),
            Tree::tracker_unreachable("dunwich", "dun-0", TrackerFailure::Auth),
            tree_of("harbour", HARBOUR),
        ],
        Vec::new(),
        Filter::LiveAgents,
    ));
    focus_on(&mut forest, "dun-7.1");
    forest.seek_here("the");
    let at = the_line_holding_roots_back(&forest, "dunwich");
    step_onto(&mut forest, at);

    let landed = forest.next_match(true);

    let Some(Landed::On { key: found, .. }) = landed else {
        panic!("nothing matched: {landed:?}")
    };
    assert_eq!(found, key("dunwich", "dun-7"));
}

/// Open every fold on the screen, with the keys a reader has, until the
/// rows are the whole of what the forest holds.
///
/// Counted out before it starts like any other walk here. Each press opens
/// one fold and draws what it was over, so a screen with a fold left shut
/// after a press per row it started with is one the walk says it never
/// got to the end of.
fn open_everything(forest: &mut Forest) {
    walk::until(
        forest,
        |forest| shut_fold(forest).is_none(),
        |forest| {
            let at = shut_fold(forest).expect("a fold to open");
            forest.select_line(at);
            forest.apply(Action::ExpandOrChild);
        },
        |forest| format!("a fold would not open: {:#?}", sketch(forest)),
    );
}

/// The first row on the screen resting shut over something.
fn shut_fold(forest: &Forest) -> Option<usize> {
    forest
        .lines()
        .iter()
        .position(|line| line.folded == Some(false))
}

/// The order a search counts in and the order the rows come out are the
/// same order, and both are the drawing's answer rather than two accounts
/// of it that agree because they were written to. Under the mode as
/// without it, because the mode is what moves the rows. A bead this tree
/// draws twice counts twice here, since `bdi-7ao.136`.
#[test]
fn a_search_enumerates_the_beads_in_the_order_the_rows_draw_them() {
    for rooted in [None, Some("dun-7.1")] {
        let mut forest = flatten(snapshot());
        if let Some(bead) = rooted {
            focus_on(&mut forest, bead);
        }
        open_everything(&mut forest);
        // A root whose tracker refused has a row and no bead on it, and a
        // search offers beads. It is named on its line rather than left
        // out, which is the one row here that stands for no bead.
        let on_screen: Vec<Place> = forest
            .lines()
            .iter()
            .filter(|line| matches!(line.content, Content::Bead(_)))
            .filter_map(|line| line.place.clone())
            .collect();

        // An empty search matches every bead, so it counts the whole
        // order: every place in it, and where each one comes.
        let mut every = forest.matches(Sought::holding(""));
        let counted: Vec<Place> = (0..every.len()).filter_map(|at| every.nth(at)).collect();
        let numbered: Vec<Option<(usize, bool)>> =
            on_screen.iter().map(|place| every.before(place)).collect();

        assert_eq!(counted, on_screen, "rooted at {rooted:?}");
        assert_eq!(
            numbered,
            (0..on_screen.len())
                .map(|at| Some((at, true)))
                .collect::<Vec<_>>(),
            "rooted at {rooted:?}"
        );
    }
}

/// A search counts the matches it can take the reader to, and a line it
/// can open is a match it can take them to.
#[test]
fn a_search_counts_a_bead_above_the_focused_one() {
    let mut forest = flatten(snapshot());
    focus_on(&mut forest, "dun-7.1");

    let landed = forest.seek_here("lift the ground station");

    let Landed::On { key: found, .. } = landed else {
        panic!("nothing matched: {landed:?}")
    };
    assert_eq!(found, key("dunwich", "dun-7"));
}
