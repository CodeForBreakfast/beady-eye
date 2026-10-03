//! A project's own line, and the loose panes and groups hanging under it.

use super::*;
use pretty_assertions::assert_eq;

/// A pane working in a project's paths that no bead claims is the
/// project's, so it is drawn under the project's own line rather than in
/// a group below the trees: one place to look for everything beneath a
/// project, whether or not its roots read.
#[test]
fn every_loose_pane_hangs_under_its_own_projects_line() {
    let forest = flatten(snapshot());
    let loose: Vec<(&str, String)> = forest
        .lines()
        .iter()
        .enumerate()
        .filter_map(|(at, line)| match &line.content {
            Content::Item(Item::Loose(pane)) => {
                Some((pane.project.as_str(), project_above(&forest, at)))
            }
            _ => None,
        })
        .collect();

    assert_eq!(
        loose.len(),
        forest.snapshot().unattributed.len(),
        "{:#?}",
        sketch(&forest)
    );
    for (project, above) in loose {
        assert_eq!(project, above, "{:#?}", sketch(&forest));
    }
}

/// The project whose line is the nearest one above `at`.
fn project_above(forest: &Forest, at: usize) -> String {
    (0..at)
        .rev()
        .find_map(|row| match &forest.lines()[row].content {
            Content::Project(line) => Some(line.project.clone()),
            _ => None,
        })
        .expect("a line under a project has one above it")
}

/// A root that would not read says so on a line of its own under its
/// project. The failure is that root's and not the project's — the
/// project answered, and this one root did not — so it rides where the
/// root's row would have been rather than on the line above.
#[test]
fn a_root_that_could_not_be_read_says_so_where_its_row_would_have_been() {
    let forest = flatten(snapshot());
    let at = forest
        .lines()
        .iter()
        .position(|line| matches!(&line.content, Content::Project(line) if line.project == "ferry"))
        .expect("ferry has a line");

    assert!(
        matches!(
            &forest.lines()[at + 1].content,
            Content::Unread(unread)
                if unread.root == "fer-2"
                    && unread.tracker == TrackerState::Unreachable(TrackerFailure::Auth)
        ),
        "{:#?}",
        sketch(&forest)
    );
}

/// A project with no loose pane draws no line saying so: a group's line
/// on every healthy project buries the one where it matters.
#[test]
fn a_project_with_no_loose_pane_draws_no_line_for_them() {
    let forest = flatten(snapshot());

    assert!(
        !forest.lines().iter().any(|line| matches!(
            &line.content,
            Content::Group(group)
                if group.kind == GroupKind::Unattributed
                    && group.project.as_deref() == Some("harbour")
        )),
        "{:#?}",
        sketch(&forest)
    );
}

/// The defect: a root drew as a tree header, which is not a bead row, so
/// nothing that reads a bead row could see the agent on it. What is the
/// project's — its name, and the panes recovered for it — is the project's
/// own line, and the root beneath it is a bead like any other.
#[test]
fn a_project_owns_its_own_line_and_its_roots_are_ordinary_bead_rows() {
    let forest = flatten(snapshot());
    let lines = forest.lines();

    assert!(
        matches!(&lines[0].content, Content::Project(line) if line.project == "dunwich"),
        "{:#?}",
        sketch(&forest)
    );
    assert!(
        matches!(&lines[1].content, Content::Bead(row) if row.id == "dun-7"),
        "{:#?}",
        sketch(&forest)
    );
}

/// `bdi-2bb.25`: the root carries a pane of its own while `dun-7.4`
/// beneath it carries another, so a count over the subtree and the root's
/// own agent cannot come out the same by chance.
#[test]
fn a_staffed_root_says_what_its_agent_is_doing_like_any_other_row() {
    let forest = flatten(snapshot());

    assert_eq!(
        row_of(&forest, "dun-7").agent.as_deref(),
        Some("◍ w:p1 · working")
    );
}

/// A root is a bead row, so the tail reaches its pane by the road every
/// other bead row takes. Before this it was the one staffed row on the
/// screen where the tail said there was no bead to show.
#[test]
fn the_tail_follows_a_staffed_root_as_it_follows_any_other_bead() {
    let mut forest = flatten(snapshot());
    forest.apply(Action::Move(Motion::FirstRow));
    forest.apply(Action::Move(Motion::NextRow));

    assert_eq!(
        crate::view::tail::target(&forest).pane(),
        Some(&pane_key("w:p1")),
        "{:#?}",
        sketch(&forest)
    );
}

/// A project's line counts the beads of its own trees and no other's.
/// `built` puts a tree under dunwich and one under harbour, so a count
/// taken over the wrong run of trees shows on one line or the other.
#[test]
fn a_project_line_counts_its_own_trees_and_no_others() {
    let forest = flatten(built(Filter::All));

    assert_eq!(
        header_of(&forest, "dunwich").counts,
        tree_of("dunwich", DUNWICH).counts
    );
    assert_eq!(
        header_of(&forest, "harbour").counts,
        tree_of("harbour", HARBOUR).counts
    );
}

fn header_of<'a>(forest: &'a Forest, project: &str) -> &'a ProjectLine {
    forest
        .lines()
        .iter()
        .find_map(|line| match &line.content {
            Content::Project(line) if line.project == project => Some(line),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{project} has a line"))
}

/// The defect: a group's lines were drawn and could not be reached, so
/// nothing the forest holds in a group could be looked at or acted on.
#[test]
fn the_selection_can_walk_onto_a_line_under_a_group() {
    let mut forest = flatten(snapshot());
    let panes: Vec<usize> = forest
        .lines()
        .iter()
        .enumerate()
        .filter(|(_, line)| matches!(line.content, Content::Item(Item::Loose(_))))
        .map(|(at, _)| at)
        .collect();

    assert_eq!(panes.len(), 3, "{:#?}", sketch(&forest));
    for at in panes {
        forest.apply(Action::Move(Motion::FirstRow));
        walk::until(
            &mut forest,
            |forest| forest.selected_line() >= at,
            |forest| {
                forest.apply(Action::Move(Motion::NextRow));
            },
            |forest| format!("line {at} cannot be reached: {:#?}", sketch(forest)),
        );
        assert_eq!(
            forest.selected_line(),
            at,
            "line {at} cannot be reached: {:#?}",
            sketch(&forest)
        );
    }
}

/// The identity has to be the pane itself and never where it sat, or a
/// refresh that reorders a group moves the selection to another pane
/// while everything on screen still looks right.
#[test]
fn a_selected_pane_survives_a_refresh_that_reorders_its_group() {
    let mut forest = flatten(snapshot());
    select_item(&mut forest, "w:p4");

    forest.refresh(reordered_groups());

    assert_eq!(selected_item(&forest), Some("w:p4".to_string()));
}

/// Every group's items, so no kind is selectable by accident and none is
/// left behind: a group whose lines cannot be reached is the defect.
#[test]
fn every_kind_of_thing_a_group_holds_can_hold_the_selection() {
    let mut forest = flatten(built(Filter::LiveAgents));
    let groups: Vec<(GroupKind, Option<String>)> = layout::every_group(forest.snapshot()).collect();
    for (kind, project) in groups {
        forest.folds.set(Handle::Group(kind, project), true);
    }
    forest.refresh(built(Filter::LiveAgents));

    let items: Vec<usize> = forest
        .lines()
        .iter()
        .enumerate()
        .filter(|(_, line)| matches!(line.content, Content::Item(_)))
        .map(|(at, _)| at)
        .collect();

    assert_eq!(items.len(), 6, "{:#?}", sketch(&forest));
    for at in items {
        assert!(
            selectable(&forest.lines()[at]),
            "{:?} cannot hold the selection",
            forest.lines()[at].content
        );
    }
}

/// A pane that goes away leaves the selection on the group it was in,
/// rather than at the top of the forest. Work ending should not look
/// like the screen jumping.
#[test]
fn a_selection_on_a_pane_that_goes_away_falls_back_to_its_group() {
    let mut forest = flatten(snapshot());
    select_item(&mut forest, "w:p4");

    forest.refresh(built_without_the_conflicting_panes());

    assert!(
        matches!(
            forest.lines()[forest.selected_line()].content,
            Content::Group(Group {
                kind: GroupKind::Unattributed,
                ..
            })
        ),
        "{:#?}",
        forest.lines()[forest.selected_line()]
    );
}

/// A project's group that empties leaves the selection on the project's
/// line, which is where the group hung: the project is still there, and
/// the top of the forest is not where the reader was.
#[test]
fn a_selection_on_a_pane_whose_group_empties_falls_back_to_its_project() {
    let mut forest = flatten(snapshot());
    select_item(&mut forest, "w:p9");

    forest.refresh(built_without_ferrys_panes());

    assert!(
        matches!(
            &forest.lines()[forest.selected_line()].content,
            Content::Project(line) if line.project == "ferry"
        ),
        "{:#?}",
        sketch(&forest)
    );
}

/// The same from the group's own line.
#[test]
fn a_selection_on_a_projects_group_that_empties_falls_back_to_the_project() {
    let mut forest = flatten(snapshot());
    let group = forest
        .lines()
        .iter()
        .position(|line| {
            matches!(&line.content, Content::Group(group)
                if group.kind == GroupKind::Unattributed
                    && group.project.as_deref() == Some("ferry"))
        })
        .expect("ferry has a pane no bead claims");
    assert!(forest.select_line(group));

    forest.refresh(built_without_ferrys_panes());

    assert!(
        matches!(
            &forest.lines()[forest.selected_line()].content,
            Content::Project(line) if line.project == "ferry"
        ),
        "{:#?}",
        sketch(&forest)
    );
}

/// The same snapshot with the one pane working in ferry gone, which
/// empties the group ferry's line holds it in.
fn built_without_ferrys_panes() -> Snapshot {
    let mut snapshot = snapshot();
    snapshot.unattributed.retain(|pane| pane.project != "ferry");
    snapshot
}

/// The handle a project's group is held by carries the project, so a
/// reader opening one project's quiet trees opens nobody else's.
#[test]
fn opening_one_projects_hidden_trees_leaves_another_projects_shut() {
    let colliding = edited(SLIPWAY, "hbr-9", "hbr-3");
    let mut forest = flatten(gather(
        vec![tree_of("harbour", HARBOUR), tree_of("dunwich", &colliding)],
        Vec::new(),
        Filter::LiveAgents,
    ));
    select_hidden_tree(&mut forest);
    assert_eq!(cursor(&forest), Some(&key("dunwich", "hbr-3")));

    let harbours = forest
        .lines()
        .iter()
        .find(|line| {
            matches!(&line.content, Content::Group(group)
                if group.kind == GroupKind::HiddenTrees
                    && group.project.as_deref() == Some("harbour"))
        })
        .expect("harbour's quiet trees have a line");
    assert_eq!(harbours.folded, Some(false), "{:#?}", sketch(&forest));
}

/// A project with quiet trees and loose panes both: opening the line over
/// its quiet trees draws them one level in, and the line over its loose
/// panes still follows at the depth it started at, hanging under the
/// project and not under the trees just opened.
#[test]
fn a_projects_loose_panes_follow_its_opened_quiet_trees_at_their_own_depth() {
    let mut quiet = alone("dunwich", TOWER, &panes_on(&["nobody"]));
    quiet.refilter(Filter::LiveAgents);
    let mut forest = flatten(quiet);
    assert_eq!(
        sketch(&forest)[..4],
        [
            "▾ dunwich",
            "  ├─▸ [HiddenTrees dunwich] 1",
            "  └── [Unattributed dunwich] 1",
            "      └── - Loose(LoosePane { pane: PaneKey { session: \"default\", id: \"w:nobody\" }, project: \"dunwich\", cwd: \"/srv/work/dunwich\", pane_status: Working, display_agent: Some(\"nobody\"), title: None, claim_refused: false })",
        ]
    );

    select_hidden_tree(&mut forest);

    assert_eq!(
        sketch(&forest)[..5],
        [
            "▾ dunwich",
            "  ├── [HiddenTrees dunwich] 1",
            "  │   └─▸ ○ tow-1 raise the tower",
            "  └── [Unattributed dunwich] 1",
            "      └── - Loose(LoosePane { pane: PaneKey { session: \"default\", id: \"w:nobody\" }, project: \"dunwich\", cwd: \"/srv/work/dunwich\", pane_status: Working, display_agent: Some(\"nobody\"), title: None, claim_refused: false })",
        ]
    );
}

/// The filter decides where a project's trees are drawn and not whether
/// the project holds them, so its line counts every bead of every tree
/// and `a` moves nothing on it.
#[test]
fn a_projects_line_counts_the_trees_the_filter_holds_back() {
    let mut forest = flatten(snapshot());
    let counted = header_of(&forest, "harbour").counts.clone();
    assert_eq!(counted.total, 2, "{:#?}", sketch(&forest));

    forest.apply(Action::ToggleFilter);

    assert_eq!(forest.snapshot().filter, Filter::All);
    assert_eq!(header_of(&forest, "harbour").counts, counted);
}

/// A project whose tracker could not be read at all can still have panes
/// working in its paths, and they are the project's: its line is drawn
/// for them, and the failure stays where it is reported, in the group
/// below the trees.
#[test]
fn a_failed_projects_loose_panes_hang_under_its_own_line() {
    let forest = flatten(gather(
        vec![tree_of("dunwich", DUNWICH)],
        vec![FailedProject {
            project: "ferry".into(),
            tracker: TrackerFailure::Unstartable,
        }],
        Filter::LiveAgents,
    ));
    let drawn = sketch(&forest);
    let ferry = drawn
        .iter()
        .position(|line| line == "▾ ferry")
        .unwrap_or_else(|| panic!("ferry has a line: {drawn:#?}"));

    assert_eq!(
        drawn[ferry..ferry + 4],
        [
            "▾ ferry",
            "  └── [Unattributed ferry] 1",
            "      └── - Loose(LoosePane { pane: PaneKey { session: \"default\", id: \"w:p9\" }, project: \"ferry\", cwd: \"/srv/work/ferry\", pane_status: Blocked, display_agent: None, title: None, claim_refused: false })",
            "▸ [FailedProjects] 1",
        ]
    );
}

/// A forest of projects drawn only for the panes working in their paths
/// holds no root, so there is nothing for `select_first_root` to open on.
/// The selection still settles somewhere and stays: `lay_out` runs before
/// it and leaves the cursor on the first thing the forest holds, so a key
/// that lays the forest out again and says nothing about the selection
/// finds it already held.
#[test]
fn a_project_drawn_only_for_its_loose_panes_keeps_the_line_it_opened_on() {
    let mut forest = flatten(gather(
        Vec::new(),
        vec![FailedProject {
            project: "ferry".into(),
            tracker: TrackerFailure::Unstartable,
        }],
        Filter::LiveAgents,
    ));
    let drawn = sketch(&forest);
    assert!(
        !drawn
            .iter()
            .any(|line| line.contains('◐') || line.contains('○')),
        "no root is drawn, which is what leaves nothing to open on: {drawn:#?}"
    );
    let opened_on = forest.selected_line();
    assert_eq!(
        drawn[opened_on], "  └── [Unattributed dunwich] 2",
        "the selection settles on the first thing drawn, not on nothing: {drawn:#?}"
    );

    for action in [Action::ToggleFilter, Action::RestoreDefault] {
        forest.apply(action);
        assert_eq!(
            sketch(&forest)[forest.selected_line()],
            drawn[opened_on],
            "{action:?}"
        );
    }
}

/// The pane id on the line the selection sits on, where it sits on one.
fn selected_item(forest: &Forest) -> Option<String> {
    match &forest.lines()[forest.selected_line()].content {
        Content::Item(Item::Loose(pane)) => Some(pane.pane.id.clone()),
        Content::Item(Item::Unconfigured(pane)) => Some(pane.pane.id.clone()),
        _ => None,
    }
}

/// Put the selection on the line for one pane, by moving down to it.
fn select_item(forest: &mut Forest, pane: &str) {
    forest.apply(Action::Move(Motion::FirstRow));
    walk::until(
        forest,
        |forest| selected_item(forest).as_deref() == Some(pane),
        |forest| {
            forest.apply(Action::Move(Motion::NextRow));
        },
        |_| format!("{pane} is not reachable by moving down"),
    );
}

/// The same snapshot with every group's items in the other order, which
/// is what a collect that re-read them may hand over.
fn reordered_groups() -> Snapshot {
    let mut snapshot = snapshot();
    snapshot.unattributed.reverse();
    snapshot.conflicts.reverse();
    snapshot.hidden_trees.reverse();
    snapshot
}
