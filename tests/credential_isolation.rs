//! Which tracker's credential each call `bdi` makes goes out carrying.
//!
//! `tests/credential.rs` proves a real subprocess is handed the credential it
//! is given and never inherits one. This proves the other half: that a whole
//! collection hands each project's bd exactly that project's credential, and
//! hands everything else none. Only a double can see the absence — a call
//! carrying a superfluous but correct-looking credential still succeeds.
//!
//! It gets a test binary to itself for the reason `tests/credential.rs` does:
//! the ambient credential is a variable set in this process, and a process's
//! environment is shared by every thread in it. One test, alone in a binary,
//! has no other threads to race.

use std::path::Path;

use beady_eye::collect::run::{Env, CREDENTIAL_VAR};
use beady_eye::config::Config;
use beady_eye::model::snapshot::Filter;
use chrono::{DateTime, Utc};

mod canned;

use canned::{Call, Canned};

const ORBITAL_DIR: &str = "/srv/work/orbital";
const HARBOUR_DIR: &str = "/srv/work/harbour";
const SOLO_DIR: &str = "/srv/work/solo";

/// The credential the shell `bdi` was launched from holds.
const AMBIENT: &str = "the-launching-shells-password";

/// Two trackers, each with a credential of its own. A config naming more than
/// one project is refused unless every one of them does.
const TWO_PROJECTS: &str = r#"
[[projects]]
name = "orbital"
path = "/srv/work/orbital"
credential_command = "pass show orbital/tracker"

[[projects]]
name = "harbour"
path = "/srv/work/harbour"
credential_command = "pass show harbour/tracker"
"#;

/// The zero-config shape: one project, no credential command, reaching its
/// tracker on the credential the launching shell holds.
const ONE_PROJECT: &str = r#"
[[projects]]
name = "solo"
path = "/srv/work/solo"
"#;

/// One in-flight bead under a closed epic, so the climb to a root is made as
/// well as discovered — every call bd is asked for is one this checks. What
/// each tracker holds does not matter here; that every call to it carries the
/// right credential does.
fn tracker(runner: Canned, cwd: &str, id: &str) -> Canned {
    runner
        .answering_in(
            cwd,
            "bd list --status open,in_progress,blocked,deferred --limit 0 --json",
            &format!(
                r#"[{{"id":"{id}.1","title":"the work","status":"in_progress","parent":"{id}"}}]"#
            ),
        )
        .answering_in(cwd, "bd ready --limit 0 --json", "[]")
        .answering_in(cwd, "bd blocked --json", "[]")
        .answering_in(
            cwd,
            &format!("bd show {id} --json"),
            &format!(r#"[{{"id":"{id}","parent":null}}]"#),
        )
        .answering_in(
            cwd,
            "bd list --all --limit 0 --json",
            &format!(
                r#"[{{"id":"{id}","title":"the work","status":"in_progress","parent_id":"",
                      "priority":1,"issue_type":"task","started_at":"2026-08-29T09:00:00Z",
                      "updated_at":"2026-08-29T09:00:00Z"}}]"#
            ),
        )
}

const NO_PANES: &str = r#"{"id":"cli:agent:list","result":{"agents":[]}}"#;

fn across_two_projects() -> Canned {
    let runner = Canned::default()
        .answering("herdr agent list", NO_PANES)
        .answering("sh -c pass show orbital/tracker", "orbital-secret\n")
        .answering("sh -c pass show harbour/tracker", "harbour-secret\n");
    let runner = tracker(runner, ORBITAL_DIR, "orb-7");
    tracker(runner, HARBOUR_DIR, "har-3")
}

fn on_the_ambient_credential() -> Canned {
    let runner = Canned::default().answering("herdr agent list", NO_PANES);
    tracker(runner, SOLO_DIR, "solo-1")
}

fn now() -> DateTime<Utc> {
    "2026-08-30T12:00:00Z".parse().expect("the instant parses")
}

/// Collect over a config, and hand back every call that was made.
fn calls_made_reading(config: &str, runner: &Canned) -> Vec<Call> {
    let cfg = Config::from_toml(config).expect("the config parses");
    beady_eye::app::run(&cfg, runner, Filter::All, now());
    runner.calls()
}

fn bd_calls_in<'a>(calls: &'a [Call], cwd: &str) -> Vec<&'a Call> {
    calls
        .iter()
        .filter(|call| call.argv.starts_with("bd ") && call.cwd.as_deref() == Some(Path::new(cwd)))
        .collect()
}

fn holding(password: &str) -> Env {
    Env::from([(CREDENTIAL_VAR.to_string(), password.to_string())])
}

/// The property in one test, because setting the ambient credential is what
/// makes each part of it mean anything and it can only be set once here.
#[test]
fn each_call_carries_the_credential_of_the_tracker_it_is_for_and_no_other() {
    std::env::set_var(CREDENTIAL_VAR, AMBIENT);

    let calls = calls_made_reading(TWO_PROJECTS, &across_two_projects());

    for (dir, secret) in [
        (ORBITAL_DIR, "orbital-secret"),
        (HARBOUR_DIR, "harbour-secret"),
    ] {
        let asked = bd_calls_in(&calls, dir);
        assert!(!asked.is_empty(), "no bd call was made in {dir}");
        for call in asked {
            assert_eq!(
                call.env,
                holding(secret),
                "`{}` in {dir} went out with an environment that is not {dir}'s \
                 credential alone",
                call.argv
            );
        }
    }

    let elsewhere: Vec<&Call> = calls
        .iter()
        .filter(|call| !call.argv.starts_with("bd "))
        .collect();
    assert!(
        elsewhere.iter().any(|call| call.argv == "herdr agent list"),
        "herdr was never asked, so nothing here says what it was given"
    );
    assert_eq!(
        elsewhere
            .iter()
            .filter(|call| call.argv.starts_with("sh -c pass show"))
            .count(),
        2,
        "both credential commands were run, or this says nothing about them"
    );
    for call in elsewhere {
        assert_eq!(
            call.env,
            Env::new(),
            "`{}` is not a tracker and was given a credential",
            call.argv
        );
    }

    let calls = calls_made_reading(ONE_PROJECT, &on_the_ambient_credential());
    let asked = bd_calls_in(&calls, SOLO_DIR);
    assert!(!asked.is_empty(), "no bd call was made in {SOLO_DIR}");
    for call in asked {
        assert_eq!(
            call.env,
            holding(AMBIENT),
            "`{}` did not go out with the credential the launching shell holds, alone",
            call.argv
        );
    }
}
