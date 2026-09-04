//! What a reader whose machine cannot run git is told about the name their
//! project is drawn under.
//!
//! The name is half of every key `bdi` holds, and without git it is the
//! directory's rather than the repository's. All three states are here,
//! because the bead is that they draw the same forest and only one of them
//! says anything: a machine with no git has been guessed at, a repository
//! with no `origin` has not, and a reader who set `BDI_PROJECT` said the name
//! outright.

mod terminal;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::{
    shims_first_with_nothing_called, ShimmedHerdr, ShimmedTracker, A_PANE, A_SESSION,
};
use terminal::{a_socket_of_its_own, contains, ENTER_ALTERNATE_SCREEN};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// A gap this long between bytes means the collection is over and drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// What `bd list --all --json` said about this project's own tracker.
const THE_TRACKER: &str = include_str!("fixtures/bd_list.json");

/// The one root in that capture, drawn because the pane below is working in
/// the project. Asserted alongside every absence here, so a screen that drew
/// nothing at all cannot pass for one that drew a forest and said nothing.
const THE_ROOT: &[u8] = "bdi-2bb".as_bytes();

/// The remedy the foot notice names, and the whole of what a reader can act
/// on. Nothing else `bdi` writes says it, on either stream, so its absence is
/// the notice's absence.
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

/// Everything the run put on the terminal, once it has stopped writing.
fn drawn_by(bdi: &mut Driven) -> Vec<u8> {
    bdi.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);
    bdi.everything()
}

/// Every directory on `PATH` that holds a git dropped, and the shims put
/// where they can still be found — so the answer is the same on a machine
/// that has git and one that never did.
fn with_no_git(environment: &mut Vec<(String, String)>, home: &Path) {
    environment.retain(|(key, _)| key != "PATH");
    environment.push(shims_first_with_nothing_called("git", home));
}

/// The bead: a machine that cannot run git names the project after its
/// directory, and says so at the foot with the one line that settles it.
#[test]
fn a_run_that_cannot_run_git_is_told_the_name_is_a_guess() {
    let home = a_home_with_no_config("no-git-guessed");
    let (mut environment, _tracker, _herdr) = a_run_in(&home);
    with_no_git(&mut environment, &home);

    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &environment);
    let drawn = drawn_by(&mut bdi);

    assert!(
        contains(&drawn, THE_ROOT),
        "the tracker's root was not drawn: {}",
        bdi.timeline()
    );
    assert!(
        contains(&drawn, THE_REMEDY),
        "nothing said the project's name was guessed, or how to settle it: {}",
        bdi.timeline()
    );

    std::fs::remove_dir_all(&home).expect("the directory is ours to remove");
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

    std::fs::remove_dir_all(&home).expect("the directory is ours to remove");
}

/// The call a collection opens each project's read with, as `collect::bd`
/// asks it. Written out rather than reached for, so a change to that call
/// shows up here as a hang that never starts.
const COLLECTING_THE_ROWS: &str = "list --all";

/// The state this must not fire on. A repository with no `origin` is the
/// documented answer and the reader can see it for themselves; a notice here
/// is the warning they learn to ignore, and it would take the one above with
/// it.
#[test]
fn a_repository_with_no_origin_is_not_warned() {
    let home = a_home_with_no_config("no-origin");
    git_in(&home, &["init"]);
    let (environment, _tracker, _herdr) = a_run_in(&home);

    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &environment);
    let drawn = drawn_by(&mut bdi);

    assert!(
        contains(&drawn, THE_ROOT),
        "the tracker's root was not drawn: {}",
        bdi.timeline()
    );
    assert!(
        !contains(&drawn, THE_REMEDY),
        "a repository with no origin was warned about its name: {}",
        bdi.timeline()
    );

    std::fs::remove_dir_all(&home).expect("the directory is ours to remove");
}

/// The other state that is not a guess: the reader named the project
/// outright, and `BDI_PROJECT` outranks whatever git would have said. There
/// is nothing to tell them, and nothing to tell them to do.
#[test]
fn a_project_the_environment_names_is_not_warned_with_no_git_either() {
    let home = a_home_with_no_config("no-git-named");
    let (mut environment, _tracker, _herdr) = a_run_in(&home);
    with_no_git(&mut environment, &home);
    environment.push(("BDI_PROJECT".to_string(), "orbital".to_string()));

    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &environment);
    let drawn = drawn_by(&mut bdi);

    assert!(
        contains(&drawn, THE_ROOT),
        "the tracker's root was not drawn: {}",
        bdi.timeline()
    );
    assert!(
        !contains(&drawn, THE_REMEDY),
        "a project the reader named was warned about its name: {}",
        bdi.timeline()
    );

    std::fs::remove_dir_all(&home).expect("the directory is ours to remove");
}

/// git run with an identity and a default branch of our own, and with every
/// config file it would otherwise read shut out — so what it makes is the
/// same repository on a machine whose git is configured differently and on
/// one where it is not configured at all.
fn git_in(cwd: &Path, args: &[&str]) {
    let out = Command::new("git")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .args(["-c", "init.defaultBranch=main"])
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?} in {}: {}",
        cwd.display(),
        String::from_utf8_lossy(&out.stderr)
    );
}
