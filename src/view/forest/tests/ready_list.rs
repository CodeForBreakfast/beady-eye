//! The ready list: a tree's ready and in-progress beads, each once.

use super::*;
use pretty_assertions::assert_eq;

/// The lighthouse as bd writes it. `lgt-4.6` is a child of the root and
/// blocks `lgt-4.5`, so the forest draws it twice. Someone is on `lgt-4.4`,
/// on `lgt-4.5`, which waits on `lgt-4.6`, and on `lgt-4.7`, which is closed.
/// `lgt-4.8` is open and waits on nothing bd answered ready.
const LIGHTHOUSE: &str = r#"[
  {"id":"lgt-4","title":"relight the lighthouse","status":"open",
   "priority":1,"issue_type":"epic","created_at":"2026-08-01T09:00:00Z"},
  {"id":"lgt-4.1","title":"scrape the lantern glass","status":"open",
   "dependencies":[{"depends_on_id":"lgt-4","type":"parent-child"}],
   "priority":2,"issue_type":"task","created_at":"2026-08-20T09:00:00Z"},
  {"id":"lgt-4.2","title":"re-silver the reflector","status":"open",
   "dependencies":[{"depends_on_id":"lgt-4","type":"parent-child"}],
   "priority":1,"issue_type":"task","created_at":"2026-08-22T09:00:00Z"},
  {"id":"lgt-4.3","title":"rewire the lamp","status":"open",
   "dependencies":[{"depends_on_id":"lgt-4","type":"parent-child"}],
   "priority":2,"issue_type":"task","created_at":"2026-08-10T09:00:00Z"},
  {"id":"lgt-4.4","title":"test the fog signal","status":"in_progress",
   "dependencies":[{"depends_on_id":"lgt-4","type":"parent-child"}],
   "priority":2,"issue_type":"task","created_at":"2026-08-12T09:00:00Z",
   "updated_at":"2026-08-29T12:00:00Z"},
  {"id":"lgt-4.5","title":"replace the gallery rail","status":"open",
   "dependencies":[{"depends_on_id":"lgt-4","type":"parent-child"},
                   {"depends_on_id":"lgt-4.6","type":"blocks"}],
   "priority":1,"issue_type":"task","created_at":"2026-08-14T09:00:00Z"},
  {"id":"lgt-4.6","title":"cast the rail stanchions","status":"open",
   "dependencies":[{"depends_on_id":"lgt-4","type":"parent-child"}],
   "priority":3,"issue_type":"task","created_at":"2026-08-05T09:00:00Z"},
  {"id":"lgt-4.7","title":"log the keeper's hours","status":"closed",
   "dependencies":[{"depends_on_id":"lgt-4","type":"parent-child"}],
   "priority":2,"issue_type":"task","created_at":"2026-08-02T09:00:00Z",
   "closed_at":"2026-08-28T09:00:00Z"},
  {"id":"lgt-4.8","title":"paint the daymark","status":"open",
   "dependencies":[{"depends_on_id":"lgt-4","type":"parent-child"}],
   "priority":2,"issue_type":"task","created_at":"2026-08-03T09:00:00Z"}
]"#;

/// The beads bd answers `bd ready` with in the lighthouse. The epic is one
/// of them: bd calls it ready, and the list takes bd's word for an epic as
/// for any other bead.
const LIGHTHOUSE_READY: &[&str] = &["lgt-4", "lgt-4.1", "lgt-4.2", "lgt-4.3", "lgt-4.6"];

fn lighthouse() -> Snapshot {
    ready_alone(
        "dunwich",
        LIGHTHOUSE,
        &panes_on(&["lgt-4.4", "lgt-4.5", "lgt-4.7"]),
        LIGHTHOUSE_READY,
    )
}

fn listed(snapshot: Snapshot) -> Forest {
    let mut forest = flatten(snapshot);
    assert!(forest.apply(Action::ToggleReadyList));
    forest
}

/// The project's own line, which says how fresh its rows are, then the
/// ready beads, then the in-progress ones and the one with an agent on
/// blocked work, each once, by priority and then oldest first.
#[test]
fn the_ready_list_draws_each_bead_once_by_priority_then_age() {
    let forest = listed(lighthouse());

    assert_eq!(
        sketch(&forest),
        vec![
            "dunwich",
            "# Ready 5",
            "  ○ lgt-4 relight the lighthouse",
            "▸ ○ lgt-4.2 re-silver the reflector",
            "▸ ○ lgt-4.3 rewire the lamp",
            "▸ ○ lgt-4.1 scrape the lantern glass",
            "▸ ○ lgt-4.6 cast the rail stanchions",
            "# InProgress 2",
            "▸ ○ lgt-4.5 replace the gallery rail",
            "▸ ◐ lgt-4.4 test the fog signal",
            "! ToCheck(1)",
        ]
    );
}

/// A bead waiting on another with a live agent on it is in the second
/// section, so the seat on blocked work still shows.
#[test]
fn a_blocked_bead_with_a_live_agent_is_in_progress() {
    let forest = listed(lighthouse());

    let in_progress = sketch(&forest)
        .into_iter()
        .skip_while(|line| !line.starts_with("# InProgress"))
        .collect::<Vec<_>>();
    assert!(
        in_progress.iter().any(|line| line.contains("lgt-4.5")),
        "{in_progress:#?}"
    );
    assert!(row_of(&forest, "lgt-4.5").agent.is_some());
}

/// The closed bead with a pane still on it is not listed, and its anomaly
/// is counted at the foot rather than lost with its row.
#[test]
fn an_anomaly_on_a_bead_the_list_does_not_draw_is_counted_at_its_foot() {
    let forest = listed(lighthouse());

    assert!(lines_of(&forest, "lgt-4.7").is_empty());
    assert_eq!(
        forest
            .lines()
            .iter()
            .last()
            .map(|line| line.content.clone()),
        Some(Content::Note(Note::ToCheck(1)))
    );
}

/// A tree with nothing wrong in it has no foot.
#[test]
fn a_list_with_no_anomaly_has_no_foot() {
    let forest = listed(ready_alone(
        "dunwich",
        LIGHTHOUSE,
        &panes_on(&["lgt-4.4"]),
        LIGHTHOUSE_READY,
    ));

    assert!(!sketch(&forest).iter().any(|line| line.starts_with('!')));
}

/// The same key puts the forest back, and again brings the list.
#[test]
fn the_key_switches_between_the_list_and_the_forest() {
    let forest_first = sketch(&flatten(lighthouse()));
    let mut forest = listed(lighthouse());
    let list = sketch(&forest);

    assert!(forest.apply(Action::ToggleReadyList));
    assert_eq!(sketch(&forest), forest_first);
    assert!(!forest.is_ready_list());

    assert!(forest.apply(Action::ToggleReadyList));
    assert_eq!(sketch(&forest), list);
}

/// A collection landing keeps the reader in the list, drawn from what it
/// brought back.
#[test]
fn a_refresh_keeps_the_list() {
    let mut forest = listed(lighthouse());

    forest.refresh(ready_alone(
        "dunwich",
        LIGHTHOUSE,
        &panes_on(&["lgt-4.4", "lgt-4.5", "lgt-4.7"]),
        &[
            "lgt-4", "lgt-4.1", "lgt-4.2", "lgt-4.3", "lgt-4.6", "lgt-4.8",
        ],
    ));

    assert!(forest.is_ready_list());
    assert!(sketch(&forest).contains(&"# Ready 6".to_string()));
}

/// Each row is the bead's own, so the selection sits on it and walks the
/// list from bead to bead, past the section headings and the foot.
#[test]
fn the_selection_walks_the_beads_and_not_the_headings() {
    let mut forest = listed(lighthouse());
    forest.apply(Action::Move(Motion::FirstRow));

    let last = key("dunwich", "lgt-4.4");
    let mut visited = vec![forest.selected_line()];
    walk::until(
        &mut forest,
        |forest| cursor(forest) == Some(&last),
        |forest| {
            forest.apply(Action::Move(Motion::NextRow));
            visited.push(forest.selected_line());
        },
        |forest| format!("the selection never reached lgt-4.4: {:#?}", sketch(forest)),
    );
    let beads: Vec<String> = visited
        .into_iter()
        .filter_map(|at| forest.lines()[at].bead().map(|bead| bead.id.clone()))
        .collect();

    assert_eq!(
        beads,
        ["lgt-4", "lgt-4.2", "lgt-4.3", "lgt-4.1", "lgt-4.6", "lgt-4.5", "lgt-4.4"]
    );
}

/// The bead selected in the list is the bead selected in the forest it
/// switches to, opened out to wherever the forest draws it.
#[test]
fn the_selection_crosses_from_the_list_to_the_forest() {
    let mut forest = flatten(lighthouse());
    forest.apply(Action::CollapseForest);
    assert!(lines_of(&forest, "lgt-4.6").is_empty());
    assert!(forest.apply(Action::ToggleReadyList));
    select_bead(&mut forest, "lgt-4.6");

    assert!(forest.apply(Action::ToggleReadyList));

    assert_eq!(cursor(&forest), Some(&key("dunwich", "lgt-4.6")));
    assert!(forest.apply(Action::ToggleReadyList));
    assert_eq!(cursor(&forest), Some(&key("dunwich", "lgt-4.6")));
}

/// A bead named on the command line roots the list as it roots the forest:
/// only what is at or beneath it is listed.
#[test]
fn a_named_root_lists_only_its_own_tree() {
    let mut forest = flatten(lighthouse());
    forest.focus_when_drawn(vec![key("dunwich", "lgt-4.5")]);
    assert!(forest.apply(Action::ToggleReadyList));

    assert_eq!(
        sketch(&forest),
        vec![
            "dunwich",
            "# Ready 1",
            "▸ ○ lgt-4.6 cast the rail stanchions",
            "# InProgress 1",
            "▸ ○ lgt-4.5 replace the gallery rail",
        ]
    );
}

/// A tree the live-agent filter holds back is still listed: work nobody is
/// on is the work there is to pick up.
#[test]
fn a_tree_with_no_live_agent_is_listed() {
    let mut quiet = ready_alone("harbour", HARBOUR, &[], &["hbr-3.1"]);
    quiet.refilter(Filter::LiveAgents);
    assert!(!quiet.hidden_trees.is_empty());
    let forest = listed(quiet);

    assert_eq!(
        sketch(&forest),
        vec![
            "harbour",
            "# Ready 1",
            "▸ ○ hbr-3.1 survey the silt",
            "# InProgress 0",
        ]
    );
}

/// A root whose tracker refused has no beads to list, and the list says so
/// rather than leave it out.
#[test]
fn a_tree_that_would_not_read_is_said_at_the_foot() {
    let forest = listed(snapshot());

    let foot: Vec<String> = sketch(&forest)
        .into_iter()
        .filter(|line| line.starts_with('!'))
        .collect();
    assert!(foot.contains(&"! Unread(1)".to_string()), "{foot:#?}");
    assert!(
        foot.contains(&"! FailedProjects(1)".to_string()),
        "{foot:#?}"
    );
}

/// The foot of the screen's lines, as the sketch draws them.
fn foot_of(forest: &Forest) -> Vec<String> {
    sketch(forest)
        .into_iter()
        .filter(|line| line.starts_with('!'))
        .collect()
}

/// A bead waiting on work no tracker holds, and a loop the walk cut, are
/// said at the foot as the forest says them under their tree.
#[test]
fn an_orphaned_dependency_and_a_loop_are_said_at_the_foot() {
    let orphaned = foot_of(&listed(snapshot()));
    let looped = foot_of(&listed(alone("dunwich", LOOPED, &panes_on(&["cyc-1.1"]))));

    assert!(
        orphaned.contains(&"! OrphanedDependencies(1)".to_string()),
        "{orphaned:#?}"
    );
    assert!(looped.contains(&"! Cycle(1)".to_string()), "{looped:#?}");
}

/// A slipway as bd writes it. Draining it is ready, and three things wait
/// on it: the root, as its parent; `slp-1.2.1`, which waits on nothing
/// else and is the only child of `slp-1.2`; and `slp-1.3`, which also waits
/// on the keel `slp-1.4`.
const SLIPWAY: &str = r#"[
  {"id":"slp-1","title":"refit the lifeboat","status":"open",
   "priority":2,"issue_type":"epic","created_at":"2026-09-01T09:00:00Z"},
  {"id":"slp-1.1","title":"drain the slipway","status":"open",
   "dependencies":[{"depends_on_id":"slp-1","type":"parent-child"}],
   "priority":2,"issue_type":"task","created_at":"2026-09-02T09:00:00Z"},
  {"id":"slp-1.2","title":"haul out the hull","status":"open",
   "dependencies":[{"depends_on_id":"slp-1","type":"parent-child"}],
   "priority":2,"issue_type":"epic","created_at":"2026-09-03T09:00:00Z"},
  {"id":"slp-1.2.1","title":"lower the cradle","status":"open",
   "dependencies":[{"depends_on_id":"slp-1.2","type":"parent-child"},
                   {"depends_on_id":"slp-1.1","type":"blocks"}],
   "priority":2,"issue_type":"task","created_at":"2026-09-04T09:00:00Z"},
  {"id":"slp-1.3","title":"launch the lifeboat","status":"open",
   "dependencies":[{"depends_on_id":"slp-1","type":"parent-child"},
                   {"depends_on_id":"slp-1.1","type":"blocks"},
                   {"depends_on_id":"slp-1.4","type":"blocks"}],
   "priority":2,"issue_type":"task","created_at":"2026-09-05T09:00:00Z"},
  {"id":"slp-1.4","title":"order the new keel","status":"open",
   "dependencies":[{"depends_on_id":"slp-1","type":"parent-child"}],
   "priority":2,"issue_type":"task","created_at":"2026-09-06T09:00:00Z"}
]"#;

/// Another root of the same project, whose one task waits on draining the
/// slipway and nothing else.
const BOATHOUSE: &str = r#"[
  {"id":"slp-2","title":"paint the boathouse","status":"open",
   "priority":2,"issue_type":"epic","created_at":"2026-09-07T09:00:00Z"},
  {"id":"slp-2.1","title":"strip the old paint","status":"open",
   "dependencies":[{"depends_on_id":"slp-2","type":"parent-child"},
                   {"depends_on_id":"slp-1.1","type":"blocks"}],
   "priority":2,"issue_type":"task","created_at":"2026-09-08T09:00:00Z"}
]"#;

const SLIPWAY_READY: &[&str] = &["slp-1", "slp-1.1", "slp-1.4"];

fn slipway() -> Snapshot {
    ready_alone("dunwich", SLIPWAY, &[], SLIPWAY_READY)
}

/// The list's lines from one bead's row to the next line that is not
/// beneath it, as the sketch draws them: the row and what it expands to.
fn expansion_of(forest: &Forest, id: &str) -> Vec<String> {
    let at = *lines_of(forest, id)
        .first()
        .unwrap_or_else(|| panic!("{id} is not drawn: {:#?}", sketch(forest)));
    let mut lines = sketch(forest).into_iter().skip(at);
    let row = lines.next().expect("the row itself");
    std::iter::once(row)
        .chain(lines.take_while(|line| {
            line.starts_with("  ") && line.chars().nth(2).is_some_and(|at| "├│└ ".contains(at))
        }))
        .collect()
}

/// Expanding a row draws what the bead holds up, then what each of those
/// holds up, all the way to the root, hung on the forest's connectors with
/// an arrow saying the tree reads upward. A step waiting on nothing but the
/// steps beneath it would be unblocked, and one waiting on more says how
/// many more.
#[test]
fn expanding_a_row_draws_what_it_unblocks_up_to_the_root() {
    let mut forest = listed(slipway());
    select_bead(&mut forest, "slp-1.1");

    assert!(forest.apply(Action::ExpandOrChild));

    assert_eq!(
        expansion_of(&forest, "slp-1.1"),
        vec![
            "▾ ○ slp-1.1 drain the slipway",
            "  ├─↑ ○ slp-1 refit the lifeboat · waits on 3 more",
            "  ├┄↑ ○ slp-1.2.1 lower the cradle · would be unblocked",
            "  │   └─↑ ○ slp-1.2 haul out the hull · would be unblocked",
            "  │       └─↑ ○ slp-1 refit the lifeboat · waits on 2 more",
            "  └┄↑ ○ slp-1.3 launch the lifeboat · waits on 1 more",
            "      └─↑ ○ slp-1 refit the lifeboat · waits on 2 more",
        ]
    );
}

/// The mark is a count of what else is in the way, so a step the expanded
/// bead alone holds up and one still waiting on another unfinished bead are
/// told apart by the row itself.
#[test]
fn a_step_the_bead_alone_holds_up_is_marked_apart_from_one_waiting_on_more() {
    let mut forest = listed(slipway());
    select_bead(&mut forest, "slp-1.1");
    forest.apply(Action::ExpandOrChild);

    assert_eq!(step_of(&forest, "slp-1.2.1").also_waits_on, Some(0));
    assert_eq!(step_of(&forest, "slp-1.3").also_waits_on, Some(1));
}

/// A row starts shut, a row with nothing waiting on it has nothing to
/// expand, and shutting the row again puts the list back as it was.
#[test]
fn a_row_shuts_over_what_it_unblocks() {
    let mut forest = listed(slipway());
    let shut = sketch(&forest);
    assert!(shut.contains(&"▸ ○ slp-1.1 drain the slipway".to_string()));
    assert!(shut.contains(&"  ○ slp-1 refit the lifeboat".to_string()));
    select_bead(&mut forest, "slp-1.1");

    forest.apply(Action::ExpandOrChild);
    forest.apply(Action::CollapseOrParent);

    assert_eq!(sketch(&forest), shut);
}

/// Expanding a row in the list opens nothing in the forest, where expanding
/// already means what must finish first.
#[test]
fn expanding_a_row_leaves_the_forest_as_it_was() {
    let mut forest = flatten(slipway());
    let trees = sketch(&forest);
    forest.apply(Action::ToggleReadyList);
    select_bead(&mut forest, "slp-1.1");
    forest.apply(Action::ExpandOrChild);

    forest.apply(Action::ToggleReadyList);

    assert_eq!(sketch(&forest), trees);
}

/// A bead the expansion reaches more than one way is drawn on several
/// lines, and the selection keeps to the one it is on through a
/// collection landing.
#[test]
fn the_selection_keeps_to_its_step_through_a_refresh() {
    let mut forest = listed(slipway());
    select_bead(&mut forest, "slp-1.1");
    forest.apply(Action::ExpandOrChild);
    let last = *lines_of(&forest, "slp-1")
        .last()
        .expect("the root is drawn");
    step_onto(&mut forest, last);

    forest.refresh(slipway());

    assert_eq!(forest.selected_line(), last);
    assert_eq!(cursor(&forest), Some(&key("dunwich", "slp-1")));
}

/// What a bead unblocks under another root is drawn too, up to that root.
#[test]
fn a_waiter_under_another_root_is_drawn_up_to_its_root() {
    let mut forest = listed(ready_together(
        "dunwich",
        &[SLIPWAY, BOATHOUSE],
        &[],
        SLIPWAY_READY,
    ));
    select_bead(&mut forest, "slp-1.1");

    forest.apply(Action::ExpandOrChild);

    let expanded = expansion_of(&forest, "slp-1.1");
    assert!(
        expanded.contains(&"  └┄↑ ○ slp-2.1 strip the old paint · would be unblocked".to_string()),
        "{expanded:#?}"
    );
    assert!(
        expanded
            .contains(&"      └─↑ ○ slp-2 paint the boathouse · would be unblocked".to_string()),
        "{expanded:#?}"
    );
}

/// The row a step is drawn on, found by the whole id its line carries.
fn step_of<'a>(forest: &'a Forest, id: &str) -> &'a Row {
    forest
        .lines()
        .iter()
        .find_map(|line| match (line.bead(), &line.content) {
            (Some(bead), Content::Waiting(step)) if bead.id == id => Some(&step.row),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{id} is drawn as no step: {:#?}", sketch(forest)))
}
