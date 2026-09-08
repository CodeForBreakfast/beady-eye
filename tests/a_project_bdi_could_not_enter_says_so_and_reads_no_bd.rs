//! The last rung of the ladder: a project that asked to be read in a captured
//! environment and did not get one says so, in its own words, and no `bd` is
//! run for it.
//!
//! Both halves are the point. The words are what a reader can act on — it is
//! usually an `.envrc` still wanting `direnv allow` — and every other sentence
//! `bdi` has for a project that would not read sends them to bd, which was
//! never asked anything here.
//!
//! Not running bd is the other half, and it is the one that is not obvious.
//! Falling back to `bdi`'s own environment would read the project's tracker
//! with the bd on `bdi`'s `PATH`, which is not the bd the project asked to be
//! read with — and `docs/design.md`'s *Reading a tracker is not leaving it
//! alone* measured what that costs: bd rewrites `.beads/.local_version` and
//! runs its schema auto-migration on finding itself newer than the bd that
//! last opened a tracker, before the subcommand, whatever the subcommand is,
//! and `--readonly` stops neither. A tracker cannot be put back; a sentence
//! can be read.
//!
//! The cases here run the binary because what is under test crosses every
//! layer, and because two of them turn on a `PATH` — which program the machine
//! holds — and a `PATH` belongs to a process rather than to a call.

mod terminal;

use std::path::Path;
use std::process::{Command, Output};
use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::ShimmedTracker;
use terminal::{
    a_home_naming_one_project, a_home_naming_one_project_settled, contains, ENTER_ALTERNATE_SCREEN,
};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// A gap this long between bytes means the collection is over and drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// What `bd list --all --json` said about this repository's own tracker.
const THE_TRACKER: &str = include_str!("fixtures/bd_list.json");

/// One word of the sentence that nothing else on the screen says.
///
/// One word rather than the sentence, because a repaint reaches the wire a
/// word at a time with a cursor move where each space would be: the phrase as
/// written is never in the bytes, and a test looking for it would fail on a
/// screen that says it.
const A_WORD_OF_IT: &[u8] = "produce".as_bytes();

/// How long a keystroke gets before waiting for it is called stalling.
const LONG_ENOUGH_TO_ANSWER: Duration = Duration::from_secs(10);

/// `l`, which opens whatever the selection is on.
const OPEN_WHAT_IS_SELECTED: &[u8] = b"l";

/// What makes a directory one direnv would enter. Its contents are read by
/// nothing — `bdi` looks for the file — and the shimmed direnv refuses
/// whatever is in it unless a test has said what entering produces, which is
/// the `.envrc` wanting `direnv allow` this strand is about.
fn entered_by_direnv(project: &Path) {
    std::fs::write(project.join(".envrc"), "use flake\n").expect("the .envrc is ours to write");
}

/// `bdi` reading one project and writing the snapshot rather than drawing it.
fn bdi_json(home: &Path, environment: &[(String, String)]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_bdi"))
        .arg("--json")
        .current_dir(home)
        .env("HOME", home)
        .env_remove("BEADS_DIR")
        .env_remove("BDI_PROJECT")
        .envs(environment.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .output()
        .expect("bdi runs")
}

/// The snapshot `bdi` wrote, and what it said on the way if it wrote none.
fn snapshot_of(out: &Output) -> serde_json::Value {
    assert!(
        out.status.success(),
        "bdi exited {}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap_or_else(|why| {
        panic!(
            "bdi wrote something that is not a snapshot ({why}): {}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

/// How each failed project is reported, read as fields rather than searched
/// for as text: `bdi` writes the snapshot indented, so a compact spelling of
/// a pair matches nothing however right it is.
fn failed_projects(snapshot: &serde_json::Value) -> Vec<(&str, &str)> {
    snapshot["failed_projects"]
        .as_array()
        .expect("the snapshot reports the projects that could not be read")
        .iter()
        .map(|failed| {
            (
                failed["project"].as_str().expect("each one is named"),
                failed["tracker"]["reason"]
                    .as_str()
                    .expect("and carries why it could not be read"),
            )
        })
        .collect()
}

/// What the published contract calls it.
const NO_ENVIRONMENT: &str = "no-environment";

/// The bead: an `.envrc` the reader has not allowed leaves the project saying
/// it asked for an environment `bdi` could not produce, on the screen a person
/// is looking at.
///
/// The group holding it rests shut, as it does for every project that would
/// not read, so the sentence is one keystroke down rather than on the first
/// frame. `l` is what opens it, and the selection is already there because a
/// run with no tree to read has no bead row to start on.
#[test]
fn a_project_whose_envrc_will_not_run_says_so_on_the_screen() {
    let home = a_home_naming_one_project("unentered");
    entered_by_direnv(&home);
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_TRACKER);
    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &tracker.environment());
    bdi.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);

    assert_eq!(
        tracker.direnv_runs(),
        vec!["exec . env -0".to_string()],
        "the directory was never entered, so the case this is about did not \
         arise and whatever the screen says it does not say it about this"
    );

    bdi.send(OPEN_WHAT_IS_SELECTED);
    bdi.settle(A_SILENCE, LONG_ENOUGH_TO_ANSWER);

    let screen = bdi.everything();
    assert!(
        contains(&screen, A_WORD_OF_IT),
        "a project bdi could not enter drew nothing to say so. What bdi \
         wrote: {:?}\n{}",
        String::from_utf8_lossy(&screen),
        bdi.timeline()
    );
}

/// The same project in the published snapshot, reported as its own failure
/// rather than as anything about bd — and with no bd asked anything.
///
/// The absence is read off the shim's own record. A shimmed `bd` that is never
/// called leaves the project drawing nothing either way, so nothing about the
/// snapshot could tell *bd was not asked* from *bd answered with nothing*.
#[test]
fn the_snapshot_reports_it_as_its_own_failure_and_no_bd_was_asked() {
    let home = a_home_naming_one_project("unentered-json");
    entered_by_direnv(&home);
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_TRACKER);

    let snapshot = snapshot_of(&bdi_json(&home, &tracker.environment()));

    assert_eq!(failed_projects(&snapshot), vec![("atlas", NO_ENVIRONMENT)]);
    assert!(
        !tracker.read_the_tracker(),
        "bd was asked something for a project bdi could not enter, which is \
         the read this exists to stop"
    );

    std::fs::remove_dir_all(&home).expect("the directory is ours to remove");
}

/// A project that asked for nothing is read as ever. This is the common case
/// — a machine with bd and nothing else — and it must not be caught by any of
/// this.
#[test]
fn a_project_that_asked_for_no_environment_is_read_as_ever() {
    let home = a_home_naming_one_project("ambient-json");
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_TRACKER);

    let snapshot = snapshot_of(&bdi_json(&home, &tracker.environment()));

    assert_eq!(
        tracker.direnv_runs(),
        Vec::<String>::new(),
        "a directory with no .envrc was entered, so what this reads is not \
         the ambient project it means"
    );
    assert_eq!(
        failed_projects(&snapshot),
        Vec::new(),
        "a project that asked for nothing failed: {snapshot}"
    );
    assert!(
        tracker.read_the_tracker(),
        "and it was read, which is what makes the row above about the \
         environment rather than about a bdi that reads nothing"
    );

    std::fs::remove_dir_all(&home).expect("the directory is ours to remove");
}

/// The other way a project asks: a wrapper its config names, which is the rung
/// above detection. Both arrive at the same failure, because what the reader
/// does next is look at whichever of the two they wrote.
///
/// This is the case the hazard is sharpest in. A config naming `nix develop
/// -c` is a project pinned to its own bd, and reading it with the ambient one
/// is exactly what the version gate in `docs/design.md` was declined on the
/// impossibility of.
#[test]
fn a_configured_wrapper_that_will_not_run_reaches_the_same_failure() {
    let home = a_home_naming_one_project_settled(
        "unrunnable-wrapper",
        "environment_command = \"no-such-wrapper-anywhere exec .\"\n",
    );
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_TRACKER);

    let snapshot = snapshot_of(&bdi_json(&home, &tracker.environment()));

    assert_eq!(failed_projects(&snapshot), vec![("atlas", NO_ENVIRONMENT)]);
    assert!(
        !tracker.read_the_tracker(),
        "bd was asked something for a project whose configured wrapper is \
         not installed"
    );

    std::fs::remove_dir_all(&home).expect("the directory is ours to remove");
}
