//! The projects a run reads, and the poll each of them keeps.
//!
//! One list settles which projects ask to be read again and which names a
//! producer is answered `ok` for, so the two cannot come apart. Whatever
//! drives it, a view or a listener with no view, hands it what came back and
//! asks it what is due.

use std::collections::BTreeMap;
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::collect::changes::Reported;

use super::armed::Armed;
use super::Wanted;

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

/// The projects this run reads, in the two places a config decides them:
/// which of them ask for themselves, and which of them the inbound channel
/// accepts a report for.
///
/// One value rather than two, because one list settles both. A project the
/// loop polls and a project the channel answers `ok` to are the same project
/// by construction here; held apart they would be two lists agreeing by
/// argument, and the argument is what a reader of a bug report is left
/// checking. Only [`Self::now_reading`] writes either, and it writes both.
pub struct Reading {
    polling: Vec<Armed>,
    accepted: Reported,
    /// The projects the config names and this run is not reading, each
    /// armed for the collection that reads it on demand.
    unread: Vec<Armed>,
}

impl Reading {
    pub fn of(polling: Vec<Armed>, accepted: Reported) -> Self {
        Self {
            polling,
            accepted,
            unread: Vec::new(),
        }
    }

    pub fn unread(self, unread: Vec<Armed>) -> Self {
        Self { unread, ..self }
    }

    /// A collection has come back, having read `projects`, after the read
    /// `read` names went out — or nothing, where it was asked for by nobody.
    ///
    /// Every project that read covered now has nothing coming, so this is
    /// where each of them arms its next ask. The only place: a read that
    /// never comes back arms nothing, and the project says its tracker has
    /// stopped answering rather than being quietly polled over.
    pub fn came_back(
        &mut self,
        read: Option<Wanted>,
        projects: &[String],
        speaks_until: &BTreeMap<String, DateTime<Utc>>,
        now: DateTime<Utc>,
    ) {
        // A project the collection read on demand was read by it as much as
        // any the read named.
        let gained = self.read_on_demand(projects);
        let Some(read) = read else {
            return;
        };
        for project in &mut self.polling {
            let speaks_until = speaks_until.get(project.project()).copied();
            if gained.iter().any(|named| named == project.project()) {
                project.was_read(now, speaks_until);
            } else {
                project.came_back(&read, now, speaks_until);
            }
        }
    }

    /// Something outside says it covers `covered` and nothing in it has
    /// moved.
    pub fn covered(&mut self, covered: &str, now: DateTime<Utc>) {
        for project in self
            .polling
            .iter_mut()
            .filter(|armed| armed.project() == covered)
        {
            project.covered(now);
        }
    }

    /// The reads the projects that arm themselves are now due to ask for.
    ///
    /// Every project that is due, not the first: several come due together
    /// after a read of everything, and stopping at one would leave the rest
    /// armed in the past.
    pub fn due(&mut self, now: DateTime<Utc>) -> Vec<Wanted> {
        self.polling
            .iter_mut()
            .filter_map(|project| project.asks(now))
            .collect()
    }

    /// The projects nothing has vouched for lately, as `now` finds them.
    pub fn lapsed(&self, now: DateTime<Utc>) -> Vec<String> {
        self.polling
            .iter()
            .filter(|project| project.lapsed(now))
            .map(|project| project.project().to_string())
            .collect()
    }

    /// How long until a project asks for itself or lapses, whichever is
    /// sooner, or nothing where none of them will.
    pub fn next_due_in(&self, now: DateTime<Utc>) -> Option<Duration> {
        self.polling
            .iter()
            .flat_map(|project| [project.asks_in(now), project.lapses_in(now)])
            .flatten()
            .min()
    }

    /// Take the projects a collection has read into what the run reads, and
    /// hand back those it was not reading until now.
    fn read_on_demand(&mut self, read: &[String]) -> Vec<String> {
        let (gained, unread) = std::mem::take(&mut self.unread)
            .into_iter()
            .partition::<Vec<_>, _>(|project| read.iter().any(|named| named == project.project()));
        self.unread = unread;
        if gained.is_empty() {
            return Vec::new();
        }
        let names = gained
            .iter()
            .map(|project| project.project().to_string())
            .collect();
        self.polling.extend(gained);
        self.accept_what_polls();
        names
    }

    fn accept_what_polls(&self) {
        self.accepted.now_watching(
            self.polling
                .iter()
                .map(|project| project.project().to_string()),
        );
    }

    /// The projects a config the reader has written names, as what the run
    /// reads from here on.
    ///
    /// `still_due` is where what the file settles and what its last read
    /// settled are told apart, so the channel is told what came out of that
    /// rather than what went into it.
    pub fn now_reading(&mut self, named: Vec<Armed>, unread: Vec<Armed>) {
        self.polling = still_armed(std::mem::take(&mut self.polling), named);
        self.unread = unread;
        self.accept_what_polls();
    }
}
