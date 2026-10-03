use super::*;
use pretty_assertions::assert_eq;

#[test]
fn a_snapshot_flattens_to_the_lines_the_design_draws() {
    let forest = flatten(snapshot());

    assert_eq!(
        sketch(&forest),
        vec![
            "▾ dunwich",
            "  ├── ◐ dun-7 lift the ground station",
            "  │   ├── ! OrphanedDependencies(1)",
            "  │   ├─▸ ○ .1 re-point the dish",
            "  │   ├── ○ .7 log the survey marks",
            "  │   ├── ✓ .4 clear the access road",
            "  │   └─▸ … 3 more",
            "  └── [Unattributed dunwich] 2",
            "      ├── - Loose(LoosePane { pane: PaneKey { session: \"default\", id: \"w:p3\" }, project: \"dunwich\", cwd: \"/srv/work/dunwich\", pane_status: Working, display_agent: Some(\"dun-7.1\"), title: None, claim_refused: true })",
            "      └── - Loose(LoosePane { pane: PaneKey { session: \"default\", id: \"w:p4\" }, project: \"dunwich\", cwd: \"/srv/work/dunwich\", pane_status: Idle, display_agent: Some(\"dun-7.1\"), title: None, claim_refused: true })",
            "▾ ferry",
            "  ├── ⚠ fer-2 unread",
            "  └── [Unattributed ferry] 1",
            "      └── - Loose(LoosePane { pane: PaneKey { session: \"default\", id: \"w:p9\" }, project: \"ferry\", cwd: \"/srv/work/ferry\", pane_status: Blocked, display_agent: None, title: None, claim_refused: false })",
            "▾ harbour",
            "  └─▸ [HiddenTrees harbour] 1",
            "▸ [FailedProjects] 1",
            "▾ [Unconfigured] 1",
            "  └── - Unconfigured(UnconfiguredPane { pane: PaneKey { session: \"default\", id: \"w:pF\" }, cwd: \"/srv/spike\", pane_status: Idle })",
            "▾ [Conflicts] 1",
            "  └── - Conflict(SeveralPanesNameOneBead { bead: BeadKey { project: \"dunwich\", id: \"dun-7.1\" }, panes: [PaneKey { session: \"default\", id: \"w:p3\" }, PaneKey { session: \"default\", id: \"w:p4\" }] })",
        ]
    );
}

/// The selection starts on the first root and not on the project line
/// above it: a project has no pane, so opening there would spend the tail
/// band saying there is nothing to show.
#[test]
fn the_selection_starts_on_the_first_root() {
    let forest = flatten(snapshot());

    assert_eq!(forest.selected_line(), 1);
    assert_eq!(cursor(&forest), Some(&key("dunwich", "dun-7")));
}

/// The fold state is the user's and the live work's, and moving is
/// neither: walking out of a tree leaves it exactly as it was drawn.
#[test]
fn a_root_stays_as_it_was_when_the_selection_walks_out_of_it() {
    let mut forest = flatten(snapshot());
    let was = sketch(&forest);

    forest.apply(Action::Move(Motion::LastRow));

    assert_eq!(sketch(&forest), was);
}

/// Compared on content alone: folding also redraws the header's marker
/// and the elbow on what is now the last line under it, and neither of
/// those is a line the fold removed.
#[test]
fn folding_a_root_removes_exactly_its_subtree() {
    let mut forest = flatten(snapshot());
    let before = contents(&forest);

    assert!(forest.apply(Action::ToggleFold));

    let after = contents(&forest);
    let gone: Vec<&String> = before.iter().filter(|said| !after.contains(said)).collect();

    assert_eq!(
        gone,
        vec![
            "○ .1 re-point the dish",
            "○ .7 log the survey marks",
            "✓ .4 clear the access road",
            "… 3 more",
        ]
    );
}

fn contents(forest: &Forest) -> Vec<String> {
    forest
        .lines()
        .iter()
        .map(|line| said(&line.content))
        .collect()
}

/// Folding is where a finding is easiest to lose, so a folded tree keeps
/// every one of them.
#[test]
fn a_trees_findings_are_drawn_whether_it_is_folded_or_not() {
    let mut forest = flatten(snapshot());
    forest.apply(Action::ToggleFold);

    assert_eq!(
        sketch(&forest)[..3],
        [
            "▾ dunwich",
            "  ├─▸ ◐ dun-7 lift the ground station",
            "  │   └── ! OrphanedDependencies(1)",
        ]
    );
}

#[test]
fn a_fold_made_by_hand_outlives_moving_away_from_it() {
    let mut forest = flatten(snapshot());
    forest.apply(Action::ToggleFold);
    forest.apply(Action::Move(Motion::LastRow));
    forest.apply(Action::Move(Motion::FirstRow));

    assert!(
        !sketch(&forest).iter().any(|line| line.contains(".1.1")),
        "{:#?}",
        sketch(&forest)
    );
}

/// The same tree with nobody on it and the named beads ready, so the two
/// halves of the fold default can be asked the same question.
fn tower_ready(on: &[&str]) -> Snapshot {
    ready_alone("dunwich", TOWER, &[], on)
}

/// The default the bead is about: the first screen is the work a reader
/// needs next and the path down to it. Four quiet forebears open because
/// of one bead at the bottom; the branch beside them, holding neither an
/// agent nor ready work, stays shut.
///
/// Two kinds of bead earn that opening and no third does. An agent on one
/// says the work is happening; `bd` calling one ready says it can start.
/// Both are asked of the same tree here, because the claim is that the
/// screen cannot tell them apart — and either way the bead that earned
/// the fold does not open its own, and the unfinished work `bd` will not
/// start is still folded away.
#[test]
fn the_default_opens_every_forebear_of_a_live_agent_or_of_ready_work_and_nothing_else() {
    let opened = vec![
        "▾ dunwich",
        "  └── ○ tow-1 raise the tower",
        "      ├── ○ .1 stand the mast",
        "      │   └── ○ .1 bolt the sections",
        "      │       └── ○ .1 dress the cables",
        "      └─▸ ○ .2 pour the base",
    ];

    let staffed = flatten(tower_staffed(&["tow-1.1.1.1"]));
    let ready = flatten(tower_ready(&["tow-1.1.1.1"]));

    assert_eq!(sketch(&staffed), opened);
    assert_eq!(sketch(&ready), opened);
    for forest in [&staffed, &ready] {
        for forebear in ["tow-1", "tow-1.1", "tow-1.1.1"] {
            assert_eq!(fold_of(forest, forebear), Some(true), "{forebear} is shut");
        }
        assert_eq!(fold_of(forest, "tow-1.2"), Some(false));
    }
}

/// The third case, and the one a claim makes: a seat has taken the bead
/// and no pane has joined it yet. `bd ready` drops a bead the moment it
/// goes `in_progress`, so nothing here is ready and nobody is staffed,
/// and the forebears open anyway.
///
/// They open on the anomaly. A claim with no pane behind it is an
/// `orphan-claim` from the first collection, and that is what `quiet`
/// answers to — so the rule keeping a booting seat's bead on screen lives
/// in `model/anomaly.rs`, not in this file. Narrow that rule and this
/// goes red, which is the whole reason it is written down here.
#[test]
fn the_default_opens_every_forebear_of_a_bead_someone_has_claimed() {
    let claimed = edited(
        TOWER,
        r#"{"id":"tow-1.1.1.1","title":"dress the cables","status":"open","#,
        r#"{"id":"tow-1.1.1.1","title":"dress the cables","status":"in_progress",
   "updated_at":"2026-08-30T11:00:00Z","#,
    );
    let forest = flatten(ready_alone("dunwich", &claimed, &[], &[]));

    assert_eq!(
        sketch(&forest),
        vec![
            "▾ dunwich",
            "  └── ○ tow-1 raise the tower",
            "      ├── ○ .1 stand the mast",
            "      │   └── ○ .1 bolt the sections",
            "      │       └── ◐ .1 dress the cables",
            "      └─▸ ○ .2 pour the base",
        ]
    );
    for forebear in ["tow-1", "tow-1.1", "tow-1.1.1"] {
        assert_eq!(fold_of(&forest, forebear), Some(true), "{forebear} is shut");
    }
}

/// The other half of the same rule. A tree nobody is working holds no
/// spine to open, so it rests as the one line saying it is there.
#[test]
fn a_tree_with_nothing_live_in_it_rests_as_its_header() {
    let forest = flatten(tower_staffed(&[]));

    assert_eq!(
        sketch(&forest),
        vec!["▾ dunwich", "  └─▸ ○ tow-1 raise the tower"]
    );
}

/// A default, not a lock: the user shuts a node holding an agent and it
/// stays shut, refresh after refresh, for as long as what is under there
/// is what they folded away.
#[test]
fn a_fold_set_by_hand_survives_a_refresh_that_brings_nothing_new_under_it() {
    let mut forest = flatten(tower_staffed(&["tow-1.1.1.1"]));
    select(&mut forest, &key("dunwich", "tow-1.1"));
    forest.apply(Action::ToggleFold);

    forest.refresh(tower_staffed(&["tow-1.1.1.1"]));

    assert_eq!(fold_of(&forest, "tow-1.1"), Some(false));
    assert!(
        !sketch(&forest).iter().any(|line| line.contains(".1.1")),
        "{:#?}",
        sketch(&forest)
    );
}

/// Work dying down is not news, so it re-opens nothing the user shut.
/// The agent moves to the other branch, which is what keeps the tree
/// open for the shut one to still be drawn under.
#[test]
fn a_fold_set_by_hand_outlives_the_work_it_was_shut_over_going_away() {
    let mut forest = flatten(tower_staffed(&["tow-1.1.1.1"]));
    select(&mut forest, &key("dunwich", "tow-1.1"));
    forest.apply(Action::ToggleFold);

    forest.refresh(tower_staffed(&["tow-1.2.1"]));

    assert_eq!(fold_of(&forest, "tow-1.1"), Some(false));
}

/// The hard half. A fold says *I have seen what is under here and do not
/// want it*, which stops being true the moment something new is under it,
/// so an agent arriving on a bead the user never folded away hands the
/// node back to the default.
#[test]
fn a_fold_set_by_hand_is_spent_when_live_work_arrives_beneath_it() {
    let mut forest = flatten(tower_staffed(&["tow-1.1.1.1"]));
    select(&mut forest, &key("dunwich", "tow-1.1"));
    forest.apply(Action::ToggleFold);

    forest.refresh(tower_staffed(&["tow-1.1.1.1", "tow-1.1.1"]));

    assert_eq!(fold_of(&forest, "tow-1.1"), Some(true));
    assert!(
        sketch(&forest)
            .iter()
            .any(|line| line.contains(".1 dress the cables")),
        "{:#?}",
        sketch(&forest)
    );
}

/// A search step opening a fold the reader shut leaves it theirs to be
/// spent, so the step after cannot shut it back over what arrived.
#[test]
fn a_fold_shut_by_hand_is_spent_by_what_arrives_while_a_search_step_has_it_open() {
    let mut forest = flatten(tower_staffed(&["tow-1.1.1.1"]));
    select(&mut forest, &key("dunwich", "tow-1.1"));
    forest.apply(Action::ToggleFold);
    forest.seek_here("tow-1.1.1");
    forest.refresh(tower_staffed(&["tow-1.1.1.1", "tow-1.1.1"]));

    forest.seek_here("tow-1.2");

    assert_eq!(cursor(&forest), Some(&key("dunwich", "tow-1.2")));
    assert_eq!(fold_of(&forest, "tow-1.1"), Some(true));
}

/// The default reads the agents, not which trees are drawn, so dropping
/// the filter adds trees below and changes no fold above.
#[test]
fn dropping_the_filter_leaves_the_default_fold_state_alone() {
    let mut forest = flatten(snapshot());
    let staffed: Vec<String> = sketch(&forest)
        .into_iter()
        .take_while(|line| !line.contains("ferry"))
        .collect();

    forest.apply(Action::ToggleFilter);

    assert_eq!(sketch(&forest)[..staffed.len()], staffed[..]);
}

/// A group over live panes is a fold `bdi` chose, and a count is not a
/// view of what it holds: it says they exist and nothing about which they
/// are. What collection and the filter did is a report, and rests shut.
/// In the order the groups are drawn: dunwich's and ferry's loose panes,
/// harbour's hidden tree, then the failed project, the unconfigured pane
/// and the conflict below the trees.
#[test]
fn a_group_rests_open_when_what_it_holds_is_live() {
    let forest = flatten(snapshot());
    let markers: Vec<&str> = forest
        .lines()
        .iter()
        .filter_map(|line| match &line.content {
            Content::Group(_) => Some(marker(line.folded == Some(true))),
            _ => None,
        })
        .collect();

    assert_eq!(markers, vec![OPEN, OPEN, SHUT, SHUT, OPEN, OPEN]);
}

#[test]
fn a_run_of_quiet_closed_siblings_collapses_to_a_count() {
    let forest = flatten(snapshot());

    assert!(sketch(&forest).contains(&"  │   └─▸ … 3 more".to_string()));
}

/// The count is the only account the screen gives of the beads it stands
/// for, so the line has to be reachable to be worth anything.
#[test]
fn an_elided_run_can_hold_the_selection() {
    let mut forest = flatten(snapshot());

    select_run(&mut forest);

    assert_eq!(
        sketch(&forest)[forest.selected_line()],
        "  │   └─▸ … 3 more"
    );
}

#[test]
fn opening_an_elided_run_draws_the_beads_it_counted() {
    let mut forest = flatten(snapshot());
    select_run(&mut forest);

    forest.apply(Action::ToggleFold);

    let drawn = sketch(&forest);
    let from_the_run: Vec<&String> = drawn
        .iter()
        .skip_while(|line| !line.contains("… 3 more"))
        .collect();

    assert_eq!(
        from_the_run[..4],
        [
            "  │   └── … 3 more",
            "  │       ├── ✓ .2 survey the mast",
            "  │       ├── ✓ .3 pour the pad",
            "  │       └── ✓ .5 set the guard rail",
        ]
    );
}

#[test]
fn shutting_an_open_elided_run_puts_the_count_back() {
    let mut forest = flatten(snapshot());
    let was = sketch(&forest);
    select_run(&mut forest);

    forest.apply(Action::ExpandOrChild);
    forest.apply(Action::CollapseOrParent);

    assert_eq!(sketch(&forest), was);
}
