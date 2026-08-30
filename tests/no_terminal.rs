//! What `bdi` does when its output is not a terminal. The view is drawn on
//! the alternate screen, so the decision has to be made before anything is
//! written — and it is main's, so the test runs the binary.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A directory holding a config that names one project, so `bdi` gets past
/// config assembly and as far as the decision under test.
fn configured(named: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("bdi-{named}-{}", std::process::id()));
    std::fs::create_dir_all(home.join(".config/beady-eye")).expect("the directory is ours to make");
    std::fs::write(
        home.join(".config/beady-eye/config.toml"),
        format!(
            "[[projects]]\nname = \"atlas\"\npath = \"{}\"\n",
            home.display()
        ),
    )
    .expect("the config is ours to write");
    home
}

fn bdi(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_bdi"))
        .args(args)
        .current_dir(home)
        .env("HOME", home)
        .env_remove("BEADS_DIR")
        .env_remove("COMMY_PROJECT")
        .output()
        .expect("bdi runs")
}

/// A `Command`'s output is a pipe, which is the case under test: the view
/// would be escape codes in whatever the caller redirected into.
#[test]
fn a_view_asked_for_down_a_pipe_says_it_needs_a_terminal() {
    let home = configured("piped");

    let out = bdi(&home, &[]);
    let said = String::from_utf8_lossy(&out.stderr).to_string();

    assert_eq!(out.status.code(), Some(2), "got: {said}");
    assert!(said.contains("terminal"), "got: {said}");
    assert!(said.contains("--json"), "got: {said}");
    assert!(out.stdout.is_empty(), "bdi wrote to a pipe it had refused");
}
