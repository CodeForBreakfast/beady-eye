//! The walk a test takes over the rows on screen, counted out before it
//! starts.
//!
//! A test that wants the selection somewhere presses the key until it is
//! there. What ends the loop decides what a mutation can do to it: a loop
//! that stops when the code under test says the screen stopped moving is a
//! loop a mutation can leave running for ever, and cargo-mutants scores the
//! hang as a timeout — which on the tally reads exactly like a mutant that
//! does not terminate in production. A loop counted out before it starts ends
//! whatever production returns, and says what it never reached.
//!
//! This module is where that count lives, so a test reaching for a walk
//! reaches for a bounded one. `screen-walks-are-bounded` in `flake.nix` is
//! what keeps the other kind from being written beside it.

use crate::view::forest::Forest;

/// A screen whose rows can be counted before a walk over them starts.
pub(crate) trait Rows {
    /// The rows drawn now.
    fn rows(&self) -> usize;
}

impl Rows for Forest {
    fn rows(&self) -> usize {
        self.lines().len()
    }
}

/// Press until `wanted` says the screen has arrived, and say how many presses
/// it took. Panics with what `unreached` says of the screen it gave up on.
///
/// The rows counted before the first press are the bound, because every walk
/// here presses at most once per row: pressing draws rows below where the
/// walk has got to, never a new place it still has to reach.
///
/// One more turn than that, because a walk that presses on every one of them
/// has still arrived somewhere and the last row is a place worth asking
/// about. The press that turn is spent on is the one before the panic, so
/// nothing after it can care.
pub(crate) fn until<S: Rows>(
    screen: &mut S,
    wanted: impl Fn(&S) -> bool,
    mut press: impl FnMut(&mut S),
    unreached: impl FnOnce(&S) -> String,
) -> usize {
    for pressed in 0..=screen.rows() {
        if wanted(screen) {
            return pressed;
        }
        press(screen);
    }
    panic!("{}", unreached(screen));
}
