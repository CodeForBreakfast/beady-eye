//! What a reader with a tracker and nothing else sees.
//!
//! The commonest machine `bdi` will ever run on, and the one no test could
//! reach before: it has bd, it has no herdr, and it has never heard of one.
//! Both states with no panes in them are here, because the whole of the bead
//! is that they draw the same forest and say different things about why.

mod terminal;

use std::path::{Path, PathBuf};
use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::{shims_first_on_path, shims_first_with_nothing_called, ShimmedTracker};
use terminal::{a_home_naming_one_project, contains, ENTER_ALTERNATE_SCREEN};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// A gap this long between bytes means the collection is over and drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// What `bd list --all --json` said about this project's own tracker.
const THE_TRACKER: &str = include_str!("fixtures/bd_list.json");

/// The one root in that capture. No pane sits in the temp `HOME`, so this row
/// is on the screen only because a run with no panes draws every tree.
const THE_ROOT: &[u8] = "bdi-2bb".as_bytes();

/// One word of the foot notice, *no herdr session · which agents are alive is
/// unknown*, that nothing else `bdi` writes says. One word rather than the
/// phrase, because a repaint reaches the wire a word at a time with a cursor
/// move where each space would be.
///
/// Two words are ruled out and both would have looked fine. *herdr* is on the
/// key row. *session* is on `bdi`'s own stderr — it says a session has no
/// runtime directory to put its socket in — which shares this pty with the
/// screen, so it is in what a test reads back on every machine with no
/// `XDG_RUNTIME_DIR`. That is the build sandbox and not this one, so the
/// wrong word passed here and failed there.
const A_WORD_OF_THE_NOTICE: &[u8] = "unknown".as_bytes();

/// The same for the tail band's *no agent provider · bdi is reading beads
/// alone*.
const A_WORD_OF_THE_BAND: &[u8] = "provider".as_bytes();

/// A `bdi` run against a tracker holding the capture, in `environment`.
fn drawn_by(bdi: &mut Driven) -> Vec<u8> {
    bdi.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);
    bdi.everything()
}

/// The tracker every run here reads, beside a `HOME` naming its project.
fn a_project_in(home: &Path) -> ShimmedTracker {
    let tracker = ShimmedTracker::beside(home);
    tracker.holds(THE_TRACKER);
    tracker
}

/// A runtime directory of this run's own, so it opens its own inbound socket.
///
/// Every test here reads the foot, and the foot is one row that gives up
/// notices from the end when they will not all fit. A run that cannot open
/// its socket carries a second notice — because the machine has no runtime
/// directory, which the build sandbox has not, or because another `bdi` holds
/// the socket, which this machine's does — and the two together are wider
/// than the screen, so the one this test is about is the one dropped.
///
/// The default is therefore a foot that says something different on every
/// machine. This makes it the same one everywhere, and takes the run out of
/// contention with whatever `bdi` the reader has open.
fn a_socket_of_its_own(home: &Path) -> (String, String) {
    ("XDG_RUNTIME_DIR".to_string(), home.display().to_string())
}

/// The bead: a machine with nothing installed to provide agents draws its
/// work and is not warned about a program it has never had.
#[test]
fn a_run_with_no_provider_installed_draws_every_tree_and_is_not_warned() {
    let home = a_home_naming_one_project("no-provider");
    let tracker = a_project_in(&home);
    let mut environment = tracker.environment();
    environment.retain(|(key, _)| key != "PATH");
    environment.push(shims_first_with_nothing_called("herdr", &home));
    environment.push(a_socket_of_its_own(&home));

    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &environment);
    let drawn = drawn_by(&mut bdi);

    assert!(
        contains(&drawn, THE_ROOT),
        "the tracker's root was not drawn: {}",
        bdi.timeline()
    );
    assert!(
        !contains(&drawn, A_WORD_OF_THE_NOTICE),
        "a machine with no herdr was warned about herdr: {}",
        bdi.timeline()
    );
    assert!(
        contains(&drawn, A_WORD_OF_THE_BAND),
        "nothing said why there is no pane to read: {}",
        bdi.timeline()
    );
}

/// The other state, told apart from it: a provider that is installed and will
/// not answer is a finding, and it is said at the foot where nothing can hide
/// it. The forest is the same forest.
#[test]
fn a_run_whose_provider_stops_answering_is_warned_and_still_draws_every_tree() {
    let home = a_home_naming_one_project("provider-silent");
    let tracker = a_project_in(&home);
    let mut environment = tracker.environment();
    environment.push(shims_first_on_path());
    environment.push((
        "BDI_SHIM_HERDR_SESSIONS".to_string(),
        no_session_here(&home).display().to_string(),
    ));
    environment.push(a_socket_of_its_own(&home));

    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &environment);
    let drawn = drawn_by(&mut bdi);

    assert!(
        contains(&drawn, A_WORD_OF_THE_NOTICE),
        "a herdr that would not answer was not said at the foot: {}",
        bdi.timeline()
    );
    assert!(
        contains(&drawn, THE_ROOT),
        "the tracker's root went with the provider: {}",
        bdi.timeline()
    );
}

/// A path the shimmed herdr is told to list its sessions from and that is not
/// there, so it exits non-zero the way a herdr with no session to report
/// does. That is a provider which ran, which is what tells this state from
/// the one above.
fn no_session_here(home: &Path) -> PathBuf {
    home.join("herdr-has-no-session")
}
