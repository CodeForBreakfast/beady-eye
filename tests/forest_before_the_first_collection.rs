//! What `bdi` has on the screen while its first collection is still running.
//!
//! `bdi` used to read every tracker before it opened the screen, so a reader
//! who typed `bdi` sat in front of their own shell with nothing written to it
//! — measured at 8.2 to 8.4 seconds against three projects on 2026-09-01, and
//! at 3.4 to 3.8 against two the day before. Whatever the figure, it is the
//! whole of the collection, and against a tracker that never answers it is
//! the whole of the run.
//!
//! Only a driven terminal can tell what this file asks. A clock can say the
//! frame appeared quickly; it cannot say the frame appeared *before the
//! collection came back*, and the difference between those is the whole
//! change. So `bd` is held here rather than made slow: a collection that
//! cannot return is one no frame can be drawn after.

mod terminal;

use terminal::a_home_naming_one_project_settled;
use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::ShimmedTracker;

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// A refresh interval no test here will ever reach.
///
/// It is what makes these tests about the collection `bdi` asks for at
/// startup rather than about collections in general. Left at the default of
/// thirty seconds, the fallback timer asks for one of its own and every
/// assertion below is satisfied by that instead — measured, on a tree with
/// the startup ask deleted: both tests still passed, in 31 seconds rather
/// than one. The interval is what takes that second road away.
const NOTHING_ELSE_WILL_COLLECT: &str = "[tui]\nrefresh_seconds = 600\n";

/// The one project the config below names. Its own line on the forest, which
/// is drawn from the configured name rather than from anything a tracker
/// said, so it is on the screen before any tracker has answered.
const THE_PROJECT: &[u8] = "atlas".as_bytes();

/// Two frames of the collecting mark, from `view::phrase`. Written out rather
/// than asked of `bdi`, so a mark changed by hand is a test to change by
/// hand.
///
/// Two, because the claim is that the mark *turns*. One frame says only that
/// a glyph was drawn, which a still mark over a dead collection would also
/// satisfy — and a still mark is what `bdi` draws for a collection that has
/// stopped answering, so the two states have to be told apart here.
const A_FRAME_OF_THE_MARK: &[u8] = "⠋".as_bytes();
const ANOTHER_FRAME_OF_THE_MARK: &[u8] = "⠸".as_bytes();

/// The mark a project wears once the collection reading it has gone
/// unanswered for longer than it may, from `view::phrase`: the turning mark
/// held still.
const THE_MARK_HELD_STILL: &[u8] = "⠿".as_bytes();

/// The forest is on the screen before the first collection comes back.
///
/// The ordering is the proof and it is in the two lines below. `bd` is held
/// before `bdi` starts and never let go, so a collection that has begun can
/// never return; `wait_until_holding` is what says one has begun. Everything
/// read after that was drawn with the collection outstanding.
#[test]
fn the_forest_is_on_the_screen_before_the_first_collection_comes_back() {
    let home = a_home_naming_one_project_settled("forest-first", NOTHING_ELSE_WILL_COLLECT);
    let tracker = ShimmedTracker::beside(&home);
    tracker.hang();

    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &tracker.environment());
    tracker.wait_until_holding(GIVING_UP);

    bdi.read_until(THE_PROJECT, GIVING_UP);
    bdi.read_until(A_FRAME_OF_THE_MARK, GIVING_UP);
    bdi.read_until(ANOTHER_FRAME_OF_THE_MARK, GIVING_UP);
}

/// A tracker that never answers at startup says so, rather than leaving the
/// reader to work it out by killing the run.
///
/// The wait before the first frame used to be the one wait nothing could
/// report: there was no screen to draw a mark on and no `Awaited` to measure
/// it against, so a tracker that hung was a blank terminal for as long as the
/// reader would stand it. Now it is a project line like any other, and the
/// deadline the config sets applies to it like any other — which is what this
/// asks, with the deadline turned down to a second so the test does not have
/// to wait out the default.
#[test]
fn a_tracker_that_does_not_answer_at_startup_is_visible_as_such() {
    let home = a_home_naming_one_project_settled(
        "startup-unanswered",
        &format!("{NOTHING_ELSE_WILL_COLLECT}unanswered_after_seconds = 1\n"),
    );
    let tracker = ShimmedTracker::beside(&home);
    tracker.hang();

    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &tracker.environment());
    tracker.wait_until_holding(GIVING_UP);

    bdi.read_until(THE_PROJECT, GIVING_UP);
    bdi.read_until(THE_MARK_HELD_STILL, GIVING_UP);
}
