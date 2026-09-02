//! Everything `bdi` learns by running another program.
//!
//! One module for the running itself, one for each of the two programs it
//! asks, one each for what a tracker and an agent provider answer whichever
//! program it is, one for the environment each project's tracker is asked in,
//! one for what it works out where nothing configured it, one for the pane
//! text a tail shows, and one for the socket a setup pokes to say a project's
//! work changed.

pub mod agents;
pub mod bd;
pub mod changes;
pub mod discovery;
pub mod environment;
pub mod herdr;
pub mod panes;
pub mod run;
pub mod tracker;
pub mod worktree;
