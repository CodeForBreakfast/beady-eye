//! bd's rows and herdr's panes, joined into the snapshot the view draws.
//!
//! One module for the vocabulary both sides are held in, one for what nests
//! what among the rows, one for the join itself, one for the rules that read
//! a disagreement out of it, one for the badge a row carries beside its
//! title, and one for the snapshot they all assemble into.

pub mod anomaly;
pub mod badges;
pub mod join;
pub mod snapshot;
pub mod tree;
pub mod types;
