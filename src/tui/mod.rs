//! Where the loop, its sources and its screen are put together.
//!
//! One concern, and it is an ordering: what `bdi` starts, in the order it
//! has to start it in. `run` below says why that order is the one it is.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use chrono::Utc;
use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

use crate::app::{Asked, Wanted};
use crate::collect::agents::Agents;
use crate::collect::changes::Reported;
use crate::config::Config;
use crate::model::snapshot::{Filter, Snapshot};
use crate::view::Notice;

#[cfg(test)]
mod fixtures;

mod armed;
mod clipboard;
mod drive;
mod due;
mod keys;
mod reload;
mod screen;
mod wire;

pub(crate) use armed::{Armed, Arming};
pub(crate) use reload::{Reload, CHECKED_EVERY};

/// What the collector thread does with each thing it is asked for.
///
/// A read answers with a snapshot. A config the reader has written answers
/// with nothing at all, because it draws nothing by itself: what it changes
/// is what every read after it reads.
pub type Collecting = Box<dyn FnMut(Asked) -> Option<Snapshot> + Send>;

use drive::{drive, Outstanding, Reading, View};
use screen::{Drawing, Screen};
use wire::wire;

/// Draw the snapshot until the user quits, re-collecting on a refresh.
///
/// `cfg` is the config the run starts on, and everything the loop is set up
/// from comes out of it: the projects drawn before any of them is read, the
/// scope the foot says the directory chose, and the `[tui]` table both of
/// the loop's own clocks come from — how long a read may go unanswered
/// before the project it names says its rows have stopped coming rather than
/// that they are on their way, and how long the band under the forest waits
/// after the provider answers before asking for the selected pane again.
///
/// `arms` is asked for the projects that poll, here and again whenever the
/// reader writes a config, and `reload` is the config file to look at as the
/// run goes on, where the run read one — a run that found no file has
/// nothing to look at and passes nothing.
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
///
/// What resolving the config found this run cannot do is not handed in: it
/// rides on `cfg` and reaches the foot through the snapshot, which is also
/// what `--json` prints. What the wiring finds has no snapshot to ride on and
/// is handed to the screen directly, so the foot draws both without knowing
/// which is which.
pub fn run(
    cfg: &Config,
    filter: Filter,
    arms: Arming,
    agents: Arc<dyn Agents>,
    listening_on: Option<PathBuf>,
    collect: Collecting,
    reload: Option<Reload>,
) -> anyhow::Result<()> {
    // Taken here rather than on the thread that waits on them, so that they
    // are ours before the screen is opened below. A registration racing the
    // screen would leave a moment in which the terminal is in raw mode and a
    // signal still kills outright.
    let asked_to_stop = Signals::new([SIGHUP, SIGINT, SIGTERM])
        .context("asking to be told about the signals that would otherwise kill bdi")?;
    let armed = arms(cfg);
    let projects: Vec<String> = armed
        .iter()
        .map(|project| project.project().to_string())
        .collect();
    let awaiting = Snapshot::awaiting(
        projects.clone(),
        cfg.projects_named_without_git(),
        agents.name(),
        cfg.scope.clone(),
        filter,
        Utc::now(),
    );
    // Kept as well as handed over, because the names the channel accepts are
    // the config's and the config is read again as the run goes on. The loop
    // is where a config the reader has written is taken, so the loop is what
    // writes them.
    let reported = Reported::watching(projects);
    // Held, not discarded: the socket comes off the filesystem when this
    // returns, so the run that made it is the run that clears it away.
    let (events, ask, panes, _socket, from_the_wiring) = wire(
        reported.clone(),
        agents,
        listening_on,
        collect,
        asked_to_stop,
    );
    // Only this process's own, and only the wiring's: what settling the config
    // could not do is on the snapshot, where both mouths read it.
    let at_startup: Vec<Notice> = from_the_wiring.into_iter().collect();
    // Asked for before the screen is opened, so the collection is under way
    // while ratatui is still taking the terminal, and the first frame drawn
    // already carries the mark saying every project is being read. A forest
    // of empty projects with no mark beside them would be a forest that looks
    // read and is not.
    //
    // It is this read coming back that arms every project for its first poll,
    // which is why nothing is armed here: a project armed at startup would
    // ask for a second read of what is already being collected.
    let mut outstanding = Outstanding::for_a_run(cfg.tui.unanswered_after());
    outstanding.ask(Wanted::Everything, Utc::now());

    let mut screen = Screen::showing(awaiting, panes, at_startup, Drawing::to(cfg))?;
    screen.collecting(outstanding.awaited());

    drive(
        &mut screen,
        &events,
        &ask,
        outstanding,
        Reading::of(armed, reported),
        &arms,
        reload,
    )
}
