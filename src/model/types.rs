//! What a tracker and a session say, as `bdi` holds it.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Open,
    InProgress,
    Blocked,
    Closed,
    Deferred,
    Pinned,
    Hooked,
    #[serde(untagged)]
    Other(String),
}

impl Status {
    /// Render order: work in flight first, finished last.
    pub fn rank(&self) -> u8 {
        match self {
            Status::InProgress => 0,
            Status::Hooked => 1,
            Status::Blocked => 2,
            Status::Open => 3,
            Status::Deferred => 4,
            Status::Pinned => 5,
            Status::Closed => 6,
            Status::Other(_) => 7,
        }
    }

    /// Whether somebody closed it. A pinned bead is never closed.
    pub fn is_closed(&self) -> bool {
        matches!(self, Status::Closed)
    }

    /// Whether it holds no work: closed, or pinned, which bd keeps
    /// indefinitely and leaves out of `bd ready`, `bd blocked` and its default
    /// list, and whose dependents it never blocks.
    pub fn is_finished(&self) -> bool {
        matches!(self, Status::Closed | Status::Pinned)
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
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Every value the row held that no text field here holds, under the key
    /// that names it: a field by its own name, and a member of a field's
    /// object by the two joined with a dot. So a field bd grows is drawable
    /// without `bdi` holding one of its own for it.
    pub values: BTreeMap<String, String>,
    /// Every list of values the row held, under the key that names it as in
    /// `values`, each member that is one value in the order the row wrote it.
    pub lists: BTreeMap<String, Vec<String>>,
    /// The row's `created_by`. The row's `owner` is an address, which no
    /// surface draws, so it is not held.
    pub created_by: Option<String>,
    pub assignee: Option<String>,
    pub labels: Vec<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub started_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
    /// When the tracker stops holding this bead back. It does not call the
    /// bead ready before this instant and does after, and nothing is written
    /// when it passes — so it is the one thing a tracker says that turns over
    /// on the clock rather than on a write.
    pub defer_until: Option<DateTime<Utc>>,
    /// The row as the tracker printed it, every field it wrote whether `bdi`
    /// reads it or not. The description and notes are read from it rather
    /// than held beside it.
    pub row: Printed,
}

impl Bead {
    /// The keys whose text a field of `Bead` holds, which `values` therefore
    /// does not.
    pub const TEXT_FIELDS: [&str; 8] = [
        "id",
        "title",
        "issue_type",
        "parent",
        "created_by",
        "assignee",
        "description",
        "notes",
    ];

    /// The value the row held under `key`, which is what a badge reads. A text
    /// held in a field is no value when it is empty or spells an object, as in
    /// `values`. An object's members are values of their own, in `values`.
    pub fn value(&self, key: &str) -> Option<Cow<'_, str>> {
        let text = match key {
            "id" => Some(Cow::Borrowed(self.id.as_str())),
            "title" => Some(Cow::Borrowed(self.title.as_str())),
            "issue_type" => Some(Cow::Borrowed(self.issue_type.as_str())),
            "parent" => self.parent.as_deref().map(Cow::Borrowed),
            "created_by" => self.created_by.as_deref().map(Cow::Borrowed),
            "assignee" => self.assignee.as_deref().map(Cow::Borrowed),
            "description" | "notes" => self.row.text(key).map(Cow::Owned),
            _ => {
                return self
                    .values
                    .get(key)
                    .map(|value| Cow::Borrowed(value.as_str()))
            }
        };
        text.filter(|text| {
            !text.is_empty()
                && serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(text).is_err()
        })
    }

    /// Every value the row held under `key`: the one value where it holds
    /// one, each member of a list, and none where it holds neither.
    pub fn members(&self, key: &str) -> Vec<Cow<'_, str>> {
        match self.value(key) {
            Some(value) => vec![value],
            None => self
                .lists
                .get(key)
                .map(|list| {
                    list.iter()
                        .map(|member| Cow::Borrowed(member.as_str()))
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
}

/// One row as the tracker printed it, held as the JSON text its fields write
/// out as, which is what a watch line carries.
#[derive(Debug, Clone)]
pub struct Printed(Arc<RawValue>);

impl Printed {
    pub fn of(fields: &serde_json::Map<String, serde_json::Value>) -> serde_json::Result<Self> {
        Ok(Self(serde_json::value::to_raw_value(fields)?.into()))
    }

    /// Every field the row holds.
    pub fn fields(&self) -> serde_json::Map<String, serde_json::Value> {
        serde_json::from_str(self.0.get()).expect("a row printed from its fields reads back")
    }

    /// A row saying nothing but `description` and `notes`.
    #[cfg(any(test, feature = "testing"))]
    pub fn saying(description: &str, notes: &str) -> Self {
        let serde_json::Value::Object(fields) =
            serde_json::json!({ "description": description, "notes": notes })
        else {
            unreachable!("a row is an object")
        };
        Self::of(&fields).expect("a row of two texts prints")
    }

    /// The text the row holds under `field`, where it holds a string there.
    pub fn text(&self, field: &str) -> Option<String> {
        match self.fields().remove(field) {
            Some(serde_json::Value::String(text)) => Some(text),
            _ => None,
        }
    }
}

impl PartialEq for Printed {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0.get() == other.0.get()
    }
}

impl Eq for Printed {}

impl Serialize for Printed {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(s)
    }
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
    /// Which agent the provider detected in the pane, by the provider's own
    /// name for it. `bdi` never reads anything into the name: it is the key
    /// the reader's `[tail.crop]` is written against.
    pub agent: Option<String>,
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
            agent: None,
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
