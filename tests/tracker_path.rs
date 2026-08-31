//! Which tracker a subprocess reads, and which it does not.
//!
//! The sibling of `credential.rs`. A project's credential and the tracker it
//! authenticates against are one identity, and inheriting either one reaches
//! the wrong database. bd prefers `BEADS_DIR` to the working directory it is
//! given, so a variable set in the launching shell outranks the directory
//! `bdi` picked for a project.
//!
//! Proved against real processes for the same reason: a Runner double cannot
//! see an inherited variable, because it never launches anything. It gets a
//! test binary to itself because setting a variable here stands in for that
//! launching shell, and a process's environment is shared by every thread in
//! it.

use beady_eye::collect::run::{Env, RealRunner, Runner};

/// The variable bd reads in preference to its working directory.
const TRACKER_VAR: &str = "BEADS_DIR";

/// What a child makes of the tracker it was launched with, or `unset` where
/// it was launched with none.
fn what_a_child_sees(env: &Env) -> String {
    let report = format!("printf '%s' \"${{{TRACKER_VAR}-unset}}\"");

    RealRunner
        .run("sh", &["-c", &report], None, env)
        .expect("sh runs")
}

/// One test because both halves are one fact: a subprocess reads the tracker
/// it is given, and only that. Asserting the overlay alone would pass with
/// the inheritance still in place.
#[test]
fn a_subprocess_is_given_its_tracker_and_never_inherits_one() {
    std::env::set_var(TRACKER_VAR, "/the/launching/shells/tracker/.beads");

    assert_eq!(
        what_a_child_sees(&Env::new()),
        "unset",
        "the shell bdi was launched from reached a child that was given no tracker"
    );

    let its_own = Env::from([(
        TRACKER_VAR.to_string(),
        "/this/projects/.beads".to_string(),
    )]);

    assert_eq!(
        what_a_child_sees(&its_own),
        "/this/projects/.beads",
        "a project's own tracker no longer reaches its subprocess"
    );
}
