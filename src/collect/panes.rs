//! The agent provider's panes, asked for on a thread of their own.
//!
//! Nothing here is herdr's. The threads, the patience and the one question at
//! a time are what shelling out to any provider costs, so they are written
//! once, over the seam, and an adapter inherits them.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use crate::collect::agents::Agents;
use crate::collect::run::{FailureKind, RunFailure};
use crate::model::types::PaneKey;

/// How long the tail waits on the provider before saying the pane could not
/// be read.
///
/// A liveness backstop rather than a latency budget: a provider is a process
/// on this machine and a healthy one answers in milliseconds. What this
/// bounds is what the band under the forest says, and nothing else — no
/// thread that reads keys or draws waits on it, so `q` and `^C` are answered
/// while a read is still outstanding.
const PATIENCE: Duration = Duration::from_secs(2);

/// Reading a pane, and focusing it — everything the tail asks of the provider.
///
/// Asking is the whole of it: neither of these waits for what it asked, and
/// the answer arrives later as an [`Answer`]. That is what keeps the provider
/// off the thread that reads keys, and it is a seam as well as a thread, so
/// the loop can be driven over a provider that answers whatever a test needs
/// it to, including nothing.
pub trait Panes {
    fn read(&self, pane: &PaneKey, lines: u16);

    /// Bring a pane to the front. The only write `bdi` performs.
    fn focus(&self, pane: &PaneKey);
}

/// What the provider said, and which pane it said it about.
///
/// The pane is carried back because the selection moves while the provider is
/// answering: an answer about a pane the reader has already left is dropped,
/// and nothing but the pane's own name in the answer can say that it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Read {
        pane: PaneKey,
        read: Result<Vec<String>, RunFailure>,
    },
    Focused {
        pane: PaneKey,
        focused: Result<(), RunFailure>,
    },
}

/// What the tail asks the provider for.
enum Job {
    Read { pane: PaneKey, lines: u16 },
    Focus { pane: PaneKey },
}

/// What came back. The two arms are kept apart so a reply that outstayed its
/// welcome cannot be read as the answer to the next question.
enum Done {
    Read(Result<Vec<String>, RunFailure>),
    Focus(Result<(), RunFailure>),
}

/// The provider, asked aside: on threads of its own, so nothing the reader is
/// sitting in front of waits on it.
///
/// Two threads, and each has one job. One asks the provider and blocks in it
/// for as long as the provider takes. The other is the one that waits
/// `PATIENCE` for that answer and gives up on it: asking is a send, and what
/// comes back arrives wherever `answers` goes.
///
/// One question at a time, on both. A question that outstays `PATIENCE` is
/// not abandoned — its answer is thrown away when it finally arrives — and
/// none is asked while one is still out, so a provider that has stopped
/// answering costs one waiting thread rather than one per poll.
pub struct Aside {
    wanted: Sender<Job>,
}

impl Aside {
    /// Start the provider's threads, with every answer sent on to `answers`.
    pub fn new<P, A>(provider: P, answers: Sender<A>) -> Self
    where
        P: Agents + 'static,
        A: From<Answer> + Send + 'static,
    {
        let (wanted, wants) = mpsc::channel();
        let (asking, asked) = mpsc::channel();
        let (answered, done) = mpsc::channel();

        let unanswered = no_answer(provider.name());
        thread::spawn(move || work(&provider, &asked, &answered));
        thread::spawn(move || {
            ask(
                &wants,
                &mut Patience {
                    asking,
                    done,
                    outstanding: 0,
                    unanswered,
                },
                &answers,
            );
        });

        Self { wanted }
    }
}

/// How long the provider gets, what is still owed from the last time it was
/// asked, and what `bdi` says when it never answers.
struct Patience {
    asking: Sender<Job>,
    done: Receiver<Done>,
    outstanding: usize,
    unanswered: RunFailure,
}

impl Patience {
    /// Ask the provider one thing and wait `PATIENCE` for it.
    fn waited(&mut self, job: Job) -> Option<Done> {
        while self.outstanding > 0 && self.done.try_recv().is_ok() {
            self.outstanding -= 1;
        }
        if self.outstanding > 0 || self.asking.send(job).is_err() {
            return None;
        }

        self.outstanding = 1;
        let answer = self.done.recv_timeout(PATIENCE).ok();
        self.outstanding = usize::from(answer.is_none());
        answer
    }

    /// The same, as the answer the tail is owed either way. A question the
    /// provider did not answer in time is answered here instead, because the
    /// band under the forest is drawn whatever the provider does.
    fn answer(&mut self, job: Job) -> Answer {
        match job {
            Job::Read { pane, lines } => {
                let read = match self.waited(Job::Read {
                    pane: pane.clone(),
                    lines,
                }) {
                    Some(Done::Read(read)) => read,
                    _ => Err(self.unanswered.clone()),
                };
                Answer::Read { pane, read }
            }
            Job::Focus { pane } => {
                let focused = match self.waited(Job::Focus { pane: pane.clone() }) {
                    Some(Done::Focus(focused)) => focused,
                    _ => Err(self.unanswered.clone()),
                };
                Answer::Focused { pane, focused }
            }
        }
    }
}

/// Put the tail's questions to the provider, one at a time, until it stops
/// asking or there is nobody left to tell.
fn ask<A: From<Answer>>(wants: &Receiver<Job>, patience: &mut Patience, to: &Sender<A>) {
    while let Ok(job) = wants.recv() {
        if to.send(A::from(patience.answer(job))).is_err() {
            return;
        }
    }
}

/// What `bdi` says happened when the provider said nothing at all.
///
/// `detail` never reaches the screen — the phrases do — so this says what a
/// reader of the code needs and not what a reader of the screen does.
fn no_answer(provider: &str) -> RunFailure {
    RunFailure {
        kind: FailureKind::Unavailable,
        program: provider.to_string(),
        detail: format!("{provider} did not answer in time"),
    }
}

/// Put the questions to the provider until the tail stops asking.
fn work(provider: &dyn Agents, asked: &Receiver<Job>, to: &Sender<Done>) {
    while let Ok(job) = asked.recv() {
        let done = match job {
            Job::Read { pane, lines } => Done::Read(provider.read(&pane, lines)),
            Job::Focus { pane } => Done::Focus(provider.focus(&pane)),
        };
        if to.send(done).is_err() {
            return;
        }
    }
}

impl Panes for Aside {
    fn read(&self, pane: &PaneKey, lines: u16) {
        let _ = self.wanted.send(Job::Read {
            pane: pane.clone(),
            lines,
        });
    }

    fn focus(&self, pane: &PaneKey) {
        let _ = self.wanted.send(Job::Focus { pane: pane.clone() });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::agents::testing::Asked;
    use crate::model::types::testing::key;
    use crate::model::types::Pane;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    /// Long enough that waiting past it means the answer is never coming.
    /// Never asserted against — what it does is turn a test that would hang
    /// into one that fails and says what it was owed.
    const LONG_ENOUGH: Duration = Duration::from_secs(30);

    /// What a provider that never answers calls itself.
    const WEDGED: &str = "a wedged provider";

    /// A provider asked aside, and the answers it sends back.
    fn asking<P: Agents + 'static>(provider: P) -> (Aside, Receiver<Answer>) {
        let (answered, answers) = mpsc::channel();
        (Aside::new(provider, answered), answers)
    }

    /// The next answer, or a failure saying none came.
    fn answer(answers: &Receiver<Answer>) -> Answer {
        answers
            .recv_timeout(LONG_ENOUGH)
            .expect("the provider answers every question it is asked, in time or not")
    }

    /// The lines an answer about a read carries.
    fn read(answer: Answer) -> Result<Vec<String>, RunFailure> {
        match answer {
            Answer::Read { read, .. } => read,
            Answer::Focused { .. } => panic!("a read was asked for and a focus came back"),
        }
    }

    /// A provider that answers every read with the same lines, and remembers
    /// what it was asked.
    struct Echo {
        said: Vec<String>,
        asked: Arc<Mutex<Vec<Asked>>>,
        late: AtomicBool,
    }

    impl Echo {
        fn saying(said: &[&str]) -> (Self, Arc<Mutex<Vec<Asked>>>) {
            let asked = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    said: said.iter().map(|line| (*line).to_string()).collect(),
                    asked: Arc::clone(&asked),
                    late: AtomicBool::new(false),
                },
                asked,
            )
        }

        /// The same, except that the first answer arrives long after the tail
        /// gave up waiting for it.
        fn saying_late(said: &[&str]) -> (Self, Arc<Mutex<Vec<Asked>>>) {
            let (echo, asked) = Self::saying(said);
            (
                Self {
                    late: AtomicBool::new(true),
                    ..echo
                },
                asked,
            )
        }

        fn note(&self, question: Asked) {
            self.asked
                .lock()
                .expect("no test panics holding this")
                .push(question);
            if self.late.swap(false, Ordering::SeqCst) {
                thread::sleep(PATIENCE * 2);
            }
        }
    }

    impl Agents for Echo {
        fn name(&self) -> &'static str {
            "an echoing provider"
        }

        fn sessions(&self) -> Result<Vec<String>, RunFailure> {
            panic!("the tail asks a provider to read and to focus, never for its sessions")
        }

        fn list(&self, _session: &str) -> Result<Vec<Pane>, RunFailure> {
            panic!("the tail asks a provider to read and to focus, never to list")
        }

        fn read(&self, pane: &PaneKey, lines: u16) -> Result<Vec<String>, RunFailure> {
            self.note(Asked::Read {
                pane: pane.clone(),
                lines,
            });
            Ok(self.said.clone())
        }

        fn focus(&self, pane: &PaneKey) -> Result<(), RunFailure> {
            self.note(Asked::Focus { pane: pane.clone() });
            Ok(())
        }
    }

    /// A provider that takes every question and answers none of them.
    struct Wedged;

    impl Agents for Wedged {
        fn name(&self) -> &'static str {
            WEDGED
        }

        fn sessions(&self) -> Result<Vec<String>, RunFailure> {
            panic!("the tail asks a provider to read and to focus, never for its sessions")
        }

        fn list(&self, _session: &str) -> Result<Vec<Pane>, RunFailure> {
            panic!("the tail asks a provider to read and to focus, never to list")
        }

        fn read(&self, _pane: &PaneKey, _lines: u16) -> Result<Vec<String>, RunFailure> {
            thread::sleep(Duration::from_secs(60));
            Ok(Vec::new())
        }

        fn focus(&self, _pane: &PaneKey) -> Result<(), RunFailure> {
            thread::sleep(Duration::from_secs(60));
            Ok(())
        }
    }

    /// What the provider was asked, in order.
    fn questions(asked: &Arc<Mutex<Vec<Asked>>>) -> Vec<Asked> {
        asked.lock().expect("the worker is done with it").clone()
    }

    fn a_read_of(pane: &str, lines: u16) -> Asked {
        Asked::Read {
            pane: key(pane),
            lines,
        }
    }

    #[test]
    fn a_read_asks_the_provider_for_what_is_on_the_pane_now() {
        let (echo, asked) = Echo::saying(&["one line", "and another"]);
        let (panes, answers) = asking(echo);

        panes.read(&key("w:p1"), 6);

        assert_eq!(
            answer(&answers),
            Answer::Read {
                pane: key("w:p1"),
                read: Ok(vec!["one line".to_string(), "and another".to_string()]),
            }
        );
        assert_eq!(questions(&asked), [a_read_of("w:p1", 6)]);
    }

    #[test]
    fn a_focus_is_the_one_thing_bdi_writes() {
        let (echo, asked) = Echo::saying(&[]);
        let (panes, answers) = asking(echo);

        panes.focus(&key("w:p1"));

        assert_eq!(
            answer(&answers),
            Answer::Focused {
                pane: key("w:p1"),
                focused: Ok(()),
            }
        );
        assert_eq!(questions(&asked), [Asked::Focus { pane: key("w:p1") }]);
    }

    /// A provider that never answers must not leave the tail waiting on it for
    /// ever: the band under the forest says what it is waiting for while a
    /// read is out, and it must go on to say the pane could not be read.
    #[test]
    fn a_provider_that_never_answers_is_waited_on_only_so_long() {
        let (panes, answers) = asking(Wedged);
        let started = std::time::Instant::now();

        panes.read(&key("w:p1"), 6);
        assert_eq!(
            read(answer(&answers)).map_err(|f| f.kind),
            Err(FailureKind::Unavailable)
        );
        assert!(
            started.elapsed() < PATIENCE * 3,
            "the read waited {:?}",
            started.elapsed()
        );

        let again = std::time::Instant::now();
        panes.read(&key("w:p1"), 6);
        assert!(read(answer(&answers)).is_err());
        assert!(
            again.elapsed() < PATIENCE,
            "a question is not asked while one is still out, so the second read did not wait again"
        );
    }

    /// The provider a question outstayed is the one named in what the tail is
    /// told instead, so the failure a reader of the code finds says which of
    /// them went quiet rather than naming whichever one was written down.
    #[test]
    fn an_unanswered_question_is_answered_against_the_provider_that_owed_it() {
        let (panes, answers) = asking(Wedged);

        panes.read(&key("w:p1"), 6);

        let failure = read(answer(&answers)).expect_err("a wedged provider answers nothing");
        assert_eq!(failure.program, WEDGED);
        assert!(
            failure.detail.contains(WEDGED),
            "the failure does not say who owed the answer: {failure:?}"
        );
    }

    /// An answer that came too late must cost the tail that one reading and
    /// no more. The late reply is thrown away before the next question is
    /// asked, so the pane is read again once the provider has caught up; were
    /// it left standing, every later ask would short-circuit and the pane
    /// would stop updating for the rest of the session with nothing to say it
    /// had.
    #[test]
    fn the_tail_reads_again_after_a_read_that_timed_out() {
        let (echo, asked) = Echo::saying_late(&["back from the dead"]);
        let (panes, answers) = asking(echo);

        panes.read(&key("w:p1"), 6);
        assert_eq!(
            read(answer(&answers)).map_err(|f| f.kind),
            Err(FailureKind::Unavailable),
            "the first answer outstays PATIENCE"
        );

        // Nothing is asked while a question is still out, so the tail asks
        // again on each refresh tick until the late answer has landed and
        // been drained. A tail that never recovers never leaves this loop.
        let gave_up_at = std::time::Instant::now() + PATIENCE * 5;
        let read = loop {
            panes.read(&key("w:p1"), 6);
            match read(answer(&answers)) {
                Ok(read) => break read,
                Err(failure) => assert!(
                    std::time::Instant::now() < gave_up_at,
                    "the tail never read the pane again: {failure:?}"
                ),
            }
            thread::sleep(Duration::from_millis(20));
        };

        assert_eq!(read, ["back from the dead"]);
        assert_eq!(
            questions(&asked),
            [a_read_of("w:p1", 6), a_read_of("w:p1", 6)],
            "the second reading came from the provider, not from the answer to the first"
        );
    }

    /// Every answer says which pane it is about, so the tail can drop one
    /// about a pane the reader has already moved off.
    #[test]
    fn an_answer_names_the_pane_it_is_about() {
        let (echo, _) = Echo::saying(&[]);
        let (panes, answers) = asking(echo);

        panes.read(&key("w:p1"), 6);
        panes.focus(&key("w:p2"));

        assert!(matches!(answer(&answers), Answer::Read { pane, .. } if pane == key("w:p1")));
        assert!(matches!(answer(&answers), Answer::Focused { pane, .. } if pane == key("w:p2")));
    }
}
