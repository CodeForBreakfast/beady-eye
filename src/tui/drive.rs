//! Answering one event at a time, until the user quits.
//!
//! Everything that happens to `bdi` reaches here as an `Event` on one
//! channel, and everything the loop does about one it does through `View`.
//! Those two types are the whole of what it knows: it never names a
//! terminal, a forest or a tail, and nothing that produces an event names
//! the loop.

use std::collections::BTreeMap;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};

use ratatui::crossterm::event::KeyEvent;

use crate::app::{Asked, Awaited, Wanted};
use crate::collect::panes::Answer;
use crate::model::snapshot::Snapshot;
use crate::view::{Action, Motion};

use super::armed::{Armed, Arming};
use super::keys::action;
use super::reload::{Reload, Reloaded};

/// Everything that reaches the loop.
///
/// A `Snapshot` is large and the other three carry almost nothing, so the
/// collected one is boxed rather than widening every event to its size.
#[cfg_attr(test, derive(Debug, PartialEq))]
pub(super) enum Event {
    Key(KeyEvent),
    /// A left click, on the row of the screen it landed on.
    Clicked(u16),
    /// A wheel notch, as the move it asks the selection to make.
    Scrolled(Motion),
    Resize,
    /// Work has moved on, and what has to be read to see it.
    Changed(Wanted),
    /// A collection has come back.
    Collected(Box<Snapshot>),
    /// The provider has said what is on a pane, or would not say.
    Tailed(Answer),
    /// Something outside has asked `bdi` to stop.
    ///
    /// Its own event rather than a keystroke standing in for one: the loop
    /// answers any key at all by taking the bindings window away, so a
    /// synthesised 'q' arriving while that window is up would close the
    /// window and leave `bdi` running.
    Signalled,
}

/// Every answer the provider gives reaches the loop as one of these, which is
/// the
/// whole of what `collect::panes` knows about the loop: it is handed a
/// `Sender` and told nothing about where it goes.
impl From<Answer> for Event {
    fn from(answer: Answer) -> Self {
        Event::Tailed(answer)
    }
}

/// What the screen has on it.
///
/// The loop holds this rather than the view because it decides what a
/// keystroke means: while the bindings are up every keystroke means "take
/// them away", and while a bead is up a motion moves the bead rather than
/// the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Showing {
    Forest,
    Bindings,
    /// The selected bead, whole, in a window over the forest.
    Bead,
}

/// What a click over the bead window came to.
///
/// Three answers where a press usually gives two, because taking the window
/// down is the loop's and what was under the pointer is the view's, and
/// neither can answer for the other.
#[derive(Clone, Copy)]
#[cfg_attr(test, derive(Debug, PartialEq))]
pub(super) enum Landed {
    /// A reference the forest can reach, and the window has gone to the bead
    /// it names.
    Followed,
    /// The page, and nothing on it to go to.
    Nothing,
    /// Off the page — the window's own border, or the forest round it —
    /// which is how a pointer takes the window away.
    Away,
}

/// The rows on the screen and what the user has done to them.
///
/// The seam the loop steers the view across: the loop knows the actions and
/// nothing about rows, and the view knows the rows and only the names of the
/// keys it is handed.
pub(super) trait View {
    /// Show a snapshot just collected, in place of the one on the screen.
    fn collected(&mut self, snapshot: Snapshot);

    /// Say which reads are outstanding, or that none are, reporting whether
    /// the screen has changed.
    ///
    /// The view is told when a read is asked for and told again when one
    /// comes back, so what is on the screen and what the trackers are being
    /// asked are never more than one event apart. It is told *which
    /// projects*, because each project's line says for itself whether its own
    /// rows are about to be replaced — and *when each was asked for*, because
    /// a read that has stopped getting anywhere is drawn exactly like one
    /// just asked for until something measures the wait.
    ///
    /// A sequence rather than the one in flight: a read waiting its turn is
    /// as outstanding as the one being served, and the projects it names have
    /// nothing else on the screen to say so.
    fn collecting(&mut self, awaited: &[Awaited]) -> bool;

    /// How long what is drawn goes on being true with nothing happening, or
    /// nothing where it stays true however long the reader leaves it.
    ///
    /// The loop asks the view rather than deciding for itself, because what
    /// goes stale is what is drawn: an age is a duration and says a different
    /// thing a second later, and a mark part way through turning is a frame
    /// behind by the time the next one is due.
    ///
    /// Measured from `drawn_at`, the instant the frame on the screen was
    /// drawn at, and not from a read of the clock: what is drawn was true
    /// then, and a deadline decided against a later instant is a deadline
    /// for a frame nobody drew. A mark that came to rest between the draw
    /// and the ask holds for nothing at all, and the frame showing it
    /// turning would stand until some unrelated event arrived.
    fn holds_for(&self, drawn_at: DateTime<Utc>) -> Option<Duration>;

    /// Take what the provider said about a pane it was asked to read or to focus,
    /// reporting whether the screen has changed. `now` is when the answer
    /// landed, which is what the next read of that pane is timed from.
    fn tailed(&mut self, answer: Answer, now: DateTime<Utc>) -> bool;

    /// Ask the provider for the pane the band is showing again, where the band has
    /// been showing it for its interval. Nothing on the screen changes for
    /// the ask: the rows stand until the answer lands.
    ///
    /// The tail's own clock, apart from the projects': a pane is read in a
    /// few milliseconds where a tracker is read in seconds, and a band that
    /// waited for the trackers' tick would show a pane as it was half a
    /// minute ago.
    fn reread(&mut self, now: DateTime<Utc>);

    /// How long until the band is due to read its pane again, or nothing
    /// where it is not going to: the band names no pane, or a read of it is
    /// still on its way.
    fn rereads_in(&self, now: DateTime<Utc>) -> Option<Duration>;

    /// Take what a check of the config file found, reporting whether the
    /// screen has changed. `now` is when the file was looked at, which is
    /// what an interval the reader has just written is measured from.
    ///
    /// Handed the outcome rather than a verdict, for the reason `tailed` is
    /// handed the provider's answer: what the foot says about a config that
    /// will not load is the view's to decide, and the loop's part is knowing
    /// when the file was looked at.
    ///
    /// And the outcome carries the config, so what the view draws with is a
    /// read of the file in force rather than of the file the run opened on.
    /// Every setting the view owns is settled in one place from it — a
    /// setting the view learns some other way is one a reload cannot reach,
    /// which is what this hands it rather than the verdict alone.
    fn reloaded(&mut self, reloaded: Reloaded<'_>, now: DateTime<Utc>) -> bool;

    /// Take note that the reader has pressed something — a key, a button, a
    /// wheel notch — before the loop works out what it means, reporting
    /// whether the screen has changed for the press alone.
    ///
    /// The loop's business rather than the view's, because the loop is where
    /// every press arrives and the view hears only the ones the mapping
    /// answers: a key bound to nothing, `?`, `^R` and the key that closes
    /// the bindings never reach `apply`, and what the view says until the
    /// reader's next press has to go on their next press.
    fn pressed(&mut self) -> bool;

    /// Apply one action, reporting whether the screen has changed.
    fn apply(&mut self, action: Action) -> bool;

    /// Move the bead view by one motion, reporting whether the screen has
    /// changed. The selection under it does not move: the view is what the
    /// motion is about while a bead is up, and the loop is what knows one is.
    fn scroll(&mut self, motion: Motion) -> bool;

    /// Whether the bead view is still on the bead it was opened on.
    ///
    /// A collection can move the selection off it — the bead closed and
    /// folded into a run, or left the tracker — and a view drawn from the
    /// selection would then show another bead, or nothing, under a title the
    /// reader did not open. The loop asks after every collection, and takes
    /// the view down when the answer is no.
    fn bead_still_shown(&self) -> bool;

    /// Go to the bead the bead view's ring is on, reporting whether it went.
    ///
    /// Nothing where the ring is on no bead, or on one the forest draws
    /// nowhere — the loop leaves the key to mean what it meant before, which
    /// is to focus the pane.
    fn follow(&mut self) -> bool;

    /// Go back to the bead a reference was followed from, reporting whether
    /// there was one. Nothing on the bead the view was opened on, which is
    /// where the way back leads out of the view altogether.
    fn retrace(&mut self) -> bool;

    /// Select whatever is drawn on one row of the screen, reporting whether
    /// the screen has changed.
    ///
    /// A row is a fact about the frame rather than about the forest, so this
    /// is the loop's other seam: the loop knows where the pointer was and
    /// nothing about what is drawn there.
    fn clicked(&mut self, row: u16) -> bool;

    /// Go to the bead a reference drawn on one row of the bead window names,
    /// reporting what the click came to.
    ///
    /// The same seam again over the window: the loop knows a bead is up and
    /// where the pointer was, and the view knows what it drew there. What
    /// the window is drawn over is not the forest's answer to that row, so
    /// this is a question of its own rather than a `clicked` the view
    /// answers differently while a window is up.
    fn clicked_bead(&mut self, row: u16) -> Landed;

    /// Draw the screen as it stands at `now`, the instant every project's age
    /// and the frame its mark is on are measured against.
    fn draw(&mut self, showing: Showing, now: DateTime<Utc>) -> anyhow::Result<()>;
}

/// What the loop waited for and got.
enum Waited {
    Event(Event),
    /// What is drawn has stopped being true and nothing has happened: an age
    /// has moved on, or the collecting mark is due its next frame.
    Aged,
}

/// The next thing the loop answers, or nothing where there will never be
/// another.
///
/// Every wait in here is a wait on the one channel: a keystroke, a resize, a
/// project reporting a change and a collection coming back are the same kind
/// of thing to the loop. The one deadline it ever sleeps until is `holds_for`
/// — how long what is drawn goes on being true — because the screen now says
/// things that go stale on their own: an age, and a mark part way through
/// turning. Where it says neither, there is no deadline and the loop waits as
/// long as it has to.
fn wait(events: &Receiver<Event>, holds_for: Option<Duration>) -> Option<Waited> {
    let Some(holds_for) = holds_for else {
        return events.recv().ok().map(Waited::Event);
    };
    match events.recv_timeout(holds_for) {
        Ok(event) => Some(Waited::Event(event)),
        Err(RecvTimeoutError::Timeout) => Some(Waited::Aged),
        Err(RecvTimeoutError::Disconnected) => None,
    }
}

/// Read events until the user quits.
///
/// The collection already in flight is handed in rather than started here.
/// The run's first collection is asked for before the screen opens, so that
/// the first frame this draws already carries the mark saying every project
/// is being read — a forest with no rows and no mark would be a forest that
/// looks read and is not. Where that ask happens is `run`'s business, which
/// is where the order everything starts in is decided.
pub(super) fn drive(
    view: &mut dyn View,
    events: &Receiver<Event>,
    ask: &Sender<Asked>,
    mut outstanding: Outstanding,
    mut armed: Vec<Armed>,
    arms: &Arming,
    mut reload: Option<Reload>,
) -> anyhow::Result<()> {
    let mut showing = Showing::Forest;
    let mut drawn_at = Utc::now();
    view.draw(showing, drawn_at)?;

    while let Some(waited) = wait(
        events,
        sleeps_for(
            view,
            &outstanding,
            &armed,
            reload.as_ref(),
            drawn_at,
            Utc::now(),
        ),
    ) {
        let woken = match waited {
            // Nothing has happened and a deadline is up. Where it is the
            // frame's, what is drawn is out of date, which is the whole of
            // what makes the mark turn and the ages advance: a collection
            // is dozens of round trips and reports nothing until it is done.
            // The other deadlines — a project asking for itself again, a
            // read leaving once its window is out, the band reading its pane
            // again — change nothing on the screen by themselves, and what
            // each of them does about the screen it says below.
            Waited::Aged => ran_out(view, drawn_at, Utc::now()),
            Waited::Event(event) => {
                let Some(changed) =
                    answered(view, &mut outstanding, &mut armed, &mut showing, event)
                else {
                    return Ok(());
                };
                changed
            }
        };

        // Whatever has fallen due, whether a deadline or an event woke us: a
        // project's own ask coming round, the read at the front leaving, and
        // the band's next read of its pane. Done in one place rather than on
        // each arm, so that the loop cannot answer an event and forget to
        // look.
        let now = Utc::now();
        let told = asks_for_what_is_due(view, &mut outstanding, &mut armed, now);
        outstanding.sends(ask, now);
        view.reread(now);
        // A run reading a config file looks at it here; a run that found no
        // file to read has nothing to look at, and never will — the whole of
        // what it is working to came from the directory it was started in.
        let noticed = looked_at(
            view,
            reload.as_mut(),
            &mut armed,
            arms,
            ask,
            &mut outstanding,
            now,
        );

        if woken || told || noticed {
            drawn_at = now;
            view.draw(showing, drawn_at)?;
        }
    }

    Ok(())
}

/// Look at the config file where the run has one, and do what a config the
/// reader has written asks of the rest of the run. Whether the screen changed
/// for what the check found.
///
/// The file is read here, on the loop's own thread, and what a reload
/// produces is what goes behind the collector's seam. The two are not the
/// same cost and they are not the same wait: a check is a few kilobytes off
/// the page cache and a handful of local `git` calls, once per edit, where
/// the collector's queue is bounded only by a tracker that never answers.
/// Put the check behind that seam and a reader who breaks their config while
/// a tracker is hung is never told it will not load — which is the
/// disappearance the notice exists to prevent. So the check stays where the
/// reader's answer can be drawn on the next frame, and the collection it
/// causes goes where every other collection already goes.
///
/// Four things follow a config the reader has written: the projects that
/// poll become the ones it names, how long a read may go unanswered becomes
/// what it says, the collector is told to work to it, and every project is
/// read under it. The last two are in that order because a collection carries
/// no config with it — the collector reads under whatever it is working to
/// when the read arrives — so a read that overtook the config would draw a
/// whole screen read under the file the reader has just replaced. Nothing
/// here arranges that: the channel is in order, and the read waits out its
/// window behind the config already on it.
///
/// The patience is the loop's own share of what the view's is: every setting
/// `[tui]` names is a gap somebody waits out, and the two the *screen* waits
/// out go to the view through `reloaded` above. This one is here because a
/// read that has stopped getting anywhere is the loop's to notice.
fn looked_at(
    view: &mut dyn View,
    reload: Option<&mut Reload>,
    armed: &mut Vec<Armed>,
    arms: &Arming,
    ask: &Sender<Asked>,
    outstanding: &mut Outstanding,
    now: DateTime<Utc>,
) -> bool {
    let Some(reload) = reload else {
        return view.reloaded(Reloaded::Untouched, now);
    };
    let reloaded = reload.checks(now);
    let noticed = view.reloaded(reloaded, now);
    let Reloaded::Fresh(written) = reloaded else {
        return noticed;
    };
    *armed = still_armed(std::mem::take(armed), arms(written));
    outstanding.waits_out(written.tui.unanswered_after());
    if ask
        .send(Asked::Reloaded(Box::new(written.clone())))
        .is_err()
    {
        return noticed;
    }
    let told = asked_for(view, outstanding, Wanted::Everything);
    told || noticed
}

/// The projects that poll, as the config the reader has just written names
/// them: one the file has gained polls, one it has lost stops asking, and one
/// it still names polls as the file now says — `Armed::still_due` is where
/// what the file settles and what its last read settled are told apart.
///
/// A project the file has gained is disarmed, exactly as every project is at
/// startup: what arms it is the read this reload asks for coming back, and
/// arming it here would ask a second time for what is already being
/// collected.
fn still_armed(standing: Vec<Armed>, named: Vec<Armed>) -> Vec<Armed> {
    let mut standing: BTreeMap<String, Armed> = standing
        .into_iter()
        .map(|project| (project.project().to_string(), project))
        .collect();
    named
        .into_iter()
        .map(|named| match standing.remove(named.project()) {
            Some(standing) => standing.still_due(named),
            None => named,
        })
        .collect()
}

/// Ask for whatever the projects that arm themselves are now due to ask for.
/// Whether the screen changed for it.
///
/// Through `asked_for`, the same way a message and `^R` ask, so that a poll's
/// own read is said on the screen without this having to remember to say it.
/// A further way of making a read outstanding belongs on that call for the
/// same reason: the marking is the rule's, not each caller's.
fn asks_for_what_is_due(
    view: &mut dyn View,
    outstanding: &mut Outstanding,
    armed: &mut [Armed],
    now: DateTime<Utc>,
) -> bool {
    // Every project that is due, not the first: several come due together
    // after a read of everything, and a short-circuiting `any` would ask for
    // one of them and leave the rest armed in the past.
    let mut told = false;
    for wanted in armed.iter_mut().filter_map(|project| project.asks(now)) {
        told |= asked_for(view, outstanding, wanted);
    }
    told
}

/// Whether the frame drawn at `drawn_at` has run out by `now`: the view said
/// how long it would hold, and that long has passed.
fn ran_out(view: &dyn View, drawn_at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    let since_drawn = (now - drawn_at).to_std().unwrap_or_default();
    view.holds_for(drawn_at)
        .is_some_and(|held| held <= since_drawn)
}

/// How long the loop may sleep: until what is drawn stops being true, until a
/// project asks for itself, until the read at the front is due to leave,
/// until the band is due to read its pane again, or until the config file is
/// due to be looked at — whichever comes first, and nothing where none of
/// them will.
///
/// Five deadlines where there was one, and the loop tells them apart only by
/// doing all five things when it wakes. What that costs is asking each of
/// the five whether it is due on a wake that was one of the others'; what it
/// saves is a second way for the loop to be woken.
///
/// Two instants, because the screen's deadline is about the frame on it and
/// the other two are about now: `drawn_at` is when that frame was drawn, and
/// a loop woken by an event that drew nothing is still sleeping on the frame
/// from before. The view says how long the frame holds from then, and what
/// is slept from now is whatever of that is left.
fn sleeps_for(
    view: &dyn View,
    outstanding: &Outstanding,
    armed: &[Armed],
    reload: Option<&Reload>,
    drawn_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Option<Duration> {
    let since_drawn = (now - drawn_at).to_std().unwrap_or_default();
    let holds_for = view
        .holds_for(drawn_at)
        .map(|held| held.saturating_sub(since_drawn));

    [
        holds_for,
        outstanding.sends_in(now),
        view.rereads_in(now),
        reload.and_then(|reload| reload.checks_in(now)),
    ]
    .into_iter()
    .chain(armed.iter().map(|project| project.asks_in(now)))
    .flatten()
    .min()
}

/// Answer one event, reporting whether the screen has changed — or nothing
/// where it was the event that ends the run.
fn answered(
    view: &mut dyn View,
    outstanding: &mut Outstanding,
    armed: &mut [Armed],
    showing: &mut Showing,
    event: Event,
) -> Option<bool> {
    // Heard before the press is read, so a `y` pressed twice sets its line
    // on the foot after the first press has taken it off, not before.
    let pressed = matches!(
        event,
        Event::Key(_) | Event::Clicked(_) | Event::Scrolled(_)
    ) && view.pressed();
    let changed = match event {
        // Any key at all, because a reader who opened the bindings by
        // accident must not have to find the one key that closes them.
        Event::Key(_) if *showing == Showing::Bindings => {
            *showing = Showing::Forest;
            true
        }
        // The bead view is the hub: a motion moves the bead, Tab moves it on
        // to the next bead this one names, Enter goes to that bead or else
        // focuses the pane, `f` focuses it whatever the view is on, and Esc
        // goes back — to the bead a reference was followed from where there
        // is one, and out to the forest on the bead the view was opened on.
        // `q` leaves altogether, as it does from the bindings, so the forest
        // a reader was looking at is still there to quit from — the whole way
        // back out rather than one step of it, which is what makes it worth
        // having beside Esc. A refresh lands behind the view and leaves it
        // up; the bindings go up over the forest, since the key that takes
        // them away lands there.
        Event::Key(key) if *showing == Showing::Bead => match action(key) {
            Some(Action::Back) => {
                if !view.retrace() {
                    *showing = Showing::Forest;
                }
                true
            }
            Some(Action::Quit) => {
                *showing = Showing::Forest;
                true
            }
            Some(Action::Move(motion)) => view.scroll(motion),
            Some(Action::NextRelated) => view.apply(Action::NextRelated),
            Some(Action::ShowBead) => view.follow() || view.apply(Action::Focus),
            Some(Action::Focus) => view.apply(Action::Focus),
            Some(Action::CopyId) => view.apply(Action::CopyId),
            Some(Action::ShowBindings) => {
                *showing = Showing::Bindings;
                true
            }
            Some(Action::Refresh) => asked_for(view, outstanding, Wanted::Everything),
            Some(
                Action::CollapseOrParent
                | Action::ExpandOrChild
                | Action::ToggleFold
                | Action::ExpandSubtree
                | Action::CollapseSubtree
                | Action::RestoreDefault
                | Action::ToggleFilter,
            )
            | None => false,
        },
        Event::Key(key) => match action(key) {
            Some(Action::Quit) => return None,
            Some(Action::ShowBindings) => {
                *showing = Showing::Bindings;
                true
            }
            // Up only where there is a bead to show: the view says whether
            // the selection is on one, and a row that is not a bead leaves
            // the forest exactly as it was.
            Some(Action::ShowBead) => {
                let opened = view.apply(Action::ShowBead);
                if opened {
                    *showing = Showing::Bead;
                }
                opened
            }
            // The same line the inbound channel's arm is, and that is
            // Graeme's ruling rather than a tidy-up: the refresh key acts
            // exactly like a notification, with no bypass and no path of its
            // own. What it does not share is which projects it names, because
            // a key nobody aimed at a project asks about all of them.
            Some(Action::Refresh) => asked_for(view, outstanding, Wanted::Everything),
            // The ring is over a window that is not up, so there is nothing
            // here for these to step. Answered where the key is read rather
            // than passed down for the forest to decline, so what a key means
            // in each view is settled in the one place that knows which view
            // is up.
            Some(Action::NextRelated) => false,
            Some(action) => view.apply(action),
            None => false,
        },
        // A click or a notch takes the bindings away and does no more,
        // for the same reason a key does: the window is over the forest,
        // so the rows under the pointer are rows nobody can see.
        Event::Clicked(_) | Event::Scrolled(_) if *showing == Showing::Bindings => {
            *showing = Showing::Forest;
            true
        }
        // Over the bead view a click is answered by whatever it landed on,
        // because there the rows under the pointer are rows the reader is
        // looking at — and the references the window draws are the one thing
        // on that screen a pointer could otherwise not reach.
        Event::Clicked(row) if *showing == Showing::Bead => match view.clicked_bead(row) {
            Landed::Followed => true,
            Landed::Nothing => false,
            Landed::Away => {
                *showing = Showing::Forest;
                true
            }
        },
        // A notch moves the bead, the way a key does.
        Event::Scrolled(motion) if *showing == Showing::Bead => view.scroll(motion),
        Event::Clicked(row) => view.clicked(row),
        Event::Scrolled(motion) => view.apply(Action::Move(motion)),
        Event::Resize => true,
        Event::Changed(wanted) => asked_for(view, outstanding, wanted),
        Event::Collected(snapshot) => {
            let now = Utc::now();
            if let Some(read) = outstanding.came_back() {
                // Every project that read covered now has nothing coming, so
                // this is where each of them arms its next ask. The only
                // place: a read that never comes back arms nothing, and the
                // project says its tracker has stopped answering rather than
                // being quietly polled over.
                for project in armed.iter_mut() {
                    project.came_back(&read, now);
                }
            }
            view.collected(*snapshot);
            // A collection that moved the selection off the bead the view
            // was opened on takes the view down with it: drawn from the
            // selection, it would show another bead, or nothing, under a
            // title the reader did not open, and a forest that looked
            // ordinary would go on answering keys as a bead.
            if *showing == Showing::Bead && !view.bead_still_shown() {
                *showing = Showing::Forest;
            }
            // Told after the rows land, and told whatever came of the
            // collection that ended: another may have been waiting behind
            // it, and where none was, a line left saying it was being
            // read would say so over rows that had already arrived.
            view.collecting(outstanding.awaited());
            true
        }
        Event::Tailed(answer) => view.tailed(answer, Utc::now()),
        // The same `None` 'q' hands back, and for the same reason: it is
        // returning that drops the screen, and dropping the screen is
        // what hands the terminal back.
        Event::Signalled => return None,
    };
    Some(pressed || changed)
}

/// Ask for a read, and say so on the screen at the instant it was asked for
/// rather than at the instant it leaves.
///
/// The two are no longer the same moment — a read waits out its window before
/// it is sent — and it is the ask the reader is owed. `^R` did nothing a
/// reader could see once already, and a mark that waited for the window would
/// be that bead again on a shorter timescale.
///
/// **Every way of making a read outstanding goes through here**, so that
/// saying so is the ask's own business rather than something each caller has
/// to remember. A message on the inbound channel, `^R`, and a project asking
/// for itself are the three; a fourth belongs on this call and not beside it.
fn asked_for(view: &mut dyn View, outstanding: &mut Outstanding, wanted: Wanted) -> bool {
    outstanding.ask(wanted, Utc::now()) && view.collecting(outstanding.awaited())
}

/// How long a read is held after it is asked for before it is sent, so that
/// a burst about one project costs one read rather than one each.
///
/// A producer with nothing to lose by talking — a hook firing per commit, a
/// key held down — says the same thing many times in a moment, and without
/// this each saying is a read. It runs from the first notification and is not
/// reset by the ones after it, so a `^R` held down still gets the read it was
/// pressed for; a resetting window would withhold it for as long as the key
/// was down.
///
/// **It has to stay well under `[tui] unanswered_after_seconds`.** A read
/// waiting out its window is drawn exactly like one a tracker has stopped
/// answering, because `Awaited::unanswered_at` measures from the ask and
/// deliberately not from the send — `Outstanding::came_back` says why that
/// stamp cannot move. The two are kept apart by the config key counting in
/// whole seconds: the shortest wait it can name that is not "immediately" is
/// a second, and this is a fifth of it.
///
/// `a_read_goes_before_its_project_can_be_said_to_have_stopped_being_read`
/// holds that against the shortest patience the key can name, through the
/// predicate rather than by comparing two constants, and on the pair
/// `Outstanding::for_a_run` builds rather than on this constant — so it
/// holds whichever value the window comes to be read from. A
/// `debug_assert!(window < patience)` in `waiting` would cover it too and is
/// not available: the tests that are about the window construct one longer
/// than the patience on purpose.
const WINDOW: TimeDelta = TimeDelta::milliseconds(200);

/// What has been asked for and not yet collected.
///
/// A project is in one of three states here and never in none of them: its
/// read is in flight, or its read is asked for and waiting — out its window,
/// or behind the read in front of it — or it has nothing here at all and is
/// `Armed` to ask again. The last transition is `came_back`'s, and it is what
/// makes the three cover every project: a read that comes back arms, a read
/// that never comes back stays here and is drawn as unanswered.
///
/// A request arriving while a collection is in flight used to be dropped, on
/// the grounds that the collection already running was reading exactly what
/// it would ask for. A refresh that names a project is what ends that: the
/// one running may be reading a different project entirely, and dropping the
/// request would lose the change it was sent for — the failure the inbound
/// channel exists to prevent. So a request waits its turn instead. A whole
/// collection absorbs the single projects it would read anyway, so what waits
/// is never more than one per project.
pub(super) struct Outstanding {
    /// Every read asked for and not yet come back, in the order they will be
    /// served: the one the collector has, then whatever is waiting for it.
    ///
    /// What each names rather than that some read is running — a project line
    /// says for itself whether its own rows are on their way, so the screen
    /// needs to know which projects and not only that some are — and *when
    /// each was asked for*, because that is the only measure of the wait
    /// there is. Nothing downstream can recover it: the collector blocks in
    /// `Command::output()`, which has no deadline of its own, and reports
    /// nothing until it is done, so a tracker hung for an hour and one asked
    /// half a second ago look identical from every side but this one. A read
    /// that has not been sent yet is worse still, because there is nothing to
    /// report from at all.
    ///
    /// One sequence rather than the one in flight beside a stash of what
    /// waits. The question a project line asks is how long its rows have been
    /// on their way, and being sent is a step along that wait rather than the
    /// start of it — so the two belong to one list, ordered by when each was
    /// asked for, which is also the order the collector takes them in.
    awaited: Vec<Awaited>,
    /// How long a read this asks for may go unanswered before the project it
    /// names is reported as having stopped being read. Held here because this
    /// is where a read is asked for, and carried on each one so that whoever
    /// draws it needs nothing else to decide.
    patience: TimeDelta,
    /// How long the read at the front is held before it is sent, so that a
    /// burst of notifications about one project costs one read rather than
    /// one each. See `WINDOW`.
    window: TimeDelta,
    /// Whether the read at the front has gone to the collector. False while
    /// it is waiting out its window, and false again the moment the read it
    /// named comes back.
    sent: bool,
}

impl Outstanding {
    /// Nothing outstanding, at the two waits a run drives with: the patience
    /// its config names, and `WINDOW`.
    ///
    /// The only place production pairs them, so that a guard calling this
    /// holds the relationship between them against the pair a run is built
    /// with rather than against `WINDOW`.
    pub(super) fn for_a_run(patience: TimeDelta) -> Self {
        Self::waiting(patience, WINDOW)
    }

    /// Wait this long on a read from here on, as the config the reader has
    /// just written says.
    ///
    /// The reads already asked for keep the patience they were stamped with,
    /// because that is what the screen has been drawing them against: a read
    /// the reader has been watching for a minute would otherwise be reported
    /// as having stopped answering by an edit that said nothing about it.
    pub(super) fn waits_out(&mut self, patience: TimeDelta) {
        self.patience = patience;
    }

    pub(super) fn waiting(patience: TimeDelta, window: TimeDelta) -> Self {
        Self {
            awaited: Vec::new(),
            patience,
            window,
            sent: false,
        }
    }

    /// Ask for a collection, or keep it until the one running comes back.
    ///
    /// Reports whether what is outstanding is any different for it, which is
    /// not the same as whether a collection started: a request arriving
    /// mid-collection waits its turn, and its project's line says so.
    ///
    /// Asking never sends. Every read waits out its window first, and
    /// `sends` is where the leaving happens — so the screen says a read is
    /// coming at the instant it was asked for, whatever the window then does
    /// about when it goes.
    pub(super) fn ask(&mut self, wanted: Wanted, now: DateTime<Utc>) -> bool {
        if self.awaited.is_empty() {
            self.awaited.push(self.stamped(wanted, now));
            return true;
        }
        self.queue(wanted, now)
    }

    /// Send the read at the front, where its window is out and no other is in
    /// flight.
    ///
    /// One at a time, as it has always been: a collection is dozens of round
    /// trips per project and two at once would double what a tracker is
    /// asked without halving anything.
    pub(super) fn sends(&mut self, ask: &Sender<Asked>, now: DateTime<Utc>) {
        if self.sent {
            return;
        }
        let Some(next) = self.awaited.first() else {
            return;
        };
        if now < next.asked_at + self.window {
            return;
        }
        if ask.send(Asked::Read(next.wanted.clone())).is_err() {
            self.awaited.clear();
            return;
        }
        self.sent = true;
    }

    /// How long until the read at the front leaves, or nothing where none is
    /// waiting to leave: the loop sleeps until this among its other
    /// deadlines, because nothing else is going to wake it for a window
    /// running out.
    pub(super) fn sends_in(&self, now: DateTime<Utc>) -> Option<Duration> {
        if self.sent {
            return None;
        }
        self.awaited.first().map(|next| {
            (next.asked_at + self.window - now)
                .to_std()
                .unwrap_or_default()
        })
    }

    /// Keep a request until its turn, where nothing already waiting covers
    /// it.
    ///
    /// Covered by what is *waiting* rather than by what is in flight: the
    /// collection running may have passed the project before the change was
    /// reported, so a change arriving mid-collection always earns a read of
    /// its own. What it does not earn is a second one.
    fn queue(&mut self, wanted: Wanted, now: DateTime<Utc>) -> bool {
        match wanted {
            // A whole collection reads every project, so it stands in for the
            // single ones waiting with it — and its wait begins when it was
            // asked for, not when the earliest of theirs did. It names every
            // project on the screen, including the ones nothing had asked
            // about, and one instant cannot be true of both: an inherited one
            // would put a wait those projects never had beside their names,
            // and a whole screen of marks saying the reads have stopped is
            // what a reader gets for one project changing.
            //
            // What that costs is the projects it absorbs. Their wait was
            // longer and this understates it, for one patience, after which
            // the mark says what it said before. Understating a wait is what
            // patience is: it is the length of not-saying-yet the project has
            // already decided on, and it is bounded. Overstating one is a
            // claim about a tracker nobody asked.
            Wanted::Everything => {
                if self.queued().any(|it| it.wanted == Wanted::Everything) {
                    return false;
                }
                self.awaited.truncate(self.in_flight());
                self.awaited.push(self.stamped(Wanted::Everything, now));
                true
            }
            // A project reported for again while it waits is the same wait: a
            // project nothing reports for is polled every refresh interval,
            // and the poll goes on naming it for as long as it is uncovered,
            // so a stamp taken from the latest ask would be pushed forward by
            // the very polling that proves nothing has been read.
            Wanted::Project(project) => {
                if self.queued().any(|it| it.wanted.names(&project)) {
                    return false;
                }
                self.awaited
                    .push(self.stamped(Wanted::Project(project), now));
                true
            }
        }
    }

    /// The reads that have not been sent.
    ///
    /// Every one but the first, once the first has gone — and every one of
    /// them while the first is still waiting out its window, which is what
    /// makes the window a debounce at all. A hundred messages about one
    /// project arriving into an idle `bdi` find their own unsent read at the
    /// front and are dropped against it; were the front skipped they would
    /// queue ninety-nine reads behind it.
    fn queued(&self) -> impl Iterator<Item = &Awaited> {
        self.awaited.iter().skip(self.in_flight())
    }

    /// How many reads the collector has: one, or none while the front is
    /// still waiting out its window.
    fn in_flight(&self) -> usize {
        usize::from(self.sent)
    }

    fn stamped(&self, wanted: Wanted, asked_at: DateTime<Utc>) -> Awaited {
        Awaited {
            wanted,
            asked_at,
            patience: self.patience,
        }
    }

    /// Take the read that came back, and say what it read.
    ///
    /// What it read is what arms the projects it covered for their next ask,
    /// which is the only thing that arms them: a read nobody answers arms
    /// nothing, and a project with nothing coming is what the unanswered mark
    /// is for.
    ///
    /// Whatever waited behind it is left to `sends`. The one that reaches the
    /// front keeps the stamp it queued at rather than being stamped again.
    /// Its project's rows have been on their way since the change that wanted
    /// them was reported, and a wait that started over on reaching the front
    /// would tell a reader whose project had been stranded ten minutes behind
    /// a hung tracker that its own tracker had just been asked. Its window
    /// ran out while it waited, so `sends` finds it due and it leaves at once.
    fn came_back(&mut self) -> Option<Wanted> {
        if self.awaited.is_empty() {
            return None;
        }
        self.sent = false;
        Some(self.awaited.remove(0).wanted)
    }

    /// Every read outstanding and since when, for the screen to say beside
    /// the projects each of them names.
    pub(super) fn awaited(&self) -> &[Awaited] {
        &self.awaited
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::tui::fixtures::{a_snapshot, atlas, ferry, reading, A_MOMENT, PATIENCE};
    use crate::tui::keys::tests::{control, key};
    use crate::tui::wire::collector;
    use crate::view::phrase;
    use ratatui::crossterm::event::KeyCode;
    use std::cell::RefCell;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Instant;

    /// A view that remembers what the loop did to it.
    #[derive(Default)]
    struct Recorder {
        applied: Vec<Action>,
        /// The motions the bead view was moved by, apart from the actions,
        /// because the whole question is which of the two a key reached.
        scrolled: Vec<Motion>,
        clicked: Vec<u16>,
        /// The rows a click over the bead window asked about, apart from the
        /// forest's, because the whole question is which of the two the loop
        /// sent one to.
        clicked_bead: Vec<u16>,
        /// What the bead window says a click over it landed on. Nothing where
        /// a test has not said, which is a click that landed on the page and
        /// found nothing to go to.
        lands_on: Option<Landed>,
        /// Whether the selection is on a row with no bead to show, for the
        /// tests about Enter on one.
        not_a_bead: bool,
        /// Whether the bead view's ring is on a bead the forest can go to,
        /// for the tests about the key that follows one. Off, so a test that
        /// says nothing about references gets Enter meaning what it meant
        /// before there were any.
        on_a_reference: bool,
        /// Whether a reference has been followed and there is a bead to go
        /// back to, for the tests about the way back.
        somewhere_to_go_back_to: bool,
        /// How many times the view was asked to follow a reference and to go
        /// back, in the order the loop asked, so a test can tell a key the
        /// loop swallowed from one it passed on.
        followed: usize,
        retraced: usize,
        /// Whether a collection takes the selection off the bead the view
        /// was opened on, for the tests about one landing behind the view.
        collection_moves_the_selection: bool,
        collected: usize,
        /// What the view was told is outstanding, in the order it was told.
        /// What and not how many: a project line answers for its own rows, so
        /// a test that only counted could not tell a refresh of one project
        /// from a refresh of the lot.
        awaited: Vec<Vec<Awaited>>,
        /// The instant each frame was drawn at, in the order they were drawn.
        drawn_at: Vec<DateTime<Utc>>,
        /// The instant each deadline was asked to be measured from, in the
        /// order the loop asked.
        measured_at: RefCell<Vec<DateTime<Utc>>>,
        showing: Vec<Showing>,
        /// What a click reports back, for the tests about a click that lands
        /// on no row.
        nothing_under_the_pointer: bool,
        /// What a press reports back, for the tests about a screen that
        /// changes for the press alone.
        pressing_changes: bool,
        /// How many actions had been applied when each press was heard, in
        /// the order they were heard: the order the view hears things in is
        /// the whole of what these record.
        pressed_after: Vec<usize>,
        /// How long this view says it is from reading its pane again, for
        /// the tests about the band's own clock. Nothing, as a band with no
        /// pane says.
        rereads_in: Option<Duration>,
        /// The instant of each time the loop asked the band to read its pane
        /// again, in the order it asked.
        ///
        /// Also the instant the loop looked at everything else that had
        /// fallen due, and the only record a test has of one: the loop takes
        /// the clock once a pass and hands that same instant to the projects'
        /// asks, to the read at the front and to the band. So a test asking
        /// when the armed projects were last consulted reads this.
        reread_at: Vec<DateTime<Utc>>,
        /// Sent the instant of every pass of the loop, where a test drives
        /// the loop by what it does rather than for a window of wall clock.
        /// The loop asks the band to read its pane once a pass whether or not
        /// one is due, so this is where the view sees a pass go by. A pass
        /// after the test has stopped listening has nobody left to tell,
        /// which is not a failure.
        ///
        /// The instant and not a bare tick, because a test can want a pass at
        /// a particular time rather than an nth pass: a project's poll comes
        /// round on the clock, so the pass that proves it was declined is the
        /// first one past its interval however many came before.
        went_round: Option<Sender<DateTime<Utc>>>,
    }

    impl Recorder {
        /// How many frames the loop drew.
        fn drawn(&self) -> usize {
            self.drawn_at.len()
        }

        /// Which projects the view was told about, in the order it was told,
        /// with the instants left out. Most of these tests are about which
        /// collections the loop starts and in what order; the ones about the
        /// stamps read `awaited` itself.
        fn collecting(&self) -> Vec<Vec<Wanted>> {
            self.awaited
                .iter()
                .map(|told| told.iter().map(|it| it.wanted.clone()).collect())
                .collect()
        }

        /// When each read the view was told about was asked for, in the order
        /// it was told, flattened: the tests that read this are about how one
        /// stamp compares with another rather than about which telling each
        /// came in.
        fn asked_at(&self) -> Vec<chrono::DateTime<Utc>> {
            self.awaited
                .iter()
                .flat_map(|told| told.iter().map(|it| it.asked_at))
                .collect()
        }
    }

    impl View for Recorder {
        fn collected(&mut self, _snapshot: Snapshot) {
            self.collected += 1;
        }

        fn collecting(&mut self, awaited: &[Awaited]) -> bool {
            self.awaited.push(awaited.to_vec());
            true
        }

        /// Nothing on this view says anything about a config, and no test
        /// here hands the loop a file to look at. What a check does to the
        /// foot is `Shown`'s, where a test can read the row it lands on.
        fn reloaded(&mut self, _reloaded: Reloaded<'_>, _now: DateTime<Utc>) -> bool {
            false
        }

        /// The same rule `Shown` keeps, so a loop test is asking the loop
        /// what it asks a real screen: a read outstanding is a frame away
        /// from being out of date, and this view has no ages on it.
        fn holds_for(&self, drawn_at: DateTime<Utc>) -> Option<Duration> {
            self.measured_at.borrow_mut().push(drawn_at);
            let told = self.awaited.last()?;
            (!told.is_empty()).then_some(phrase::FRAME)
        }

        fn tailed(&mut self, _answer: Answer, _now: DateTime<Utc>) -> bool {
            true
        }

        fn reread(&mut self, now: DateTime<Utc>) {
            self.reread_at.push(now);
            if let Some(went_round) = &self.went_round {
                let _ = went_round.send(now);
            }
        }

        fn rereads_in(&self, _now: DateTime<Utc>) -> Option<Duration> {
            self.rereads_in
        }

        fn pressed(&mut self) -> bool {
            self.pressed_after.push(self.applied.len());
            self.pressing_changes
        }

        fn apply(&mut self, action: Action) -> bool {
            self.applied.push(action);
            !(action == Action::ShowBead && self.not_a_bead)
        }

        fn scroll(&mut self, motion: Motion) -> bool {
            self.scrolled.push(motion);
            true
        }

        fn bead_still_shown(&self) -> bool {
            !(self.collection_moves_the_selection && self.collected > 0)
        }

        fn follow(&mut self) -> bool {
            self.followed += 1;
            self.on_a_reference
        }

        fn retrace(&mut self) -> bool {
            self.retraced += 1;
            self.somewhere_to_go_back_to
        }

        fn clicked(&mut self, row: u16) -> bool {
            self.clicked.push(row);
            !self.nothing_under_the_pointer
        }

        fn clicked_bead(&mut self, row: u16) -> Landed {
            self.clicked_bead.push(row);
            self.lands_on.unwrap_or(Landed::Nothing)
        }

        fn draw(&mut self, showing: Showing, now: DateTime<Utc>) -> anyhow::Result<()> {
            self.drawn_at.push(now);
            self.showing.push(showing);
            Ok(())
        }
    }

    /// An event source holding everything the loop will see, in order.
    /// What is outstanding with no window in front of it, which is what
    /// every test about something other than the window wants: the read goes
    /// the moment the loop next looks, so a test that drives the loop over a
    /// fixed list of events and stops sees it sent.
    ///
    /// A window of nothing is a setting `bdi` could be run at rather than a
    /// stand-in: `WINDOW` is the only value production uses, and the tests
    /// that are about the window say so by naming one.
    fn at_once() -> Outstanding {
        Outstanding::waiting(PATIENCE, TimeDelta::zero())
    }

    /// The projects that ask for themselves, where the test is not about
    /// them. Nothing is armed at the start of a run either — the first read
    /// arms every project when it comes back.
    fn nothing_armed() -> Vec<Armed> {
        Vec::new()
    }

    /// A run with no config file to look at again. What the loop does with
    /// one is `Reload`'s own tests and the pty test that drives the binary;
    /// what these say is that the loop reaches everything else without one.
    fn nothing_watched() -> Option<Reload> {
        None
    }

    /// The reads that reached the collector, in order.
    ///
    /// Every test but the ones about a config the reader has written is
    /// asking about these, and a run with no file to look at puts nothing
    /// else on the channel — so a `Reloaded` reaching here is a loop that
    /// told the collector about a config nobody wrote.
    fn reads(asked: impl IntoIterator<Item = Asked>) -> Vec<Wanted> {
        asked.into_iter().map(read).collect()
    }

    /// One of them.
    fn read(asked: Asked) -> Wanted {
        match asked {
            Asked::Read(wanted) => wanted,
            Asked::Reloaded(cfg) => {
                panic!("nobody wrote a config, and the collector was sent {cfg:?}")
            }
        }
    }

    /// One poll per project the config names, at the interval these tests
    /// wait out, and none for a project whose own key says it does not poll
    /// — which is what a run makes of the config where the command line said
    /// nothing about polling.
    ///
    /// A run with nothing to look at never asks for this; the tests about a
    /// config the reader has written are the ones it answers.
    fn polling_every_interval() -> Arming {
        Box::new(|cfg: &Config| {
            cfg.read()
                .map(|project| {
                    Armed::polling(project.name.clone(), project.poll.then_some(AN_INTERVAL))
                })
                .collect()
        })
    }

    fn waiting(events: Vec<Event>) -> Receiver<Event> {
        let (to, from) = mpsc::channel();
        for event in events {
            to.send(event)
                .expect("the loop's end of the channel is open");
        }
        from
    }

    fn typing(keys: [KeyEvent; 4]) -> Receiver<Event> {
        waiting(keys.into_iter().map(Event::Key).collect())
    }

    /// Everything the loop will see, on a channel held open until the loop
    /// has gone round `passes` times and closed then.
    ///
    /// The counterpart to `waiting` for the tests about what the loop does
    /// when nothing is happening: a list that ran out would close the channel
    /// and end the run before the first deadline was up. What holds it open
    /// is the loop saying it has gone round, so how many passes it gets is
    /// the test's to decide. A window of wall clock instead asks the machine
    /// how much work fits in one, and a machine with several of these running
    /// at once has answered "one pass in a hundred milliseconds" — which
    /// fails a test wanting more, with nothing wrong with the loop.
    ///
    /// `A_MOMENT` bounds a loop that has stopped going round at all, so that
    /// one fails on the count the test asserts rather than hanging the suite.
    /// There is nothing to join: the thread's last act is the drop that ends
    /// the run.
    fn going_round(view: &mut Recorder, passes: usize, first: Vec<Event>) -> Receiver<Event> {
        let (to, from) = mpsc::channel();
        for event in first {
            to.send(event)
                .expect("the loop's end of the channel is open");
        }
        let (went_round, rounds) = mpsc::channel();
        view.went_round = Some(went_round);
        thread::spawn(move || {
            for _ in 0..passes {
                if rounds.recv_timeout(A_MOMENT).is_err() {
                    break;
                }
            }
            drop(to);
        });
        from
    }

    #[test]
    fn a_keypress_reaches_the_view_as_the_action_it_is_bound_to() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let events = typing([
            key(KeyCode::Char('j')),
            key(KeyCode::Char(' ')),
            key(KeyCode::Char('q')),
            key(KeyCode::Char('k')),
        ]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(
            view.applied,
            [Action::Move(Motion::NextRow), Action::ToggleFold],
            "q ends the loop, so nothing after it is applied"
        );
    }

    /// `?` puts the bindings up and holds them there, and the next keystroke
    /// takes them away whatever it is — including one nothing is bound to,
    /// which is why the dismissal cannot live in the mapping.
    #[test]
    fn a_question_mark_shows_the_bindings_and_the_next_key_takes_them_away() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let events = typing([
            key(KeyCode::Char('?')),
            key(KeyCode::Char('z')),
            key(KeyCode::Char('?')),
            key(KeyCode::Char('j')),
        ]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(
            view.showing,
            [
                Showing::Forest,
                Showing::Bindings,
                Showing::Forest,
                Showing::Bindings,
                Showing::Forest,
            ]
        );
        assert!(
            view.applied.is_empty(),
            "j closed the bindings rather than moving the selection: {:?}",
            view.applied
        );
    }

    /// While the bindings are up, `q` is a key like any other: it puts them
    /// away. The forest a reader was looking at is still there to quit from.
    #[test]
    fn quitting_from_the_bindings_takes_two_presses_and_the_first_is_not_lost() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let events = typing([
            key(KeyCode::Char('?')),
            key(KeyCode::Char('q')),
            key(KeyCode::Char('q')),
            key(KeyCode::Char('j')),
        ]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(
            view.showing,
            [Showing::Forest, Showing::Bindings, Showing::Forest],
            "the second q ended the loop, so nothing was drawn after it"
        );
    }

    /// A collection landing behind the bindings is taken, and the bindings
    /// stay up: a view that vanished under the refresh tick would be one a
    /// reader could not finish reading.
    #[test]
    fn a_collection_arriving_behind_the_bindings_leaves_them_up() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Key(key(KeyCode::Char('?'))),
            Event::Collected(Box::new(a_snapshot())),
        ]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(view.collected, 1);
        assert_eq!(
            view.showing,
            [Showing::Forest, Showing::Bindings, Showing::Bindings]
        );
    }

    /// Enter puts the selected bead up and Esc takes it away, and the row it
    /// was opened from is untouched: nothing but the opening reached the
    /// forest, so the reader is back where they were.
    #[test]
    fn enter_shows_the_bead_and_esc_goes_back_to_the_forest() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let events = typing([
            key(KeyCode::Enter),
            key(KeyCode::Esc),
            key(KeyCode::Char('j')),
            key(KeyCode::Char('q')),
        ]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(
            view.showing,
            [
                Showing::Forest,
                Showing::Bead,
                Showing::Forest,
                Showing::Forest
            ]
        );
        assert_eq!(
            view.applied,
            [Action::ShowBead, Action::Move(Motion::NextRow)],
            "j after Esc moved the selection, so the forest was back"
        );
    }

    /// A row with no bead on it has nothing to show, so Enter there puts no
    /// view up and the next key reaches the forest as it always did.
    #[test]
    fn enter_on_a_row_that_is_not_a_bead_leaves_the_forest_up() {
        let mut view = Recorder {
            not_a_bead: true,
            ..Recorder::default()
        };
        let (ask, _asked) = mpsc::channel();
        let events = typing([
            key(KeyCode::Enter),
            key(KeyCode::Char('j')),
            key(KeyCode::Char('q')),
            key(KeyCode::Char('k')),
        ]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(
            view.showing,
            [Showing::Forest, Showing::Forest],
            "nothing was drawn for the Enter, and j drew the forest"
        );
        assert_eq!(
            view.applied,
            [Action::ShowBead, Action::Move(Motion::NextRow)]
        );
        assert!(view.scrolled.is_empty(), "{:?}", view.scrolled);
    }

    /// While a bead is up a motion moves the bead, and the selection under
    /// it stays where the view was opened from.
    #[test]
    fn a_motion_in_the_bead_view_scrolls_the_bead_and_not_the_forest() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let events = typing([
            key(KeyCode::Enter),
            key(KeyCode::Char('j')),
            control('d'),
            key(KeyCode::Esc),
        ]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(view.scrolled, [Motion::NextRow, Motion::HalfScreenDown]);
        assert_eq!(
            view.applied,
            [Action::ShowBead],
            "no motion reached the forest"
        );
    }

    /// The view is the hub: Enter and `f` from inside it focus the bead's
    /// pane, `y` copies its id, and the view stays up for the reader to come
    /// back to.
    #[test]
    fn enter_f_and_y_in_the_bead_view_act_on_the_bead_and_leave_the_view_up() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Key(key(KeyCode::Enter)),
            Event::Key(key(KeyCode::Enter)),
            Event::Key(key(KeyCode::Char('f'))),
            Event::Key(key(KeyCode::Char('y'))),
            Event::Key(key(KeyCode::Char('q'))),
        ]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(
            view.applied,
            [
                Action::ShowBead,
                Action::Focus,
                Action::Focus,
                Action::CopyId
            ]
        );
        assert_eq!(
            view.showing,
            [
                Showing::Forest,
                Showing::Bead,
                Showing::Bead,
                Showing::Bead,
                Showing::Bead,
                Showing::Forest
            ]
        );
    }

    /// `q` in the bead view goes back, as it does from the bindings: the
    /// forest a reader was looking at is still there to quit from.
    /// `Enter` in the bead view asks the view to follow first, and focuses the
    /// pane only where there was nothing to follow. One key, two meanings,
    /// decided by what the window is on rather than by a second binding.
    #[test]
    fn enter_in_the_bead_view_focuses_the_pane_where_there_is_nothing_to_follow() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Key(key(KeyCode::Enter)),
            Event::Key(key(KeyCode::Enter)),
        ]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(view.followed, 1, "the follow was not tried");
        assert_eq!(view.applied, [Action::ShowBead, Action::Focus]);
    }

    #[test]
    fn enter_on_a_bead_the_window_names_follows_it_and_focuses_nothing() {
        let mut view = Recorder {
            on_a_reference: true,
            ..Recorder::default()
        };
        let (ask, _asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Key(key(KeyCode::Enter)),
            Event::Key(key(KeyCode::Enter)),
        ]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(view.followed, 1);
        assert_eq!(
            view.applied,
            [Action::ShowBead],
            "the pane was focused on a press that went somewhere"
        );
        assert_eq!(
            view.showing,
            [Showing::Forest, Showing::Bead, Showing::Bead],
            "following a bead left the view"
        );
    }

    /// `Esc` goes back a bead where there is one, and leaves for the forest
    /// only when there is not — the browser's back, which is what `back`
    /// already means to everyone.
    #[test]
    fn esc_goes_back_a_bead_before_it_leaves_the_view() {
        let mut view = Recorder {
            somewhere_to_go_back_to: true,
            ..Recorder::default()
        };
        let (ask, _asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Key(key(KeyCode::Enter)),
            Event::Key(key(KeyCode::Esc)),
        ]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(view.retraced, 1);
        assert_eq!(
            view.showing,
            [Showing::Forest, Showing::Bead, Showing::Bead],
            "Esc left the view with a bead still to go back to"
        );
    }

    #[test]
    fn esc_leaves_the_view_where_there_is_no_bead_to_go_back_to() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Key(key(KeyCode::Enter)),
            Event::Key(key(KeyCode::Esc)),
        ]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(view.retraced, 1, "the way back was not tried");
        assert_eq!(
            view.showing,
            [Showing::Forest, Showing::Bead, Showing::Forest]
        );
    }

    /// `q` is the whole way out rather than one step of it, which is what
    /// makes it worth having beside `Esc`: a reader three beads deep gets
    /// back to the forest they were reading without pressing four times.
    #[test]
    fn q_leaves_the_bead_view_whatever_there_is_to_go_back_to() {
        let mut view = Recorder {
            somewhere_to_go_back_to: true,
            ..Recorder::default()
        };
        let (ask, _asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Key(key(KeyCode::Enter)),
            Event::Key(key(KeyCode::Char('q'))),
        ]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(view.retraced, 0, "q asked for a step back");
        assert_eq!(
            view.showing,
            [Showing::Forest, Showing::Bead, Showing::Forest]
        );
    }

    /// `Tab` reaches the view only from the bead view. In the forest there is
    /// no window naming beads, so it is a key that does nothing rather than
    /// one that moves the selection.
    #[test]
    fn tab_steps_the_ring_in_the_bead_view_and_does_nothing_in_the_forest() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let events = typing([
            key(KeyCode::Tab),
            key(KeyCode::Enter),
            key(KeyCode::Tab),
            key(KeyCode::Tab),
        ]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(
            view.applied,
            [Action::ShowBead, Action::NextRelated, Action::NextRelated],
            "the Tab pressed in the forest reached the view"
        );
    }

    #[test]
    fn quitting_from_the_bead_view_takes_two_presses_and_the_first_goes_back() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let events = typing([
            key(KeyCode::Enter),
            key(KeyCode::Char('q')),
            key(KeyCode::Char('q')),
            key(KeyCode::Char('j')),
        ]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(
            view.showing,
            [Showing::Forest, Showing::Bead, Showing::Forest],
            "the second q ended the loop, so nothing was drawn after it"
        );
        assert_eq!(view.applied, [Action::ShowBead]);
    }

    /// A collection landing behind the bead view is taken, and the view
    /// stays up: one that vanished under the refresh tick would be one a
    /// reader could not finish reading.
    #[test]
    fn a_collection_arriving_behind_the_bead_view_leaves_it_up() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Key(key(KeyCode::Enter)),
            Event::Collected(Box::new(a_snapshot())),
        ]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(view.collected, 1);
        assert_eq!(
            view.showing,
            [Showing::Forest, Showing::Bead, Showing::Bead]
        );
    }

    /// `codex review` on this change, and it is right: a collection that
    /// moved the selection off the bead left the loop in the bead view with
    /// nothing drawn in it, so a forest that looked ordinary ignored every
    /// motion and answered `q` by leaving a view nobody could see. The view
    /// comes down instead, and the next key reaches the forest.
    #[test]
    fn a_collection_that_moves_the_selection_off_the_bead_takes_the_view_down() {
        let mut view = Recorder {
            collection_moves_the_selection: true,
            ..Recorder::default()
        };
        let (ask, _asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Key(key(KeyCode::Enter)),
            Event::Collected(Box::new(a_snapshot())),
            Event::Key(key(KeyCode::Char('j'))),
        ]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(
            view.showing,
            [
                Showing::Forest,
                Showing::Bead,
                Showing::Forest,
                Showing::Forest
            ]
        );
        assert_eq!(
            view.applied,
            [Action::ShowBead, Action::Move(Motion::NextRow)],
            "j after the collection moved the selection rather than the bead"
        );
        assert!(view.scrolled.is_empty(), "{:?}", view.scrolled);
    }

    /// A notch over the bead view moves the bead as a key would, rather than
    /// the selection under it.
    #[test]
    fn a_notch_over_the_bead_view_scrolls_it() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Key(key(KeyCode::Enter)),
            Event::Scrolled(Motion::NextRow),
            Event::Key(key(KeyCode::Char('q'))),
        ]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(view.scrolled, [Motion::NextRow]);
        assert!(
            view.applied
                .iter()
                .all(|action| *action == Action::ShowBead),
            "the notch moved the selection: {:?}",
            view.applied
        );
    }

    /// A click over the bead view goes to the window rather than to the
    /// forest, whatever it lands on: the rows under the pointer are the rows
    /// the reader is looking at, which is what the bindings window cannot
    /// say.
    #[test]
    fn a_click_over_the_bead_view_asks_the_window_and_not_the_forest() {
        let view = a_click_over_the_bead_view(Landed::Nothing);

        assert_eq!(view.clicked_bead, [3]);
        assert!(
            view.clicked.is_empty(),
            "the click reached the forest as well: {:?}",
            view.clicked
        );
    }

    /// A click the window went to a bead on leaves the window up, and the
    /// screen is drawn again for the bead it went to.
    #[test]
    fn a_click_that_followed_a_reference_leaves_the_window_up() {
        let view = a_click_over_the_bead_view(Landed::Followed);

        assert_eq!(
            view.showing,
            [Showing::Forest, Showing::Bead, Showing::Bead, Showing::Bead]
        );
        assert_eq!(view.scrolled, [Motion::NextRow], "{:?}", view.applied);
    }

    /// A click on the page that went nowhere leaves the window up and the
    /// screen unredrawn: nothing about it moved.
    #[test]
    fn a_click_that_went_nowhere_leaves_the_window_up_and_draws_nothing() {
        let view = a_click_over_the_bead_view(Landed::Nothing);

        assert_eq!(
            view.showing,
            [Showing::Forest, Showing::Bead, Showing::Bead]
        );
        assert_eq!(view.scrolled, [Motion::NextRow], "{:?}", view.applied);
    }

    /// And a click off the page takes the window away, which is what a click
    /// over the bead view has always done. The notch behind it lands on the
    /// forest, which is what says the window has really gone rather than been
    /// drawn over.
    #[test]
    fn a_click_off_the_page_takes_the_window_away() {
        let view = a_click_over_the_bead_view(Landed::Away);

        assert_eq!(
            view.showing,
            [
                Showing::Forest,
                Showing::Bead,
                Showing::Forest,
                Showing::Forest
            ]
        );
        assert!(view.scrolled.is_empty(), "{:?}", view.scrolled);
        assert_eq!(
            view.applied,
            [Action::ShowBead, Action::Move(Motion::NextRow)]
        );
    }

    /// Show a bead, click a row over it and turn the wheel, with the window
    /// answering that click the way the test says. The notch is what asks
    /// where the loop thinks it is afterwards: over a window it moves the
    /// bead, and over the forest it moves the selection.
    fn a_click_over_the_bead_view(lands_on: Landed) -> Recorder {
        let mut view = Recorder {
            lands_on: Some(lands_on),
            ..Recorder::default()
        };
        let (ask, _asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Key(key(KeyCode::Enter)),
            Event::Clicked(3),
            Event::Scrolled(Motion::NextRow),
        ]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");
        view
    }

    #[test]
    fn a_forced_refresh_asks_for_a_collection_and_the_loop_reads_on() {
        let mut view = Recorder::default();
        let (ask, asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Key(control('r')),
            Event::Key(key(KeyCode::Char('j'))),
        ]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(reads(asked.try_iter()), [Wanted::Everything]);
        assert_eq!(
            view.applied,
            [Action::Move(Motion::NextRow)],
            "the loop read on rather than waiting for the collection"
        );
    }

    /// The bead this came from: `^R` did nothing a reader could see. The
    /// keystroke asked for a collection and then evaluated to no change at
    /// all, so `view.draw` was never called and the screen stayed
    /// byte-identical for the seconds the collection took.
    #[test]
    fn the_refresh_key_puts_something_on_the_screen_before_the_snapshot_lands() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let events = waiting(vec![Event::Key(control('r'))]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(
            view.collecting(),
            [vec![Wanted::Everything]],
            "the view was told a collection began, and over which projects"
        );
        assert_eq!(
            view.drawn(),
            2,
            "the first frame, and one for the keystroke"
        );
    }

    /// A window long enough that nothing in a test falls out of it, so a read
    /// is asked for and demonstrably not sent.
    const A_LONG_WINDOW: TimeDelta = TimeDelta::seconds(30);

    /// What is outstanding, gathering for `window` before anything leaves.
    fn gathering(window: TimeDelta) -> Outstanding {
        Outstanding::waiting(PATIENCE, window)
    }

    /// The property the debounce must not cost: the screen says a read is
    /// coming at the instant it was asked for, not at the instant it goes.
    ///
    /// `^R` did nothing a reader could see once already — the bead two tests
    /// up — and a window is exactly the thing that would do it again. The two
    /// halves are asserted together on purpose: `view.collecting` alone
    /// passes under any window at all, because the marking path never
    /// observes the send, so it discriminates nothing on its own.
    #[test]
    fn a_read_is_said_on_the_screen_before_its_window_lets_it_go() {
        let mut view = Recorder::default();
        let (ask, asked) = mpsc::channel();
        let events = waiting(vec![Event::Key(control('r'))]);

        drive(
            &mut view,
            &events,
            &ask,
            gathering(A_LONG_WINDOW),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(
            view.collecting(),
            [vec![Wanted::Everything]],
            "the mark belongs to the ask"
        );
        assert!(
            asked.try_iter().next().is_none(),
            "and the read had not gone: marked before sent, not after"
        );
    }

    /// What the window is for. A producer with nothing to lose by talking
    /// says the same thing many times in a moment, and each saying used to be
    /// a read of its own — absorbed only once one was already in flight.
    #[test]
    fn a_burst_about_one_project_inside_the_window_costs_one_read() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let events = waiting((0..100).map(|_| Event::Changed(atlas())).collect());

        drive(
            &mut view,
            &events,
            &ask,
            gathering(A_LONG_WINDOW),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(
            view.collecting(),
            [vec![atlas()]],
            "a hundred messages, one read, and the screen told once"
        );
    }

    /// The loop sleeps until the first of three deadlines, and a screen that
    /// nothing can stale with no read waiting and nothing armed does not sleep
    /// at all — it waits on the channel until something happens.
    ///
    /// Asserted rather than left to the loop, because a deadline of nothing at
    /// all turns the loop into a spin: it wakes, finds nothing due, redraws and
    /// waits nothing again. That costs a processor and produces no test
    /// failure, only a slow suite.
    #[test]
    fn a_loop_with_nothing_due_sleeps_until_something_happens() {
        assert_eq!(
            sleeps_for(
                &Recorder::default(),
                &at_once(),
                &nothing_armed(),
                None,
                Utc::now(),
                Utc::now()
            ),
            None
        );
    }

    #[test]
    fn the_loop_sleeps_until_the_soonest_of_what_it_is_waiting_for() {
        let now = Utc::now();
        let mut outstanding = gathering(A_LONG_WINDOW);
        outstanding.ask(atlas(), now);
        let mut sooner = Armed::polling("ferry".to_string(), Some(AN_INTERVAL));
        sooner.came_back(&ferry(), now);

        assert_eq!(
            sleeps_for(
                &Recorder::default(),
                &outstanding,
                &[sooner],
                None,
                now,
                now
            ),
            Some(AN_INTERVAL),
            "the poll comes round long before the window is out"
        );
        assert_eq!(
            sleeps_for(
                &Recorder::default(),
                &outstanding,
                &nothing_armed(),
                None,
                now,
                now
            ),
            A_LONG_WINDOW.to_std().ok(),
            "and with nothing armed, the window is what is left to wait for"
        );
    }

    /// The band's next read of its pane is the fourth deadline, and it is
    /// the view's to say: the loop knows nothing about panes.
    #[test]
    fn the_loop_sleeps_until_the_band_is_due_to_read_its_pane_again() {
        let now = Utc::now();
        let mut outstanding = gathering(A_LONG_WINDOW);
        outstanding.ask(atlas(), now);
        let view = Recorder {
            rereads_in: Some(AN_INTERVAL),
            ..Recorder::default()
        };

        assert_eq!(
            sleeps_for(&view, &outstanding, &nothing_armed(), None, now, now),
            Some(AN_INTERVAL),
            "the pane falls due long before the window is out"
        );
    }

    /// The config check has to be in the deadline set, and nothing about a
    /// running `bdi` says so: a screen with an age on it goes stale every
    /// second, so the loop wakes far more often than the check falls due and
    /// looks at the file on somebody else's clock. A screen saying nothing
    /// that goes stale is where the term is the whole of it — a `bdi` with
    /// no age drawn, no read outstanding and no project polling would sleep
    /// until the reader touched a key, and a config edited under it would
    /// never be read.
    #[test]
    fn a_loop_with_nothing_else_due_still_wakes_to_look_at_the_config() {
        let now = Utc::now();
        let reload = a_config_looked_at_every(AN_INTERVAL, now);

        assert_eq!(
            sleeps_for(
                &Recorder::default(),
                &at_once(),
                &nothing_armed(),
                Some(&reload),
                now,
                now
            ),
            Some(AN_INTERVAL),
            "nothing else is going to wake it"
        );
    }

    /// A file `bdi` would look at every `every`, from `now`. Nothing here
    /// reads it — `checks_in` answers off the deadline alone — so the path
    /// names nothing and the parse is never reached.
    fn a_config_looked_at_every(every: Duration, now: DateTime<Utc>) -> Reload {
        Reload::watching(
            std::path::PathBuf::from("/a/config/nothing/here/opens"),
            every,
            crate::config::Config::naming(Vec::new()),
            Box::new(crate::config::Config::from_toml),
            now,
        )
    }

    const ATLAS_ALONE: &str = "[[projects]]\nname = \"atlas\"\npath = \"/srv/work/atlas\"\n";
    const FERRY_ALONE: &str = "[[projects]]\nname = \"ferry\"\npath = \"/srv/work/ferry\"\n";
    const ATLAS_AND_FERRY: &str = "[[projects]]\nname = \"atlas\"\npath = \"/srv/work/atlas\"\n\n[[projects]]\nname = \"ferry\"\npath = \"/srv/work/ferry\"\n";
    const ATLAS_TOLD_INSTEAD: &str = "[[projects]]\nname = \"atlas\"\npath = \"/srv/work/atlas\"\npoll = false\n\n[[projects]]\nname = \"ferry\"\npath = \"/srv/work/ferry\"\n";

    fn a_config(text: &str) -> Config {
        Config::from_toml(text).expect("the fixture parses")
    }

    /// A config file of this test's own, saying `written`, which `bdi` is
    /// working to `in_force` and looks at every `AN_INTERVAL` from now.
    ///
    /// A real file and a real parse, unlike `a_config_looked_at_every`: what
    /// these tests are about is the config a check produces reaching the rest
    /// of the run, so the check has to produce one.
    fn a_config_file_saying(named: &str, written: &str, in_force: &str) -> Reload {
        let path =
            std::env::temp_dir().join(format!("bdi-drive-{named}-{}.toml", std::process::id()));
        std::fs::write(&path, written).expect("the config is ours to write");
        Reload::watching(
            path,
            AN_INTERVAL,
            a_config(in_force),
            Box::new(Config::from_toml),
            Utc::now(),
        )
    }

    /// The collector is told the config the reader has written, and it is
    /// that one rather than the one the run started on.
    ///
    /// The order the two arrive in is asserted below and is not what this
    /// discriminates *while the ask and the read share one ordered channel*.
    /// No edit to `looked_at` puts the read first today, because the ask only
    /// queues and the send is a pass later. Take that construction away — a
    /// second channel, a queue between them — and the assertion is the thing
    /// that notices, which is why it is worth keeping: a collection carries
    /// no config with it, so a read that overtook one would draw a whole
    /// screen read under the file the reader has just replaced.
    #[test]
    fn the_collector_is_told_the_config_the_reader_has_written() {
        let mut view = Recorder::default();
        let (ask, asked) = mpsc::channel();
        let events = going_round(&mut view, A_FEW_PASSES, Vec::new());

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            Some(a_config_file_saying("gained", ATLAS_AND_FERRY, ATLAS_ALONE)),
        )
        .expect("the loop runs");

        assert_eq!(
            asked.try_iter().collect::<Vec<_>>(),
            [
                Asked::Reloaded(Box::new(a_config(ATLAS_AND_FERRY))),
                Asked::Read(Wanted::Everything)
            ],
            "the collector was handed the config the reader wrote, and then \
             asked to read every project under it"
        );
    }

    /// The whole sequence a config the reader has written sets off, driven
    /// until the first project asks for itself: `armed` is what the run was
    /// polling, `written` is what the reader has left in the file, and what
    /// comes back is everything that reached the collector.
    ///
    /// It has to be the whole sequence rather than the set of polls. Nothing
    /// the reload does arms a project — a project the file names is disarmed
    /// exactly as every project is at startup — and what arms them is the
    /// read the reload asked for coming back, so the answer to *which
    /// projects poll* is one round trip past the edit.
    fn until_a_project_asks_for_itself(
        named: &str,
        armed: Vec<Armed>,
        in_force: &str,
        written: &str,
    ) -> Vec<Asked> {
        let mut view = Recorder::default();
        let (ask, asked) = mpsc::channel();
        let (send, events) = mpsc::channel();

        let holding = thread::spawn(move || {
            let mut reached = Vec::new();
            while let Ok(one) = asked.recv_timeout(A_MOMENT) {
                let arms_them = one == Asked::Read(Wanted::Everything);
                let polled = matches!(one, Asked::Read(Wanted::Project(_)));
                reached.push(one);
                if arms_them {
                    send.send(Event::Collected(Box::new(a_snapshot())))
                        .expect("the loop's end of the channel is open");
                }
                if polled {
                    break;
                }
            }
            drop(send);
            reached
        });

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            armed,
            &polling_every_interval(),
            Some(a_config_file_saying(named, written, in_force)),
        )
        .expect("the loop runs");

        holding.join().expect("the thread ran")
    }

    /// The projects that poll are the projects the config now names: the one
    /// the file gained asks for itself, and the one it lost stops asking.
    #[test]
    fn the_projects_that_poll_are_the_ones_the_config_now_names() {
        let reached = until_a_project_asks_for_itself(
            "swapped",
            vec![Armed::polling("atlas".to_string(), Some(AN_INTERVAL))],
            ATLAS_ALONE,
            FERRY_ALONE,
        );

        assert_eq!(
            reached.last(),
            Some(&Asked::Read(ferry())),
            "the project the config gained asked for itself once its read \
             came back"
        );
        assert!(
            !reached.contains(&Asked::Read(atlas())),
            "and the project the config lost asked for nothing: {reached:?}"
        );
    }

    /// And a project the reader has stopped polling stops polling, though the
    /// file still names it and `bdi` goes on reading it.
    ///
    /// `poll` is one of the settings a project's own entry carries, and a
    /// reader turning it off has said something about the run they are
    /// looking at rather than about the next one: they have deployed
    /// something that reports this project's changes and want to see whether
    /// it is working. A poll that outlived the edit would be `bdi` quietly
    /// covering for the very producer they are testing.
    #[test]
    fn a_project_the_reader_has_stopped_polling_asks_no_more() {
        let reached = until_a_project_asks_for_itself(
            "told-instead",
            vec![
                Armed::polling("atlas".to_string(), Some(AN_INTERVAL)),
                Armed::polling("ferry".to_string(), Some(AN_INTERVAL)),
            ],
            ATLAS_AND_FERRY,
            ATLAS_TOLD_INSTEAD,
        );

        assert_eq!(
            reached.last(),
            Some(&Asked::Read(ferry())),
            "the project the edit left alone asked for itself"
        );
        assert!(
            !reached.contains(&Asked::Read(atlas())),
            "and the one it stopped polling asked for nothing, though it is \
             still read: {reached:?}"
        );
    }

    /// When the band's interval is up the loop asks it to read its pane
    /// again, and not before: the band is what decides whether anything is
    /// due, so the loop asks on every wake and the first wake is the
    /// interval.
    #[test]
    fn the_band_is_asked_to_read_its_pane_again_once_its_interval_is_up() {
        let mut view = Recorder {
            rereads_in: Some(AN_INTERVAL),
            ..Recorder::default()
        };
        let (ask, _asked) = mpsc::channel();
        let events = going_round(&mut view, 1, Vec::new());
        let started = Utc::now();

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        let first = view
            .reread_at
            .first()
            .expect("the loop went round before it was let go");
        assert!(
            *first - started >= TimeDelta::from_std(AN_INTERVAL).expect("a short interval"),
            "the band was asked at {first}, before its interval was up from {started}"
        );
    }

    /// A wake for the band's deadline alone draws nothing: the ask changes
    /// nothing on the screen, and the answer, when it lands, says for itself
    /// whether it did. A loop that redrew for every one of these would write
    /// to the terminal four times a second for as long as a pane was
    /// selected, whether or not the pane had said anything.
    #[test]
    fn a_wake_for_the_bands_read_alone_draws_nothing() {
        let mut view = Recorder {
            rereads_in: Some(AN_INTERVAL),
            ..Recorder::default()
        };
        let (ask, _asked) = mpsc::channel();
        let events = going_round(&mut view, A_FEW_PASSES, Vec::new());

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert!(
            view.reread_at.len() >= A_FEW_PASSES,
            "the loop was let go after {} passes, so the frames below say nothing about repeated wakes",
            view.reread_at.len()
        );
        assert_eq!(view.drawn(), 1, "the first frame, and no other");
    }

    /// The view says how long its frame holds from the instant it was drawn,
    /// and the loop sleeps from now — so what it sleeps is what is left of
    /// the frame, not the whole of it again. A wake that draws nothing is
    /// where the two instants part: a key bound to nothing at 30 ms into an
    /// 80 ms frame leaves 50 ms, and one after the frame has run out leaves
    /// nothing, which is a wake rather than a full frame more of a stale
    /// screen.
    #[test]
    fn a_deadline_decided_after_the_frame_is_what_is_left_of_it() {
        let now = Utc::now();
        let mut view = Recorder::default();
        View::collecting(&mut view, &[reading(atlas(), now)]);

        let drawn_at = now - TimeDelta::milliseconds(30);
        assert_eq!(
            sleeps_for(&view, &at_once(), &nothing_armed(), None, drawn_at, now),
            Some(phrase::FRAME - Duration::from_millis(30)),
        );
        let drawn_at = now - TimeDelta::milliseconds(100);
        assert_eq!(
            sleeps_for(&view, &at_once(), &nothing_armed(), None, drawn_at, now),
            Some(Duration::ZERO),
            "the frame ran out before the loop asked, so it wakes at once"
        );
    }

    /// A poll coming due beside an event that changed nothing still reaches
    /// the screen.
    ///
    /// The narrow case the loop's `told` exists for. A poll usually arrives as
    /// its own deadline, and the loop redraws for a deadline whatever else it
    /// finds — so the only way a due poll can go undrawn is by falling in the
    /// same pass as an event the view had nothing to say about.
    #[test]
    fn a_poll_falling_beside_a_keystroke_that_changed_nothing_still_redraws() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let events = waiting(vec![Event::Key(key(KeyCode::Char('x')))]);
        let mut overdue = Armed::polling("atlas".to_string(), Some(AN_INTERVAL));
        overdue.came_back(&atlas(), Utc::now() - TimeDelta::hours(1));

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            vec![overdue],
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(
            view.collecting(),
            [vec![atlas()]],
            "the poll asked, and said so"
        );
        assert_eq!(
            view.drawn(),
            2,
            "the first frame, and one for the read the keystroke landed beside"
        );
    }

    /// One read comes back for a project that polls, and the loop is driven
    /// until that project's own ask reaches the collector.
    ///
    /// The events channel is held open by a thread rather than drained from a
    /// list, because what is being waited on is the loop's own deadline: a
    /// list that ran out would close the channel and end the run before the
    /// interval was up.
    fn until_atlas_asks_for_itself(view: &mut Recorder) -> Option<Wanted> {
        let (ask, asked) = mpsc::channel();
        let (send, events) = mpsc::channel();
        send.send(Event::Collected(Box::new(a_snapshot())))
            .expect("the loop's end of the channel is open");
        let holding = thread::spawn(move || {
            let asked_for = asked.recv_timeout(A_MOMENT);
            drop(send);
            asked_for
        });

        drive(
            view,
            &events,
            &ask,
            started(),
            vec![Armed::polling("atlas".to_string(), Some(AN_INTERVAL))],
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        holding.join().expect("the thread ran").ok().map(read)
    }

    /// The invariant the refresh path rests on, and neither half can see it
    /// alone: a project always has a read outstanding or an ask armed, so a
    /// project whose reads keep coming back never stops being read.
    ///
    /// Nothing but a read coming back arms one. This is the arming half; the
    /// test below it is the half that must not happen.
    #[test]
    fn a_project_asks_for_itself_again_once_its_read_comes_back() {
        let mut view = Recorder::default();

        let asked_for = until_atlas_asks_for_itself(&mut view);

        assert_eq!(
            asked_for,
            Some(atlas()),
            "the read that came back armed atlas, and its interval came round"
        );
    }

    /// And the reader is told, exactly as they are told about a read a
    /// message or `^R` asked for. A project that asked for itself and said
    /// nothing about it is the bead `^R` came from over again: rows replaced
    /// under a reader with nothing to mark that they were about to be.
    ///
    /// The case is live rather than theoretical, which is why this is a test
    /// rather than a line of `asks_for_what_is_due`'s doc: the poll is the
    /// one way of asking that `a_collection_nobody_asked_for_is_still_said_on
    /// _the_screen` cannot reach, because a poll does not arrive as an
    /// `Event::Changed`.
    #[test]
    fn a_project_that_asked_for_itself_says_so_on_the_screen() {
        let mut view = Recorder::default();

        until_atlas_asks_for_itself(&mut view);

        assert_eq!(
            view.collecting().last(),
            Some(&vec![atlas()]),
            "the poll's own read is said on the screen like any other"
        );
    }

    /// The mirror, and it is the hazard arming-from-completion deliberately
    /// keeps: a read that never comes back arms nothing, so a hung tracker
    /// goes quiet here rather than piling reads up behind itself. What says
    /// so on the screen is the unanswered mark on the read still standing.
    ///
    /// Two things hold this and only one of them is `Armed`'s, which is worth
    /// knowing before reading a pass here as proof of either. Measured by
    /// breaking each: taking the disarm out of `Armed::asks` leaves this
    /// green, because `queue` drops an ask for a project that already has one
    /// outstanding, so a free-running poll reaches the collector once
    /// whatever it does here — `a_project_whose_read_never_comes_back_asks_no_more`
    /// in `armed` is what guards the disarm. What this one guards is the
    /// loop: that arming happens on the read coming back, and that no second
    /// read is ever sent for an ask nobody answered. Emptying
    /// `Armed::came_back` fails it.
    ///
    /// The window it is about opens *after* the one ask the loop is entitled
    /// to, and the loop is held open until the window has opened rather than
    /// for a length of wall clock. The ask goes out on the pass its interval
    /// comes round, so a run that got that pass and no other satisfies the
    /// assertion below having given the loop no opportunity to misbehave —
    /// which is a pass, not a flake, and so nothing draws attention to it.
    /// What the loop is let go on instead is a pass a whole interval past
    /// that ask: atlas came due again with the ask still outstanding, and
    /// `Armed::asks` declined it.
    #[test]
    fn a_project_whose_ask_is_never_answered_asks_no_more() {
        let mut view = Recorder::default();
        let (ask, asked) = mpsc::channel();
        let (send, events) = mpsc::channel();
        let (went_round, rounds) = mpsc::channel();
        view.went_round = Some(went_round);
        send.send(Event::Collected(Box::new(a_snapshot())))
            .expect("the loop's end of the channel is open");
        let holding = thread::spawn(move || {
            let first = asked.recv_timeout(A_MOMENT).ok();
            let comes_due_again = Utc::now() + an_interval();
            while rounds
                .recv_timeout(A_MOMENT)
                .is_ok_and(|looked_at| looked_at < comes_due_again)
            {}
            drop(send);
            (first, asked)
        });

        drive(
            &mut view,
            &events,
            &ask,
            started(),
            vec![Armed::polling("atlas".to_string(), Some(AN_INTERVAL))],
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        let (first, asked) = holding.join().expect("the thread ran");
        let reached_the_collector = reads(first.into_iter().chain(asked.try_iter()));
        let asked_at = *view.asked_at().first().expect("atlas asked once");
        let last_looked = *view.reread_at.last().expect("the loop went round");
        assert!(
            last_looked - asked_at >= an_interval(),
            "the loop last looked at what was due {} after the ask, so atlas \
             never came due again while that ask stood",
            last_looked - asked_at
        );
        assert_eq!(
            reached_the_collector,
            [atlas()],
            "atlas came due again and was declined: nothing had answered it"
        );
    }

    /// `AN_INTERVAL` measured the way the loop's own instants are.
    fn an_interval() -> TimeDelta {
        TimeDelta::from_std(AN_INTERVAL).expect("a short interval")
    }

    /// Short enough that a test waiting out several is not slow, and long
    /// enough that a loop doing its work in between is not racing it.
    const AN_INTERVAL: Duration = Duration::from_millis(20);

    /// Enough passes of the loop that what it does on a second one is being
    /// asked about, and few enough that waiting them out is quick.
    const A_FEW_PASSES: usize = 5;

    /// What is outstanding as a run has it when the loop starts: the first
    /// collection asked for and not yet come back. Nothing is armed until it
    /// does.
    fn started() -> Outstanding {
        let mut outstanding = at_once();
        outstanding.ask(Wanted::Everything, Utc::now());
        outstanding
    }

    /// The timer and the inbound socket start collections nobody pressed a
    /// key for, which is the case the bead calls worse: rows appear and
    /// statuses flip under a reader with nothing to mark that they did.
    #[test]
    fn a_collection_nobody_asked_for_is_still_said_on_the_screen() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let events = waiting(vec![Event::Changed(atlas())]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(view.collecting(), [vec![atlas()]]);
        assert_eq!(view.drawn(), 2);
    }

    // ---- the mark on a collecting project turns ---------------------------

    /// `bdi-7ao.43` drove the loop on a pty at `9cb221d`: with `bd` stubbed to
    /// hang, `bdi` wrote 64 bytes once, at +0.018s, and never again in 35
    /// seconds, while still answering keys in 21 ms. A collection reports
    /// nothing until it is done, so nothing was ever going to redraw the mark
    /// but a deadline of the loop's own.
    #[test]
    fn a_wait_gives_up_when_what_is_drawn_stops_being_true() {
        // An event well after the deadline, so a wait that kept none answers
        // with that instead of never answering at all: a loop that stopped
        // redrawing must fail here rather than hang.
        let (send, events) = mpsc::channel();
        thread::spawn(move || {
            thread::sleep(phrase::FRAME * 8);
            let _ = send.send(Event::Resize);
        });
        let began = Instant::now();

        let waited = wait(&events, Some(phrase::FRAME));

        assert!(matches!(waited, Some(Waited::Aged)));
        assert!(began.elapsed() >= phrase::FRAME, "{:?}", began.elapsed());
    }

    /// And only where something is going to go stale. A screen that says
    /// nothing time can falsify is woken by events alone: this returns the
    /// event rather than a deadline that would have come first.
    #[test]
    fn a_wait_on_a_screen_nothing_can_stale_sleeps_until_an_event() {
        let (send, events) = mpsc::channel();
        thread::spawn(move || {
            thread::sleep(phrase::FRAME * 3);
            let _ = send.send(Event::Resize);
        });

        assert!(matches!(
            wait(&events, None),
            Some(Waited::Event(Event::Resize))
        ));
    }

    /// The deadline running out is not an event: nothing has happened, so the
    /// loop draws the screen as it now is and asks the view nothing.
    #[test]
    fn a_frame_running_out_redraws_the_screen_and_nothing_else() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        // Two passes: the collection starting, and the frame it drew running
        // out. A window of wall clock would ask instead how many frames a
        // real 320 ms buys, which is the scheduler's answer and not the
        // loop's.
        let events = going_round(&mut view, 2, vec![Event::Changed(atlas())]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert!(
            view.drawn() > 2,
            "the first frame, the collection starting, and the mark turning: {}",
            view.drawn()
        );
        assert_eq!(view.collecting(), [vec![atlas()]], "no second collection");
        assert_eq!(view.applied, [], "and no action for a frame running out");
    }

    /// A request that arrived mid-collection is sent the moment the one in
    /// flight comes back, from inside `came_back` rather than from an event.
    /// A view told only about the collections an event started would say the
    /// screen was resting while a second one ran.
    #[test]
    fn the_collection_that_starts_behind_another_is_said_too() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Changed(atlas()),
            Event::Changed(ferry()),
            Event::Collected(Box::new(a_snapshot())),
        ]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(
            view.collecting(),
            [vec![atlas()], vec![atlas(), ferry()], vec![ferry()]],
            "the one the event asked for, then it with ferry waiting behind \
             it, then ferry alone once the first came back"
        );
        assert_eq!(view.collected, 1);
    }

    /// The bead: nothing on the collection path measures the wait, so a
    /// tracker hung for an hour is drawn as one asked half a second ago. The
    /// loop is where the ask happens, so the loop is where it is stamped.
    #[test]
    fn a_collection_is_stamped_with_the_moment_it_was_asked_for() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let before = Utc::now();
        let events = waiting(vec![Event::Changed(atlas())]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        let asked_at = view.asked_at();
        assert_eq!(asked_at.len(), 1, "one collection was started");
        assert!(
            (before..=Utc::now()).contains(&asked_at[0]),
            "the stamp is the moment of the ask: {before} .. {} .. {}",
            asked_at[0],
            Utc::now()
        );
    }

    /// The reason the stamp is taken at the ask and not inherited from
    /// whatever the project was last told about: a project reported for again
    /// while it is being read is a new wait, and one that carried the running
    /// collection's stamp would be drawn as having stopped answering the
    /// moment it began.
    #[test]
    fn a_project_reported_for_again_while_it_is_read_starts_a_wait_of_its_own() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Changed(atlas()),
            Event::Changed(atlas()),
            Event::Collected(Box::new(a_snapshot())),
        ]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(
            view.collecting(),
            [vec![atlas()], vec![atlas(), atlas()], vec![atlas()]],
            "the one in flight, then it with the second waiting, then the \
             second alone"
        );
        let told = &view.awaited[1];
        assert!(
            told[1].asked_at > told[0].asked_at,
            "the waiting one kept the running one's stamp: {told:?}"
        );
    }

    /// A collection coming back with nothing waiting behind it leaves the
    /// view resting. Saying otherwise would leave a mark turning on the
    /// project's line until the next refresh, over rows that had already
    /// arrived.
    ///
    /// The snapshot arriving is the end of the collection that produced it,
    /// and this is where that is said. The view is told what is in flight
    /// after every collection ends, so nothing it holds has to be unset by
    /// the arrival of rows.
    #[test]
    fn a_collection_with_nothing_behind_it_leaves_the_view_resting() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Changed(atlas()),
            Event::Collected(Box::new(a_snapshot())),
        ]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(
            view.collecting(),
            [vec![atlas()], vec![]],
            "the one the event asked for, and nothing once it landed"
        );
    }

    /// `bdi-7ao.81`: a request made while a collection is running is kept
    /// rather than sent, and the project it names has to say so. Nothing else
    /// on its line can — the rows under it are the last collection's and look
    /// exactly as they did, so a line that waited for the request to be sent
    /// would draw a resting mark over rows nothing is on its way to replace,
    /// for as long as the collection in front of it takes.
    #[test]
    fn a_request_waiting_its_turn_is_said_beside_the_one_in_flight() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();
        let events = waiting(vec![Event::Changed(atlas()), Event::Changed(ferry())]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(
            view.collecting(),
            [vec![atlas()], vec![atlas(), ferry()]],
            "the one in flight, then it and the one behind it"
        );
        assert_eq!(view.drawn(), 3, "and the screen changed for it");
    }

    #[test]
    fn only_one_collection_runs_at_a_time() {
        let mut view = Recorder::default();
        let (ask, asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Changed(atlas()),
            Event::Changed(ferry()),
            Event::Key(control('r')),
        ]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(
            reads(asked.try_iter()),
            [atlas()],
            "the two behind it wait for the one in flight to come back"
        );
    }

    /// What a refresh naming a project costs: the collection in flight is no
    /// longer reading what every other request would ask for, so a request
    /// dropped while it runs is a change lost — which is the failure the
    /// inbound channel exists to prevent.
    #[test]
    fn a_change_that_arrived_mid_collection_is_asked_for_when_it_comes_back() {
        let mut view = Recorder::default();
        let (ask, asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Changed(atlas()),
            Event::Changed(ferry()),
            Event::Collected(Box::new(a_snapshot())),
        ]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(reads(asked.try_iter()), [atlas(), ferry()]);
    }

    /// A project reported for again while it is being read is read again: the
    /// collection in flight may have passed it before the message arrived.
    #[test]
    fn a_project_reported_for_twice_is_read_again_rather_than_deduped() {
        let mut view = Recorder::default();
        let (ask, asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Changed(atlas()),
            Event::Changed(atlas()),
            Event::Collected(Box::new(a_snapshot())),
        ]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(reads(asked.try_iter()), [atlas(), atlas()]);
    }

    /// Whatever waits behind a collection is bounded by the projects there
    /// are: a whole collection reads them all, so it stands in for every
    /// single project waiting with it.
    #[test]
    fn a_whole_collection_absorbs_the_projects_waiting_beside_it() {
        let mut view = Recorder::default();
        let (ask, asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Changed(atlas()),
            Event::Changed(ferry()),
            Event::Key(control('r')),
            Event::Changed(ferry()),
            Event::Collected(Box::new(a_snapshot())),
            Event::Collected(Box::new(a_snapshot())),
        ]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(
            reads(asked.try_iter()),
            [atlas(), Wanted::Everything],
            "ferry was going to be read by the whole collection anyway"
        );
    }

    /// What an interval shorter than a collection costs, which is what the
    /// refresh interval is set against: the ticks that pass while a
    /// collection runs collapse into the one waiting behind it, so the work
    /// is one collection per collection rather than one per tick, and a
    /// timer set faster than the tracker can answer cannot pile up.
    #[test]
    fn the_intervals_passing_during_a_collection_cost_one_collection_between_them() {
        let mut view = Recorder::default();
        let (ask, asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Changed(Wanted::Everything),
            Event::Changed(Wanted::Everything),
            Event::Changed(Wanted::Everything),
            Event::Collected(Box::new(a_snapshot())),
            Event::Collected(Box::new(a_snapshot())),
        ]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(
            reads(asked.try_iter()),
            [Wanted::Everything, Wanted::Everything],
            "three ticks over one collection asked for one more, not two"
        );
    }

    #[test]
    fn a_collection_that_comes_back_reaches_the_view() {
        let mut view = Recorder::default();
        let (ask, asked) = mpsc::channel();
        let events = waiting(vec![
            Event::Changed(atlas()),
            Event::Collected(Box::new(a_snapshot())),
            Event::Changed(ferry()),
        ]);

        drive(
            &mut view,
            &events,
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(view.collected, 1);
        assert_eq!(
            reads(asked.try_iter()),
            [atlas(), ferry()],
            "the collection was over, so the second change asked for its own"
        );
    }

    #[test]
    fn a_collection_that_takes_its_time_does_not_hold_up_the_loop() {
        let (to_the_loop, events) = mpsc::channel();
        let (ask, asked) = mpsc::channel();
        let (release, held) = mpsc::channel::<()>();

        let collecting = to_the_loop.clone();
        let worker = thread::spawn(move || {
            collector(
                Box::new(move |_| {
                    let _ = held.recv();
                    Some(a_snapshot())
                }),
                &asked,
                &collecting,
            );
        });

        // The user forces a refresh, then scrolls while it is outstanding.
        for event in [
            Event::Key(control('r')),
            Event::Key(key(KeyCode::Char('j'))),
            Event::Key(key(KeyCode::Char('q'))),
        ] {
            to_the_loop.send(event).expect("the loop is listening");
        }

        // The outstanding collection holds the channel open, so `q` reaching
        // `Quit` is the only thing that ends this loop. A thread of its own is
        // what lets the wait for that run out, rather than run on forever.
        let (finished, ended) = mpsc::channel();
        let driving = thread::spawn(move || {
            let mut view = Recorder::default();
            let outcome = drive(
                &mut view,
                &events,
                &ask,
                at_once(),
                nothing_armed(),
                &polling_every_interval(),
                nothing_watched(),
            );
            let _ = finished.send(());
            (view, outcome)
        });
        ended
            .recv_timeout(A_MOMENT)
            .expect("the loop ended on q with the collection still outstanding");
        let (view, outcome) = driving.join().expect("the loop's thread ends");
        outcome.expect("the loop runs");

        assert_eq!(view.applied, [Action::Move(Motion::NextRow)]);
        assert_eq!(view.collected, 0, "the collection is still outstanding");

        drop(release);
        worker.join().expect("the collector ends with its channels");
    }

    #[test]
    fn a_loop_whose_events_run_out_ends() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();

        drive(
            &mut view,
            &waiting(Vec::new()),
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert!(view.applied.is_empty());
    }

    /// The loop ends the way 'q' ends it, so the screen is dropped and the
    /// terminal put back. Nothing else in the loop can do that.
    ///
    /// The signal is waiting in the channel before the loop starts, which is
    /// also the case `run` produces: the signals are taken before the screen
    /// is opened, so one arriving in between is read as the loop's first
    /// event. The resize behind it is what makes a failure show — without it
    /// the channel runs dry and the loop ends whatever the arm does.
    #[test]
    fn a_signal_ends_the_loop() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();

        drive(
            &mut view,
            &waiting(vec![Event::Signalled, Event::Resize]),
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(
            view.drawn(),
            1,
            "the first draw and no other: the loop returned rather than \
             going on to the resize behind the signal"
        );
    }

    /// The bindings window swallows any key at all, which is why a signal is
    /// not one. A synthesised 'q' here would take the window away and leave
    /// `bdi` running.
    #[test]
    fn a_signal_ends_the_loop_with_the_bindings_up() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();

        drive(
            &mut view,
            &waiting(vec![
                Event::Key(key(KeyCode::Char('?'))),
                Event::Signalled,
                Event::Resize,
            ]),
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(
            view.drawn(),
            2,
            "the first draw and the bindings: the signal ended the run rather \
             than closing the window"
        );
    }

    #[test]
    fn a_resize_redraws_and_nothing_else() {
        let mut view = Recorder::default();
        let (ask, asked) = mpsc::channel();

        drive(
            &mut view,
            &waiting(vec![Event::Resize]),
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert!(view.applied.is_empty());
        assert_eq!(asked.try_iter().count(), 0);
        assert_eq!(view.drawn(), 2, "the first draw, and the resize");
    }

    /// The screen is drawn for what changed it. A key bound to nothing
    /// changed nothing.
    #[test]
    fn a_key_bound_to_nothing_does_not_redraw() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();

        drive(
            &mut view,
            &waiting(vec![Event::Key(key(KeyCode::Char('z')))]),
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(view.drawn(), 1, "the first draw and no other");
    }

    /// The foot says what the reader copied until their next press, and a
    /// key bound to nothing is a press. A screen that changed for the press
    /// alone is redrawn for it.
    #[test]
    fn a_key_bound_to_nothing_redraws_where_the_press_alone_changed_the_screen() {
        let mut view = Recorder {
            pressing_changes: true,
            ..Recorder::default()
        };
        let (ask, _asked) = mpsc::channel();

        drive(
            &mut view,
            &waiting(vec![Event::Key(key(KeyCode::Char('z')))]),
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(view.drawn(), 2, "the first draw and one for the press");
    }

    /// Every press reaches the view before what it means does, so a `y`
    /// pressed twice says *copied* rather than taking its own line off.
    /// Keys, clicks and wheel notches alike: each is the reader's doing.
    #[test]
    fn the_view_hears_a_press_before_it_hears_what_the_press_means() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();

        drive(
            &mut view,
            &waiting(vec![
                Event::Key(key(KeyCode::Char('y'))),
                Event::Clicked(3),
                Event::Scrolled(Motion::NextRow),
                Event::Key(key(KeyCode::Char('?'))),
            ]),
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(
            view.applied,
            [Action::CopyId, Action::Move(Motion::NextRow)]
        );
        assert_eq!(view.clicked, [3]);
        assert_eq!(
            view.pressed_after,
            [0, 1, 1, 2],
            "a press is heard before the action it turns out to be"
        );
    }

    /// `bdi-6so`: every deadline the loop sleeps on is measured from the
    /// instant the frame on the screen was drawn at, never from a later read
    /// of the clock. A deadline decided later than the frame is a deadline
    /// for a frame nobody drew: a mark that came to rest in between holds
    /// for nothing at all, and the frame showing it turning stands until some
    /// unrelated event arrives.
    ///
    /// Across a keystroke that draws nothing as well as one that draws,
    /// because that is where a fresh read would be furthest from the frame:
    /// the loop wakes, decides a deadline, and the frame on the screen is
    /// still the one from before.
    #[test]
    fn a_deadline_is_measured_from_the_instant_the_frame_on_the_screen_was_drawn_at() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();

        drive(
            &mut view,
            &waiting(vec![Event::Resize, Event::Key(key(KeyCode::Char('z')))]),
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        let [first, second] = view.drawn_at[..] else {
            panic!(
                "the first frame and one for the resize: {:?}",
                view.drawn_at
            );
        };
        assert_eq!(
            *view.measured_at.borrow(),
            [first, second, second],
            "one deadline after each frame, and one after the key that drew nothing"
        );
    }

    // ---- the pointer ------------------------------------------------------

    #[test]
    fn a_click_reaches_the_view_as_the_row_it_landed_on() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();

        drive(
            &mut view,
            &waiting(vec![Event::Clicked(9)]),
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(view.clicked, [9]);
        assert!(view.applied.is_empty(), "a click asks for no action");
        assert_eq!(view.drawn(), 2, "the first draw, and the click");
    }

    /// The screen is drawn for what changed it. A click on the tail, the key
    /// row or a blank row past the last line changed nothing.
    #[test]
    fn a_click_that_lands_on_no_row_does_not_redraw() {
        let mut view = Recorder {
            nothing_under_the_pointer: true,
            ..Recorder::default()
        };
        let (ask, _asked) = mpsc::channel();

        drive(
            &mut view,
            &waiting(vec![Event::Clicked(21)]),
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(view.clicked, [21]);
        assert_eq!(view.drawn(), 1, "the first draw and no other");
    }

    /// `bdi` holds no scroll of its own — the window is a pure function of
    /// where the selection sits — so the wheel moves the selection, which is
    /// the only thing the window follows.
    #[test]
    fn a_wheel_notch_moves_the_selection_one_row() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();

        drive(
            &mut view,
            &waiting(vec![
                Event::Scrolled(Motion::PreviousRow),
                Event::Scrolled(Motion::NextRow),
            ]),
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(
            view.applied,
            [
                Action::Move(Motion::PreviousRow),
                Action::Move(Motion::NextRow)
            ]
        );
        assert!(view.clicked.is_empty(), "a wheel notch points at no row");
    }

    /// The bindings window sits over the forest, so while it is up the rows
    /// under the pointer are rows the reader cannot see. A click takes the
    /// window away and selects nothing, for the same reason any key does.
    #[test]
    fn a_click_over_the_bindings_window_closes_it_and_selects_nothing() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();

        drive(
            &mut view,
            &waiting(vec![
                Event::Key(key(KeyCode::Char('?'))),
                Event::Clicked(9),
                Event::Clicked(9),
            ]),
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert_eq!(
            view.clicked,
            [9],
            "the first click took the window away; only the second reached the forest"
        );
        assert_eq!(
            view.showing,
            [
                Showing::Forest,
                Showing::Bindings,
                Showing::Forest,
                Showing::Forest
            ]
        );
    }

    #[test]
    fn a_wheel_notch_over_the_bindings_window_closes_it_and_moves_nothing() {
        let mut view = Recorder::default();
        let (ask, _asked) = mpsc::channel();

        drive(
            &mut view,
            &waiting(vec![
                Event::Key(key(KeyCode::Char('?'))),
                Event::Scrolled(Motion::NextRow),
            ]),
            &ask,
            at_once(),
            nothing_armed(),
            &polling_every_interval(),
            nothing_watched(),
        )
        .expect("the loop runs");

        assert!(view.applied.is_empty());
        assert_eq!(
            view.showing,
            [Showing::Forest, Showing::Bindings, Showing::Forest]
        );
    }

    /// What waits behind a collection, asked of `Outstanding` directly.
    ///
    /// The loop takes its instants from the clock, and every property here is
    /// about one stamp against another — the deadline a queued read crosses
    /// is thirty seconds away and the refresh that re-reports it is thirty
    /// seconds apart, so a test that let real time supply the difference
    /// would be deciding on scheduling jitter rather than on the rule.
    mod what_waits {
        use super::*;
        use crate::config::Tui;
        use chrono::TimeZone;
        use pretty_assertions::assert_eq;

        /// `second` seconds into the run, which is how these are written: a
        /// wait is a difference between two of them and the wall clock they
        /// sit on says nothing.
        fn at(second: i64) -> chrono::DateTime<Utc> {
            Utc.with_ymd_and_hms(2026, 9, 1, 10, 0, 0).unwrap() + TimeDelta::seconds(second)
        }

        /// Asked at `at(0)`, sent, and never answered — so everything after
        /// it waits.
        fn hung_on(project: Wanted) -> (Outstanding, Sender<Asked>, Receiver<Asked>) {
            let (ask, asked) = mpsc::channel();
            let mut outstanding = at_once();
            outstanding.ask(project, at(0));
            outstanding.sends(&ask, at(0));
            (outstanding, ask, asked)
        }

        fn stamps(outstanding: &Outstanding) -> Vec<(Wanted, chrono::DateTime<Utc>)> {
            outstanding
                .awaited()
                .iter()
                .map(|it| (it.wanted.clone(), it.asked_at))
                .collect()
        }

        /// The bead: a project queued behind a tracker that has stopped
        /// answering has to be able to say its own reads have stopped, and
        /// the only thing that can say it is how long the read has been
        /// waiting.
        #[test]
        fn a_read_that_cannot_be_sent_yet_is_stamped_when_it_joins_the_queue() {
            let (mut outstanding, _ask, _asked) = hung_on(atlas());

            outstanding.ask(ferry(), at(5));

            assert_eq!(stamps(&outstanding), [(atlas(), at(0)), (ferry(), at(5))]);
        }

        /// A project with nothing reporting for it is polled every refresh
        /// interval, and the poll goes on reporting it for as long as it is
        /// uncovered — so a queued read is asked for again and again while it
        /// waits. Its wait is how long its rows have been on their way, which
        /// the first of those asks started; a stamp taken from the latest
        /// would be reset by the very polling that proves nothing has been
        /// read, and the mark would never turn.
        #[test]
        fn a_project_reported_for_again_while_it_waits_keeps_the_wait_it_has() {
            let (mut outstanding, _ask, _asked) = hung_on(atlas());

            outstanding.ask(ferry(), at(5));
            outstanding.ask(ferry(), at(35));

            assert_eq!(stamps(&outstanding), [(atlas(), at(0)), (ferry(), at(5))]);
        }

        /// `codex review` on this change, and it is right: a whole collection
        /// absorbing what waits must not inherit their wait.
        ///
        /// It names every project, including every one nothing had asked
        /// about — so an inherited instant is a wait those projects never had,
        /// and one project changing puts a mark saying the reads have stopped
        /// beside every name on the screen. The first version of this took the
        /// earliest absorbed wait, reasoning only about the projects being
        /// absorbed and never about the ones the collection newly covers.
        ///
        /// What it costs the absorbed ones is a patience: ferry has been
        /// waiting since :05 and this says :40, so its mark goes back from
        /// stopped to turning and returns one patience later. That is the
        /// length of not-saying-yet the project has already chosen, and it is
        /// bounded. The other way round is unbounded and about trackers
        /// nobody asked.
        #[test]
        fn a_whole_collection_absorbing_what_waits_starts_a_wait_of_its_own() {
            let (mut outstanding, _ask, _asked) = hung_on(atlas());

            outstanding.ask(ferry(), at(5));
            outstanding.ask(Wanted::Everything, at(40));

            assert_eq!(
                stamps(&outstanding),
                [(atlas(), at(0)), (Wanted::Everything, at(40))],
                "not :05, which would be every other project's wait too"
            );
        }

        /// The absorption `truncate` could not do before, because before the
        /// window there was no such thing as an unsent read: `ask` sent the
        /// moment the queue was empty, so the front was always in flight and
        /// always had to be kept. No existing test asserts this case because
        /// no existing code could produce it.
        #[test]
        fn a_whole_collection_absorbs_a_read_that_has_not_been_sent() {
            let mut outstanding = Outstanding::waiting(PATIENCE, A_LONG_WINDOW);
            outstanding.ask(atlas(), at(0));

            outstanding.ask(Wanted::Everything, at(1));

            assert_eq!(
                stamps(&outstanding),
                [(Wanted::Everything, at(1))],
                "the whole collection reads atlas anyway, and nothing had gone yet"
            );
        }

        /// And the one in flight is still kept, because the collection
        /// running is not the collection about to be asked for.
        #[test]
        fn a_whole_collection_keeps_the_read_already_in_flight() {
            let (mut outstanding, _ask, _asked) = hung_on(atlas());

            outstanding.ask(Wanted::Everything, at(1));

            assert_eq!(
                stamps(&outstanding),
                [(atlas(), at(0)), (Wanted::Everything, at(1))]
            );
        }

        /// A held-down `^R` gets the read it was pressed for. The window runs
        /// from the first notification and is not reset by the ones after it;
        /// under reset the read would be withheld for as long as the key was
        /// down, which is the one thing that key exists to force.
        #[test]
        fn a_window_is_not_pushed_back_by_the_notifications_that_arrive_in_it() {
            let mut outstanding = Outstanding::waiting(PATIENCE, TimeDelta::seconds(2));
            let (ask, asked) = mpsc::channel();
            outstanding.ask(Wanted::Everything, at(0));

            for pressed in 1..=10 {
                outstanding.ask(Wanted::Everything, at(pressed));
                outstanding.sends(&ask, at(pressed));
            }

            assert_eq!(
                reads(asked.try_iter()),
                [Wanted::Everything],
                "sent two seconds after the first press, not two after the last"
            );
        }

        /// The relationship between the window and the patience, through the
        /// predicate that would get it wrong rather than between two
        /// constants: a read held for the whole of the window goes before the
        /// shortest patience the config key can name has run out, so a
        /// project waiting out a window is never drawn as one whose tracker
        /// has stopped answering.
        ///
        /// It names no window of its own: the pair is the one
        /// `Outstanding::for_a_run` builds, and the read is held for as long
        /// as that pair says its window is. So a window wired to a config key
        /// arrives here as the length of the wait, and this holds the
        /// relationship at whatever value production has come to use.
        #[test]
        fn a_read_goes_before_its_project_can_be_said_to_have_stopped_being_read() {
            let shortest = Tui {
                unanswered_after_seconds: 1,
                ..Tui::default()
            }
            .unanswered_after();
            let (ask, asked) = mpsc::channel();
            let mut outstanding = Outstanding::for_a_run(shortest);
            outstanding.ask(atlas(), at(0));

            let held_for = outstanding
                .sends_in(at(0))
                .expect("the read just asked for is waiting out its window");
            let out = at(0)
                + TimeDelta::from_std(held_for).expect("a window is a length chrono can carry");
            outstanding.sends(&ask, out);

            assert_eq!(
                reads(asked.try_iter()),
                [atlas()],
                "the window was out, so the read went"
            );
            assert!(
                !outstanding.awaited()[0].unanswered_at(out),
                "and atlas was not yet said to have stopped being read"
            );
        }

        /// Reaching the front is not being asked for. A read carries the wait
        /// it has had all along, so a project stranded ten minutes behind a
        /// hung tracker does not tell the reader its tracker was asked a
        /// moment ago the instant that tracker answers.
        #[test]
        fn a_read_that_reaches_the_front_keeps_the_stamp_it_queued_at() {
            let (mut outstanding, _ask, _asked) = hung_on(atlas());
            outstanding.ask(ferry(), at(5));

            outstanding.came_back();

            assert_eq!(stamps(&outstanding), [(ferry(), at(5))]);
        }

        /// Every read outstanding, in the order they will be served, is what
        /// the collector is served from — so the first is the one it is
        /// working on and the rest follow in turn.
        #[test]
        fn the_reads_are_sent_in_the_order_they_were_asked_for() {
            let (mut outstanding, ask, asked) = hung_on(atlas());
            outstanding.ask(ferry(), at(5));
            outstanding.ask(Wanted::Project("harbour".to_string()), at(6));

            outstanding.came_back();
            outstanding.sends(&ask, at(7));
            outstanding.came_back();
            outstanding.sends(&ask, at(8));

            assert_eq!(
                reads(asked.try_iter()),
                [atlas(), ferry(), Wanted::Project("harbour".to_string())]
            );
        }

        /// A collection nothing is waiting for is nothing to take: the loop
        /// answers whatever arrives on the one channel, in whatever order it
        /// arrives, and a snapshot with no ask behind it must leave it as it
        /// was rather than end the run.
        #[test]
        fn a_collection_nothing_asked_for_leaves_the_queue_alone() {
            let (ask, _asked) = mpsc::channel();
            let mut outstanding = at_once();

            assert_eq!(outstanding.came_back(), None);

            outstanding.sends(&ask, at(0));

            assert_eq!(stamps(&outstanding), []);
        }
    }
}
