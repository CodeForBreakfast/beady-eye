//! What a project's tracker answers, whichever program answers it.
//!
//! `app/` asks these four questions of every project and nothing else, in
//! the model's own types. How a question reaches a tracker — bd's CLI today,
//! a database file or a socket some day — is an adapter's business, and this
//! is the line it stays behind.

use std::collections::{BTreeMap, BTreeSet};

use crate::collect::run::RunFailure;
use crate::config::Project;
use crate::model::types::Bead;

/// One project's tracker, opened for one read.
pub trait Tracker {
    /// One value that moves when anything the tracker holds does, and stays
    /// put when a read costs nothing — or `None` from a tracker that has no
    /// such thing to offer, which is read in full every time.
    ///
    /// An error is a tracker that could not be compared against, never one
    /// that has not moved.
    fn fingerprint(&self) -> Option<Result<String, RunFailure>>;

    /// Every bead the tracker holds, finished or not, each carrying the beads
    /// it depends on and the bead it hangs under.
    fn all(&self) -> Result<Vec<Bead>, RunFailure>;

    /// The ids the tracker itself considers ready to start.
    fn ready(&self) -> Result<BTreeSet<String>, RunFailure>;

    /// Every blocker of every blocked bead.
    fn blocked(&self) -> Result<BTreeMap<String, Vec<String>>, RunFailure>;
}

/// Why a project drew nothing: its tracker could not be opened, or bd would
/// not answer once it was.
///
/// The first two are the halves of what opening does — settle the
/// environment, then produce the credential — and neither reaches bd, so
/// neither carries a kind. A reader does something different about each and
/// the screen has to be able to say which, but in both cases what they do is
/// about the config they wrote rather than about a program bd's failures
/// classify.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenFailure {
    /// The project asked to be read in a captured environment — by the command
    /// its config names, or by the `.envrc` in its own directory — and the
    /// capture did not happen.
    ///
    /// Nothing is opened and no bd is run for that project, which is the whole
    /// of why this is a failure rather than a fallback. The bd on `bdi`'s own
    /// `PATH` is not the bd the project asked to be read with, and bd rewrites
    /// `.beads/.local_version` and runs its schema auto-migration on finding
    /// itself newer than the bd that last opened a tracker — before the
    /// subcommand, whatever the subcommand is, and `--readonly` stops neither.
    /// `docs/design.md`'s *Reading a tracker is not leaving it alone* has the
    /// measurement and the decision this rests on.
    NoEnvironment,
    /// The project's own `credential_command` would not run, so nothing was
    /// opened and no bd ran for it either.
    ///
    /// Whichever way it failed is dropped here rather than at the screen. The
    /// kinds classify the machine's `sh` and a tracker the command never spoke
    /// to; what the reader does about any of them is look at the command their
    /// config names.
    NoCredential,
    /// A program bd's own read ran would not answer, which is every failure
    /// that happens after the tracker is open.
    Refused(RunFailure),
}

/// The read's own failures, which reach the caller by `?` from `read_project`
/// once the tracker is open. Opening's two failures are named outright, so
/// this conversion is only ever the read's.
impl From<RunFailure> for OpenFailure {
    fn from(failure: RunFailure) -> Self {
        OpenFailure::Refused(failure)
    }
}

/// How each configured project's tracker is reached.
///
/// `Sync` because a collection reads its projects together, each on a thread
/// of its own, through the one instance it was given.
pub trait Trackers: Sync {
    /// The tracker `project` is read from, opened in the environment its
    /// config asks for. Opening can fail — an environment `bdi` could not
    /// produce, a credential command that does not run — and that failure is
    /// the project's, before anything was asked of the tracker.
    fn of(&self, project: &Project) -> Result<Box<dyn Tracker + '_>, OpenFailure>;
}

impl<T: Tracker + ?Sized> Tracker for &T {
    fn fingerprint(&self) -> Option<Result<String, RunFailure>> {
        (**self).fingerprint()
    }

    fn all(&self) -> Result<Vec<Bead>, RunFailure> {
        (**self).all()
    }

    fn ready(&self) -> Result<BTreeSet<String>, RunFailure> {
        (**self).ready()
    }

    fn blocked(&self) -> Result<BTreeMap<String, Vec<String>>, RunFailure> {
        (**self).blocked()
    }
}

/// Trackers that answer from what a test staged, in the model's own types,
/// and count what they were asked. Shared by the tests inside the crate and
/// the ones under `tests/`, which is why it sits behind the feature rather
/// than `cfg(test)`.
#[cfg(feature = "testing")]
pub mod testing {
    use std::sync::Mutex;

    use super::*;

    /// One of the four questions, as a fake records being asked it.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Asked {
        Fingerprint,
        All,
        Ready,
        Blocked,
    }

    /// One project's tracker, answering each question from what was staged.
    ///
    /// Its fingerprint never moves unless a test moves it, so a second read
    /// of the same fake is a read of a tracker that has not changed.
    pub struct Fake {
        fingerprint: Option<Result<String, RunFailure>>,
        all: Result<Vec<Bead>, RunFailure>,
        ready: Result<BTreeSet<String>, RunFailure>,
        blocked: Result<BTreeMap<String, Vec<String>>, RunFailure>,
        asked: Mutex<Vec<Asked>>,
    }

    /// The one fingerprint every fake starts with.
    const UNMOVED: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    impl Fake {
        /// A tracker holding `beads`, with nothing ready, nothing blocked and
        /// a fingerprint that has not moved.
        pub fn holding(beads: Vec<Bead>) -> Self {
            Self {
                fingerprint: Some(Ok(UNMOVED.to_string())),
                all: Ok(beads),
                ready: Ok(BTreeSet::new()),
                blocked: Ok(BTreeMap::new()),
                asked: Mutex::new(Vec::new()),
            }
        }

        /// The same tracker holding `beads` as well: a tracker answers for
        /// everything it holds in one listing, so staging a second root is
        /// adding to the one answer rather than staging another.
        pub fn also(mut self, beads: Vec<Bead>) -> Self {
            if let Ok(all) = &mut self.all {
                all.extend(beads);
            }
            self
        }

        /// The ids this tracker considers ready.
        pub fn ready<'a>(mut self, ids: impl IntoIterator<Item = &'a str>) -> Self {
            self.ready = Ok(ids.into_iter().map(str::to_string).collect());
            self
        }

        /// `id` is blocked by every one of `by`.
        pub fn blocked(mut self, id: &str, by: &[&str]) -> Self {
            if let Ok(blocked) = &mut self.blocked {
                blocked.insert(id.to_string(), by.iter().map(|b| b.to_string()).collect());
            }
            self
        }

        /// The same tracker after something in it moved.
        pub fn moved(mut self) -> Self {
            self.fingerprint = Some(Ok("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()));
            self
        }

        /// A tracker with nothing to fingerprint by, which is read in full
        /// every time.
        pub fn without_a_fingerprint(mut self) -> Self {
            self.fingerprint = None;
            self
        }

        /// The tracker fails `question` with `failure` rather than answering
        /// it.
        pub fn failing(mut self, question: Asked, failure: RunFailure) -> Self {
            match question {
                Asked::Fingerprint => self.fingerprint = Some(Err(failure)),
                Asked::All => self.all = Err(failure),
                Asked::Ready => self.ready = Err(failure),
                Asked::Blocked => self.blocked = Err(failure),
            }
            self
        }

        /// Every question this tracker has been asked, in order.
        pub fn asked(&self) -> Vec<Asked> {
            self.asked.lock().unwrap().clone()
        }

        fn note(&self, question: Asked) {
            self.asked.lock().unwrap().push(question);
        }
    }

    impl Tracker for Fake {
        fn fingerprint(&self) -> Option<Result<String, RunFailure>> {
            self.note(Asked::Fingerprint);
            self.fingerprint.clone()
        }

        fn all(&self) -> Result<Vec<Bead>, RunFailure> {
            self.note(Asked::All);
            self.all.clone()
        }

        fn ready(&self) -> Result<BTreeSet<String>, RunFailure> {
            self.note(Asked::Ready);
            self.ready.clone()
        }

        fn blocked(&self) -> Result<BTreeMap<String, Vec<String>>, RunFailure> {
            self.note(Asked::Blocked);
            self.blocked.clone()
        }
    }

    /// The trackers of every configured project, by project name.
    ///
    /// A project no fake was staged for panics when opened, the way a fake
    /// runner panics on a call nobody staged: a collection reaching a
    /// tracker the test did not expect is the test's finding.
    #[derive(Default)]
    pub struct Fakes {
        by_project: BTreeMap<String, Fake>,
        unopenable: BTreeMap<String, OpenFailure>,
    }

    impl Fakes {
        pub fn with(mut self, project: &str, tracker: Fake) -> Self {
            self.by_project.insert(project.to_string(), tracker);
            self
        }

        /// `project`'s tracker cannot be opened at all, because the credential
        /// command its config names would not run — so nothing was opened and
        /// no bd ran for it.
        ///
        /// It takes no kind, because there is none left to take: the way the
        /// command failed is dropped where it happens, in `tracker_env`, and
        /// `every_way_a_credential_command_can_fail_is_the_same_failure_to_the_project`
        /// is where that is held.
        pub fn without_the_credential_it_asked_for(mut self, project: &str) -> Self {
            self.unopenable
                .insert(project.to_string(), OpenFailure::NoCredential);
            self
        }

        /// `project` asked to be read in a captured environment and `bdi`
        /// could not produce one, so nothing was opened and no bd ran for it.
        pub fn without_the_environment_it_asked_for(mut self, project: &str) -> Self {
            self.unopenable
                .insert(project.to_string(), OpenFailure::NoEnvironment);
            self
        }

        /// The fake staged for `project`, to read what it was asked.
        pub fn tracker(&self, project: &str) -> &Fake {
            self.by_project
                .get(project)
                .unwrap_or_else(|| panic!("no fake tracker was staged for {project}"))
        }
    }

    impl Trackers for Fakes {
        fn of(&self, project: &Project) -> Result<Box<dyn Tracker + '_>, OpenFailure> {
            if let Some(failure) = self.unopenable.get(&project.name) {
                return Err(failure.clone());
            }
            Ok(Box::new(self.tracker(&project.name)))
        }
    }
}
