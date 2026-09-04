//! Detection: where `bdi` can see how a project's directory would be entered
//! and the machine can enter it, that is how the project's tracker is read,
//! with nothing in the config saying so.
//!
//! It is the rung below `environment_command`, and it is what makes the
//! common case cost a reader nothing. The mechanism detected is direnv,
//! because an `.envrc` is a file `bdi` can see and the others are not: nix
//! and mise are entered by a command a person types, and nothing in a
//! directory says which. Those stay named in config, which is the rung above.
//!
//! Every case runs the binary, because what is under test is a `PATH` — which
//! program the machine holds — and a `PATH` belongs to a process rather than
//! to a call. One run cannot be a machine with direnv and a machine without
//! one, so a unit test cannot ask this question at all.

mod terminal;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use terminal::shims::{shims_first_with_nothing_called, ShimmedTracker};

/// A directory of this test's own, named for the case that wants it, so the
/// cases can run at once without treading on each other's trackers.
fn a_project(named: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("bdi-detect-{named}-{}", std::process::id()));
    std::fs::create_dir_all(&path).expect("the directory is ours to make");
    path
}

/// What makes a directory one direnv would enter. Its contents are read by
/// nothing: `bdi` looks for the file, and the shim answers from what the test
/// wrote down rather than evaluating anything.
fn entered_by_direnv(project: &Path) {
    std::fs::write(project.join(".envrc"), "use flake\n").expect("the .envrc is ours to write");
}

/// `bdi` as someone with no config runs it, in a directory bd tracks.
fn bdi(cwd: &Path, environment: &[(String, String)]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_bdi"))
        .arg("--json")
        .current_dir(cwd)
        .env("HOME", cwd)
        .env_remove("BEADS_DIR")
        .env_remove("BDI_PROJECT")
        .envs(environment.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .output()
        .expect("bdi runs")
}

/// What `bd list --all --json` said about this repository's own tracker: the
/// capture the shimmed `bd` serves, and here the one the *launching shell*
/// reaches.
const THE_LAUNCHING_SHELLS: &str = include_str!("fixtures/bd_list.json");

/// The one root in that capture.
const THE_LAUNCHING_SHELLS_ROOT: &str = "bdi-2bb";

/// The same capture under another prefix, standing for the tracker a
/// project's own directory reaches. Every field but the ids is the wire shape
/// the fixture captured, because that is what it is.
const ANOTHER_PREFIX: &str = "own-";
const THE_PROJECTS_OWN_ROOT: &str = "own-2bb";

fn the_projects_own() -> String {
    THE_LAUNCHING_SHELLS.replace("bdi-", ANOTHER_PREFIX)
}

/// The bead: a project whose directory holds an `.envrc`, on a machine that
/// has direnv, is read by entering it — with nothing in the config.
///
/// What says the environment was *used* rather than merely captured is which
/// tracker answers. The shimmed `bd` reads its answers from a directory named
/// in its environment, so an entered directory naming a different one is a
/// project read with the `bd` its own directory yields rather than the one
/// the launching shell holds. That is what this strand is for, and the
/// project's own beads coming back is the reading of it.
#[test]
fn a_project_whose_directory_direnv_would_enter_is_read_by_entering_it() {
    let cwd = a_project("entered");
    entered_by_direnv(&cwd);
    let launching_shell = ShimmedTracker::beside(&cwd);
    launching_shell.tracks(&cwd);
    launching_shell.holds(THE_LAUNCHING_SHELLS);

    let inside = cwd.join("what-entering-produces");
    std::fs::create_dir_all(&inside).expect("the directory is ours to make");
    let projects_own = ShimmedTracker::beside(&inside);
    projects_own.holds(&the_projects_own());
    launching_shell.enters_with(&[(
        "BDI_SHIM_BD_ANSWERS",
        &inside.join("bd-answers").display().to_string(),
    )]);

    let out = bdi(&cwd, &launching_shell.environment());
    let snapshot = String::from_utf8_lossy(&out.stdout).to_string();
    let said = String::from_utf8_lossy(&out.stderr).to_string();

    assert!(out.status.success(), "bdi exited {}: {said}", out.status);
    assert_eq!(
        launching_shell.direnv_runs(),
        vec!["exec . env -0".to_string()],
        "a directory holding an .envrc was not entered, on a machine whose \
         PATH holds a direnv"
    );
    assert!(
        snapshot.contains(&format!("\"{THE_PROJECTS_OWN_ROOT}\"")),
        "the tracker the entered directory reaches was not the one read: {snapshot}"
    );
    assert!(
        !snapshot.contains(&format!("\"{THE_LAUNCHING_SHELLS_ROOT}\"")),
        "the launching shell's tracker was read for a project whose own \
         directory names another: {snapshot}"
    );

    std::fs::remove_dir_all(&cwd).expect("the directory is ours to remove");
}

/// A machine with no direnv reads the project ambient, and runs nothing to
/// find that out.
///
/// Whatever stops `bdi` entering a directory stops a person entering it too —
/// their own shell there gets the ambient `bd` as well — so this is the rule
/// of thumb's answer rather than a fallback that has given something up.
#[test]
fn a_machine_with_no_direnv_reads_the_project_ambient() {
    let cwd = a_project("no-direnv");
    entered_by_direnv(&cwd);
    let tracker = ShimmedTracker::beside(&cwd);
    tracker.tracks(&cwd);
    tracker.holds(THE_LAUNCHING_SHELLS);
    let mut environment = tracker.environment();
    environment[0] = shims_first_with_nothing_called("direnv", &cwd);

    let out = bdi(&cwd, &environment);
    let snapshot = String::from_utf8_lossy(&out.stdout).to_string();
    let said = String::from_utf8_lossy(&out.stderr).to_string();

    assert!(out.status.success(), "bdi exited {}: {said}", out.status);
    assert_eq!(
        tracker.direnv_runs(),
        Vec::<String>::new(),
        "a direnv was run on a machine that has none"
    );
    assert!(
        snapshot.contains(&format!("\"{THE_LAUNCHING_SHELLS_ROOT}\"")),
        "a project with an .envrc was not read at all on a machine with no \
         direnv: {snapshot}"
    );

    std::fs::remove_dir_all(&cwd).expect("the directory is ours to remove");
}

/// A config that names an `environment_command` is obeyed, and the detection
/// underneath it never runs. The reader said how this project is entered, and
/// a guess that overrode them would make the setting unable to express the
/// case it exists for.
///
/// The two spellings are told apart by the argv the shim writes down, which
/// is what keeps this from passing on a machine where detection could not
/// have fired anyway: detection asks for `exec .`, and this config asks for
/// somewhere else, so the recorded line says which of them was obeyed rather
/// than only that direnv ran once.
#[test]
fn a_configured_environment_command_wins_over_what_the_directory_implies() {
    let cwd = a_project("configured");
    entered_by_direnv(&cwd);
    let tracker = ShimmedTracker::beside(&cwd);
    tracker.holds(THE_LAUNCHING_SHELLS);
    tracker.enters_with(&[]);
    let asked_for = cwd.join("what-the-config-named");
    configured(
        &cwd,
        &format!(
            "[[projects]]\nname = \"atlas\"\npath = \"{}\"\nenvironment_command = \"direnv exec {}\"\n",
            cwd.display(),
            asked_for.display()
        ),
    );

    let out = bdi(&cwd, &tracker.environment());
    let said = String::from_utf8_lossy(&out.stderr).to_string();

    assert!(out.status.success(), "bdi exited {}: {said}", out.status);
    assert_eq!(
        tracker.direnv_runs(),
        vec![format!("exec {} env -0", asked_for.display())],
        "the directory the config named was not the one entered"
    );

    std::fs::remove_dir_all(&cwd).expect("the directory is ours to remove");
}

/// The search that decides whether to enter a directory is made from that
/// directory, because the spawn it decides on is.
///
/// POSIX gives an empty `PATH` entry the working directory, and the working
/// directory of the child that runs the wrapper is the project's own — so a
/// `direnv` a project supplies for itself is one `bdi` would successfully
/// run, and a search made from wherever `bdi` happens to have been started
/// answers about a different machine. The two have to agree, because one of
/// them decides whether the other happens at all.
///
/// `bdi` is run somewhere else entirely, which is the whole of what makes
/// this readable: run from inside the project, both searches look in the same
/// place and the question cannot be asked.
#[test]
fn a_direnv_the_project_supplies_is_found_from_the_projects_own_directory() {
    let elsewhere = a_project("elsewhere");
    let project = elsewhere.join("the-project");
    std::fs::create_dir_all(&project).expect("the directory is ours to make");
    entered_by_direnv(&project);
    std::os::unix::fs::symlink(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("shims")
            .join("direnv"),
        project.join("direnv"),
    )
    .expect("the link is ours to make");

    let tracker = ShimmedTracker::beside(&elsewhere);
    tracker.holds(THE_LAUNCHING_SHELLS);
    tracker.enters_with(&[]);
    configured(
        &elsewhere,
        &format!(
            "[[projects]]\nname = \"atlas\"\npath = \"{}\"\n",
            project.display()
        ),
    );

    let mut environment = tracker.environment();
    let (_, without_direnv) = shims_first_with_nothing_called("direnv", &elsewhere);
    environment[0] = ("PATH".to_string(), format!(":{without_direnv}"));

    let out = bdi(&elsewhere, &environment);
    let said = String::from_utf8_lossy(&out.stderr).to_string();

    assert!(out.status.success(), "bdi exited {}: {said}", out.status);
    assert_eq!(
        tracker.direnv_runs(),
        vec!["exec . env -0".to_string()],
        "a direnv the project's own directory supplies was not found, because \
         the search was made from wherever bdi was started rather than from \
         the directory the wrapper would have run in"
    );

    std::fs::remove_dir_all(&elsewhere).expect("the directory is ours to remove");
}

/// A config under the `HOME` the run is given, which is where `bdi` looks for
/// one.
fn configured(home: &Path, toml: &str) {
    let at = home.join(".config").join("beady-eye");
    std::fs::create_dir_all(&at).expect("the directory is ours to make");
    std::fs::write(at.join("config.toml"), toml).expect("the config is ours to write");
}

/// The other half: a directory with no `.envrc` runs no direnv, on a machine
/// that has one and would have answered.
///
/// The absence is read off the file the shim writes rather than off a
/// duration. `direnv exec` on a directory with no `.envrc` is a 3 to 5ms
/// pass-through, so a timing assertion passes whether or not the call was
/// made, and would report the skip it was written to catch either way.
#[test]
fn a_directory_with_no_envrc_runs_no_direnv_on_a_machine_that_has_one() {
    let cwd = a_project("no-envrc");
    let tracker = ShimmedTracker::beside(&cwd);
    tracker.tracks(&cwd);
    tracker.holds(THE_LAUNCHING_SHELLS);
    tracker.enters_with(&[("BDI_SHIM_BD_ANSWERS", "/nowhere-a-tracker-answers-from")]);

    let out = bdi(&cwd, &tracker.environment());
    let snapshot = String::from_utf8_lossy(&out.stdout).to_string();
    let said = String::from_utf8_lossy(&out.stderr).to_string();

    assert!(out.status.success(), "bdi exited {}: {said}", out.status);
    assert_eq!(
        tracker.direnv_runs(),
        Vec::<String>::new(),
        "a directory with no .envrc was handed to direnv anyway"
    );
    assert!(
        snapshot.contains(&format!("\"{THE_LAUNCHING_SHELLS_ROOT}\"")),
        "the project was not read in the ambient environment: {snapshot}"
    );

    std::fs::remove_dir_all(&cwd).expect("the directory is ours to remove");
}
