//! Another project's bead, drawn under a bead of this one.

use super::*;
use pretty_assertions::assert_eq;

/// Harbour's bead waiting on dunwich's, with a pane working the task
/// under dunwich's bead, drawn as a collection draws both projects.
fn harbour_waiting_on_dunwich() -> Snapshot {
    harbour_waiting_on_dunwich_staffed(&["dun-7.1"])
}

/// The same, with a pane on each of `on`. Harbour's bead also waits on a
/// finished dunwich bead that no tree of dunwich's own draws.
fn harbour_waiting_on_dunwich_staffed(on: &[&str]) -> Snapshot {
    harbour_and_dunwich(
        HARBOUR_WAITING,
        DUNWICH_WAITED_ON,
        &[("dunwich", "dun-7"), ("harbour", "hbr-1")],
        on,
    )
}

/// The same with harbour's tracker failing to answer: dunwich's tree alone,
/// and harbour among the failed projects.
fn harbour_failing_to_answer() -> Snapshot {
    let mut failing = harbour_and_dunwich(
        HARBOUR_WAITING,
        DUNWICH_WAITED_ON,
        &[("dunwich", "dun-7")],
        &["dun-7.1"],
    );
    failing.failed_projects.push(FailedProject {
        project: "harbour".into(),
        tracker: TrackerFailure::Unstartable,
    });
    failing
}

const HARBOUR_WAITING: &str = r#"[{"id":"hbr-1","title":"clear the berth","status":"blocked",
     "dependencies":[{"depends_on_id":"dun-7","type":"blocks"},
                     {"depends_on_id":"dun-8","type":"blocks"}]}]"#;

const DUNWICH_WAITED_ON: &str = r#"[{"id":"dun-7","title":"lift the ground station","status":"open"},
    {"id":"dun-7.1","title":"re-point the dish","status":"open",
     "dependencies":[{"depends_on_id":"dun-7","type":"parent-child"}]},
    {"id":"dun-8","title":"survey the mast","status":"closed"}]"#;

/// Harbour's epic over a task `bd` calls ready, which waits on dunwich's
/// `dun-7` in the status given.
fn a_task_bd_calls_ready_waiting_on_dunwich(status: &str) -> Snapshot {
    harbour_and_dunwich_ready(
        r#"[{"id":"hbr-0","title":"open the harbour","status":"open","issue_type":"epic"},
            {"id":"hbr-1","title":"clear the berth","status":"open",
             "dependencies":[{"depends_on_id":"hbr-0","type":"parent-child"},
                             {"depends_on_id":"dun-7","type":"blocks"}]}]"#,
        &format!(r#"[{{"id":"dun-7","title":"lift the ground station","status":"{status}"}}]"#),
        &[("harbour", "hbr-0"), ("dunwich", "dun-7")],
        &[],
        &["hbr-1"],
    )
}

/// Ready work opens the folds over it, and a bead waiting on another
/// project's open bead is not ready, whatever `bd` says. Once that bead
/// closes, `bd`'s answer stands and the fold opens.
#[test]
fn a_bead_waiting_on_another_projects_open_bead_opens_no_fold_as_ready_work() {
    let waiting = flatten(a_task_bd_calls_ready_waiting_on_dunwich("open"));
    let free = flatten(a_task_bd_calls_ready_waiting_on_dunwich("closed"));

    assert!(
        lines_of(&waiting, "hbr-1").is_empty(),
        "{:#?}",
        sketch(&waiting)
    );
    assert!(!lines_of(&free, "hbr-1").is_empty(), "{:#?}", sketch(&free));
}

/// The window on a bead waiting on another project's open bead says it is
/// blocked by that bead, and leaves out the finished one it also waits on.
#[test]
fn the_window_on_a_bead_waiting_on_another_projects_open_bead_says_what_blocks_it() {
    let mut forest = flatten(harbour_waiting_on_dunwich());
    let waiting = lines_of(&forest, "hbr-1")[0];
    step_onto(&mut forest, waiting);

    let node = crate::view::show::selected(&forest).expect("a bead is selected");
    let page = crate::view::show::said(node, None, 80, 40, &|_| false);
    let said: Vec<String> = page
        .rows
        .iter()
        .map(|row| row.iter().map(|span| span.content.as_ref()).collect())
        .collect();

    assert!(
        said.iter().any(|row| row.trim() == "blocked by: dun-7"),
        "{said:#?}"
    );
}

/// Another project's bead is its own project's wherever it is drawn: the
/// line under the bead waiting on it names the same bead as the line in
/// its own project's tree.
#[test]
fn another_projects_bead_is_keyed_on_its_own_project_under_the_bead_waiting_on_it() {
    let forest = flatten(harbour_waiting_on_dunwich());

    let keys: Vec<&BeadKey> = lines_of(&forest, "dun-7.1")
        .into_iter()
        .filter_map(|at| forest.lines()[at].bead())
        .collect();
    assert_eq!(
        keys,
        vec![&key("dunwich", "dun-7.1"); 2],
        "{:#?}",
        sketch(&forest)
    );
}

/// A live agent on another project's bead opens the spine down to it
/// from the bead waiting on it, as one on a bead of its own would.
#[test]
fn a_live_agent_on_another_projects_bead_opens_the_bead_waiting_on_it() {
    let forest = flatten(harbour_waiting_on_dunwich());

    let waiting = lines_of(&forest, "hbr-1")[0];
    assert_eq!(
        forest.lines()[waiting].folded,
        Some(true),
        "{:#?}",
        sketch(&forest)
    );
}

/// The bead window on another project's bead names that project's beads,
/// so the reader can follow its parent from the copy under the bead
/// waiting on it.
#[test]
fn the_window_on_another_projects_bead_follows_its_own_projects_parent() {
    let mut forest = flatten(harbour_waiting_on_dunwich());
    let under_the_waiting_bead = lines_of(&forest, "dun-7.1")[1];
    step_onto(&mut forest, under_the_waiting_bead);

    let node = crate::view::show::selected(&forest).expect("a bead is selected");
    let parent = node.parent.as_ref().expect("dun-7.1 names its parent");
    assert_eq!(
        crate::view::show::key_of(&forest, parent),
        Some(key("dunwich", "dun-7"))
    );
    assert!(crate::view::show::followable(&forest, parent));
}

/// The bead window on the bead waiting names the other project's bead it
/// waits on as that bead, and the reader can follow it there.
#[test]
fn the_window_on_a_bead_waiting_on_another_projects_bead_follows_it_there() {
    let mut forest = flatten(harbour_waiting_on_dunwich());
    let waiting = lines_of(&forest, "hbr-1")[0];
    step_onto(&mut forest, waiting);

    let node = crate::view::show::selected(&forest).expect("a bead is selected");
    let blocker = &node.depends_on[0];
    assert_eq!(
        (
            blocker.id.as_str(),
            blocker.status.as_ref(),
            blocker.title.as_deref()
        ),
        (
            "dun-7",
            Some(&crate::model::types::Status::Open),
            Some("lift the ground station")
        )
    );
    assert!(crate::view::show::followable(&forest, blocker));

    let followed = crate::view::show::key_of(&forest, blocker).expect("the bead is keyed");
    assert!(forest.go_to(&followed));
    assert_eq!(
        forest.place().map(|place| place.key()),
        Some(&key("dunwich", "dun-7"))
    );
}

/// A search matches every drawn copy, and the copy under the bead
/// waiting on another project's bead is a match on that project's bead.
#[test]
fn a_search_matches_another_projects_bead_as_that_projects() {
    let forest = flatten(harbour_waiting_on_dunwich());

    assert_eq!(
        searched(&forest),
        vec![
            key("dunwich", "dun-7"),
            key("dunwich", "dun-7.1"),
            key("harbour", "hbr-1"),
            key("dunwich", "dun-7"),
            key("dunwich", "dun-7.1"),
            key("dunwich", "dun-8"),
        ]
    );
}

/// The snapshot finds a bead only another project's tree draws, and
/// finds no bead under a key naming the wrong project for its id.
#[test]
fn the_snapshot_finds_a_bead_by_its_own_project_in_any_tree() {
    let snapshot = harbour_waiting_on_dunwich();

    assert_eq!(
        snapshot
            .node(&key("dunwich", "dun-8"))
            .map(|node| node.key()),
        Some(key("dunwich", "dun-8"))
    );
    assert!(snapshot.node(&key("harbour", "dun-7")).is_none());
}

/// A scope the reader shut over the bead waiting on another project's
/// bead is spent along the way down to an agent arriving on that
/// project's work, as it would be for work of its own project.
#[test]
fn a_scope_shut_over_another_projects_bead_is_spent_down_to_what_arrived_there() {
    let mut forest = flatten(harbour_waiting_on_dunwich_staffed(&["dun-7"]));
    select_bead(&mut forest, "hbr-1");
    forest.apply(Action::CollapseSubtree);
    assert_eq!(
        lines_of(&forest, "dun-7").len(),
        1,
        "{:#?}",
        sketch(&forest)
    );

    forest.refresh(harbour_waiting_on_dunwich_staffed(&["dun-7", "dun-7.1"]));

    assert_eq!(
        lines_of(&forest, "dun-7.1").len(),
        2,
        "{:#?}",
        sketch(&forest)
    );
}

/// The same, with the scope the reader shut being harbour's own line.
#[test]
fn a_project_shut_over_another_projects_bead_is_spent_down_to_what_arrived_there() {
    let mut forest = flatten(harbour_waiting_on_dunwich_staffed(&["dun-7"]));
    let harbour = forest
        .lines()
        .iter()
        .position(
            |line| matches!(&line.content, Content::Project(line) if line.project == "harbour"),
        )
        .expect("harbour has a line");
    step_onto(&mut forest, harbour);
    forest.apply(Action::CollapseSubtree);
    assert!(
        lines_of(&forest, "hbr-1").is_empty(),
        "{:#?}",
        sketch(&forest)
    );

    forest.refresh(harbour_waiting_on_dunwich_staffed(&["dun-7", "dun-7.1"]));

    assert_eq!(
        lines_of(&forest, "dun-7.1").len(),
        2,
        "{:#?}",
        sketch(&forest)
    );
}

/// A bead only another project's tree draws is still somewhere to go:
/// the forest takes the reader to it under the bead waiting on it.
#[test]
fn going_to_a_bead_only_another_projects_tree_draws_lands_on_it() {
    let mut forest = flatten(harbour_waiting_on_dunwich());

    assert!(
        forest.go_to(&key("dunwich", "dun-8")),
        "{:#?}",
        sketch(&forest)
    );
    assert_eq!(
        forest.place().map(Place::key),
        Some(&key("dunwich", "dun-8"))
    );
}

/// An id is a bead only within its project, so a bead of this project
/// is not found at another project's bead of the same id that this
/// project's tree happens to draw.
#[test]
fn going_to_a_bead_does_not_land_on_another_projects_bead_of_the_same_id() {
    let mut forest = flatten(harbour_waiting_on_dunwich());

    assert!(
        !forest.go_to(&key("harbour", "dun-7")),
        "{:#?}",
        sketch(&forest)
    );
}

/// Focused on the bead waiting on another project's bead, going to that
/// bead lands beneath the focus rather than in the other project's tree.
#[test]
fn going_to_another_projects_bead_from_a_focus_over_it_lands_beneath_the_focus() {
    let mut forest = flatten(harbour_waiting_on_dunwich());
    focus_on(&mut forest, "hbr-1");

    assert!(
        forest.go_to(&key("dunwich", "dun-7.1")),
        "{:#?}",
        sketch(&forest)
    );
    let place = forest.place().expect("the selection is on a bead");
    assert_eq!(
        (&place.tree, place.key()),
        (&key("harbour", "hbr-1"), &key("dunwich", "dun-7.1"))
    );
}

/// Focused on another project's bead under the bead waiting on it, every
/// line beneath the focus is still that project's.
#[test]
fn a_focus_on_another_projects_bead_keeps_what_it_draws_that_projects() {
    let mut forest = flatten(harbour_waiting_on_dunwich());
    let under_the_waiting_bead = lines_of(&forest, "dun-7")[1];
    step_onto(&mut forest, under_the_waiting_bead);
    assert!(forest.apply(Action::FocusForest), "{:#?}", sketch(&forest));
    toggle_fold_of(&mut forest, "dun-7");
    toggle_fold_of(&mut forest, "dun-7");

    let keys: Vec<&BeadKey> = forest
        .lines()
        .iter()
        .filter_map(Line::bead)
        .filter(|key| key.id.starts_with("dun-"))
        .collect();
    assert!(!keys.is_empty(), "{:#?}", sketch(&forest));
    assert!(keys.iter().all(|key| key.project == "dunwich"), "{keys:#?}");
    let searched: Vec<BeadKey> = searched(&forest)
        .into_iter()
        .filter(|key| key.id.starts_with("dun-"))
        .collect();
    assert!(
        searched.iter().all(|key| key.project == "dunwich"),
        "{searched:#?}"
    );
}

/// A bead no tree of its own project draws is read through the tree of the
/// project waiting on it, so that project failing to answer has not said
/// the bead is gone either.
#[test]
fn a_focus_on_a_bead_only_another_project_draws_holds_while_that_project_does_not_answer() {
    let mut forest = flatten(harbour_waiting_on_dunwich());
    assert!(
        forest.go_to(&key("dunwich", "dun-8")),
        "{:#?}",
        sketch(&forest)
    );
    assert!(forest.apply(Action::FocusForest), "{:#?}", sketch(&forest));

    forest.refresh(harbour_failing_to_answer());

    assert!(
        forest.is_focused(),
        "the mode ended: {:#?}",
        sketch(&forest)
    );
    assert_eq!(
        forest.place().map(|place| place.key().clone()),
        Some(key("dunwich", "dun-8"))
    );
}

/// Nor has the bead's own project failing to answer, which leaves the
/// project waiting on it nothing to reach it through.
#[test]
fn a_focus_on_another_projects_bead_holds_while_its_own_project_does_not_answer() {
    let mut forest = flatten(harbour_waiting_on_dunwich());
    assert!(
        forest.go_to(&key("dunwich", "dun-8")),
        "{:#?}",
        sketch(&forest)
    );
    assert!(forest.apply(Action::FocusForest), "{:#?}", sketch(&forest));

    let mut dunwich_failing =
        harbour_and_dunwich(HARBOUR_WAITING, "[]", &[("harbour", "hbr-1")], &[]);
    dunwich_failing.failed_projects.push(FailedProject {
        project: "dunwich".into(),
        tracker: TrackerFailure::Unstartable,
    });
    forest.refresh(dunwich_failing);

    assert!(
        forest.is_focused(),
        "the mode ended: {:#?}",
        sketch(&forest)
    );
}

/// Every bead a search counts, in the order it counts them: an empty
/// search matches every bead.
fn searched(forest: &Forest) -> Vec<BeadKey> {
    let mut every = forest.matches(Sought::holding(""));
    (0..every.len())
        .filter_map(|at| every.nth(at))
        .map(|place| place.key().clone())
        .collect()
}

/// Harbour's tree holding a bead of harbour's and a bead of dunwich's
/// under one id. Harbour's `dun-2` is a leaf drawn first; dunwich's is
/// reached through `dun-9` and has two halves waiting on the agent's
/// bead beneath it. Harbour's bead also waits on a dunwich bead whose id
/// runs on from its own.
fn one_id_in_two_projects() -> Snapshot {
    harbour_and_dunwich(
        r#"[{"id":"hbr-1","title":"clear the berth","status":"blocked",
             "dependencies":[{"depends_on_id":"dun-9","type":"blocks"},
                             {"depends_on_id":"hbr-1.1","type":"blocks"}]},
            {"id":"dun-2","title":"moor the tender","status":"open","priority":1,
             "dependencies":[{"depends_on_id":"hbr-1","type":"parent-child"}]}]"#,
        r#"[{"id":"dun-9","title":"rig the sheerlegs","status":"open","priority":2,
             "dependencies":[{"depends_on_id":"dun-2","type":"blocks"}]},
            {"id":"dun-2","title":"step the derrick","status":"open","priority":1},
            {"id":"dun-2.1","title":"seat the shoe","status":"open","priority":2,
             "dependencies":[{"depends_on_id":"dun-2","type":"parent-child"},
                             {"depends_on_id":"dun-1","type":"blocks"}]},
            {"id":"dun-2.2","title":"trim the stay","status":"open","priority":2,
             "dependencies":[{"depends_on_id":"dun-2","type":"parent-child"},
                             {"depends_on_id":"dun-1","type":"blocks"}]},
            {"id":"dun-1","title":"turn the pintle","status":"open","priority":2},
            {"id":"dun-1.1","title":"ream the pintle","status":"open","priority":2,
             "dependencies":[{"depends_on_id":"dun-1","type":"parent-child"}]},
            {"id":"hbr-1.1","title":"sound the channel","status":"open","priority":3}]"#,
        &[("harbour", "hbr-1")],
        &["dun-1.1"],
    )
}

/// A rule set on another project's bead chooses among that bead's ways
/// down, not those of the bead of the same id its tree holds first.
#[test]
fn a_rule_set_on_another_projects_bead_chooses_among_its_own_ways_down() {
    let mut forest = under_every_copy(one_id_in_two_projects());
    let dunwichs = forest
        .lines()
        .iter()
        .position(|line| line.bead() == Some(&key("dunwich", "dun-2")))
        .expect("dunwich's dun-2 is drawn");
    step_onto(&mut forest, dunwichs);
    assert_eq!(
        lines_of(&forest, "dun-1").len(),
        2,
        "{:#?}",
        sketch(&forest)
    );

    put_in_force(&mut forest, Spine::Deepest, Action::CycleSpine);

    assert_eq!(
        lines_of(&forest, "dun-1").len(),
        1,
        "{:#?}",
        sketch(&forest)
    );
}

/// Another project's bead keeps its whole id under a bead whose id its
/// own runs on from, because a bare suffix would place it in the other
/// project.
#[test]
fn another_projects_bead_keeps_its_whole_id_under_a_bead_its_id_runs_on_from() {
    let forest = flatten(one_id_in_two_projects());

    assert_eq!(row_of(&forest, "hbr-1.1").id, "hbr-1.1");
}
