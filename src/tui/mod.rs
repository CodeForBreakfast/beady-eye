//! Where the loop, its sources and its screen are put together.
//!
//! One concern, and it is an ordering: what `bdi` starts, in the order it
//! has to start it in. `run` below says why that order is the one it is.

use std::time::Duration;

use anyhow::Context;
use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

use crate::app::Wanted;
use crate::collect::changes::Reported;
use crate::model::snapshot::Snapshot;

#[cfg(test)]
mod fixtures;

mod drive;
mod keys;
mod screen;
mod wire;

use drive::drive;
use screen::Screen;
use wire::wire;

/// Draw the snapshot until the user quits, re-collecting on a refresh.
///
/// The first collection is made before the alternate screen opens, so the
/// wait happens where the user can still see their own terminal; every one
/// after it runs on a worker thread.
///
/// The signals are taken between the two, and that is the whole of what
/// leaves a window in which one still kills `bdi` outright. Taking them
/// earlier would be worse rather than better: a signal during that first
/// collection would then be answered by finishing the collection, opening
/// the screen and closing it again, where dying on the spot costs the reader
/// nothing — the terminal has not been touched yet.
pub fn run(
    refresh: Duration,
    projects: Vec<String>,
    mut collect: Box<dyn FnMut(&Wanted) -> Snapshot + Send>,
) -> anyhow::Result<()> {
    let first = collect(&Wanted::Everything);
    // Taken here rather than on the thread that waits on them, so that they
    // are ours before the screen is opened on the line after next. A
    // registration racing the screen would leave a moment in which the
    // terminal is in raw mode and a signal still kills outright.
    let asked_to_stop = Signals::new([SIGHUP, SIGINT, SIGTERM])
        .context("asking to be told about the signals that would otherwise kill bdi")?;
    // Held, not discarded: the socket comes off the filesystem when this
    // returns, so the run that made it is the run that clears it away.
    let (events, ask, panes, _socket, at_startup) = wire(
        refresh,
        Reported::watching(projects),
        collect,
        asked_to_stop,
    );
    let mut screen = Screen::showing(first, panes, at_startup)?;

    drive(&mut screen, &events, &ask)
}
