//! Where the loop, its sources and its screen are put together.
//!
//! One concern, and it is an ordering: what `bdi` starts, in the order it
//! has to start it in. `run` below says why that order is the one it is.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use chrono::{TimeDelta, Utc};
use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

use crate::app::Wanted;
use crate::collect::agents::Agents;
use crate::collect::changes::Reported;
use crate::config::Scope;
use crate::model::snapshot::{Filter, Snapshot};

#[cfg(test)]
mod fixtures;

mod armed;
mod clipboard;
mod drive;
mod due;
mod keys;
mod screen;
mod wire;

pub(crate) use armed::Armed;
use drive::{drive, Outstanding, View};
use screen::Screen;
use wire::wire;

/// Draw the snapshot until the user quits, re-collecting on a refresh.
///
/// `patience` is how long a read may go unanswered before the project it
/// names says its rows have stopped coming rather than that they are on
/// their way. `tail_every` is how long the band under the forest waits after
/// the provider answers before asking for the selected pane again.
///
/// The screen opens on the projects the config names, before any of them has
/// been read, and every collection — the first one included — runs on a
/// worker thread and fills its project in when it comes back. So what the
/// reader gets for typing `bdi` is the forest, in the time it takes to draw
/// one, with a mark turning beside every project.
///
/// The signals are taken before the screen, and that is the whole of what
/// leaves a window in which one still kills `bdi` outright — the window
/// between this process starting and `Signals::new` returning, in which
/// nothing has been drawn and nothing has to be put back. Taking them later
/// would be worse: `Screen::showing` is what puts the terminal in raw mode,
/// and a registration racing it would leave a moment in which a signal kills
/// outright and the terminal stays as `bdi` left it.
///
/// So a `^C` while the trackers are being read is answered by closing a
/// screen rather than by there being none to close, and putting the terminal
/// back rests on `Screen`'s `Drop` — which rests in turn on the build
/// unwinding, the condition `Drop for Screen` states.
pub fn run(
    patience: TimeDelta,
    tail_every: Duration,
    filter: Filter,
    scope: Scope,
    armed: Vec<Armed>,
    agents: Arc<dyn Agents>,
    collect: Box<dyn FnMut(&Wanted) -> Snapshot + Send>,
) -> anyhow::Result<()> {
    // Taken here rather than on the thread that waits on them, so that they
    // are ours before the screen is opened below. A registration racing the
    // screen would leave a moment in which the terminal is in raw mode and a
    // signal still kills outright.
    let asked_to_stop = Signals::new([SIGHUP, SIGINT, SIGTERM])
        .context("asking to be told about the signals that would otherwise kill bdi")?;
    let projects: Vec<String> = armed
        .iter()
        .map(|project| project.project().to_string())
        .collect();
    let awaiting = Snapshot::awaiting(projects.clone(), agents.name(), scope, filter, Utc::now());
    // Held, not discarded: the socket comes off the filesystem when this
    // returns, so the run that made it is the run that clears it away.
    let (events, ask, panes, _socket, at_startup) =
        wire(Reported::watching(projects), agents, collect, asked_to_stop);
    // Asked for before the screen is opened, so the collection is under way
    // while ratatui is still taking the terminal, and the first frame drawn
    // already carries the mark saying every project is being read. A forest
    // of empty projects with no mark beside them would be a forest that looks
    // read and is not.
    //
    // It is this read coming back that arms every project for its first poll,
    // which is why nothing is armed here: a project armed at startup would
    // ask for a second read of what is already being collected.
    let mut outstanding = Outstanding::for_a_run(patience);
    outstanding.ask(Wanted::Everything, Utc::now());

    let mut screen = Screen::showing(awaiting, panes, at_startup, tail_every)?;
    screen.collecting(outstanding.awaited());

    drive(&mut screen, &events, &ask, outstanding, armed)
}
