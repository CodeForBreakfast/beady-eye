//! Proof that a pty test started through `bdi_on` with no `PATH` of its own
//! cannot reach whatever `herdr` the machine running the suite has.
//!
//! `bdi` finds `herdr` the same way it finds `bd`: a `PATH` lookup. A test
//! that gave neither a `PATH` used to inherit the machine's whole one, so a
//! developer's own herdr answered for a run that asserted on nothing about
//! it — quiet everywhere but the pane it drew. This puts a `herdr` the suite
//! owns on the process's own `PATH`, in a directory of its own so a `bd` on a
//! different one survives the same default, and shows `bdi_on`'s default
//! never reaches it.

mod terminal;

use std::path::Path;
use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::{shim, ShimmedHerdr, ShimmedTracker, A_PANE};
use terminal::{a_home_naming_one_project, contains, ENTER_ALTERNATE_SCREEN};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// A gap this long between bytes means the collection is over and drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// What `bd list --all --json` said about this project's own tracker.
const THE_TRACKER: &str = include_str!("fixtures/bd_list.json");

/// The one root in that capture, drawn only if `bd` was reached — so its
/// presence is what says the absence below is about herdr and nothing else.
const THE_ROOT: &[u8] = b"bdi-2bb";

/// A directory holding nothing but a symlink to `program`, standing in for
/// wherever the machine running the suite keeps its real one: dropping one
/// program's directory off `PATH` must not take another's down with it.
fn a_directory_holding_only(home: &Path, named: &str, program: &Path) -> std::path::PathBuf {
    let only = home.join(format!("only-{named}"));
    std::fs::create_dir_all(&only).expect("the directory is ours to make");
    std::os::unix::fs::symlink(program, only.join(named)).expect("the link is ours to make");
    only
}

/// It gets a test binary to itself for the reason `credential_isolation.rs`
/// does: `PATH` is a variable set in this process, and a process's
/// environment is shared by every thread in it. One test, alone in a binary,
/// has no other threads to race setting it.
#[test]
fn a_run_given_no_path_reads_no_herdr_however_the_machine_running_it_is_set_up() {
    let home = a_home_naming_one_project("no-herdr-leak");
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_TRACKER);
    let herdr = ShimmedHerdr::beside(&home);

    // Everything but PATH from each shim: the child is given no PATH of its
    // own, so bdi_on's default is what has to find bd and keep herdr out.
    let mut environment = tracker.environment();
    environment.retain(|(key, _)| key != "PATH");
    environment.extend(
        herdr
            .environment()
            .into_iter()
            .filter(|(key, _)| key != "PATH"),
    );

    // `bd` and `herdr` each alone in a directory of their own, and `cat`
    // alone in a third: the shims exec it to hand back what they were told
    // to answer with, so bd needs it reachable to answer at all — from
    // whichever directory of the real PATH it lives in, wherever that is.
    let only_bd = a_directory_holding_only(&home, "bd", &shim("bd"));
    let only_herdr = a_directory_holding_only(&home, "herdr", &shim("herdr"));
    let only_cat = a_directory_holding_only(
        &home,
        "cat",
        &which_cat().expect("cat is somewhere on this machine's PATH"),
    );
    std::env::set_var(
        "PATH",
        format!(
            "{}:{}:{}",
            only_bd.display(),
            only_herdr.display(),
            only_cat.display()
        ),
    );

    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &environment);
    bdi.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);
    let drawn = bdi.everything();

    assert!(
        contains(&drawn, THE_ROOT),
        "bd was not reached even through its own directory, so this proves \
         nothing about herdr: {:?}\n{}",
        String::from_utf8_lossy(&drawn),
        bdi.timeline()
    );
    assert!(
        !contains(&drawn, A_PANE.as_bytes()),
        "the herdr on this process's own PATH was reached by a run given \
         none of its own: {}",
        bdi.timeline()
    );
}

/// Where `cat` is on the PATH this test process inherited, before it is
/// narrowed down to the directories built above.
fn which_cat() -> Option<std::path::PathBuf> {
    std::env::var("PATH")
        .ok()?
        .split(':')
        .find_map(|directory| {
            let candidate = Path::new(directory).join("cat");
            candidate.exists().then_some(candidate)
        })
}
