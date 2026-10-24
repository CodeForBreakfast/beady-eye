//! `bdi bd <project> human respond …` runs bd against that project's tracker
//! and no other, and refuses every other command line before bd is reached.
//!
//! The cases run the binary, because what is under test is the command line a
//! person or a seat types and the bd it ends up running.

mod terminal;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use terminal::shims::ShimmedTracker;

/// What bd says on closing a bead with a response.
const CLOSED: &str = "✔ Bead kad-1 closed with response.\n";

/// The call as the shim records it, past the `-C` naming the tracker.
const RESPONDED: &str = "human respond kad-1 -r yes";

/// A `HOME` whose config names two projects, each in a directory of its own,
/// with `kadath` settled by `settings`.
fn a_home_naming_two_projects(named: &str, settings: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("bdi-{named}-{}", std::process::id()));
    for project in ["arkham", "kadath"] {
        std::fs::create_dir_all(home.join(project)).expect("the directory is ours to make");
    }
    std::fs::create_dir_all(home.join(".config/beady-eye")).expect("the directory is ours to make");
    std::fs::write(
        home.join(".config/beady-eye/config.toml"),
        format!(
            "[[projects]]\nname = \"arkham\"\npath = \"{arkham}\"\n\n\
             [[projects]]\nname = \"kadath\"\npath = \"{kadath}\"\n{settings}",
            arkham = home.join("arkham").display(),
            kadath = home.join("kadath").display(),
        ),
    )
    .expect("the config is ours to write");
    home
}

fn bdi(home: &Path, tracker: &ShimmedTracker, args: &str) -> Output {
    bdi_exporting(home, tracker, args, &[])
}

/// `bdi` run from a shell that also exports `exported`.
fn bdi_exporting(
    home: &Path,
    tracker: &ShimmedTracker,
    args: &str,
    exported: &[(&str, &str)],
) -> Output {
    Command::new(env!("CARGO_BIN_EXE_bdi"))
        .args(args.split_whitespace())
        .current_dir(home)
        .env("HOME", home)
        .env_remove("BEADS_DIR")
        .env_remove("BEADS_DOLT_PASSWORD")
        .env_remove("BDI_PROJECT")
        .envs(tracker.environment())
        .envs(exported.iter().copied())
        .output()
        .expect("bdi runs")
}

fn said(out: &[u8]) -> String {
    String::from_utf8_lossy(out).into_owned()
}

/// Only kadath's tracker has an answer, so bd reaching arkham's instead would
/// be refused rather than served.
#[test]
fn a_response_is_run_against_the_named_projects_tracker_and_bd_answers_the_caller() {
    let home = a_home_naming_two_projects("respond-reaches-kadath", "");
    let tracker = ShimmedTracker::beside(&home);
    tracker.answers_for("kadath", RESPONDED, CLOSED);

    let out = bdi(&home, &tracker, "bd kadath human respond kad-1 -r yes");

    assert!(out.status.success(), "{}", said(&out.stderr));
    assert_eq!(said(&out.stdout), CLOSED);
    assert_eq!(tracker.calls(), [RESPONDED]);
}

/// Whether a tracker keeps the journal is its config's to say, so the
/// caller's own setting reaches bd no more than its choice of database does.
#[test]
fn a_journal_the_caller_turns_on_does_not_reach_bd() {
    let home = a_home_naming_two_projects("respond-ignores-exported-journal", "");
    let tracker = ShimmedTracker::beside(&home);
    tracker.answers_for("kadath", RESPONDED, CLOSED);

    let out = bdi_exporting(
        &home,
        &tracker,
        "bd kadath human respond kad-1 -r yes",
        &[("BD_EVENTS_JOURNAL", "1")],
    );

    assert!(out.status.success(), "{}", said(&out.stderr));
    assert_eq!(tracker.calls(), [RESPONDED]);
    assert_eq!(tracker.journalled_calls(), Vec::<String>::new());
}

/// A permission rule reads the command line, and an exported variable is not
/// on it. So a database the caller exports never reaches bd, and the response
/// lands where kadath's own config says. Only kadath has an answer, so a
/// redirected call is refused.
#[test]
fn a_database_the_caller_exports_does_not_redirect_the_response() {
    let home = a_home_naming_two_projects("respond-ignores-exported-database", "");
    let tracker = ShimmedTracker::beside(&home);
    tracker.answers_for("kadath", RESPONDED, CLOSED);

    let out = bdi_exporting(
        &home,
        &tracker,
        "bd kadath human respond kad-1 -r yes",
        &[("BEADS_DOLT_SERVER_DATABASE", "arkham")],
    );

    assert!(out.status.success(), "{}", said(&out.stderr));
    assert_eq!(said(&out.stdout), CLOSED);
    assert_eq!(tracker.calls(), [RESPONDED]);
}

/// Entering the directory inherits the caller's shell, so the variable would
/// otherwise come back in what direnv captured.
#[test]
fn a_database_the_caller_exports_does_not_come_back_through_direnv() {
    let home = a_home_naming_two_projects("respond-ignores-database-via-direnv", "");
    std::fs::write(home.join("kadath/.envrc"), "use flake\n").expect("the .envrc is ours to write");
    let tracker = ShimmedTracker::beside(&home);
    tracker.enters_with(&[]);
    tracker.answers_for("kadath", RESPONDED, CLOSED);

    let out = bdi_exporting(
        &home,
        &tracker,
        "bd kadath human respond kad-1 -r yes",
        &[("BEADS_DOLT_SERVER_DATABASE", "arkham")],
    );

    assert!(out.status.success(), "{}", said(&out.stderr));
    assert_eq!(
        tracker.direnv_runs(),
        ["exec . env -0", "status --json"],
        "kadath was entered"
    );
    assert_eq!(tracker.calls(), [RESPONDED]);
}

/// What the project's own directory sets is its own config, so it still
/// picks the database over the caller's.
#[test]
fn a_database_the_projects_own_directory_sets_still_reaches_bd() {
    let home = a_home_naming_two_projects("respond-keeps-the-projects-database", "");
    std::fs::write(home.join("kadath/.envrc"), "use flake\n").expect("the .envrc is ours to write");
    let tracker = ShimmedTracker::beside(&home);
    tracker.enters_with(&[("BEADS_DOLT_SERVER_DATABASE", "kadath-db")]);
    tracker.answers_for("kadath-db", RESPONDED, CLOSED);

    let out = bdi_exporting(
        &home,
        &tracker,
        "bd kadath human respond kad-1 -r yes",
        &[("BEADS_DOLT_SERVER_DATABASE", "arkham")],
    );

    assert!(out.status.success(), "{}", said(&out.stderr));
    assert_eq!(tracker.calls(), [RESPONDED]);
}

/// A project read from its server takes a response the same way, from the
/// directory `bdi` keeps for its database.
#[test]
fn a_response_reaches_a_tracker_on_a_server() {
    let home = std::env::temp_dir().join(format!("bdi-respond-on-a-server-{}", std::process::id()));
    std::fs::create_dir_all(home.join(".config/beady-eye")).expect("the directory is ours to make");
    std::fs::write(
        home.join(".config/beady-eye/config.toml"),
        "[dolt]\nhost = \"db.example.invalid\"\nport = 3306\nuser = \"kadath\"\n\n\
         [[projects]]\nname = \"kadath\"\ndatabase = \"kadath\"\n",
    )
    .expect("the config is ours to write");
    let tracker = ShimmedTracker::beside(&home);
    tracker.answers_for("kadath", RESPONDED, CLOSED);

    let out = bdi(&home, &tracker, "bd kadath human respond kad-1 -r yes");

    assert!(out.status.success(), "{}", said(&out.stderr));
    assert_eq!(said(&out.stdout), CLOSED);
    assert!(home
        .join(".cache/beady-eye/trackers/kadath/.beads/metadata.json")
        .exists());
    std::fs::remove_dir_all(home).expect("the home is ours to remove");
}

/// What bd says and how it exits are the caller's, so a widget can tell a
/// bead already closed from one recorded.
#[test]
fn bd_refusing_the_response_reaches_the_caller_with_its_exit_code() {
    let home = a_home_naming_two_projects("respond-refused-by-bd", "");
    let tracker = ShimmedTracker::beside(&home);

    let out = bdi(&home, &tracker, "bd kadath human respond kad-1 -r yes");

    assert_eq!(out.status.code(), Some(127), "the shim's own exit code");
    assert!(
        said(&out.stderr).contains("which nothing wrote an answer for"),
        "bd's own words reach the caller: {}",
        said(&out.stderr)
    );
}

#[test]
fn any_command_line_but_a_response_is_refused_before_bd_is_reached() {
    let home = a_home_naming_two_projects("respond-refusals", "");
    let tracker = ShimmedTracker::beside(&home);

    for args in [
        "bd innsmouth human respond kad-1 yes",
        "--json bd kadath human respond kad-1 yes",
    ] {
        let out = bdi(&home, &tracker, args);

        assert!(!out.status.success(), "{args:?} was run");
        assert_eq!(tracker.calls(), Vec::<String>::new(), "{args:?} reached bd");
    }
}

/// The credential is the project's own, so a credential command that will not
/// run leaves nothing to write with.
#[test]
fn a_project_whose_credential_command_fails_writes_nothing() {
    let home =
        a_home_naming_two_projects("respond-no-credential", "credential_command = \"false\"\n");
    let tracker = ShimmedTracker::beside(&home);
    tracker.answers_for("kadath", RESPONDED, CLOSED);

    let out = bdi(&home, &tracker, "bd kadath human respond kad-1 -r yes");

    assert!(!out.status.success());
    assert!(
        said(&out.stderr).contains("credential command failed"),
        "{}",
        said(&out.stderr)
    );
    assert_eq!(tracker.calls(), Vec::<String>::new());
}
