//! The integration tests that never start `bdi`. They link the library, or run
//! the shell shims directly, so the only cost of a binary each would be the
//! link. A test that sets an environment variable does not belong here: it
//! stays alone in its own binary.

#[path = "../canned/mod.rs"]
mod canned;
#[path = "../terminal/mod.rs"]
mod terminal;

mod a_shim_refuses_rather_than_hand_the_call_on;
mod conflict_directions;
mod forest_order;
mod one_beads_dependency_tree;
mod snapshot_json;
mod the_beads_that_depend_on_a_bead_are_listed;
