//! The poll stand-down a view and `bdi gates` share.

use crate::config::Poll;

/// What a producer covering a project does to its poll.
///
/// The producer replaces the poll rather than backing it up, so a project
/// that stands its poll down keeps it down for the run: a producer that goes
/// quiet afterwards lapses the project, and a poll coming back would hide
/// exactly that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Cover {
    /// Nothing: the reader chose the poll, or chose none.
    KeepsThePoll,
    /// Stands the poll down the first time a producer covers the project.
    StandsThePollDown,
    /// A producer has covered the project, and it polls no more.
    HasStoodThePollDown,
}

impl Cover {
    /// What a producer covering a project kept current as `poll` says does
    /// to its poll.
    pub(super) fn of(poll: Poll) -> Self {
        match poll {
            Poll::UntilCovered => Cover::StandsThePollDown,
            Poll::Always | Poll::Never => Cover::KeepsThePoll,
        }
    }

    /// A producer says it covers the project. Whether that stood its poll
    /// down just now.
    pub(super) fn covered(&mut self) -> bool {
        let stands_down = *self == Cover::StandsThePollDown;
        if stands_down {
            *self = Cover::HasStoodThePollDown;
        }
        stands_down
    }

    /// Whether the project still polls, where it polls at all.
    pub(super) fn keeps_the_poll(self) -> bool {
        self != Cover::HasStoodThePollDown
    }
}
