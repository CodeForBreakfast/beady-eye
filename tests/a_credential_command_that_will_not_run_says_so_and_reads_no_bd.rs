//! The credential half of the same ladder: a project whose `credential_command`
//! would not run says so, in its own words, and no `bd` is run for it.
//!
//! It is the sibling of `a_project_bdi_could_not_enter_says_so_and_reads_no_bd`
//! and it exists for the same reason. `tracker_env` has two exits — an
//! environment it could not capture, and a credential command that would not
//! run — and until this landed only the first of them was the project's own.
//! The second rode the run failure's kind into the mapping bd's own read
//! failures use, so a typo in `credential_command` drew *the tracker did not
//! answer* about a tracker nothing had spoken to, and a machine with no `sh`
//! drew *bd could not be started* about a bd that was never reached.
//!
//! The common shape is a helper the reader named that is not on this machine's
//! `PATH` — `op`, `pass`, `secret-tool`. `sh` starts, fails to find it, and
//! exits 127 with stderr matching none of `run.rs`'s phrase lists, so the
//! failure arrives as `Unavailable` for `sh`. That is the fallthrough bucket
//! rather than a kind anybody chose, which is the whole reason the screen must
//! not read a kind here at all.
//!
//! The cases run the binary because what is under test crosses every layer: a
//! real `sh`, a real spawn, and the snapshot and the screen `bdi` writes.

mod terminal;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::ShimmedTracker;
use terminal::{a_home_naming_one_project_settled, rows_of, ENTER_ALTERNATE_SCREEN};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// A gap this long between bytes means the collection is over and drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// What `bd list --all --json` said about this repository's own tracker.
const THE_TRACKER: &str = include_str!("fixtures/bd_list.json");

/// The clause the row is looked for by, read off the screen the stream would
/// have drawn rather than off the stream.
///
/// It cannot be searched for in the bytes: a repaint reaches the wire a word
/// at a time with a cursor move where each space would be, so the phrase as
/// written is never in them and a test reading them would have to settle for
/// one word. `rows_of` puts the screen back together first, which is what
/// lets this name the sentence instead — a single word is satisfied by any
/// screen that happens to hold it, and the two other phrases about a
/// credential are exactly the ones this failure must not be confused with.
///
/// The clause rather than the whole sentence, because the sentence is longer
/// than the row: it is drawn under a tree prefix at 120 columns and the tail
/// of it is cut. This is what fits, and it is the half that says which
/// command.
const THE_CLAUSE: &[u8] = "the credential command this project names would not run".as_bytes();

/// How long a keystroke gets before waiting for it is called stalling.
const LONG_ENOUGH_TO_ANSWER: Duration = Duration::from_secs(10);

/// `l`, which opens whatever the selection is on.
const OPEN_WHAT_IS_SELECTED: &[u8] = b"l";

/// What the published contract calls it.
const NO_CREDENTIAL: &str = "no-credential";

/// A name nothing on any machine holds, so `sh` cannot find it and the case
/// this file is about is the one that arises.
const NO_SUCH_HELPER: &str = "no-such-credential-helper-anywhere";

/// A config naming a credential command, which is the rung of the config this
/// whole file is about.
fn asking_for_a_credential_from(named: &str, command: &str) -> PathBuf {
    a_home_naming_one_project_settled(named, &format!("credential_command = \"{command}\"\n"))
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
                failed["tracker"]
                    .as_str()
                    .expect("and carries why it could not be read"),
            )
        })
        .collect()
}

/// The bead: a `credential_command` naming a helper this machine does not hold
/// leaves the project saying its credential command would not run, on the
/// screen a person is looking at.
///
/// The group holding it rests shut, as it does for every project that would
/// not read, so the sentence is one keystroke down rather than on the first
/// frame. `l` is what opens it, and the selection is already there because a
/// run with no tree to read has no bead row to start on.
#[test]
fn a_project_whose_credential_command_will_not_run_says_so_on_the_screen() {
    let home = asking_for_a_credential_from("no-credential", NO_SUCH_HELPER);
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_TRACKER);
    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &tracker.environment());
    bdi.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);

    bdi.send(OPEN_WHAT_IS_SELECTED);
    bdi.settle(A_SILENCE, LONG_ENOUGH_TO_ANSWER);

    let screen = bdi.everything();
    assert_eq!(
        rows_of(&screen, THE_CLAUSE).len(),
        1,
        "a project whose credential command would not run drew nothing to say \
         so. What bdi wrote: {:?}\n{}",
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
    let home = asking_for_a_credential_from("no-credential-json", NO_SUCH_HELPER);
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_TRACKER);

    let snapshot = snapshot_of(&bdi_json(&home, &tracker.environment()));

    assert_eq!(failed_projects(&snapshot), vec![("atlas", NO_CREDENTIAL)]);
    assert!(
        !tracker.read_the_tracker(),
        "bd was asked something for a project whose credential command would \
         not run, which is the read this exists to stop"
    );

    std::fs::remove_dir_all(&home).expect("the directory is ours to remove");
}

/// The control, and it is the one that makes the two rows above about the
/// credential command rather than about anything else in this config.
///
/// The same home, the same shims, the same absent `.envrc` — and a credential
/// command that answers. The project reads, and bd is asked. So the failure
/// above arrived from the one thing that differs, which is the helper `sh`
/// could not find.
#[test]
fn a_credential_command_that_answers_leaves_the_project_read_as_ever() {
    let home = asking_for_a_credential_from("credential-answers-json", "printf hunter2");
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_TRACKER);

    let snapshot = snapshot_of(&bdi_json(&home, &tracker.environment()));

    assert_eq!(
        failed_projects(&snapshot),
        Vec::new(),
        "a project whose credential command answered failed: {snapshot}"
    );
    assert!(
        tracker.read_the_tracker(),
        "and it was read, which is what makes the rows above about the \
         credential command rather than about a bdi that reads nothing"
    );

    std::fs::remove_dir_all(&home).expect("the directory is ours to remove");
}
