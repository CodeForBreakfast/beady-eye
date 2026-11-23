//! The beads left to do in one project: each of its beads not yet finished,
//! once, drawn in place of the forest.

use super::*;
use pretty_assertions::assert_eq;

/// Harbour's beads. `hbr-1.2` is closed, so it is not left to do.
const HARBOUR: &str = r#"[
  {"id":"hbr-1","title":"open the harbour","status":"open","priority":2,"issue_type":"epic"},
  {"id":"hbr-1.1","title":"sign the berth lease","status":"open","priority":2,
   "issue_type":"task",
   "dependencies":[{"depends_on_id":"hbr-1","type":"parent-child"}]},
  {"id":"hbr-1.2","title":"chart the approach","status":"closed","priority":1,
   "issue_type":"task",
   "dependencies":[{"depends_on_id":"hbr-1","type":"parent-child"}]},
  {"id":"hbr-1.3","title":"clear the silted channel","status":"in_progress","priority":1,
   "issue_type":"task",
   "dependencies":[{"depends_on_id":"hbr-1","type":"parent-child"}]}
]"#;

const DUNWICH: &str = r#"[
  {"id":"dun-7","title":"lift the ground station","status":"open","priority":2,
   "issue_type":"epic"}
]"#;

fn left_in_harbour() -> Forest {
    let mut forest = flatten(harbour_and_dunwich(
        HARBOUR,
        DUNWICH,
        &[("dunwich", "dun-7"), ("harbour", "hbr-1")],
        &[],
    ));
    forest.draw_list(Some(List::LeftToDo {
        project: "harbour".to_string(),
    }));
    forest
}

/// The list holds the project's heading and each of its beads not yet
/// finished, the most urgent first, under one section that counts them.
#[test]
fn the_list_holds_each_unfinished_bead_in_the_project_the_most_urgent_first() {
    assert_eq!(
        sketch(&left_in_harbour())[..5],
        [
            "harbour",
            "# LeftToDo { project: \"harbour\" } 3",
            "▸ ◐ hbr-1.3 clear the silted channel",
            "▸ ○ hbr-1 open the harbour",
            "▸ ○ hbr-1.1 sign the berth lease",
        ]
    );
}

/// The list holds as many beads as the project's heading says are left.
#[test]
fn the_list_holds_as_many_beads_as_its_heading_says_are_left() {
    let forest = left_in_harbour();
    let lines = forest.lines();
    let (mut left, mut listed) = (None, None);
    for (_, line) in lines.viewport(0, lines.len()) {
        match &line.content {
            Content::Project(project) => left = Some(project.counts.unfinished()),
            Content::Section { count, .. } => listed = Some(*count),
            _ => {}
        }
    }
    assert_eq!(left, Some(3));
    assert_eq!(listed, left);
}
