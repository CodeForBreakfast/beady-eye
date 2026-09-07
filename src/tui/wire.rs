//! Turning everything that happens into events on the loop's channel.
//!
//! A keystroke, a pointer, a signal, a tracker reporting that its work has
//! moved on and a collection coming back are all the same kind of thing by
//! the time they leave here: an `Event` on the one channel the loop waits
//! on. Each source blocks on its own thread so the loop never has to.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;

use ratatui::crossterm::event::{self, KeyEventKind, MouseButton, MouseEventKind};
use signal_hook::iterator::Signals;

use crate::app::{Asked, Wanted};
use crate::collect::agents::Agents;
use crate::collect::changes::{self, Reported, Socket};
use crate::collect::panes::{Aside, Panes};
use crate::view::{Motion, Notice};

use super::drive::Event;
use super::Collecting;

/// The inbound channel, or nothing and the two things said in its place.
///
/// Both are deliberate and neither is a copy of the other. The notice is what
/// a reader needs while they are looking: the view is polled rather than
/// reported, so it is only ever as fresh as the refresh interval, and no row
/// above the foot could show that. The stderr line is what they need
/// afterwards: it names the path and the error underneath it, which is the
/// actionable half — another `bdi` holding the socket is closed by hand — and
/// the half no phrase may carry. It is written before the alternate screen
/// opens, so it is still on the primary screen when the view tears down, and
/// it can be redirected to a file where a notice never can.
fn inbound(opened: Result<Socket, changes::Refused>) -> (Option<Socket>, Option<Notice>) {
    match opened {
        Ok(socket) => (Some(socket), None),
        Err(refused) => {
            eprintln!("bdi: {refused}");
            (None, Some(said_at_the_foot(&refused)))
        }
    }
}

/// Which refusal this was, in the one form the foot can draw.
///
/// Only one of them is answered by closing something, and it is the one that
/// names a process. A session with no runtime directory has nothing to close
/// and is answered by naming a path instead, which is a restart rather than
/// something to do while looking at the screen — so that remedy rides the
/// stderr line, which has room for the flag and the key that carry it. The
/// cause is carried through where the reader can act on it without leaving
/// the view and dropped where they cannot, rather than every refusal arriving
/// as the same sentence about being polled.
fn said_at_the_foot(refused: &changes::Refused) -> Notice {
    match refused {
        changes::Refused::AlreadyListening(_) => Notice::AnotherBdiHadTheInboundChannel,
        changes::Refused::NoRuntimeDirectory | changes::Refused::Unopenable(_, _) => {
            Notice::NoInboundChannel
        }
    }
}

/// The loop's ends: the events it waits on, the channel a collection is asked
/// for on, the provider to ask what is on a pane, the inbound socket for as
/// long as there is a view to keep live, and whatever this run of `bdi` has to
/// say about itself.
pub(super) type Wired = (
    Receiver<Event>,
    Sender<Asked>,
    Box<dyn Panes>,
    Option<Socket>,
    Vec<Notice>,
);

/// Start everything that produces events, and hand back the loop's ends.
pub(super) fn wire(
    reported: Reported,
    agents: Arc<dyn Agents>,
    listening_on: Option<PathBuf>,
    collect: Collecting,
    asked_to_stop: Signals,
) -> Wired {
    let (to_the_loop, events) = mpsc::channel();
    let (ask, asked) = mpsc::channel();

    // The provider's own threads, whose answers come back here like everything
    // else's: the loop waits on one channel and never on the provider.
    let panes: Box<dyn Panes> = Box::new(Aside::new(agents, to_the_loop.clone()));

    let collecting = to_the_loop.clone();
    thread::spawn(move || collector(collect, &asked, &collecting));

    let typing = to_the_loop.clone();
    thread::spawn(move || keys(&typing));

    let stopping = to_the_loop.clone();
    thread::spawn(move || signalled(asked_to_stop, &stopping));

    let (changed, changes) = mpsc::channel();
    // Asked for once, for the life of the run. The socket is a singleton, so
    // a retry cannot make a second channel — it can only move the one there
    // is, at a moment nobody chose: whichever run asks first after the holder
    // goes. What the run without it gives up is freshness and not function,
    // since a project nothing reports for is polled and the foot says so.
    //
    // `reclaim` tells a live holder from litter by connecting, which the
    // holder answers as it would a writer, so a run that keeps asking becomes
    // a standing client of the run that has it. And the notice would stop
    // being settled at startup: a channel arriving later has to retract it,
    // and its `Socket` has to reach the loop, or nothing takes the socket off
    // the filesystem when the run ends.
    let (socket, refused) = inbound(changes::listen(listening_on, &reported, changed.clone()));

    thread::spawn(move || {
        report(
            &mut Inbound {
                changes,
                _open: changed,
            },
            &to_the_loop,
        );
    });

    (events, ask, panes, socket, refused.into_iter().collect())
}

/// What tells `bdi` that a project's work has moved on.
///
/// Something outside `bdi` saying so, over the inbound channel. A project
/// nothing says it for asks for itself instead, which is `Armed`'s job and
/// happens in the loop, because what arms a project is the read that came
/// back and only the loop knows one has.
///
/// Each source runs on its own thread and blocks there, so the loop never
/// sleeps until a deadline of its own.
trait Changes: Send {
    /// Block until there is something to collect for, and say what reading it
    /// takes. Nothing, where the source has no more to report.
    fn next(&mut self) -> Option<Wanted>;
}

/// The source for the projects something else reports for: it says the work
/// has moved when a writer has said which project it moved in.
struct Inbound {
    changes: Receiver<String>,
    /// Held so the channel never runs out of writers. A source whose last
    /// writer has gone must go quiet, not report as fast as it can.
    _open: Sender<String>,
}

impl Changes for Inbound {
    fn next(&mut self) -> Option<Wanted> {
        self.changes.recv().ok().map(Wanted::Project)
    }
}

/// Report one project's changes until the loop stops listening.
fn report(source: &mut dyn Changes, to: &Sender<Event>) {
    while let Some(wanted) = source.next() {
        if to.send(Event::Changed(wanted)).is_err() {
            return;
        }
    }
}

/// Collect on demand, off the UI thread.
///
/// One collection is dozens of remote round trips per project, and the view
/// has to stay under the user's hands throughout.
///
/// A config the reader has written comes down the same channel and answers
/// with nothing, which is what keeps it in order against the reads: it is
/// taken before the collection that reads under it, and the loop that sent
/// both has to know nothing about where either has got to.
pub(super) fn collector(mut collect: Collecting, asked: &Receiver<Asked>, to: &Sender<Event>) {
    while let Ok(asked) = asked.recv() {
        let Some(snapshot) = collect(asked) else {
            continue;
        };
        if to.send(Event::Collected(Box::new(snapshot))).is_err() {
            return;
        }
    }
}

/// Wait to be told to stop, and tell the loop when we are.
///
/// A thread of its own, for the same reason `keys` is one: the loop blocks in
/// `recv` with no deadline, so a flag for it to notice on its next pass would
/// go unread through exactly the quiet the loop is waiting out. The handler
/// itself does none of this — `Sender::send` allocates, which no signal
/// handler may — so what runs in the handler is signal-hook's write to a pipe
/// and the waiting happens here.
fn signalled(mut asked_to_stop: Signals, to: &Sender<Event>) {
    if asked_to_stop.forever().next().is_some() {
        let _ = to.send(Event::Signalled);
    }
}

/// Read the terminal until it has nothing more to say.
fn keys(to: &Sender<Event>) {
    while let Ok(read) = event::read() {
        let Some(event) = incoming(read) else {
            continue;
        };
        if to.send(event).is_err() {
            return;
        }
    }
}

/// What the loop is told about one thing the terminal reported, where it is
/// told anything at all.
///
/// A key that is only being released is not a keystroke; on terminals that
/// report releases at all, taking both would act on every binding twice.
///
/// Capture turns on far more than the two gestures the forest answers.
/// Crossterm asks for any-event tracking, so the terminal reports every cell
/// the pointer crosses whether a button is down or not, and a reader dragging
/// across the screen produces hundreds. They are dropped here, on the thread
/// that reads them, because the loop must stay under the user's hands: a
/// wedged loop is a `^C` that never reaches the Quit mapping and a terminal
/// left in raw mode.
///
/// So of the pointer only two things are answered — a left click, which names
/// a row, and a wheel notch, which moves the selection. A release, a drag,
/// bare motion, the other two buttons and the horizontal wheel are each
/// dropped: none of them names a row the reader is asking for, and
/// right-click in a pane belongs to the terminal's own menu.
fn incoming(read: event::Event) -> Option<Event> {
    match read {
        event::Event::Key(key) if key.kind == KeyEventKind::Press => Some(Event::Key(key)),
        event::Event::Resize(..) => Some(Event::Resize),
        event::Event::Mouse(mouse) => match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => Some(Event::Clicked(mouse.row)),
            MouseEventKind::ScrollUp => Some(Event::Scrolled(Motion::PreviousRow)),
            MouseEventKind::ScrollDown => Some(Event::Scrolled(Motion::NextRow)),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::fixtures::{atlas, A_MOMENT};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    /// A source that reports only when the test says so.
    struct OnCue(Receiver<Wanted>);

    impl Changes for OnCue {
        fn next(&mut self) -> Option<Wanted> {
            // A test that has finished with this source drops the cue, and
            // the source goes quiet.
            self.0.recv().ok()
        }
    }

    #[test]
    fn a_project_is_reported_by_whatever_reports_its_changes() {
        let (to_the_loop, events) = mpsc::channel();
        let (cue, cued) = mpsc::channel();
        thread::spawn(move || report(&mut OnCue(cued), &to_the_loop));

        assert!(
            events.recv_timeout(Duration::from_millis(100)).is_err(),
            "nothing was reported until the source said so"
        );

        cue.send(atlas()).expect("the source is listening");

        assert_eq!(
            events.recv_timeout(A_MOMENT).ok(),
            Some(Event::Changed(atlas())),
            "the loop was told which project moved, not just that something did"
        );
    }

    /// A channel whose writers have all gone must go quiet. A source that
    /// returned from `next` the moment it had nothing left would report in a
    /// loop and collect without pause.
    #[test]
    fn an_inbound_channel_with_no_writers_left_goes_quiet() {
        let (to_the_loop, events) = mpsc::channel();
        let (changed, changes) = mpsc::channel();
        thread::spawn(move || {
            report(
                &mut Inbound {
                    changes,
                    _open: changed,
                },
                &to_the_loop,
            );
        });

        assert!(events.recv_timeout(Duration::from_millis(100)).is_err());
    }

    /// The other half of that silence. `OnCue` stands in for the source
    /// everywhere else, so this is the only place `Inbound` is asked what a
    /// name that arrived means — and the answer is the project to collect
    /// for, not just that something moved.
    #[test]
    fn a_project_named_on_the_inbound_channel_is_reported_as_that_project() {
        let (to_the_loop, events) = mpsc::channel();
        let (changed, changes) = mpsc::channel();
        let open = changed.clone();
        thread::spawn(move || {
            report(
                &mut Inbound {
                    changes,
                    _open: open,
                },
                &to_the_loop,
            );
        });

        changed
            .send("atlas".to_string())
            .expect("the source is listening");

        assert_eq!(
            events.recv_timeout(A_MOMENT).ok(),
            Some(Event::Changed(atlas())),
            "the loop was told which project a writer said had moved"
        );
    }

    /// The loop's threads are the loop's: each ends when the loop stops
    /// listening, rather than outliving the screen it was drawing for.
    #[test]
    fn a_reporter_ends_when_the_loop_stops_listening() {
        let (to_the_loop, events) = mpsc::channel();
        let (cue, cued) = mpsc::channel();
        let reporter = thread::spawn(move || report(&mut OnCue(cued), &to_the_loop));

        drop(events);
        let _ = cue.send(atlas());

        assert!(reporter.join().is_ok());
    }

    fn moused(kind: MouseEventKind, row: u16) -> event::Event {
        event::Event::Mouse(event::MouseEvent {
            kind,
            column: 17,
            row,
            modifiers: KeyModifiers::NONE,
        })
    }

    /// The row every table entry below is reported on, so that what a test
    /// asserts is the kind and not the arithmetic.
    const ON_ROW: u16 = 9;

    /// Every kind of mouse event a captured terminal reports, and what the
    /// reader makes of it.
    ///
    /// The match is what makes it every one rather than every one anybody
    /// remembered: a kind added to crossterm's enum makes it non-exhaustive,
    /// and the compiler names this function until the list above it has been
    /// decided too.
    fn every_mouse_kind() -> Vec<(MouseEventKind, Option<Event>)> {
        let every = vec![
            (
                MouseEventKind::Down(MouseButton::Left),
                Some(Event::Clicked(ON_ROW)),
            ),
            (MouseEventKind::Down(MouseButton::Right), None),
            (MouseEventKind::Down(MouseButton::Middle), None),
            (MouseEventKind::Up(MouseButton::Left), None),
            (MouseEventKind::Up(MouseButton::Right), None),
            (MouseEventKind::Up(MouseButton::Middle), None),
            (MouseEventKind::Drag(MouseButton::Left), None),
            (MouseEventKind::Drag(MouseButton::Right), None),
            (MouseEventKind::Drag(MouseButton::Middle), None),
            (MouseEventKind::Moved, None),
            (
                MouseEventKind::ScrollUp,
                Some(Event::Scrolled(Motion::PreviousRow)),
            ),
            (
                MouseEventKind::ScrollDown,
                Some(Event::Scrolled(Motion::NextRow)),
            ),
            (MouseEventKind::ScrollLeft, None),
            (MouseEventKind::ScrollRight, None),
        ];

        for (kind, _) in &every {
            match kind {
                MouseEventKind::Down(button)
                | MouseEventKind::Up(button)
                | MouseEventKind::Drag(button) => match button {
                    MouseButton::Left | MouseButton::Right | MouseButton::Middle => (),
                },
                MouseEventKind::Moved
                | MouseEventKind::ScrollUp
                | MouseEventKind::ScrollDown
                | MouseEventKind::ScrollLeft
                | MouseEventKind::ScrollRight => (),
            }
        }

        every
    }

    /// The bead this arm exists for: capture turns on every report the
    /// terminal can make, and each one is answered or dropped because it was
    /// decided, not because it fell through a gap.
    #[test]
    fn every_kind_of_mouse_report_is_answered_or_dropped_on_purpose() {
        for (kind, wanted) in every_mouse_kind() {
            assert_eq!(incoming(moused(kind, ON_ROW)), wanted, "{kind:?}");
        }
    }

    /// Motion is the flood: capture asks for a report on every cell the
    /// pointer crosses, and the loop must never be handed one. Dropping it
    /// here costs a match arm on a thread that is not the loop.
    #[test]
    fn a_pointer_moving_over_the_screen_reaches_the_loop_not_at_all() {
        let flood: Vec<Option<Event>> = (0..500)
            .map(|row| incoming(moused(MouseEventKind::Moved, row % 24)))
            .collect();

        assert!(flood.iter().all(Option::is_none));
    }

    /// A click names a row and nothing else. Every band spans the width of
    /// the screen, so the column the pointer was in names no other row.
    #[test]
    fn a_click_is_read_as_the_row_it_landed_on() {
        for row in [0, 9, 23, u16::MAX] {
            assert_eq!(
                incoming(moused(MouseEventKind::Down(MouseButton::Left), row)),
                Some(Event::Clicked(row)),
                "row {row}"
            );
        }
    }

    /// The rule that was there before the mouse was: a key only being
    /// released is not a keystroke, and nothing else the terminal reports is
    /// one either.
    #[test]
    fn a_key_release_a_focus_change_and_a_resize_are_read_as_they_were() {
        let pressed = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE);
        let released = KeyEvent::new_with_kind(
            KeyCode::Char('j'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );

        assert_eq!(
            incoming(event::Event::Key(pressed)),
            Some(Event::Key(pressed))
        );
        assert_eq!(incoming(event::Event::Key(released)), None);
        assert_eq!(incoming(event::Event::Resize(80, 24)), Some(Event::Resize));
        assert_eq!(incoming(event::Event::FocusGained), None);
        assert_eq!(incoming(event::Event::FocusLost), None);
    }

    /// A directory of this test's own, so a test that binds a socket does not
    /// collide with another run of the suite.
    fn a_socket_path(named: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("bdi-{named}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a directory to put the socket in");
        dir.join("beady-eye").join("changes.sock")
    }

    /// The bead's own case: the failure that used to reach only stderr now
    /// also reaches the view, as a notice that outlives the moment it was
    /// printed in. The stderr line is kept — see `inbound` — so this asserts
    /// the notice and leaves the printing to the test harness to capture.
    #[test]
    fn a_channel_that_would_not_open_leaves_a_notice_behind_it() {
        let (socket, notice) = inbound(Err(changes::Refused::NoRuntimeDirectory));

        assert!(socket.is_none());
        assert_eq!(notice, Some(Notice::NoInboundChannel));
    }

    /// `bdi-7ao.61`: this is the arm that flattened. Every refusal produced
    /// the one notice, so the screen said what it cost the reader and never
    /// what did it — and the reader could not tell a session with no runtime
    /// directory, where there is nothing to be done, from another `bdi`
    /// holding the socket, where there is. The pid rides along because
    /// finding the holder is the whole of the remedy.
    #[test]
    fn a_socket_another_bdi_holds_is_said_to_be_that_rather_than_just_lost() {
        let (socket, notice) = inbound(Err(changes::Refused::AlreadyListening(
            std::path::PathBuf::from("/run/user/1000/beady-eye/changes.sock"),
        )));

        assert!(socket.is_none());
        assert_eq!(notice, Some(Notice::AnotherBdiHadTheInboundChannel));
    }

    /// The refusals with nothing behind them a reader could close say what
    /// the loss costs and stop there — a foot that offered a remedy for a
    /// session with no runtime directory would send them looking for a
    /// process that does not exist.
    #[test]
    fn a_channel_lost_to_nothing_anyone_can_close_offers_no_remedy() {
        for refused in [
            changes::Refused::NoRuntimeDirectory,
            changes::Refused::Unopenable(
                std::path::PathBuf::from("/run/user/1000/beady-eye/changes.sock"),
                std::io::Error::from(std::io::ErrorKind::PermissionDenied),
            ),
        ] {
            let (_, notice) = inbound(Err(refused));

            assert_eq!(notice, Some(Notice::NoInboundChannel));
        }
    }

    /// A session with a working channel has nothing to say about itself, and
    /// a foot that warned anyway would teach a reader to ignore it.
    #[test]
    fn a_channel_that_opens_says_nothing() {
        let (changed, _changes) = mpsc::channel();
        let at = a_socket_path("inbound");

        let (socket, notice) = inbound(changes::listen(
            Some(at),
            &Reported::watching(["atlas".to_string()]),
            changed,
        ));

        assert!(socket.is_some());
        assert_eq!(notice, None);
    }

    /// The seam nothing else crosses. `collect::changes` is tested up to the
    /// `Receiver<String>` it hands over, and everything above reads that
    /// receiver as a given, so the run from a writer on the socket to an
    /// event on the loop's channel is asserted here or nowhere. What it adds
    /// over the test at the rule is that the two halves agree on what a
    /// message is: a line the channel accepts comes out as the project the
    /// collection reads.
    #[test]
    fn a_writer_on_the_socket_moves_the_project_it_named_on_the_loops_channel() {
        let (to_the_loop, events) = mpsc::channel();
        let (changed, changes) = mpsc::channel();
        let at = a_socket_path("reported");

        let _socket = changes::listen(
            Some(at.clone()),
            &Reported::watching(["atlas".to_string()]),
            changed.clone(),
        )
        .expect("a socket of this test's own");

        thread::spawn(move || {
            report(
                &mut Inbound {
                    changes,
                    _open: changed,
                },
                &to_the_loop,
            );
        });

        let mut writer = UnixStream::connect(&at).expect("bdi is listening");
        writeln!(writer, "atlas").expect("the channel takes a line");

        assert_eq!(
            events.recv_timeout(A_MOMENT).ok(),
            Some(Event::Changed(atlas())),
            "what a writer said on the socket reached the loop as a project to collect for"
        );
    }
}
