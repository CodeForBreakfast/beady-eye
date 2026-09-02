//! What `bdi` does about its config before it reads anything. Every case runs
//! the binary itself, because the decision is main's and nothing else sees it.

mod terminal;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use terminal::shims::ShimmedTracker;

/// A directory in no repository, with no beads workspace and no config under
/// the `HOME` the child is given — so a fallback there has nothing to find.
fn nowhere(named: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("bdi-{named}-{}", std::process::id()));
    std::fs::create_dir_all(&path).expect("the directory is ours to make");
    path
}

/// `bdi` as someone on a fresh machine runs it: no `BEADS_DIR` pointing at a
/// tracker, no `BDI_PROJECT` naming a project, and a `HOME` with no config.
fn bdi(cwd: &Path, args: &[&str]) -> Output {
    bdi_in(cwd, args, &[])
}

/// The same, run with `environment` set — the shims' variables, for a run
/// that is to find a tracker.
fn bdi_in(cwd: &Path, args: &[&str], environment: &[(String, String)]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_bdi"))
        .args(args)
        .current_dir(cwd)
        .env("HOME", cwd)
        .env_remove("BEADS_DIR")
        .env_remove("BDI_PROJECT")
        .envs(environment.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .output()
        .expect("bdi runs")
}

/// What `bd list --all --json` said about this project's own tracker: the
/// capture the shimmed `bd` serves.
const THE_TRACKER: &str = include_str!("fixtures/bd_list.json");

/// The one root in that capture.
const THE_ROOT: &str = "bdi-2bb";

/// The first run a new user makes: bd on PATH, no config, no direnv, in a
/// directory bd tracks. The project is read in `bdi`'s own environment and
/// its rows come back; direnv is never asked for, on this machine or on one
/// without it.
#[test]
fn a_directory_bd_tracks_is_read_with_no_config_and_no_direnv() {
    let cwd = nowhere("tracked");
    let tracker = ShimmedTracker::beside(&cwd);
    tracker.tracks(&cwd);
    tracker.holds(THE_TRACKER);

    let out = bdi_in(&cwd, &["--json"], &tracker.environment());
    let snapshot = String::from_utf8_lossy(&out.stdout).to_string();
    let said = String::from_utf8_lossy(&out.stderr).to_string();

    assert!(out.status.success(), "bdi exited {}: {said}", out.status);
    assert_eq!(
        tracker.direnv_runs(),
        Vec::<String>::new(),
        "the zero-config run entered the directory with direnv, which a \
         machine with bd and nothing else does not have"
    );
    let project = cwd
        .file_name()
        .expect("the directory has a name")
        .to_string_lossy();
    assert!(
        snapshot.contains(&format!("\"project\": \"{project}\"")),
        "the directory bd tracks was not read as a project: {snapshot}"
    );
    assert!(
        snapshot.contains(&format!("\"{THE_ROOT}\"")),
        "the tracker's root was not read: {snapshot}"
    );

    std::fs::remove_dir_all(&cwd).expect("the directory is ours to remove");
}

/// `BDI_PROJECT` is bdi's own name for the project it finds with no config,
/// and outranks the directory's. It is the one variable read: a name another
/// tool keeps in a variable of its own reaches bdi only by being exported
/// under this one too.
#[test]
fn bdi_project_names_the_project_read_with_no_config() {
    let cwd = nowhere("named-in-the-environment");
    let tracker = ShimmedTracker::beside(&cwd);
    tracker.tracks(&cwd);
    tracker.holds(THE_TRACKER);
    let mut environment = tracker.environment();
    environment.push(("BDI_PROJECT".to_string(), "orbital".to_string()));

    let out = bdi_in(&cwd, &["--json"], &environment);
    let snapshot = String::from_utf8_lossy(&out.stdout).to_string();
    let said = String::from_utf8_lossy(&out.stderr).to_string();

    assert!(out.status.success(), "bdi exited {}: {said}", out.status);
    let directory = cwd
        .file_name()
        .expect("the directory has a name")
        .to_string_lossy();
    assert!(
        snapshot.contains("\"project\": \"orbital\""),
        "the project is not called what BDI_PROJECT says: {snapshot}"
    );
    assert!(
        !snapshot.contains(&format!("\"project\": \"{directory}\"")),
        "the directory's name was used with BDI_PROJECT set: {snapshot}"
    );

    std::fs::remove_dir_all(&cwd).expect("the directory is ours to remove");
}

/// The name is read from bdi's own variable and no other tool's: commy's
/// variable for the same thing, set with `BDI_PROJECT` unset, leaves the
/// project called after its directory. A shell that wants both tools to
/// agree exports both names.
#[test]
fn another_tools_project_variable_is_not_read() {
    let cwd = nowhere("named-for-another-tool");
    let tracker = ShimmedTracker::beside(&cwd);
    tracker.tracks(&cwd);
    tracker.holds(THE_TRACKER);
    let mut environment = tracker.environment();
    environment.push(("COMMY_PROJECT".to_string(), "orbital".to_string()));

    let out = bdi_in(&cwd, &["--json"], &environment);
    let snapshot = String::from_utf8_lossy(&out.stdout).to_string();
    let said = String::from_utf8_lossy(&out.stderr).to_string();

    assert!(out.status.success(), "bdi exited {}: {said}", out.status);
    let directory = cwd
        .file_name()
        .expect("the directory has a name")
        .to_string_lossy();
    assert!(
        snapshot.contains(&format!("\"project\": \"{directory}\"")),
        "the project is not called after its directory: {snapshot}"
    );
    assert!(
        !snapshot.contains("\"project\": \"orbital\""),
        "another tool's variable named the project: {snapshot}"
    );

    std::fs::remove_dir_all(&cwd).expect("the directory is ours to remove");
}

#[test]
fn a_directory_with_no_tracker_and_no_config_says_so() {
    let cwd = nowhere("untracked");

    let out = bdi(&cwd, &["--json"]);
    let said = String::from_utf8_lossy(&out.stderr).to_string();

    assert!(!out.status.success(), "bdi exited {}: {said}", out.status);
    assert!(said.contains(&cwd.display().to_string()), "got: {said}");
    assert!(said.contains("beads tracks"), "got: {said}");
    assert!(
        said.contains(".config/beady-eye/config.toml"),
        "the config it looked for is not named: {said}"
    );
    assert!(
        out.stdout.is_empty(),
        "an empty snapshot was emitted anyway"
    );

    std::fs::remove_dir_all(&cwd).expect("the directory is ours to remove");
}

#[test]
fn a_config_the_user_named_and_has_not_written_is_an_error() {
    let cwd = nowhere("named-config");
    let absent = cwd.join("beady-eye.toml");

    let out = bdi(&cwd, &["--config", &absent.display().to_string(), "--json"]);
    let said = String::from_utf8_lossy(&out.stderr).to_string();

    assert!(!out.status.success(), "bdi exited {}: {said}", out.status);
    assert!(said.contains(&absent.display().to_string()), "got: {said}");
    assert!(
        !said.contains("beads tracks"),
        "a path the user named fell back to the current directory: {said}"
    );

    std::fs::remove_dir_all(&cwd).expect("the directory is ours to remove");
}
