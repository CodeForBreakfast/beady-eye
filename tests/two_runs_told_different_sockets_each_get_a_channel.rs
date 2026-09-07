//! Two `bdi` runs on one machine each get an inbound channel, because each
//! run can be told where to put its socket.
//!
//! The channel is asked for once, at startup, and never asked for again — so
//! a run refused one is polled for the rest of its life, however long the
//! reader leaves it up. A path derived from the login session is the same
//! path for every run in it, which made the second `bdi` a reader opened that
//! run, every time, on every machine.
//!
//! Driven through the binary because the two halves meet nowhere else. What
//! the command line says beats what the config says, and that is settled in
//! `cli` where both are in hand; the path it settles on is bound in
//! `collect::changes`, before the screen. A test below the binary can have
//! one or the other.
//!
//! **The second run's config names the socket the first run is already on.**
//! That is what makes its answer proof rather than coincidence: a run that
//! read its config here would find the path held by a live `bdi`, be refused,
//! and listen nowhere. Being answered on the path its command line named is
//! the only way that run has a channel at all.
//!
//! Neither run has a runtime directory, because [`terminal::bdi_on`] takes
//! the machine's away from every run it starts. That is the shape a Mac is —
//! `XDG_RUNTIME_DIR` is a thing Macs do not set — so the same two runs say
//! that a told path is the whole of what a channel needs there.

mod terminal;

use std::path::{Path, PathBuf};

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::ShimmedTracker;
use terminal::{a_home_naming_one_project_settled, Producer, THE_DESCRIBED_SUBTREE};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// The project both runs read, drawn once the config has been read — which
/// is after the socket has been asked for, so a run that has drawn it has
/// settled where it listens.
const ATLAS: &[u8] = "atlas".as_bytes();

/// A `HOME` whose config names one project and the socket to listen on.
fn a_home_listening_on(named: &str, socket: &Path) -> PathBuf {
    a_home_naming_one_project_settled(
        named,
        &format!("\n[changes]\nsocket = \"{}\"\n", socket.display()),
    )
}

#[test]
fn two_runs_told_different_sockets_each_get_a_channel() {
    let named_by_the_config = std::env::temp_dir().join(format!(
        "bdi-told-sockets-{}-config.sock",
        std::process::id()
    ));
    let named_on_the_command_line = std::env::temp_dir().join(format!(
        "bdi-told-sockets-{}-command-line.sock",
        std::process::id()
    ));

    let home = a_home_listening_on("told-sockets", &named_by_the_config);
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_DESCRIBED_SUBTREE);
    let environment = tracker.environment();

    let mut first = Driven::bdi(ROWS, COLS, home.clone(), &environment);
    first.read_until(ATLAS, GIVING_UP);

    let mut second = Driven::bdi_with_arguments(
        ROWS,
        COLS,
        home.clone(),
        &["--socket", &named_on_the_command_line.display().to_string()],
        &environment,
    );
    second.read_until(ATLAS, GIVING_UP);

    // Both answers taken before either is judged. They are one fact — that
    // two runs on one machine are each reachable — and a pair of assertions
    // would stop at the first run, which is the half that already worked.
    let answers = (
        Producer::connected_to(&named_by_the_config).says("atlas"),
        Producer::connected_to(&named_on_the_command_line).says("atlas"),
    );

    assert_eq!(
        (answers.0.as_str(), answers.1.as_str()),
        ("ok atlas", "ok atlas"),
        "a producer reaches each run where that run was told to listen: the \
         first on the path its config named, the second on the path its \
         command line named over the same config. Neither has a runtime \
         directory to derive one from, so a told path is the whole of what \
         either has.\nfirst:{}\nsecond:{}",
        first.timeline(),
        second.timeline()
    );

    for socket in [&named_by_the_config, &named_on_the_command_line] {
        let _ = std::fs::remove_file(socket);
    }
}
