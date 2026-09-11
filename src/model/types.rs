//! What a tracker and a session say, as `bdi` holds it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Open,
    InProgress,
    Blocked,
    Closed,
    Deferred,
    #[serde(untagged)]
    Other(String),
}

impl Status {
    /// Render order: work in flight first, finished last.
    pub fn rank(&self) -> u8 {
        match self {
            Status::InProgress => 0,
            Status::Blocked => 1,
            Status::Open => 2,
            Status::Deferred => 3,
            Status::Closed => 4,
            Status::Other(_) => 5,
        }
    }

    pub fn is_closed(&self) -> bool {
        matches!(self, Status::Closed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Edge {
    ParentChild,
    Blocks,
    #[serde(untagged)]
    Other(String),
}

/// One bead this bead depends on, and the kind of that dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    pub on: String,
    pub edge: Edge,
}

/// What is known about an answer that would not parse, beyond that it would
/// not: which read was asked for, and what the parser made of what came back.
///
/// Together they are enough to run the read by hand and land on the row that
/// broke it, which is the whole of what a reader can do about one. Neither is
/// the tool's prose — `read` is `bdi`'s own command line, and `cause` is the
/// parser describing `bdi`'s own structs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Unreadable {
    /// The subcommand whose answer would not parse, as `bdi` spells it on
    /// the command line: `list`, `ready`, `query`, `blocked`, `sql`.
    pub read: String,
    /// The shape that did not match, and where in the answer it was.
    pub cause: String,
}

/// One bead as `bdi` holds it: only the fields it uses, in its own shape.
/// How a tracker spells them on the wire is the adapter's business.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bead {
    pub id: String,
    pub title: String,
    pub status: Status,
    pub priority: u8,
    pub issue_type: String,
    /// The bead this one hangs under, or none at the top of a chain.
    pub parent: Option<String>,
    /// Every bead this one depends on, and the kind of each dependency.
    pub dependencies: Vec<Dependency>,
    /// Whatever was written into the bead's metadata, each value as the
    /// text it prints as.
    pub metadata: BTreeMap<String, String>,
    /// Every field of the row bd wrote a text for, under the name bd spells
    /// it. What a badge reads where its key names no metadata key, so a field
    /// bd grows is drawable without `bdi` holding one of its own for it.
    pub fields: BTreeMap<String, String>,
    pub owner: Option<String>,
    pub assignee: Option<String>,
    /// The bead's own account of itself, where it has one.
    pub description: Option<String>,
    /// Everything noted on it, as one text, where anything has been.
    pub notes: Option<String>,
    pub updated_at: Option<DateTime<Utc>>,
    pub started_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
    /// When the tracker stops holding this bead back. It does not call the
    /// bead ready before this instant and does after, and nothing is written
    /// when it passes — so it is the one thing a tracker says that turns over
    /// on the clock rather than on a write.
    pub defer_until: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaneStatus {
    Idle,
    Working,
    /// A TTY prompt is waiting — a permission gate, or a pane at a startup
    /// confirmation. A property of the terminal, never of the work.
    Blocked,
    Done,
    #[serde(untagged)]
    Other(String),
}

/// A pane, across every session on this machine. A session mints its own
/// pane ids from `w1` up, so an id on its own does not name a pane: two
/// sessions have held a `w1:p1` at the same instant.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct PaneKey {
    pub session: String,
    pub id: String,
}

/// One pane as `bdi` holds it: only the fields it uses. The names are
/// herdr's, under the terminology rule, and another provider maps into them.
/// How one spells them on the wire, and which it may leave out, is the
/// adapter's business.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pane {
    /// The session holding this pane, which the pane's own listing does not
    /// carry: a session answers for its own panes and names itself nowhere.
    pub session: String,
    pub pane_id: String,
    pub cwd: PathBuf,
    pub display_agent: Option<String>,
    pub title: Option<String>,
    pub state_labels: BTreeMap<String, String>,
    pub agent_status: PaneStatus,
    /// Where `cwd` sits in the main working tree of its repository, where
    /// `cwd` is in a linked worktree. No provider says anything about it; it
    /// is read off the worktree after the listing, so a pane as a provider
    /// answered it has none.
    cwd_in_the_main_working_tree: Option<PathBuf>,
}

impl Pane {
    /// A pane as a provider answered with it. What it may also have said
    /// about the pane is public and set after.
    pub fn answered(
        session: String,
        pane_id: String,
        cwd: PathBuf,
        agent_status: PaneStatus,
    ) -> Self {
        Self {
            session,
            pane_id,
            cwd,
            display_agent: None,
            title: None,
            state_labels: BTreeMap::new(),
            agent_status,
            cwd_in_the_main_working_tree: None,
        }
    }

    /// What this pane is known by wherever a pane is named.
    pub fn key(&self) -> PaneKey {
        PaneKey {
            session: self.session.clone(),
            id: self.pane_id.clone(),
        }
    }

    /// This pane, with where its directory sits in the main working tree.
    pub fn with_cwd_in_the_main_working_tree(mut self, cwd: Option<PathBuf>) -> Self {
        self.cwd_in_the_main_working_tree = cwd;
        self
    }

    /// Where this pane's directory sits in the main working tree of its
    /// repository, where that is somewhere else.
    pub fn cwd_in_the_main_working_tree(&self) -> Option<&Path> {
        self.cwd_in_the_main_working_tree.as_deref()
    }

    /// The line to show for this pane: its state label for the state it is
    /// actually in, falling back to its title.
    pub fn caption(&self) -> Option<&str> {
        let state = match &self.agent_status {
            PaneStatus::Idle => "idle",
            PaneStatus::Working => "working",
            PaneStatus::Blocked => "blocked",
            PaneStatus::Done => "done",
            PaneStatus::Other(s) => s.as_str(),
        };
        self.state_labels
            .get(state)
            .map(String::as_str)
            .or(self.title.as_deref())
    }
}

/// A pane's name as a test writes it, where the test does not care which
/// session holds it. Shared by the tests inside the crate and the ones under
/// `tests/`, which is why it sits behind the feature rather than `cfg(test)`.
#[cfg(feature = "testing")]
pub mod testing {
    use super::{PaneKey, Unreadable};

    /// The session a test's panes are in unless it says otherwise: the one
    /// herdr runs where nothing names another.
    pub const A_SESSION: &str = "default";

    /// A parse failure as one arrives from the collector: the read `bdi`
    /// asked for, and the parser's own account of the row that broke it.
    ///
    /// The cause is what `serde_json` writes for a row whose title bd sent
    /// as an explicit null, measured against `parse_beads`.
    pub fn an_unreadable() -> Unreadable {
        Unreadable {
            read: "list".to_string(),
            cause: "invalid type: null, expected a string at line 1 column 25".to_string(),
        }
    }

    /// A pane in that session, by id.
    pub fn key(id: &str) -> PaneKey {
        PaneKey {
            session: A_SESSION.to_string(),
            id: id.to_string(),
        }
    }
}
