//! The tail of the selected bead's pane, shown beneath the forest.

use std::cell::Cell;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use crate::collect::herdr;
use crate::collect::run::{FailureKind, RunFailure, Runner};
use crate::model::join::{AgentRef, BeadKey, Conflict};
use crate::model::snapshot::{HerdrState, Snapshot};
use crate::view::forest::{Content, Forest, Item};
use crate::view::phrase;

/// How many lines of the pane the tail shows. The band reserved for it is
/// this plus the rule that names the pane.
pub const LINES: u16 = 6;

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

/// What a pane has most recently written, or why there is nothing to show.
///
/// The two are one type because the band under the forest is reserved either
/// way: there is always something to draw there, and a band left blank would
/// read as a pane sitting quiet rather than as no pane at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tail {
    Pane { pane: String, lines: Vec<String> },
    Silent(&'static str),
}

/// What the selection points the tail at.
///
/// `Pane` is the pane itself rather than the agent holding it, because a pane
/// in one of the groups below the forest has no agent to hold it: it is live
/// and named, and nothing joined it to a bead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target<'a> {
    Pane(&'a str),
    /// A bead nobody is working.
    NoAgent,
    /// A tree's own line, one of the groups below the forest, or something in
    /// a group that names no pane.
    NotABead,
}

impl Target<'_> {
    pub fn pane(&self) -> Option<&str> {
        match self {
            Target::Pane(pane) => Some(pane),
            Target::NoAgent | Target::NotABead => None,
        }
    }
}

/// The pane the selection points at, where it points at one.
///
/// Two roads reach a pane. A bead's row names one only by way of the join, so
/// it is looked up from the key; a line in one of the groups below the forest
/// carries the pane already, because the thing it stands for is the pane. A
/// tree's own line takes neither road: it carries its root's key, and tailing
/// whatever happens to be on the root would answer a question nobody asked.
pub fn target(forest: &Forest) -> Target<'_> {
    let Some(line) = forest.lines().get(forest.selected_line()) else {
        return Target::NotABead;
    };

    match &line.content {
        Content::Bead(_) => line
            .bead
            .as_ref()
            .and_then(|key| agent(forest.snapshot(), key))
            .map_or(Target::NoAgent, |agent| Target::Pane(&agent.pane)),
        Content::Item(item) => named_pane(item).map_or(Target::NotABead, Target::Pane),
        Content::Tree(_) | Content::Elided { .. } | Content::Note(_) | Content::Group(_) => {
            Target::NotABead
        }
    }
}

/// The pane one of the groups' entries names, where it names exactly one.
///
/// A loose pane and an unconfigured one are panes; that is the whole of what
/// they are. A conflict is not, but two of its four shapes turn on a single
/// pane and name it. The other two name two panes and several, so there is
/// nothing to pick rather than nothing to show.
fn named_pane(item: &Item) -> Option<&str> {
    match item {
        Item::Loose(loose) => Some(&loose.pane),
        Item::Unconfigured(unconfigured) => Some(&unconfigured.pane),
        Item::Conflict(
            Conflict::SeveralBeadsNameOnePane { pane, .. }
            | Conflict::PaneInAnotherProject { pane, .. },
        ) => Some(pane),
        Item::Conflict(
            Conflict::BeadAndPaneDisagree { .. } | Conflict::SeveralPanesNameOneBead { .. },
        )
        | Item::Failed(_)
        | Item::Hidden(_) => None,
    }
}

/// One bead's agent, found the only way a bead can be found across trackers.
fn agent<'a>(snapshot: &'a Snapshot, key: &BeadKey) -> Option<&'a AgentRef> {
    snapshot
        .trees
        .iter()
        .filter(|tree| tree.project == key.project)
        .flat_map(|tree| &tree.nodes)
        .find(|node| node.id == key.id)?
        .agent
        .as_ref()
}

/// Read the tail for whatever the selection points at.
///
/// Every way this can come back empty says so in `bdi`'s own words. The order
/// matters: with no herdr there is no pane on any row, so that is answered
/// before the row is looked at.
pub fn tail(forest: &Forest, panes: &dyn Panes, lines: u16) -> Tail {
    if forest.snapshot().herdr == HerdrState::Unavailable {
        return Tail::Silent(phrase::no_herdr_to_tail());
    }

    let pane = match target(forest) {
        Target::NotABead => return Tail::Silent(phrase::no_bead_to_tail()),
        Target::NoAgent => return Tail::Silent(phrase::no_agent_to_tail()),
        Target::Pane(pane) => pane.to_string(),
    };

    match panes.read(&pane, lines) {
        Ok(lines) => Tail::Pane { pane, lines },
        Err(failure) => Tail::Silent(phrase::pane_unreadable(failure.kind)),
    }
}

/// Whether the tail must be read again for what the selection is on now.
///
/// A pane's rows are the one part of the tail worth keeping, because they are
/// the one part that costs a herdr call. Everything else the tail can say it
/// works out from the selected row alone, so a row that names no pane is
/// always read again and the reading is free.
///
/// So the tail stands only while the selection still names the pane it was
/// read from: scrolling within that pane's rows leaves it where it is, and
/// its rows are re-read on the refresh tick like everything else.
pub fn moved_on(forest: &Forest, showing: Option<&str>) -> bool {
    match target(forest).pane() {
        Some(pane) => showing != Some(pane),
        None => true,
    }
}

/// Focus the pane the selection points at, saying nothing where it points at
/// none.
///
/// Most rows carry no pane, so `⏎` on one is not a mistake and there is
/// nothing to report: there is simply nothing to focus. A pane that is named
/// and will not come is a different thing, and says so where the tail is.
pub fn focus(forest: &Forest, panes: &dyn Panes) -> Option<Tail> {
    let pane = target(forest).pane()?.to_string();
    match panes.focus(&pane) {
        Ok(()) => None,
        Err(failure) => Some(Tail::Silent(phrase::pane_unreadable(failure.kind))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::herdr::PaneStatus;
    use crate::collect::run::Env;
    use crate::model::join::JoinSource;
    use crate::model::snapshot::{
        Counts, FailedProject, Filter, LoosePane, Node, TrackerFailure, TrackerState, Tree,
        UnconfiguredPane,
    };
    use crate::model::types::{Edge, Status};
    use crate::view::forest::{self, GroupKind};
    use crate::view::{Action, Motion};
    use chrono::Utc;
    use pretty_assertions::assert_eq;
    use std::cell::RefCell;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    /// A herdr that answers however the test says, and remembers what it was
    /// asked.
    #[derive(Default)]
    struct Fake {
        read: RefCell<Option<Result<Vec<String>, RunFailure>>>,
        focus: RefCell<Option<Result<(), RunFailure>>>,
        asked: RefCell<Vec<String>>,
        focused: RefCell<Vec<String>>,
    }

    impl Fake {
        fn reading(lines: &[&str]) -> Self {
            Self {
                read: RefCell::new(Some(Ok(lines.iter().map(|l| (*l).to_string()).collect()))),
                ..Self::default()
            }
        }

        fn refusing(kind: FailureKind) -> Self {
            Self {
                read: RefCell::new(Some(Err(failure(kind)))),
                focus: RefCell::new(Some(Err(failure(kind)))),
                ..Self::default()
            }
        }
    }

    impl Panes for Fake {
        fn read(&self, pane: &str, lines: u16) -> Result<Vec<String>, RunFailure> {
            self.asked.borrow_mut().push(format!("{pane} {lines}"));
            match &*self.read.borrow() {
                Some(Ok(lines)) => Ok(lines.clone()),
                Some(Err(failure)) => Err(failure.clone()),
                None => Ok(Vec::new()),
            }
        }

        fn focus(&self, pane: &str) -> Result<(), RunFailure> {
            self.focused.borrow_mut().push(pane.to_string());
            match &*self.focus.borrow() {
                Some(Err(failure)) => Err(failure.clone()),
                _ => Ok(()),
            }
        }
    }

    fn failure(kind: FailureKind) -> RunFailure {
        RunFailure {
            kind,
            program: "herdr".to_string(),
            detail: "the test said so".to_string(),
        }
    }

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

    fn agent_on(pane: &str) -> AgentRef {
        AgentRef {
            pane: pane.to_string(),
            pane_status: PaneStatus::Working,
            title: None,
            source: JoinSource::AgentPane,
        }
    }

    fn node(id: &str, depth: u16, agent: Option<AgentRef>) -> Node {
        Node {
            id: id.to_string(),
            title: "a bead in the tree".to_string(),
            status: Status::InProgress,
            issue_type: "task".to_string(),
            priority: 2,
            depth,
            edge: if depth == 0 {
                None
            } else {
                Some(Edge::ParentChild)
            },
            ready: true,
            blocked_by: Vec::new(),
            started_at: None,
            closed_at: None,
            badges: Vec::new(),
            agent,
            anomalies: Vec::new(),
            truncated: false,
        }
    }

    /// One tree: a root, two beads one agent is on, and two beads nobody is
    /// on. The pairs are what tell a tail that stands apart from one that
    /// must be read again.
    fn snapshot(herdr: HerdrState) -> Snapshot {
        let tree = Tree {
            project: "orbital".to_string(),
            root: "orb-7".to_string(),
            title: "lift the ground station".to_string(),
            counts: Counts {
                total: 5,
                closed: 0,
                live_agents: 2,
                anomalies: 0,
            },
            tracker: TrackerState::Ok,
            nodes: vec![
                node("orb-7", 0, None),
                node("orb-7.1", 1, Some(agent_on("w:p1"))),
                node("orb-7.2", 1, None),
                node("orb-7.3", 1, None),
                node("orb-7.4", 1, Some(agent_on("w:p1"))),
            ],
            dangling: Vec::new(),
            unreachable: Vec::new(),
        };

        Snapshot {
            generated_at: Utc::now(),
            herdr,
            filter: Filter::All,
            trees: vec![tree.clone()],
            hidden_trees: Vec::new(),
            failed_projects: Vec::new(),
            unattributed: Vec::new(),
            unconfigured: Vec::new(),
            conflicts: Vec::new(),
            collected: vec![tree],
        }
    }

    /// The forest with the selection moved down `steps` rows from the header
    /// it starts on.
    fn selecting(steps: usize, herdr: HerdrState) -> Forest {
        let mut forest = forest::flatten(&snapshot(herdr));
        for _ in 0..steps {
            forest.apply(Action::Move(Motion::NextRow));
        }
        forest
    }

    #[test]
    fn the_tail_follows_the_selection() {
        let panes = Fake::reading(&["rebuilt .#thinkpad, generation 541"]);

        let on_the_agent = tail(&selecting(1, HerdrState::Ok), &panes, LINES);

        assert_eq!(
            on_the_agent,
            Tail::Pane {
                pane: "w:p1".to_string(),
                lines: vec!["rebuilt .#thinkpad, generation 541".to_string()],
            }
        );
        assert_eq!(
            tail(&selecting(2, HerdrState::Ok), &panes, LINES),
            Tail::Silent(phrase::no_agent_to_tail()),
            "the row below is a bead nobody is working"
        );
    }

    #[test]
    fn the_pane_is_asked_for_the_lines_the_tail_shows() {
        let panes = Fake::reading(&[]);

        tail(&selecting(1, HerdrState::Ok), &panes, LINES);

        assert_eq!(*panes.asked.borrow(), ["w:p1 6"]);
    }

    #[test]
    fn a_tree_header_has_no_pane_to_tail() {
        let panes = Fake::reading(&["nothing should reach the screen"]);

        assert_eq!(
            tail(&selecting(0, HerdrState::Ok), &panes, LINES),
            Tail::Silent(phrase::no_bead_to_tail())
        );
        assert!(
            panes.asked.borrow().is_empty(),
            "a header names no pane, so herdr was never asked"
        );
    }

    #[test]
    fn no_herdr_means_no_pane_to_read() {
        let panes = Fake::reading(&["nothing should reach the screen"]);

        assert_eq!(
            tail(&selecting(1, HerdrState::Unavailable), &panes, LINES),
            Tail::Silent(phrase::no_herdr_to_tail()),
            "with no herdr there is no pane on any row, whatever the row says"
        );
        assert!(panes.asked.borrow().is_empty());
    }

    /// A pane that went away between one poll and the next. The band says so
    /// rather than emptying: an empty band reads as a pane with nothing to
    /// say.
    #[test]
    fn a_pane_that_has_gone_degrades_to_a_phrase() {
        let panes = Fake::refusing(FailureKind::Gone);

        assert_eq!(
            tail(&selecting(1, HerdrState::Ok), &panes, LINES),
            Tail::Silent(phrase::pane_unreadable(FailureKind::Gone))
        );
    }

    #[test]
    fn every_way_a_read_can_fail_is_said_rather_than_drawn_blank() {
        for kind in [
            FailureKind::Auth,
            FailureKind::Unavailable,
            FailureKind::Gone,
            FailureKind::Busy,
            FailureKind::Exec,
            FailureKind::Parse,
        ] {
            let panes = Fake::refusing(kind);

            assert_eq!(
                tail(&selecting(1, HerdrState::Ok), &panes, LINES),
                Tail::Silent(phrase::pane_unreadable(kind)),
                "for {kind:?}"
            );
        }
    }

    #[test]
    fn enter_focuses_the_pane_the_selection_is_on() {
        let panes = Fake::default();

        assert_eq!(focus(&selecting(1, HerdrState::Ok), &panes), None);
        assert_eq!(*panes.focused.borrow(), ["w:p1"]);
    }

    /// Most rows carry no pane and the mock never shows one on `⏎`. There is
    /// nothing to focus and nothing has gone wrong.
    #[test]
    fn enter_on_a_row_with_no_pane_is_a_no_op_and_not_an_error() {
        let panes = Fake::default();

        for steps in [0, 2] {
            assert_eq!(focus(&selecting(steps, HerdrState::Ok), &panes), None);
        }
        assert!(panes.focused.borrow().is_empty());
    }

    #[test]
    fn a_pane_that_will_not_come_to_the_front_says_so_where_the_tail_is() {
        let panes = Fake::refusing(FailureKind::Gone);

        assert_eq!(
            focus(&selecting(1, HerdrState::Ok), &panes),
            Some(Tail::Silent(phrase::pane_unreadable(FailureKind::Gone)))
        );
    }

    #[test]
    fn the_tail_is_read_again_only_where_the_selection_has_left_the_pane() {
        assert!(
            !moved_on(&selecting(1, HerdrState::Ok), Some("w:p1")),
            "the selection is still on the pane the tail is showing"
        );
        assert!(moved_on(&selecting(2, HerdrState::Ok), Some("w:p1")));
        assert!(moved_on(&selecting(1, HerdrState::Ok), None));
    }

    /// A header and a bead nobody is working name no pane between them, and
    /// say different things. The tail read for the one is the wrong tail for
    /// the other.
    #[test]
    fn leaving_a_header_for_a_bead_nobody_is_working_is_a_move() {
        assert!(moved_on(&selecting(2, HerdrState::Ok), None));
    }

    #[test]
    fn moving_between_two_beads_nobody_is_working_is_a_move() {
        assert!(moved_on(&selecting(3, HerdrState::Ok), None));
    }

    /// One agent can be on more than one bead, and the pane its rows share is
    /// already on screen. This is what the comparison exists for.
    #[test]
    fn two_beads_on_one_pane_do_not_read_it_twice() {
        assert!(!moved_on(&selecting(4, HerdrState::Ok), Some("w:p1")));
    }

    /// The property stated rather than sampled: wherever `moved_on` says the
    /// tail stands, the tail the new row calls for *is* the tail on screen.
    /// So no phrase can outlive the row it was said for.
    #[test]
    fn a_tail_that_stands_is_the_tail_the_new_row_calls_for() {
        let panes = Fake::reading(&["rebuilt .#thinkpad, generation 541"]);

        for from in 0..5 {
            let was = selecting(from, HerdrState::Ok);
            let showing = target(&was).pane().map(str::to_string);
            let on_screen = tail(&was, &panes, LINES);

            for onto in 0..5 {
                let now = selecting(onto, HerdrState::Ok);
                if !moved_on(&now, showing.as_deref()) {
                    assert_eq!(
                        tail(&now, &panes, LINES),
                        on_screen,
                        "the tail read on row {from} was left standing on row {onto}"
                    );
                }
            }
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
    fn key(id: &str) -> BeadKey {
        BeadKey {
            project: "orbital".to_string(),
            id: id.to_string(),
        }
    }

    fn loose() -> LoosePane {
        LoosePane {
            pane: "w:p2".to_string(),
            project: "orbital".to_string(),
            cwd: "/tmp/bdi-ground/orbital".to_string(),
            pane_status: PaneStatus::Working,
        }
    }

    /// A second pane in the same group, so that a group can come back from a
    /// refresh in a different order than it went in.
    fn another_loose() -> LoosePane {
        LoosePane {
            pane: "w:p10".to_string(),
            project: "orbital".to_string(),
            cwd: "/tmp/bdi-ground/orbital".to_string(),
            pane_status: PaneStatus::Idle,
        }
    }

    fn unconfigured() -> UnconfiguredPane {
        UnconfiguredPane {
            pane: "w:p3".to_string(),
            cwd: "/tmp/bdi-ground/lander".to_string(),
            pane_status: PaneStatus::Working,
        }
    }

    fn pane_in_another_project() -> Conflict {
        Conflict::PaneInAnotherProject {
            bead: key("orb-7.2"),
            pane: "w:p4".to_string(),
            pane_project: None,
        }
    }

    fn several_beads_name_one_pane() -> Conflict {
        Conflict::SeveralBeadsNameOnePane {
            pane: "w:p5".to_string(),
            beads: vec![key("orb-7.2"), key("orb-7.3")],
        }
    }

    fn bead_and_pane_disagree() -> Conflict {
        Conflict::BeadAndPaneDisagree {
            bead: key("orb-7.2"),
            named_by_bead: "w:p6".to_string(),
            named_by_pane: "w:p7".to_string(),
        }
    }

    fn several_panes_name_one_bead() -> Conflict {
        Conflict::SeveralPanesNameOneBead {
            bead: key("orb-7.3"),
            panes: vec!["w:p8".to_string(), "w:p9".to_string()],
        }
    }

    fn failed_project() -> FailedProject {
        FailedProject {
            project: "lander".to_string(),
            tracker: TrackerFailure::Unavailable,
        }
    }

    /// The same tree, and beneath it the groups: three live panes no bead
    /// claims, all four shapes of conflict, and a project whose tracker never
    /// answered. Five of those lines turn on one pane and four turn on none,
    /// which is what tells a row the tail can follow from a row it cannot.
    fn snapshot_with_groups(herdr: HerdrState) -> Snapshot {
        Snapshot {
            failed_projects: vec![failed_project()],
            unattributed: vec![loose(), another_loose()],
            unconfigured: vec![unconfigured()],
            conflicts: vec![
                pane_in_another_project(),
                several_beads_name_one_pane(),
                bead_and_pane_disagree(),
                several_panes_name_one_bead(),
            ],
            ..snapshot(herdr)
        }
    }

    /// That forest with every group open, as a reader who pressed the key on
    /// each of them in turn would have it.
    fn with_groups_open(herdr: HerdrState) -> Forest {
        let mut forest = forest::flatten(&snapshot_with_groups(herdr));
        while let Some(shut) = forest.lines().iter().find_map(|line| match &line.content {
            Content::Group(group) if line.folded == Some(false) => Some(group.kind),
            _ => None,
        }) {
            step_onto(
                &mut forest,
                |content| matches!(content, Content::Group(group) if group.kind == shut),
            );
            forest.apply(Action::ExpandOrChild);
        }
        forest
    }

    /// Move the selection down to the row `wanted` picks out, by pressing
    /// down until it is there.
    ///
    /// A row is named by what is on it rather than by where it sits, because
    /// where it sits moves: a tree folds shut as the selection leaves it, and
    /// every line below it shifts up under a test still holding the old
    /// number.
    fn step_onto(forest: &mut Forest, wanted: impl Fn(&Content) -> bool) {
        forest.apply(Action::Move(Motion::FirstRow));
        while !wanted(&forest.lines()[forest.selected_line()].content) {
            assert!(
                forest.apply(Action::Move(Motion::NextRow)),
                "pressing down from the top never reached the row"
            );
        }
    }

    fn onto(wanted: &Item) -> impl Fn(&Content) -> bool + use<'_> {
        move |content| matches!(content, Content::Item(item) if item == wanted)
    }

    /// The forest with the selection `steps` rows below the top, every group
    /// open and the row reached by pressing down.
    fn stepping(steps: usize, herdr: HerdrState) -> Forest {
        let mut forest = with_groups_open(herdr);
        forest.apply(Action::Move(Motion::FirstRow));
        for _ in 0..steps {
            forest.apply(Action::Move(Motion::NextRow));
        }
        forest
    }

    /// How many rows pressing down from the top reaches.
    fn rows(herdr: HerdrState) -> usize {
        let mut forest = with_groups_open(herdr);
        forest.apply(Action::Move(Motion::FirstRow));
        let mut rows = 1;
        while forest.apply(Action::Move(Motion::NextRow)) {
            rows += 1;
        }
        rows
    }

    /// One line of every kind that names a pane no bead holds. Each is a live
    /// agent doing work, and the one thing `bdi` would not do was show you
    /// what it was doing.
    fn pane_bearing() -> [(Item, &'static str); 4] {
        [
            (Item::Loose(loose()), "w:p2"),
            (Item::Unconfigured(unconfigured()), "w:p3"),
            (Item::Conflict(pane_in_another_project()), "w:p4"),
            (Item::Conflict(several_beads_name_one_pane()), "w:p5"),
        ]
    }

    #[test]
    fn a_pane_no_bead_claims_can_be_tailed() {
        let panes = Fake::reading(&["waiting on the flake check"]);
        let mut forest = with_groups_open(HerdrState::Ok);

        for (item, pane) in pane_bearing() {
            step_onto(&mut forest, onto(&item));

            assert_eq!(
                tail(&forest, &panes, LINES),
                Tail::Pane {
                    pane: pane.to_string(),
                    lines: vec!["waiting on the flake check".to_string()],
                },
                "on {item:?}"
            );
        }
    }

    #[test]
    fn enter_focuses_a_pane_no_bead_claims() {
        let panes = Fake::default();
        let mut forest = with_groups_open(HerdrState::Ok);

        for (item, _) in pane_bearing() {
            step_onto(&mut forest, onto(&item));

            assert_eq!(focus(&forest, &panes), None, "on {item:?}");
        }
        assert_eq!(*panes.focused.borrow(), ["w:p2", "w:p3", "w:p4", "w:p5"]);
    }

    /// A conflict can be tailed exactly where it turns on one pane. Two of the
    /// four do not: one names the two panes that disagree and the other names
    /// every pane that claimed the bead, so there is nothing to pick rather
    /// than nothing to show. A tracker that could not be read names no pane at
    /// all.
    #[test]
    fn a_line_in_a_group_that_turns_on_no_one_pane_is_unchanged() {
        let panes = Fake::reading(&["nothing should reach the screen"]);
        let mut forest = with_groups_open(HerdrState::Ok);

        for item in [
            Item::Conflict(bead_and_pane_disagree()),
            Item::Conflict(several_panes_name_one_bead()),
            Item::Failed(failed_project()),
        ] {
            step_onto(&mut forest, onto(&item));

            assert_eq!(
                tail(&forest, &panes, LINES),
                Tail::Silent(phrase::no_bead_to_tail()),
                "on {item:?}"
            );
            assert_eq!(focus(&forest, &panes), None, "on {item:?}");
        }
        assert!(panes.asked.borrow().is_empty());
        assert!(panes.focused.borrow().is_empty());
    }

    /// A group stands for everything under it, panes included, and names no
    /// one of them. Opening it is how a reader gets to a pane; the group's own
    /// line is not one.
    #[test]
    fn a_groups_own_line_names_no_pane() {
        let panes = Fake::reading(&["nothing should reach the screen"]);
        let mut forest = with_groups_open(HerdrState::Ok);
        let kinds: Vec<GroupKind> = forest
            .lines()
            .iter()
            .filter_map(|line| match &line.content {
                Content::Group(group) => Some(group.kind),
                _ => None,
            })
            .collect();

        assert_eq!(kinds.len(), 4, "the fixture fills four of the five groups");
        for kind in kinds {
            step_onto(
                &mut forest,
                |content| matches!(content, Content::Group(group) if group.kind == kind),
            );

            assert_eq!(
                tail(&forest, &panes, LINES),
                Tail::Silent(phrase::no_bead_to_tail()),
                "on the {kind:?} group"
            );
            assert_eq!(focus(&forest, &panes), None, "on the {kind:?} group");
        }
        assert!(panes.asked.borrow().is_empty());
        assert!(panes.focused.borrow().is_empty());
    }

    /// The sequence a reader drives, rather than a state set by hand: onto a
    /// pane no bead claims, its rows read, off it, and back onto it. The tail
    /// stands while the selection is still on the pane and is read again once
    /// it has left, which for a loose pane is the rule it already was for a
    /// bead's.
    #[test]
    fn the_tail_follows_the_selection_onto_a_loose_pane_and_off_it() {
        let panes = Fake::reading(&["waiting on the flake check"]);
        let mut forest = with_groups_open(HerdrState::Ok);
        let pane = Item::Loose(loose());

        step_onto(&mut forest, onto(&pane));
        assert!(
            moved_on(&forest, None),
            "arriving on the pane, nothing on screen was read for it"
        );
        let on_arrival = tail(&forest, &panes, LINES);
        assert_eq!(
            on_arrival,
            Tail::Pane {
                pane: "w:p2".to_string(),
                lines: vec!["waiting on the flake check".to_string()],
            }
        );
        assert!(
            !moved_on(&forest, Some("w:p2")),
            "the selection has not left the pane, so its rows stand"
        );

        forest.apply(Action::Move(Motion::PreviousRow));
        assert!(
            moved_on(&forest, Some("w:p2")),
            "the row above is the group's own line and names no pane"
        );

        step_onto(&mut forest, onto(&pane));
        assert_eq!(tail(&forest, &panes, LINES), on_arrival);
        assert_eq!(
            *panes.asked.borrow(),
            ["w:p2 6", "w:p2 6"],
            "the pane was read on arriving and on returning, and not while sat on it"
        );
    }

    /// The property restated over a forest with panes no bead claims in it.
    /// `moved_on` is where widening `target` could quietly cost a herdr call
    /// on every keypress, or leave a phrase standing over a row that calls for
    /// a pane. Over every pair of rows rather than a sample, neither can
    /// happen unseen.
    #[test]
    fn a_tail_that_stands_holds_over_the_groups_too() {
        let panes = Fake::reading(&["rebuilt .#thinkpad, generation 541"]);
        let rows = rows(HerdrState::Ok);

        for from in 0..rows {
            let was = stepping(from, HerdrState::Ok);
            let showing = target(&was).pane().map(str::to_string);
            let on_screen = tail(&was, &panes, LINES);

            for onto in 0..rows {
                let now = stepping(onto, HerdrState::Ok);
                if !moved_on(&now, showing.as_deref()) {
                    assert_eq!(
                        tail(&now, &panes, LINES),
                        on_screen,
                        "the tail read on row {from} was left standing on row {onto}"
                    );
                }
            }
        }
    }

    /// With no herdr there is no pane on any row, and a row that names one in
    /// its own text is no exception.
    #[test]
    fn no_herdr_means_no_pane_on_a_loose_row_either() {
        let panes = Fake::reading(&["nothing should reach the screen"]);
        let mut forest = with_groups_open(HerdrState::Unavailable);
        step_onto(&mut forest, onto(&Item::Loose(loose())));

        assert_eq!(
            tail(&forest, &panes, LINES),
            Tail::Silent(phrase::no_herdr_to_tail())
        );
        assert!(panes.asked.borrow().is_empty());
    }
    /// A refresh that reorders a group around the selection leaves the
    /// selection on the same *pane*, not merely on some pane.
    ///
    /// `moved_on` compares pane ids, so a selection that slid onto a
    /// neighbouring pane on a refresh tick would re-read the tail and look,
    /// from outside, like a tail refusing to stand — a defect in this file
    /// caused by what the forest identifies an item by. Neither the row a
    /// refresh holds nor the tail that stands over it says this on its own.
    #[test]
    fn a_refresh_that_reorders_a_group_leaves_the_selection_on_the_same_pane() {
        let mut forest = with_groups_open(HerdrState::Ok);
        step_onto(&mut forest, onto(&Item::Loose(loose())));
        assert_eq!(target(&forest).pane(), Some("w:p2"));

        let mut reordered = snapshot_with_groups(HerdrState::Ok);
        reordered.unattributed.reverse();
        reordered.conflicts.reverse();
        forest.refresh(&reordered);

        assert_eq!(
            target(&forest).pane(),
            Some("w:p2"),
            "the group came back in a different order and took the selection with it"
        );
        assert!(
            !moved_on(&forest, Some("w:p2")),
            "the pane selected is the pane on screen, so its rows stand"
        );
    }
}
