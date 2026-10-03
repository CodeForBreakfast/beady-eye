//! Groups, chiefly the one holding the trees the filter hides, and a selection on them.

use super::*;
use pretty_assertions::assert_eq;

/// A group the collect empties puts the selection back on the first root,
/// not on whatever line happened to sit above where the group was. The
/// two answers only differ once the forest is more than a row tall, which
/// is every forest a reader has.
///
/// This is the only one of `group_drawn`'s two callers that can tell you
/// it is wrong. `first_handle` cannot: `lay_out` looks the handle it
/// returns up, does not find it drawn, and repairs the cursor from the
/// selected index — so a `group_drawn` that said yes to every kind would
/// go unnoticed down that road.
#[test]
fn a_selection_on_a_group_that_empties_goes_back_to_the_first_root() {
    let mut forest = flatten(snapshot());
    let group = forest
        .lines()
        .iter()
        .position(|line| {
            matches!(&line.content, Content::Group(group)
                if group.kind == GroupKind::FailedProjects)
        })
        .expect("the shared snapshot draws a group for the project that failed");
    step_onto(&mut forest, group);

    forest.refresh(built_without_the_failed_project());

    assert_eq!(
        forest.lines()[forest.selected_line()]
            .bead()
            .map(|bead| bead.id.clone()),
        Some("dun-7".to_string()),
        "{:#?}",
        sketch(&forest)
    );
}

/// The same snapshot with the project that failed before its roots were
/// known now reading, which empties the group it was the only member of.
fn built_without_the_failed_project() -> Snapshot {
    let mut snapshot = snapshot();
    snapshot.failed_projects.clear();
    snapshot
}

#[test]
fn an_empty_group_draws_nothing() {
    let dunwich = assembled(DUNWICH);
    let harbour = assembled(HARBOUR);
    let joined = joined(&dunwich, &harbour, &panes());
    let snapshot = snapshot::build(
        Collected {
            trees: vec![tree_of("dunwich", DUNWICH)],
            failed_projects: Vec::new(),
            read_at: every_project_read(),
            speaks_until: BTreeMap::new(),
            read_for_reach: BTreeSet::new(),
        },
        &[],
        &Joined {
            agents: joined.agents,
            refused: BTreeMap::new(),
            out_of_reach: BTreeSet::new(),
            conflicts: Vec::new(),
        },
        &cfg(),
        a_provider(ProviderState::Answering),
        Filter::LiveAgents,
        now(),
    );

    assert!(!sketch(&flatten(snapshot))
        .iter()
        .any(|line| line.contains('[')));
}

#[test]
fn a_group_draws_one_line_for_each_thing_it_holds_when_it_is_opened() {
    let mut forest = flatten(snapshot());
    let loose = |forest: &Forest| {
        sketch(forest)
            .iter()
            .filter(|line| line.contains("Loose"))
            .count()
    };
    let group = forest
        .lines()
        .iter()
        .position(|line| {
            matches!(&line.content, Content::Group(group)
                if group.kind == GroupKind::Unattributed
                    && group.project.as_deref() == Some("dunwich"))
        })
        .expect("dunwich has panes no bead claims");
    forest.select_line(group);

    // Ferry's one loose pane is under ferry's own line, and stays.
    assert!(forest.apply(Action::ToggleFold));
    assert_eq!(loose(&forest), 1, "{:#?}", sketch(&forest));

    assert!(forest.apply(Action::ToggleFold));
    assert_eq!(loose(&forest), 3, "{:#?}", sketch(&forest));
}

/// The panes under no configured project open into their own directories,
/// which is the whole use of the group: the line says a `[[projects]]`
/// entry is missing and opening it says which one.
#[test]
fn opening_the_unconfigured_group_names_the_directories() {
    let mut forest = flatten(snapshot());
    forest
        .folds
        .set(Handle::Group(GroupKind::Unconfigured, None), true);
    forest.refresh(snapshot());

    let drawn = sketch(&forest);

    assert!(
        drawn
            .iter()
            .any(|line| line.contains("Unconfigured") && line.contains("/srv/spike")),
        "{drawn:#?}"
    );
}

/// The filter's choice holds — a hidden tree is not drawn — but a group
/// that says only how many trees it hides reads like "nothing to see"
/// when one of them is waiting on work bd never returned.
#[test]
fn the_hidden_trees_group_says_how_many_of_them_have_findings() {
    let broken = edited(
        HARBOUR,
        r#"{"depends_on_id":"hbr-3","type":"parent-child"}"#,
        r#"{"depends_on_id":"hbr-3","type":"parent-child"},
                   {"depends_on_id":"hbr-9","type":"blocks"}"#,
    );
    let snapshot = gather(
        vec![tree_of("dunwich", DUNWICH), tree_of("harbour", &broken)],
        Vec::new(),
        Filter::LiveAgents,
    );

    let group = hidden_trees_group(&flatten(snapshot), "harbour");

    assert_eq!(group.count, 1);
    assert_eq!(group.with_findings, 1);
}

#[test]
fn a_hidden_tree_with_nothing_wrong_in_it_is_only_counted_as_hidden() {
    let group = hidden_trees_group(&flatten(snapshot()), "harbour");

    assert_eq!(group.count, 1);
    assert_eq!(group.with_findings, 0);
}

/// What the group says of the trees it hides was settled when the filter
/// hid them. A count that had to go back to `collected` for it would
/// cost every press a walk over the whole forest, and a forest of
/// thousands of hidden trees was paying half its keystroke for that.
#[test]
fn the_hidden_trees_group_does_not_go_back_to_the_collected_trees_for_its_count() {
    let broken = edited(
        HARBOUR,
        r#"{"depends_on_id":"hbr-3","type":"parent-child"}"#,
        r#"{"depends_on_id":"hbr-3","type":"parent-child"},
                   {"depends_on_id":"hbr-9","type":"blocks"}"#,
    );
    let mut snapshot = gather(
        vec![tree_of("dunwich", DUNWICH), tree_of("harbour", &broken)],
        Vec::new(),
        Filter::LiveAgents,
    );
    snapshot.collected.clear();

    let group = hidden_trees_group(&flatten(snapshot), "harbour");

    assert_eq!(group.count, 1);
    assert_eq!(group.with_findings, 1);
}

/// A hidden tree's findings are the ones in its own tree. Harbour hides
/// two roots and only the slipway has anything wrong in it, so a match
/// that asked the project alone would report the channel as hiding a
/// finding that is not in it.
#[test]
fn a_hidden_tree_does_not_take_a_finding_from_another_root_in_its_project() {
    let snapshot = gather(
        vec![tree_of("harbour", HARBOUR), tree_of("harbour", SLIPWAY)],
        Vec::new(),
        Filter::LiveAgents,
    );

    let group = hidden_trees_group(&flatten(snapshot), "harbour");

    assert_eq!(group.count, 2);
    assert_eq!(group.with_findings, 1);
}

/// Bead ids are numbered per tracker and the trackers do not coordinate,
/// so two projects can each hold a root called `hbr-3` and they are
/// different beads. Each is hidden under its own project, and a match
/// that asked the root alone would report harbour as hiding the finding
/// that is in dunwich's.
#[test]
fn a_hidden_tree_does_not_take_a_finding_from_the_same_root_in_another_project() {
    let colliding = edited(SLIPWAY, "hbr-9", "hbr-3");
    let forest = flatten(gather(
        vec![tree_of("harbour", HARBOUR), tree_of("dunwich", &colliding)],
        Vec::new(),
        Filter::LiveAgents,
    ));

    let harbour = hidden_trees_group(&forest, "harbour");
    let dunwich = hidden_trees_group(&forest, "dunwich");

    assert_eq!((harbour.count, harbour.with_findings), (1, 0));
    assert_eq!((dunwich.count, dunwich.with_findings), (1, 1));
}

/// The line over one project's hidden trees.
fn hidden_trees_group(forest: &Forest, project: &str) -> Group {
    forest
        .lines()
        .iter()
        .find_map(|line| match &line.content {
            Content::Group(group)
                if group.kind == GroupKind::HiddenTrees
                    && group.project.as_deref() == Some(project) =>
            {
                Some(group.clone())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("the filter hid a tree of {project}"))
}

/// The line `scope` names, and everything drawn beneath it, read off lines
/// a test has drawn whole.
///
/// Beneath is depth: the lines after it, up to the first one standing at its
/// own depth or shallower. A project is depth zero, the roots under it are
/// one, and a group's things are one under a group that is also zero, so the
/// scope of a project stops at the next project or the first group.
fn subtree_of<'a>(drawn: &'a [Line], scope: &Handle) -> &'a [Line] {
    let Some(at) = drawn
        .iter()
        .position(|line| handle_of(line).as_ref() == Some(scope))
    else {
        return &[];
    };
    let depth = drawn[at].depth;
    let end = drawn[at + 1..]
        .iter()
        .position(|line| line.depth <= depth)
        .map_or(drawn.len(), |past| at + 1 + past);
    &drawn[at..end]
}

/// The selected line and everything drawn beneath it, sketched.
fn beneath_the_selection(forest: &Forest) -> Vec<String> {
    let scope = forest
        .handle_at(forest.selected_line())
        .expect("the selection is on a line it can hold");
    let lines: Vec<Line> = forest.lines().iter().cloned().collect();
    subtree_of(&lines, &scope)
        .iter()
        .map(|line| format!("{}{}", line.prefix, said(&line.content)))
        .collect()
}

/// A hidden tree's row is its root's row, so it stands for that bead as
/// a tree's header does: the show key and `y` have a bead to work on.
#[test]
fn a_hidden_trees_row_stands_for_its_root() {
    let mut forest = flatten(snapshot());
    select_hidden_tree(&mut forest);

    assert_eq!(cursor(&forest), Some(&key("harbour", "hbr-3")));
}

/// A hidden tree is a tree, and the group is only where the filter put
/// it: it is drawn there as its project would draw it, one level further
/// in under the group's line, and opens onto the same rows.
#[test]
fn a_hidden_tree_is_drawn_in_the_group_as_its_project_would_draw_it() {
    let mut forest = flatten(snapshot());
    select_hidden_tree(&mut forest);
    assert_eq!(
        beneath_the_selection(&forest),
        ["      └─▸ ○ hbr-3 dredge the channel"],
        "a hidden tree rests shut"
    );

    assert!(forest.apply(Action::ExpandOrChild));

    let in_the_group = beneath_the_selection(&forest);
    assert_eq!(
        in_the_group,
        [
            "      └── ○ hbr-3 dredge the channel",
            "          └── ○ .1 survey the silt"
        ]
    );
    forest.apply(Action::ToggleFilter);
    assert_eq!(forest.snapshot().filter, Filter::All);
    select(&mut forest, &key("harbour", "hbr-3"));
    assert_eq!(
        beneath_the_selection(&forest),
        [
            "  └── ○ hbr-3 dredge the channel",
            "      └── ○ .1 survey the silt"
        ],
        "the same rows under its project, the fold the reader opened included"
    );
}

/// Graeme: *"the top-level trees with no live agent should not be
/// expanded by default"*. A hidden tree rests shut whatever is beneath
/// it, where the same tree shown under its project rests open onto the
/// work a reader could start; it is the one thing about a hidden tree
/// that differs from the same tree shown.
#[test]
fn a_hidden_tree_rests_shut_even_over_work_that_would_open_it_shown() {
    let ready = ready_alone("dunwich", HARBOUR, &[], &["hbr-3.1"]);
    assert_eq!(
        lines_of(&flatten(ready.clone()), "hbr-3.1").len(),
        1,
        "shown, the tree rests open onto its ready work"
    );

    let mut hidden = ready;
    hidden.refilter(Filter::LiveAgents);
    let mut forest = flatten(hidden);
    select_hidden_tree(&mut forest);

    assert_eq!(forest.lines()[forest.selected_line()].folded, Some(false));
    assert_eq!(
        lines_of(&forest, "hbr-3.1").len(),
        0,
        "{:#?}",
        sketch(&forest)
    );
}

/// Opening a hidden tree is the reader's choice and not the filter's, so
/// letting go of the folds shuts it again: the group is shut with the
/// rest, and opened once more by hand it holds the row shut over its
/// tree, as the filter left it.
#[test]
fn the_default_puts_an_opened_hidden_tree_back() {
    let mut forest = flatten(snapshot());
    select_hidden_tree(&mut forest);
    let shut = beneath_the_selection(&forest);
    forest.apply(Action::ExpandOrChild);
    assert_ne!(beneath_the_selection(&forest), shut, "the tree opened");

    forest.apply(Action::RestoreDefault);

    assert_eq!(
        hidden_trees_group(&forest, "harbour").count,
        1,
        "the group is drawn shut: {:#?}",
        sketch(&forest)
    );
    select_hidden_tree(&mut forest);
    assert_eq!(beneath_the_selection(&forest), shut);
}

/// A tree that is only its root has nothing to open onto, and a fold
/// over nothing would be a key that does nothing.
#[test]
fn a_hidden_tree_with_nothing_beneath_its_root_offers_no_fold() {
    let lone = r#"[{"id":"hbr-1","title":"moor the lightship","status":"open"}]"#;
    let mut forest = flatten(gather(
        vec![tree_of("harbour", lone)],
        Vec::new(),
        Filter::LiveAgents,
    ));
    select_hidden_tree(&mut forest);
    let row = forest.selected_line();

    assert_eq!(forest.lines()[row].folded, None);
    assert_eq!(
        beneath_the_selection(&forest),
        ["      └── ○ hbr-1 moor the lightship"]
    );
    assert!(!forest.apply(Action::ExpandOrChild), "nothing to open onto");
}

/// A hidden tree's facts are a tree's facts, answered when the forest
/// takes the snapshot: drawing it, opening it and moving through it ask
/// nothing of the tree.
#[test]
fn a_keystroke_in_the_hidden_trees_group_walks_no_subtree() {
    let mut forest = flatten(snapshot());
    let before = walks_on_this_thread();

    select_hidden_tree(&mut forest);
    forest.apply(Action::ExpandOrChild);
    forest.apply(Action::Move(Motion::NextRow));

    assert_eq!(walks_on_this_thread() - before, 0);
}

/// A hidden tree's findings are drawn under its root as any tree's are,
/// whether the root is folded or not. The group holding it shut is what
/// keeps them off the screen, and the group's line admits to them.
#[test]
fn a_hidden_trees_findings_are_drawn_under_its_root_as_any_trees_are() {
    let mut forest = flatten(gather(
        vec![tree_of("dunwich", DUNWICH), tree_of("harbour", SLIPWAY)],
        Vec::new(),
        Filter::LiveAgents,
    ));
    select_hidden_tree(&mut forest);

    assert_eq!(
        beneath_the_selection(&forest),
        [
            "      └─▸ ○ hbr-9 re-deck the slipway",
            "          └── ! OrphanedDependencies(1)"
        ]
    );
}

/// `e` on the hidden-trees group opens every hidden tree to the bottom.
/// The walk's budget is counted off the beads, and a hidden tree's beads
/// are as much of the forest as a shown tree's: a budget counted off the
/// shown trees alone runs out on a forest that shows none.
#[test]
fn expanding_the_hidden_trees_group_reaches_the_bottom_of_a_deep_hidden_tree() {
    let chain = r#"[
      {"id":"hbr-5","title":"root","status":"open"},
      {"id":"hbr-5.1","title":"one","status":"open",
       "dependencies":[{"depends_on_id":"hbr-5","type":"parent-child"}]},
      {"id":"hbr-5.1.1","title":"two","status":"open",
       "dependencies":[{"depends_on_id":"hbr-5.1","type":"parent-child"}]},
      {"id":"hbr-5.1.1.1","title":"three","status":"open",
       "dependencies":[{"depends_on_id":"hbr-5.1.1","type":"parent-child"}]}
    ]"#;
    let mut forest = flatten(gather(
        vec![tree_of("harbour", chain)],
        Vec::new(),
        Filter::LiveAgents,
    ));
    assert!(forest.snapshot().trees.is_empty(), "every tree is hidden");
    let group = forest
        .lines()
        .iter()
        .position(|line| {
            matches!(&line.content, Content::Group(group) if group.kind == GroupKind::HiddenTrees)
        })
        .expect("the group is drawn");
    forest.select_line(group);
    assert_eq!(forest.selected_line(), group);

    forest.apply(Action::ExpandSubtree);

    assert_eq!(
        lines_of(&forest, "hbr-5.1.1.1").len(),
        1,
        "{:#?}",
        sketch(&forest)
    );
}

/// A tree the filter takes from under the selection goes into the
/// hidden-trees group, and so does the selection: with the group shut
/// over it, the group's line is where the tree went, which is more than
/// whatever row happened to be nearest can say.
#[test]
fn putting_the_filter_back_over_the_selected_tree_moves_the_selection_to_the_group() {
    let mut forest = flatten(built(Filter::All));
    select(&mut forest, &key("harbour", "hbr-3"));

    forest.apply(Action::ToggleFilter);

    assert_eq!(forest.snapshot().filter, Filter::LiveAgents);
    assert!(on_the_hidden_trees_group(&forest), "{:#?}", sketch(&forest));
}

/// The same where a refresh is what hides it: the agent that kept the
/// tree on the screen has gone, and the selection was on a bead inside.
/// A pane on no bead keeps a group drawn below the hidden trees, so the
/// nearest row to where the selection was is not the group's line.
#[test]
fn a_refresh_that_hides_the_selected_tree_moves_the_selection_to_the_group() {
    let mut staffed = alone("dunwich", TOWER, &panes_on(&["tow-1.1", "nobody"]));
    staffed.refilter(Filter::LiveAgents);
    let mut forest = flatten(staffed);
    select(&mut forest, &key("dunwich", "tow-1.1"));

    forest.refresh(alone("dunwich", TOWER, &panes_on(&["nobody"])));

    assert_eq!(forest.snapshot().hidden_trees.len(), 1);
    assert_eq!(forest.snapshot().unattributed.len(), 1);
    assert!(on_the_hidden_trees_group(&forest), "{:#?}", sketch(&forest));
}

/// With the group open, the tree's root is drawn there under the same
/// handle, so the selection simply follows the tree into the group.
#[test]
fn with_the_group_open_the_selection_follows_the_tree_the_filter_hides() {
    let mut forest = flatten(built(Filter::All));
    forest.folds.set(
        Handle::Group(GroupKind::HiddenTrees, Some("harbour".into())),
        true,
    );
    select(&mut forest, &key("harbour", "hbr-3"));

    forest.apply(Action::ToggleFilter);

    assert_eq!(cursor(&forest), Some(&key("harbour", "hbr-3")));
}

/// With the group open and the selection on a bead inside a tree that
/// rested open on its own account, the root the tree now rests shut
/// under is where the tree went, which is more than the nearest row can
/// say. A fold the reader had opened by hand would have kept the bead
/// drawn, and the selection with it.
#[test]
fn with_the_group_open_a_bead_inside_the_hidden_tree_falls_back_to_its_root() {
    let mut forest = flatten(ready_alone("dunwich", HARBOUR, &[], &["hbr-3.1"]));
    forest.folds.set(
        Handle::Group(GroupKind::HiddenTrees, Some("dunwich".into())),
        true,
    );
    select(&mut forest, &key("dunwich", "hbr-3.1"));

    forest.apply(Action::ToggleFilter);

    assert_eq!(forest.snapshot().filter, Filter::LiveAgents);
    assert_eq!(cursor(&forest), Some(&key("dunwich", "hbr-3")));
}

/// Only the cursor's own tree going into the group takes the selection
/// there. Letting go of the folds shuts one over a bead in a tree that is
/// still drawn, and the selection takes the nearest row as it always has,
/// however many other trees of the same project the group is shut over.
#[test]
fn a_fold_shutting_over_the_selection_keeps_it_out_of_the_hidden_trees_group() {
    let mut staffed = together("dunwich", &[TOWER, HARBOUR], &panes_on(&["tow-1.1"]));
    staffed.refilter(Filter::LiveAgents);
    let mut forest = flatten(staffed);
    assert_eq!(forest.snapshot().hidden_trees.len(), 1);
    select(&mut forest, &key("dunwich", "tow-1.1"));
    forest.apply(Action::ToggleFold);
    forest.apply(Action::Move(Motion::NextRow));
    assert_eq!(cursor(&forest), Some(&key("dunwich", "tow-1.1.1")));

    forest.apply(Action::RestoreDefault);

    assert_eq!(
        cursor(&forest),
        Some(&key("dunwich", "tow-1.2")),
        "{:#?}",
        sketch(&forest)
    );
}

/// Only a hidden tree takes findings out of the forest with it. Every
/// other group holds its own subject in full, so none of them has
/// anything undrawn to admit to.
#[test]
fn no_other_group_claims_to_be_hiding_findings() {
    let forest = flatten(snapshot());
    let others: Vec<Group> = forest
        .lines()
        .iter()
        .filter_map(|line| match &line.content {
            Content::Group(group) if group.kind != GroupKind::HiddenTrees => Some(group.clone()),
            _ => None,
        })
        .collect();

    assert_eq!(others.len(), 5);
    assert!(
        others.iter().all(|group| group.with_findings == 0),
        "{others:#?}"
    );
}

/// Every fold state over every root, every group the snapshot draws and
/// one interior node: a few hundred of them, which is small enough to
/// visit rather than sample.
#[test]
fn nothing_reported_disappears_under_any_fold_state() {
    let snapshot = snapshot();
    let mut handles = vec![
        Handle::Bead(Place::root(key("dunwich", "dun-7"))),
        Handle::Bead(Place::root(key("ferry", "fer-2"))),
        Handle::Bead(Place::root(key("dunwich", "dun-7")).step_to(key("dunwich", "dun-7.1"))),
    ];
    handles.extend(
        layout::every_group(&snapshot)
            .filter(|(kind, project)| {
                layout::group_drawn(&snapshot, *kind, project.as_deref(), &[])
            })
            .map(|(kind, project)| Handle::Group(kind, project)),
    );
    assert_eq!(handles.len(), 9, "{handles:#?}");
    for state in 0..1 << handles.len() {
        let mut forest = flatten(snapshot.clone());
        for (bit, handle) in handles.iter().enumerate() {
            forest.folds.set(handle.clone(), state & (1 << bit) == 0);
        }
        forest.refresh(snapshot.clone());

        let hidden_trees_open = forest.lines().iter().any(|line| {
            matches!(&line.content, Content::Group(group) if group.kind == GroupKind::HiddenTrees)
                && line.folded == Some(true)
        });
        assert_eq!(
            on_screen(&forest),
            in_the_snapshot(&snapshot, hidden_trees_open),
            "fold state {state:b}"
        );
    }
}

/// What the screen owes the reader: everything in the shown trees, and
/// the hidden trees' findings too once the group holding them is open —
/// shut, that group's line admits to them as a count instead.
fn in_the_snapshot(snapshot: &Snapshot, hidden_trees_open: bool) -> Reported {
    let drawn: Vec<&Tree> = snapshot
        .trees
        .iter()
        .map(Arc::as_ref)
        .chain(
            snapshot
                .hidden_trees
                .iter()
                .filter(|_| hidden_trees_open)
                .filter_map(|hidden| snapshot.tree(&key(&hidden.project, &hidden.root))),
        )
        .collect();
    Reported {
        orphaned_dependencies: drawn.iter().map(|t| t.orphaned_dependencies.len()).sum(),
        cycles: drawn.iter().map(|t| t.cycles.len()).sum(),
        conflicts: snapshot.conflicts.len(),
        failed_projects: snapshot.failed_projects.len(),
        loose_panes: snapshot.unattributed.len(),
        unconfigured_panes: snapshot.unconfigured.len(),
    }
}
