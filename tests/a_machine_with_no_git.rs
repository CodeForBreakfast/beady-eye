//! What a reader whose machine cannot run git is told about the name their
//! project is drawn under.
//!
//! The name is half of every key `bdi` holds, and without git it is the
//! directory's rather than the repository's.

mod terminal;

use std::path::{Path, PathBuf};

use terminal::a_socket_of_its_own;
use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::{
    shims_first_with_nothing_called, ShimmedHerdr, ShimmedTracker, A_PANE, A_SESSION,
};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// What `bd list --all --json` said about this project's own tracker.
const THE_TRACKER: &str = include_str!("fixtures/bd_list.json");

/// The remedy the foot notice names, and the whole of what a reader can act
/// on. Nothing else `bdi` writes says it, on either stream.
const THE_REMEDY: &[u8] = "BDI_PROJECT".as_bytes();

/// A `HOME` with no config in it, so `bdi` discovers its one project instead
/// of being told what to draw. That is the only path the notice is on: a
/// config file names its own projects, and discovery never runs.
fn a_home_with_no_config(named: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("bdi-{named}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("the directory is ours to make");
    home
}

/// A tracker at the top of `home`, and a herdr with a seat working in it.
///
/// The seat is what puts the tree on the screen: `bdi` draws the trees with a
/// live agent unless it is asked for all of them, and nothing here can ask.
/// A herdr that answers is also what keeps the foot to one notice, since a
/// provider that will not answer puts its own beside the one under test.
fn a_run_in(home: &Path) -> (Vec<(String, String)>, ShimmedTracker, ShimmedHerdr) {
    let tracker = ShimmedTracker::beside(home);
    tracker.tracks(home);
    tracker.holds(THE_TRACKER);

    let herdr = ShimmedHerdr::beside(home);
    herdr.holds_in(
        A_SESSION,
        &format!(
            r#"{{"result":{{"agents":[{{"pane_id":"{A_PANE}","cwd":"{}","agent_status":"working"}}]}}}}"#,
            home.display()
        ),
    );

    let mut environment = tracker.environment();
    environment.extend(herdr.environment());
    environment.push(a_socket_of_its_own(home));
    (environment, tracker, herdr)
}

/// Every directory on `PATH` that holds a git dropped, and the shims put
/// where they can still be found — so the answer is the same on a machine
/// that has git and one that never did.
fn with_no_git(environment: &mut Vec<(String, String)>, home: &Path) {
    environment.retain(|(key, _)| key != "PATH");
    environment.push(shims_first_with_nothing_called("git", home));
}

/// The name is on the first frame, so the qualification is too.
///
/// `bdi` draws its forest from the configured project names before any
/// tracker has answered, so a reader is looking at a guessed name for as long
/// as the first collection takes. A screen that waited for that collection to
/// say the name was guessed would show them the wrong thing first and correct
/// it, which is the one order a warning must not arrive in.
///
/// The collection is held rather than raced: `bd list` cannot return, so
/// every frame drawn here is a frame drawn before it. `bd where` is left
/// answering because discovery asks it before there is a screen at all, and a
/// run with no config file is the only kind this notice fires on.
///
/// Read rather than settled for: the mark beside a project being collected
/// turns, so a terminal with a collection outstanding never falls silent.
#[test]
fn the_guess_is_on_the_screen_before_the_first_collection_comes_back() {
    let home = a_home_with_no_config("first-frame");
    let (mut environment, tracker, _herdr) = a_run_in(&home);
    with_no_git(&mut environment, &home);
    tracker.hang_on(COLLECTING_THE_ROWS);

    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &environment);
    tracker.wait_until_holding(GIVING_UP);

    // The project's own line is drawn from the discovered name rather than
    // from anything a tracker said, so it is on the screen with the
    // collection still outstanding — and that name is the guess itself.
    let named = home
        .file_name()
        .expect("the directory has a name")
        .to_string_lossy()
        .to_string();
    bdi.read_until(named.as_bytes(), GIVING_UP);
    bdi.read_until(THE_REMEDY, GIVING_UP);
}

/// The call a collection opens each project's read with, as `collect::bd`
/// asks it. Written out rather than reached for, so a change to that call
/// shows up here as a hang that never starts.
const COLLECTING_THE_ROWS: &str = "list --all";
