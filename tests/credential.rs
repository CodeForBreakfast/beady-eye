//! Which credential a subprocess is given, and which it is not.
//!
//! This is the leading risk in the design, so it is proved against real
//! processes: a variable set in this one, and a child that reports what it
//! was handed.
//!
//! It gets a test binary to itself because setting a variable here stands in
//! for the shell `bdi` was launched from, and a process's environment is
//! shared by every thread in it. Alone in a binary there are no other
//! threads to race.

use beady_eye::collect::run::{Env, RealRunner, Runner};

/// The variable bd authenticates its Dolt server with.
const CREDENTIAL_VAR: &str = "BEADS_DOLT_PASSWORD";

/// What a child makes of the credential it was launched with, or `unset`
/// where it was launched with none.
fn what_a_child_sees(env: &Env) -> String {
    let report = format!("printf '%s' \"${{{CREDENTIAL_VAR}-unset}}\"");

    RealRunner
        .run("sh", &["-c", &report], None, env)
        .expect("sh runs")
}

/// The property, in one test because both halves are one fact: a subprocess
/// gets the credential it is given, and only that. Asserting the overlay
/// alone would pass with the inheritance still in place.
#[test]
fn a_subprocess_is_given_its_credential_and_never_inherits_one() {
    std::env::set_var(CREDENTIAL_VAR, "the-launching-shells-password");

    assert_eq!(
        what_a_child_sees(&Env::new()),
        "unset",
        "the shell bdi was launched from reached a child that was given no credential"
    );

    let its_own = Env::from([(
        CREDENTIAL_VAR.to_string(),
        "this-projects-password".to_string(),
    )]);

    assert_eq!(
        what_a_child_sees(&its_own),
        "this-projects-password",
        "a project's own credential no longer reaches its subprocess"
    );
}
