//! The beads a description names, listed in the bead window.

use super::*;
use crate::view::show;
use pretty_assertions::assert_eq;

/// Dunwich's epic, whose description names beads in prose. `dun-9` is in no
/// tree, and `hbr-3` belongs to another project.
const NAMING: &str = r#"[
  {"id":"dun-1","title":"lift the station","status":"open","issue_type":"epic",
   "description":"Filed as dun-2. Follows dun-2.1, then dun-2 again, (dun-3), and not read-ci-verdict, dun-9, hbr-3, Dun-3, dun-3.md or dun-1 itself."},
  {"id":"dun-2","title":"true the mount","status":"open",
   "dependencies":[{"depends_on_id":"dun-1","type":"parent-child"}]},
  {"id":"dun-2.1","title":"seal the horn","status":"open",
   "description":"Part of dun-2, which waits on dun-3, and so does dun-4.",
   "dependencies":[{"depends_on_id":"dun-2","type":"parent-child"},
                   {"depends_on_id":"dun-3","type":"blocks"}]},
  {"id":"dun-3","title":"pour the pad","status":"open",
   "dependencies":[{"depends_on_id":"dun-1","type":"parent-child"}]},
  {"id":"dun-4","title":"cut the verge","status":"open",
   "dependencies":[{"depends_on_id":"dun-1","type":"parent-child"}]}
]"#;

fn window_on(id: &str) -> Forest {
    let mut forest = flatten(harbour_and_dunwich(
        "[]",
        NAMING,
        &[("dunwich", "dun-1")],
        &[],
    ));
    assert!(forest.go_to(&key("dunwich", id)));
    forest
}

fn ids(named: &[crate::model::edges::Related]) -> Vec<&str> {
    named.iter().map(|named| named.id.as_str()).collect()
}

/// A whole token shaped like an id, in the order it is first named, once.
/// A hyphenated word, an id with a second hyphen, a capital, a file name,
/// the bead itself, an id no tree draws and an id of another tracker get no
/// row. A full stop that ends a sentence is not part of the token.
#[test]
fn a_description_names_the_beads_the_forest_draws() {
    let forest = window_on("dun-1");
    let node = show::selected(&forest).expect("a bead is selected");

    let named = show::mentioned(&forest, node);

    assert_eq!(ids(&named), ["dun-2", "dun-2.1", "dun-3"]);
    assert_eq!(named[1].title.as_deref(), Some("seal the horn"));
    assert_eq!(named[1].status, Some(crate::model::types::Status::Open));
}

/// A bead the window already lists as its parent, a dependency or a waiter
/// gets one row, in the section it has.
#[test]
fn a_bead_the_window_already_lists_is_not_listed_again() {
    let forest = window_on("dun-2.1");
    let node = show::selected(&forest).expect("a bead is selected");

    assert_eq!(ids(&show::mentioned(&forest, node)), ["dun-4"]);
}

/// The ring steps through the beads a description names, after the ones the
/// window already lists, and Enter on one goes there.
#[test]
fn a_named_bead_can_be_followed() {
    let mut forest = window_on("dun-1");
    let node = show::selected(&forest).expect("a bead is selected");
    let named = show::mentioned(&forest, node);

    let followed = show::key_of(&forest, &named[2]).expect("the bead is keyed");
    assert!(show::followable(&forest, &named[2]));
    assert!(forest.go_to(&followed));
    assert_eq!(cursor(&forest), Some(&key("dunwich", "dun-3")));
}

/// The ring names a bead by its id, so a bead of another project that has
/// the id of a mention keeps the one row, and the mention is not a second
/// the ring would take for it.
#[test]
fn an_id_another_projects_row_already_has_is_listed_once() {
    let forest = window_on("dun-1");
    let mut node = show::selected(&forest).expect("a bead is selected").clone();
    node.blocks.push(crate::model::edges::Related {
        id: "dun-2".to_string(),
        project: Some("harbour".to_string()),
        edge: crate::model::types::Edge::Blocks,
        status: Some(crate::model::types::Status::Open),
        title: Some("another project's bead".to_string()),
    });

    assert_eq!(ids(&show::mentioned(&forest, &node)), ["dun-2.1", "dun-3"]);
}
