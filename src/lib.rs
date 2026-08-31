//! The library `bdi` is: the modules it holds, and how far into them a test
//! may reach.
//!
//! bdi and the tests under `tests/` are the only consumers of this library,
//! and a `pub` module is reachable by definition — so `pub mod` on every line
//! here is what switches `dead_code` off across the whole crate. These
//! modules are private instead, and the lint says so when nothing production
//! reaches an item.
//!
//! The tests reach further in than bdi does. `testing` opens the five they
//! name back up, and cargo enables it for exactly the builds that compile
//! tests, because the package dev-depends on itself. So `cargo build` sees
//! the narrow surface, `cargo test` the wide one, and one line here decides
//! both. `tui` takes no gate: no test names it, so it is private in every
//! build.

pub mod cli;

mod tui;

#[cfg(feature = "testing")]
pub mod app;
#[cfg(not(feature = "testing"))]
mod app;

#[cfg(feature = "testing")]
pub mod collect;
#[cfg(not(feature = "testing"))]
mod collect;

#[cfg(feature = "testing")]
pub mod config;
#[cfg(not(feature = "testing"))]
mod config;

#[cfg(feature = "testing")]
pub mod model;
#[cfg(not(feature = "testing"))]
mod model;

#[cfg(feature = "testing")]
pub mod view;
#[cfg(not(feature = "testing"))]
mod view;
