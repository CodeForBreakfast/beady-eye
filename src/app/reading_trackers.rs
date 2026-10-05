//! The first change source: one that reads trackers as a view reads them.
//!
//! It polls, waits out the window, reads one thing at a time and takes
//! producers' lines exactly as a view's loop does, by driving the same three
//! types. What a read finds goes to the watcher rather than to a screen.

use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};

use chrono::{DateTime, Utc};

use crate::collect::changes::Heard;
use crate::model::snapshot::Snapshot;

use super::watcher::{Answer, ChangeSource, Said};
use super::{Asked, Outstanding, Reading, Wanted};

/// One read of what `Wanted` names, as of the instant handed in: the
/// collection drawn from it, and what each project standing says to the
/// watcher.
pub type Reads = Box<dyn FnMut(&Wanted, DateTime<Utc>) -> (Snapshot, Vec<Answer>) + Send>;

/// A change source that reads every configured project's tracker, once at
/// the start and then whenever a project's poll comes round or a producer
/// reports it.
pub struct ReadingTrackers {
    reads: Reads,
    heard: Receiver<Heard>,
    outstanding: Outstanding,
    reading: Reading,
    /// Where `outstanding` sends a read when its turn comes, and where this
    /// source takes it from to make it. One thread does both, so a read in
    /// flight is a read being made.
    asking: (Sender<Asked>, Receiver<Asked>),
    told: VecDeque<Answer>,
}

impl ReadingTrackers {
    /// A source reading every project `reading` names with `reads`, told of
    /// changes on `heard`, and asking at once for everything.
    pub fn new(
        reads: Reads,
        heard: Receiver<Heard>,
        mut outstanding: Outstanding,
        reading: Reading,
    ) -> Self {
        outstanding.ask(Wanted::Everything, Utc::now());
        Self {
            reads,
            heard,
            outstanding,
            reading,
            asking: mpsc::channel(),
            told: VecDeque::new(),
        }
    }

    /// Make the read whose turn has come, where one has.
    fn read_what_is_due(&mut self, now: DateTime<Utc>) -> bool {
        for wanted in self.reading.due(now) {
            self.outstanding.ask(wanted, now);
        }
        self.outstanding.sends(&self.asking.0, now);
        let Ok(Asked::Read(wanted)) = self.asking.1.try_recv() else {
            return false;
        };
        let (snapshot, answers) = (self.reads)(&wanted, now);
        self.reading.came_back(
            self.outstanding.came_back(),
            &snapshot.projects,
            &snapshot.speaks_until,
            Utc::now(),
        );
        self.told.extend(answers);
        true
    }

    fn take(&mut self, heard: Heard) {
        let now = Utc::now();
        match heard {
            Heard::Changed(project) => {
                self.outstanding.ask(Wanted::Project(project), now);
            }
            Heard::Covered(project) => {
                self.reading.covered(&project, now);
                self.told.push_back(Answer {
                    project,
                    said: Said::Vouched { at: now },
                    journal: None,
                });
            }
        }
    }
}

impl ChangeSource for ReadingTrackers {
    fn next(&mut self) -> Option<Answer> {
        loop {
            while let Ok(heard) = self.heard.try_recv() {
                self.take(heard);
            }
            if let Some(answer) = self.told.pop_front() {
                return Some(answer);
            }
            let now = Utc::now();
            if self.read_what_is_due(now) {
                continue;
            }
            let wakes_in = [
                self.outstanding.sends_in(now),
                self.reading.next_due_in(now),
            ]
            .into_iter()
            .flatten()
            .min();
            let heard = match wakes_in {
                Some(wait) => match self.heard.recv_timeout(wait) {
                    Ok(heard) => heard,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => return None,
                },
                None => self.heard.recv().ok()?,
            };
            self.take(heard);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use chrono::TimeDelta;

    use super::*;
    use crate::app::fixtures::{dunwich, two_projects};
    use crate::app::watcher::Held;
    use crate::app::{Armed, Collection};
    use crate::collect::agents::testing::Fake as Provider;
    use crate::collect::changes::Reported;
    use crate::collect::tracker::testing::Fakes;
    use crate::model::snapshot::Filter;

    /// Every project `trackers` holds, read through a collection as
    /// `bdi watch` reads them, with no window and nothing polling.
    fn reading(trackers: Arc<Fakes>) -> (ReadingTrackers, Sender<Heard>) {
        polling_every(None, trackers)
    }

    /// As [`reading`], with every project polling `every` after each read.
    fn polling_every(
        every: Option<std::time::Duration>,
        trackers: Arc<Fakes>,
    ) -> (ReadingTrackers, Sender<Heard>) {
        let cfg = two_projects();
        let mut collection = Collection::default();
        let reads: Reads = Box::new(move |wanted, now| {
            let snapshot = collection.collect(
                &cfg,
                &Provider::holding(Vec::new()),
                trackers.as_ref(),
                wanted,
                Filter::All,
                now,
            );
            let answers = collection.answers(&snapshot);
            (snapshot, answers)
        });
        let (tell, heard) = mpsc::channel();
        let armed = ["dunwich", "ferry"]
            .into_iter()
            .map(|project| Armed::polling(project.to_string(), every))
            .collect();
        let source = ReadingTrackers::new(
            reads,
            heard,
            Outstanding::waiting(TimeDelta::seconds(30), TimeDelta::zero()),
            Reading::of(armed, Reported::default()),
        );
        (source, tell)
    }

    fn trackers() -> Arc<Fakes> {
        Arc::new(dunwich().with("ferry", crate::app::fixtures::colliding_tracker()))
    }

    fn read_ids(answer: &Answer) -> Vec<&str> {
        match &answer.said {
            Said::Read { beads, .. } => beads.keys().map(String::as_str).collect(),
            said => panic!("{} was read, and the answer says {said:?}", answer.project),
        }
    }

    #[test]
    fn a_source_starts_by_reading_every_project() {
        let (mut source, _tell) = reading(trackers());

        let first: Vec<Answer> = (0..2).map(|_| source.next().expect("an answer")).collect();

        assert_eq!(
            first.iter().map(|a| a.project.as_str()).collect::<Vec<_>>(),
            ["dunwich", "ferry"]
        );
        assert_eq!(read_ids(&first[0]), ["dun-7", "dun-7.1", "dun-7.2"]);
    }

    #[test]
    fn a_project_a_producer_reports_is_read_again() {
        let trackers = trackers();
        let (mut source, tell) = reading(Arc::clone(&trackers));
        for _ in 0..2 {
            source.next();
        }
        let read_once = trackers.tracker("dunwich").asked().len();

        tell.send(Heard::Changed("dunwich".to_string())).unwrap();
        let answers: Vec<Answer> = (0..2).map(|_| source.next().expect("an answer")).collect();

        assert!(
            trackers.tracker("dunwich").asked().len() > read_once,
            "dunwich's tracker was asked again"
        );
        assert!(answers.iter().all(|a| matches!(a.said, Said::Read { .. })));
    }

    #[test]
    fn a_project_a_producer_covers_is_vouched_for_without_a_read() {
        let trackers = trackers();
        let (mut source, tell) = reading(Arc::clone(&trackers));
        for _ in 0..2 {
            source.next();
        }
        let asked = trackers.tracker("ferry").asked().len();

        tell.send(Heard::Covered("ferry".to_string())).unwrap();
        let answer = source.next().expect("an answer");

        assert_eq!(answer.project, "ferry");
        assert!(matches!(answer.said, Said::Vouched { .. }));
        assert_eq!(trackers.tracker("ferry").asked().len(), asked);
    }

    #[test]
    fn a_producers_line_is_taken_while_polls_keep_coming_due() {
        let (mut source, tell) = polling_every(Some(std::time::Duration::ZERO), trackers());
        for _ in 0..2 {
            source.next();
        }

        tell.send(Heard::Covered("ferry".to_string())).unwrap();

        assert!(
            (0..10).any(|_| matches!(source.next().expect("an answer").said, Said::Vouched { .. })),
            "ferry's cover was never taken while polls kept coming due"
        );
    }

    #[test]
    fn a_source_no_producer_can_reach_any_longer_stops() {
        let (mut source, tell) = reading(trackers());
        for _ in 0..2 {
            source.next();
        }

        drop(tell);

        assert_eq!(source.next(), None);
    }

    /// The hold over the source, as `bdi watch` keeps it, after the
    /// source's first two answers.
    #[test]
    fn what_a_source_reads_is_held() {
        let (mut source, tell) = reading(trackers());
        let hold = Mutex::new(crate::app::watcher::Hold::default());
        drop(tell);

        crate::app::watcher::hold(&mut source, &hold);

        let held = hold.lock().unwrap();
        let dunwich = held.of("dunwich").expect("dunwich is held");
        let beads: &BTreeMap<String, Held> = dunwich.beads.as_ref().expect("dunwich was read");
        assert!(beads["dun-7.2"].ready);
        assert!(dunwich.as_of.is_some());
    }
}
