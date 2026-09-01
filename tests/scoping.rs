//! Which of the configured projects a run reads, when the command line asks
//! for fewer.
//!
//! This runs the binary because the wiring is main's and nothing below it can
//! see whether the wiring is there. Measured on this change: deleting the
//! `scoped_to` call from `cli::run` and leaving everything else alone left
//! every library test green, because each of them hands `scoped_to` a config
//! itself. A scope that never reaches the config is the regression this
//! feature would actually suffer, and no test below main can fail on it.
//!
//! `--json` rather than a pty: the question is which projects were read, and
//! the snapshot answers it in a form a test can read off a pipe.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Both projects point at the directory holding the config, which holds no
/// tracker — so each one bdi reads fails to read, and says so under its own
/// name. That failure is the observation: a project bdi did not read is not
/// in the snapshot at all, and a project it did read is there whether or not
/// its tracker answered.
///
/// `credential_command` is what keeps direnv out of it. Without one the first
/// thing a collection does is run direnv, and on a machine without direnv the
/// project fails there instead — the same shape of answer for a different
/// reason, which would leave this test passing on a machine where nothing
/// worked.
const TWO_PROJECTS: &str = "\
[[projects]]
name = \"atlas\"
path = \"{}\"
credential_command = \"printf ''\"

[[projects]]
name = \"beacon\"
path = \"{}\"
credential_command = \"printf ''\"
";

/// A config naming two projects, written where a test can hand bdi its path.
fn a_config_naming_two_projects(named: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("bdi-{named}-{}", std::process::id()));
    std::fs::create_dir_all(&home).expect("the directory is ours to make");
    let path = home.join("config.toml");
    std::fs::write(
        &path,
        TWO_PROJECTS.replace("{}", &home.display().to_string()),
    )
    .expect("the config is ours to write");
    path
}

fn bdi(config: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_bdi"))
        .arg("--config")
        .arg(config)
        .args(args)
        .output()
        .expect("bdi runs")
}

/// How the snapshot names a project it read, whether or not the tracker
/// answered. Written as the JSON spells it, so a project that is merely
/// mentioned somewhere in a pane's path is not mistaken for one bdi read.
fn named_in(snapshot: &str, project: &str) -> bool {
    snapshot.contains(&format!("\"project\": \"{project}\""))
}

/// The whole of the feature, end to end, with the unscoped run as its
/// control.
///
/// The control is what makes the second half mean anything: without it, a bdi
/// that had never read `beacon` for some unrelated reason would pass just as
/// well as one that scoped it out.
#[test]
fn a_scope_leaves_the_projects_it_does_not_name_out_of_the_run() {
    let config = a_config_naming_two_projects("scoping");

    let unscoped = bdi(&config, &["--json"]);
    let whole = String::from_utf8_lossy(&unscoped.stdout).to_string();

    assert!(unscoped.status.success(), "bdi exited {}", unscoped.status);
    assert!(named_in(&whole, "atlas"), "got: {whole}");
    assert!(
        named_in(&whole, "beacon"),
        "asking for no particular project is not asking for none: bdi reads \
         every project the config names; got: {whole}"
    );

    let scoped = String::from_utf8_lossy(&bdi(&config, &["--project", "atlas", "--json"]).stdout)
        .to_string();

    assert!(named_in(&scoped, "atlas"), "got: {scoped}");
    assert!(
        !named_in(&scoped, "beacon"),
        "beacon was not asked for, so nothing should have gone and read it; got: {scoped}"
    );
}

/// A scope that selected nothing starts bdi on an empty forest the reader
/// cannot tell from a quiet one, so it is refused instead — and the refusal
/// says what there was to choose from, because the likely cause is a typo.
#[test]
fn a_scope_naming_no_configured_project_stops_bdi_and_says_what_it_knows() {
    let config = a_config_naming_two_projects("scoping-unknown");

    let out = bdi(&config, &["--project", "cinder", "--json"]);
    let said = String::from_utf8_lossy(&out.stderr).to_string();

    assert!(!out.status.success(), "bdi exited {}: {said}", out.status);
    assert!(said.contains("cinder"), "got: {said}");
    assert!(said.contains("atlas"), "got: {said}");
    assert!(said.contains("beacon"), "got: {said}");
    assert!(
        out.stdout.is_empty(),
        "a scoped-out snapshot was emitted anyway"
    );
}
