//! What `bdi` does about its config before it reads anything. Both cases run
//! the binary itself, because the decision is main's and nothing else sees it.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A directory in no repository, with no beads workspace and no config under
/// the `HOME` the child is given — so a fallback there has nothing to find.
fn nowhere(named: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("bdi-{named}-{}", std::process::id()));
    std::fs::create_dir_all(&path).expect("the directory is ours to make");
    path
}

/// `bdi` as someone on a fresh machine runs it: no `BEADS_DIR` pointing at a
/// tracker, no `COMMY_PROJECT` naming a project, and a `HOME` with no config.
fn bdi(cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_bdi"))
        .args(args)
        .current_dir(cwd)
        .env("HOME", cwd)
        .env_remove("BEADS_DIR")
        .env_remove("COMMY_PROJECT")
        .output()
        .expect("bdi runs")
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
