//! The beads to check: each bead with an anomaly, once, across every project
//! read or within one, drawn in place of the forest.

use super::*;
use pretty_assertions::assert_eq;

/// Harbour's beads. `hbr-1.3` is in progress with nobody on it, so it is to
/// check, and so is dunwich's `dun-7.2`.
const HARBOUR: &str = r#"[
  {"id":"hbr-1","title":"open the harbour","status":"open","priority":2,"issue_type":"epic"},
  {"id":"hbr-1.1","title":"sign the berth lease","status":"open","priority":2,
   "issue_type":"task",
   "dependencies":[{"depends_on_id":"hbr-1","type":"parent-child"}]},
  {"id":"hbr-1.3","title":"clear the silted channel","status":"in_progress","priority":1,
   "issue_type":"task",
   "dependencies":[{"depends_on_id":"hbr-1","type":"parent-child"}]}
]"#;

const DUNWICH: &str = r#"[
  {"id":"dun-7","title":"lift the ground station","status":"open","priority":2,
   "issue_type":"epic"},
  {"id":"dun-7.1","title":"sign for the dish","status":"open","priority":1,
   "issue_type":"task",
   "dependencies":[{"depends_on_id":"dun-7","type":"parent-child"}]},
  {"id":"dun-7.2","title":"shore up the mast","status":"in_progress","priority":0,
   "issue_type":"task",
   "dependencies":[{"depends_on_id":"dun-7","type":"parent-child"}]}
]"#;

fn two_to_check() -> Snapshot {
    harbour_and_dunwich(
        HARBOUR,
        DUNWICH,
        &[("dunwich", "dun-7"), ("harbour", "hbr-1")],
        &[],
    )
}

fn checking(project: Option<&str>) -> Forest {
    let mut forest = flatten(two_to_check());
    forest.draw_list(Some(List::ToCheck {
        project: project.map(str::to_string),
    }));
    forest
}

/// Every bead with an anomaly is listed once, the most urgent first, under
/// one section that counts them. The list says nothing more of them at its
/// foot, since its section already counts them.
#[test]
fn the_list_holds_every_bead_to_check_the_most_urgent_first() {
    assert_eq!(
        sketch(&checking(None)),
        vec![
            "dunwich",
            "harbour",
            "# ToCheck { project: None } 2",
            "▸ ◐ dun-7.2 shore up the mast",
            "▸ ◐ hbr-1.3 clear the silted channel",
        ]
    );
}

/// Narrowed to one project, the list holds that project's heading and only
/// the beads to check in its trees.
#[test]
fn a_list_narrowed_to_a_project_holds_only_its_beads_to_check() {
    assert_eq!(
        sketch(&checking(Some("harbour"))),
        vec![
            "harbour",
            "# ToCheck { project: Some(\"harbour\") } 1",
            "▸ ◐ hbr-1.3 clear the silted channel",
        ]
    );
}

/// What the forest counts as to check is what the list then holds, and what
/// every other list counts at its foot.
#[test]
fn the_count_to_check_is_what_the_list_holds() {
    let forest = flatten(two_to_check());
    assert_eq!(forest.list_counts(false).to_check, 2);

    let mut ready = flatten(two_to_check());
    ready.apply(Action::ToggleReadyList);
    assert!(
        sketch(&ready).contains(&"! ToCheck(2)".to_string()),
        "{:#?}",
        sketch(&ready)
    );
}
