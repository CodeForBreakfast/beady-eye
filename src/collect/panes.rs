//! herdr's panes, read and focused on a thread of their own.

use std::cell::Cell;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use crate::collect::herdr;
use crate::collect::run::{FailureKind, RunFailure, Runner};

/// How long the loop waits on herdr before drawing the tail without it.
///
/// A liveness backstop rather than a latency budget: herdr is a socket on
/// this machine and a healthy one answers in milliseconds. What this bounds
/// is the loop, which reads keys on the same thread — an unbounded wait here
/// would swallow `q` and `^C` with the alternate screen still up.
const PATIENCE: Duration = Duration::from_secs(2);

/// Reading a pane, and focusing it — everything the tail asks of herdr.
///
/// A seam rather than a direct call so the loop can be driven over a herdr
/// that answers whatever a test needs it to, including nothing.
pub trait Panes {
    fn read(&self, pane: &str, lines: u16) -> Result<Vec<String>, RunFailure>;

    /// Bring a pane to the front. The only write `bdi` performs.
    fn focus(&self, pane: &str) -> Result<(), RunFailure>;
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

/// herdr, asked on a thread of its own.
///
/// One thread, one question at a time. A question that outstays `PATIENCE`
/// is not abandoned — its answer is thrown away when it finally arrives —
/// and none is asked while one is still out, so a herdr that has stopped
/// answering costs one waiting thread rather than one per poll.
pub struct Herdr {
    asking: Sender<Job>,
    answers: Receiver<Done>,
    outstanding: Cell<usize>,
}

impl Herdr {
    pub fn new<R: Runner + Send + 'static>(runner: R) -> Self {
        let (asking, asked) = mpsc::channel();
        let (answered, answers) = mpsc::channel();
        thread::spawn(move || work(&runner, &asked, &answered));

        Self {
            asking,
            answers,
            outstanding: Cell::new(0),
        }
    }

    /// Ask herdr one thing and wait `PATIENCE` for it.
    fn ask(&self, job: Job) -> Option<Done> {
        while self.outstanding.get() > 0 && self.answers.try_recv().is_ok() {
            self.outstanding.set(self.outstanding.get() - 1);
        }
        if self.outstanding.get() > 0 || self.asking.send(job).is_err() {
            return None;
        }

        self.outstanding.set(1);
        let answer = self.answers.recv_timeout(PATIENCE).ok();
        self.outstanding.set(usize::from(answer.is_none()));
        answer
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
    fn read(&self, pane: &str, lines: u16) -> Result<Vec<String>, RunFailure> {
        match self.ask(Job::Read {
            pane: pane.to_string(),
            lines,
        }) {
            Some(Done::Read(read)) => read,
            _ => Err(no_answer()),
        }
    }

    fn focus(&self, pane: &str) -> Result<(), RunFailure> {
        match self.ask(Job::Focus {
            pane: pane.to_string(),
        }) {
            Some(Done::Focus(focused)) => focused,
            _ => Err(no_answer()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::run::Env;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    /// A runner that answers every command with the same text, and remembers
    /// the command line it was given.
    struct Echo {
        said: String,
        ran: Arc<Mutex<Vec<String>>>,
        late: Cell<bool>,
    }

    impl Echo {
        fn saying(said: &str) -> (Self, Arc<Mutex<Vec<String>>>) {
            let ran = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    said: said.to_string(),
                    ran: Arc::clone(&ran),
                    late: Cell::new(false),
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
                    late: Cell::new(true),
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
            if self.late.replace(false) {
                thread::sleep(PATIENCE * 2);
            }
            Ok(self.said.clone())
        }
    }

    #[test]
    fn a_read_asks_herdr_for_what_is_on_the_pane_now() {
        let (echo, ran) = Echo::saying("one line\nand another\n");
        let herdr = Herdr::new(echo);

        assert_eq!(
            herdr.read("w:p1", 6).expect("the runner answers"),
            ["one line", "and another"]
        );
        assert_eq!(
            *ran.lock().expect("the worker is done with it"),
            ["herdr agent read w:p1 --source visible --lines 6 --format text"]
        );
    }

    #[test]
    fn a_focus_is_the_one_thing_bdi_writes() {
        let (echo, ran) = Echo::saying("");
        let herdr = Herdr::new(echo);

        assert_eq!(herdr.focus("w:p1"), Ok(()));
        assert_eq!(
            *ran.lock().expect("the worker is done with it"),
            ["herdr agent focus w:p1"]
        );
    }

    /// A herdr that never answers must not take the loop down with it: the
    /// loop reads keys on the same thread, and a wait with no end swallows
    /// `q` and `^C` with the alternate screen still up.
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

        let herdr = Herdr::new(Wedged);
        let started = std::time::Instant::now();

        assert_eq!(
            herdr.read("w:p1", 6).map_err(|f| f.kind),
            Err(FailureKind::Unavailable)
        );
        assert!(
            started.elapsed() < PATIENCE * 3,
            "the read waited {:?}",
            started.elapsed()
        );

        let again = std::time::Instant::now();
        assert!(herdr.read("w:p1", 6).is_err());
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
        let herdr = Herdr::new(echo);

        assert_eq!(
            herdr.read("w:p1", 6).map_err(|f| f.kind),
            Err(FailureKind::Unavailable),
            "the first answer outstays PATIENCE"
        );

        // Nothing is asked while a question is still out, so the tail asks
        // again on each refresh tick until the late answer has landed and
        // been drained. A tail that never recovers never leaves this loop.
        let gave_up_at = std::time::Instant::now() + PATIENCE * 5;
        let read = loop {
            match herdr.read("w:p1", 6) {
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
                "herdr agent read w:p1 --source visible --lines 6 --format text",
                "herdr agent read w:p1 --source visible --lines 6 --format text"
            ],
            "the second reading came from herdr, not from the answer to the first"
        );
    }
}
