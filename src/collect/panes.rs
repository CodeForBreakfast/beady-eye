//! herdr's panes, asked for on a thread of their own.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use crate::collect::herdr;
use crate::collect::run::{FailureKind, RunFailure, Runner};

/// How long the tail waits on herdr before saying the pane could not be read.
///
/// A liveness backstop rather than a latency budget: herdr is a socket on
/// this machine and a healthy one answers in milliseconds. What this bounds
/// is what the band under the forest says, and nothing else — no thread that
/// reads keys or draws waits on it, so `q` and `^C` are answered while a read
/// is still outstanding.
const PATIENCE: Duration = Duration::from_secs(2);

/// Reading a pane, and focusing it — everything the tail asks of herdr.
///
/// Asking is the whole of it: neither of these waits for what it asked, and
/// the answer arrives later as an [`Answer`]. That is what keeps herdr off
/// the thread that reads keys, and it is a seam as well as a thread, so the
/// loop can be driven over a herdr that answers whatever a test needs it to,
/// including nothing.
pub trait Panes {
    fn read(&self, pane: &str, lines: u16);

    /// Bring a pane to the front. The only write `bdi` performs.
    fn focus(&self, pane: &str);
}

/// What herdr said, and which pane it said it about.
///
/// The pane is carried back because the selection moves while herdr is
/// answering: an answer about a pane the reader has already left is dropped,
/// and nothing but the pane's own name in the answer can say that it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Read {
        pane: String,
        read: Result<Vec<String>, RunFailure>,
    },
    Focused {
        pane: String,
        focused: Result<(), RunFailure>,
    },
}

/// What the tail asks herdr for.
enum Job {
    Read { pane: String, lines: u16 },
    Focus { pane: String },
}

/// What came back. The two arms are kept apart so a reply that outstayed its
/// welcome cannot be read as the answer to the next question.
enum Done {
    Read(Result<Vec<String>, RunFailure>),
    Focus(Result<(), RunFailure>),
}

/// herdr, asked on threads of its own.
///
/// Two, and each has one job. One runs herdr and blocks in it for as long as
/// herdr takes. The other is the one that waits `PATIENCE` for that answer
/// and gives up on it, and it exists so that nothing the reader is sitting in
/// front of has to: asking is a send, and what comes back arrives wherever
/// `answers` goes.
///
/// One question at a time, on both. A question that outstays `PATIENCE` is
/// not abandoned — its answer is thrown away when it finally arrives — and
/// none is asked while one is still out, so a herdr that has stopped
/// answering costs one waiting thread rather than one per poll.
pub struct Herdr {
    wanted: Sender<Job>,
}

impl Herdr {
    /// Start herdr's threads, with every answer sent on to `answers`.
    pub fn new<R, A>(runner: R, answers: Sender<A>) -> Self
    where
        R: Runner + Send + 'static,
        A: From<Answer> + Send + 'static,
    {
        let (wanted, wants) = mpsc::channel();
        let (asking, asked) = mpsc::channel();
        let (answered, done) = mpsc::channel();

        thread::spawn(move || work(&runner, &asked, &answered));
        thread::spawn(move || {
            ask(
                &wants,
                &mut Patience {
                    asking,
                    done,
                    outstanding: 0,
                },
                &answers,
            );
        });

        Self { wanted }
    }
}

/// How long herdr gets, and what is still owed from the last time it was
/// asked.
struct Patience {
    asking: Sender<Job>,
    done: Receiver<Done>,
    outstanding: usize,
}

impl Patience {
    /// Ask herdr one thing and wait `PATIENCE` for it.
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

    /// The same, as the answer the tail is owed either way. A question herdr
    /// did not answer in time is answered here instead, because the band
    /// under the forest is drawn whatever herdr does.
    fn answer(&mut self, job: Job) -> Answer {
        match job {
            Job::Read { pane, lines } => {
                let read = match self.waited(Job::Read {
                    pane: pane.clone(),
                    lines,
                }) {
                    Some(Done::Read(read)) => read,
                    _ => Err(no_answer()),
                };
                Answer::Read { pane, read }
            }
            Job::Focus { pane } => {
                let focused = match self.waited(Job::Focus { pane: pane.clone() }) {
                    Some(Done::Focus(focused)) => focused,
                    _ => Err(no_answer()),
                };
                Answer::Focused { pane, focused }
            }
        }
    }
}

/// Put the tail's questions to herdr, one at a time, until it stops asking or
/// there is nobody left to tell.
fn ask<A: From<Answer>>(wants: &Receiver<Job>, patience: &mut Patience, to: &Sender<A>) {
    while let Ok(job) = wants.recv() {
        if to.send(A::from(patience.answer(job))).is_err() {
            return;
        }
    }
}

/// What `bdi` says happened when herdr said nothing at all.
///
/// `detail` never reaches the screen — the phrases do — so this says what a
/// reader of the code needs and not what a reader of the screen does.
fn no_answer() -> RunFailure {
    RunFailure {
        kind: FailureKind::Unavailable,
        program: "herdr".to_string(),
        detail: "herdr did not answer in time".to_string(),
    }
}

/// Answer herdr's questions until the tail stops asking.
fn work(runner: &dyn Runner, asked: &Receiver<Job>, to: &Sender<Done>) {
    while let Ok(job) = asked.recv() {
        let done = match job {
            Job::Read { pane, lines } => Done::Read(herdr::agent_read(runner, &pane, lines)),
            Job::Focus { pane } => Done::Focus(herdr::agent_focus(runner, &pane)),
        };
        if to.send(done).is_err() {
            return;
        }
    }
}

impl Panes for Herdr {
    fn read(&self, pane: &str, lines: u16) {
        let _ = self.wanted.send(Job::Read {
            pane: pane.to_string(),
            lines,
        });
    }

    fn focus(&self, pane: &str) {
        let _ = self.wanted.send(Job::Focus {
            pane: pane.to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::run::Env;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    /// Long enough that waiting past it means the answer is never coming.
    /// Never asserted against — what it does is turn a test that would hang
    /// into one that fails and says what it was owed.
    const LONG_ENOUGH: Duration = Duration::from_secs(30);

    /// A herdr asked over a runner of the test's choosing, and the answers it
    /// sends back.
    fn asking<R: Runner + Send + 'static>(runner: R) -> (Herdr, Receiver<Answer>) {
        let (answered, answers) = mpsc::channel();
        (Herdr::new(runner, answered), answers)
    }

    /// The next answer, or a failure saying none came.
    fn answer(answers: &Receiver<Answer>) -> Answer {
        answers
            .recv_timeout(LONG_ENOUGH)
            .expect("herdr answers every question it is asked, in time or not")
    }

    /// The lines an answer about a read carries.
    fn read(answer: Answer) -> Result<Vec<String>, RunFailure> {
        match answer {
            Answer::Read { read, .. } => read,
            Answer::Focused { .. } => panic!("a read was asked for and a focus came back"),
        }
    }

    /// A runner that answers every command with the same text, and remembers
    /// the command line it was given.
    struct Echo {
        said: String,
        ran: Arc<Mutex<Vec<String>>>,
        late: AtomicBool,
    }

    impl Echo {
        fn saying(said: &str) -> (Self, Arc<Mutex<Vec<String>>>) {
            let ran = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    said: said.to_string(),
                    ran: Arc::clone(&ran),
                    late: AtomicBool::new(false),
                },
                ran,
            )
        }

        /// The same, except that the first answer arrives long after the tail
        /// gave up waiting for it.
        fn saying_late(said: &str) -> (Self, Arc<Mutex<Vec<String>>>) {
            let (echo, ran) = Self::saying(said);
            (
                Self {
                    late: AtomicBool::new(true),
                    ..echo
                },
                ran,
            )
        }
    }

    impl Runner for Echo {
        fn run(
            &self,
            program: &str,
            args: &[&str],
            _cwd: Option<&Path>,
            _env: &Env,
        ) -> Result<String, RunFailure> {
            self.ran
                .lock()
                .expect("no test panics holding this")
                .push(format!("{program} {}", args.join(" ")));
            if self.late.swap(false, Ordering::SeqCst) {
                thread::sleep(PATIENCE * 2);
            }
            Ok(self.said.clone())
        }
    }

    #[test]
    fn a_read_asks_herdr_for_what_is_on_the_pane_now() {
        let (echo, ran) = Echo::saying("one line\nand another\n");
        let (herdr, answers) = asking(echo);

        herdr.read("w:p1", 6);

        assert_eq!(
            answer(&answers),
            Answer::Read {
                pane: "w:p1".to_string(),
                read: Ok(vec!["one line".to_string(), "and another".to_string()]),
            }
        );
        assert_eq!(
            *ran.lock().expect("the worker is done with it"),
            ["herdr agent read w:p1 --source visible --lines 6 --format ansi"]
        );
    }

    #[test]
    fn a_focus_is_the_one_thing_bdi_writes() {
        let (echo, ran) = Echo::saying("");
        let (herdr, answers) = asking(echo);

        herdr.focus("w:p1");

        assert_eq!(
            answer(&answers),
            Answer::Focused {
                pane: "w:p1".to_string(),
                focused: Ok(()),
            }
        );
        assert_eq!(
            *ran.lock().expect("the worker is done with it"),
            ["herdr agent focus w:p1"]
        );
    }

    /// A herdr that never answers must not leave the tail waiting on it for
    /// ever: the band under the forest says what it is waiting for while a
    /// read is out, and it must go on to say the pane could not be read.
    #[test]
    fn a_herdr_that_never_answers_is_waited_on_only_so_long() {
        struct Wedged;

        impl Runner for Wedged {
            fn run(
                &self,
                _program: &str,
                _args: &[&str],
                _cwd: Option<&Path>,
                _env: &Env,
            ) -> Result<String, RunFailure> {
                thread::sleep(Duration::from_secs(60));
                Ok(String::new())
            }
        }

        let (herdr, answers) = asking(Wedged);
        let started = std::time::Instant::now();

        herdr.read("w:p1", 6);
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
        herdr.read("w:p1", 6);
        assert!(read(answer(&answers)).is_err());
        assert!(
            again.elapsed() < PATIENCE,
            "a question is not asked while one is still out, so the second read did not wait again"
        );
    }

    /// An answer that came too late must cost the tail that one reading and
    /// no more. The late reply is thrown away before the next question is
    /// asked, so the pane is read again once herdr has caught up; were it
    /// left standing, every later ask would short-circuit and the pane would
    /// stop updating for the rest of the session with nothing to say it had.
    #[test]
    fn the_tail_reads_again_after_a_read_that_timed_out() {
        let (echo, ran) = Echo::saying_late("back from the dead\n");
        let (herdr, answers) = asking(echo);

        herdr.read("w:p1", 6);
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
            herdr.read("w:p1", 6);
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
            *ran.lock().expect("the worker is done with it"),
            [
                "herdr agent read w:p1 --source visible --lines 6 --format ansi",
                "herdr agent read w:p1 --source visible --lines 6 --format ansi"
            ],
            "the second reading came from herdr, not from the answer to the first"
        );
    }

    /// Every answer says which pane it is about, so the tail can drop one
    /// about a pane the reader has already moved off.
    #[test]
    fn an_answer_names_the_pane_it_is_about() {
        let (echo, _) = Echo::saying("");
        let (herdr, answers) = asking(echo);

        herdr.read("w:p1", 6);
        herdr.focus("w:p2");

        assert!(matches!(answer(&answers), Answer::Read { pane, .. } if pane == "w:p1"));
        assert!(matches!(answer(&answers), Answer::Focused { pane, .. } if pane == "w:p2"));
    }
}
