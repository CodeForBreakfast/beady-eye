//! The rows bd and herdr write, as `bdi` holds them.

use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};

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
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Dependency {
    #[serde(rename = "depends_on_id")]
    pub on: String,
    #[serde(rename = "type")]
    pub edge: Edge,
}

/// One row of a bd answer, holding only the fields `bdi` uses.
///
/// Unknown fields are ignored; a present field of the wrong type is an error.
/// Every field bd omits when empty is optional here, because bd omits it
/// rather than writing null.
///
/// `depth` is deliberately absent: bd flattens it under `--max-depth`, so the
/// tree recomputes nesting from the dependency edges instead.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Bead {
    pub id: String,
    pub title: String,
    pub status: Status,
    #[serde(default)]
    pub priority: u8,
    #[serde(default)]
    pub issue_type: String,
    /// The bead this one hangs under. bd writes the top of a chain as an
    /// empty parent, or leaves the field out; either reads as none.
    #[serde(default, deserialize_with = "empty_is_none")]
    pub parent: Option<String>,
    /// Every bead this one depends on, and the kind of each dependency.
    #[serde(default, deserialize_with = "none_is_empty")]
    pub dependencies: Vec<Dependency>,
    #[serde(default, deserialize_with = "text_of_each_value")]
    pub metadata: BTreeMap<String, String>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub closed_at: Option<DateTime<Utc>>,
    /// When bd stops holding this bead back. `bd ready` does not name a bead
    /// before this instant and does name it after, and nothing is written
    /// when it passes — so it is the one thing a tracker says that turns over
    /// on the clock rather than on a write.
    #[serde(default)]
    pub defer_until: Option<DateTime<Utc>>,
}

/// bd omits a field it has nothing for, and `#[serde(default)]` covers that.
/// It does not extend to an explicit null. A tracker is read whole, so a row
/// bd wrote the other way costs not one bead's edges but every bead in that
/// project.
fn none_is_empty<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Dependency>, D::Error> {
    Ok(Option::<Vec<Dependency>>::deserialize(d)?.unwrap_or_default())
}

/// bd spells an absent parent three ways — `""`, `null`, or no field — and
/// they all mean the top of a chain.
fn empty_is_none<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(Option::<String>::deserialize(d)?.filter(|parent| !parent.is_empty()))
}

/// A bead's metadata is whatever JSON was written into it, and bdi draws it
/// as text. So each value is read as the text it prints as, and a value that
/// is not a string costs nothing.
///
/// A tracker is read whole, so the alternative is not a bead without its
/// badge — it is every bead in that project, gone.
fn text_of_each_value<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<String, String>, D::Error> {
    let raw = BTreeMap::<String, serde_json::Value>::deserialize(d)?;
    Ok(raw
        .into_iter()
        .map(|(key, value)| match value {
            serde_json::Value::String(text) => (key, text),
            written => (key, written.to_string()),
        })
        .collect())
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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Pane {
    pub pane_id: String,
    pub cwd: PathBuf,
    #[serde(default)]
    pub display_agent: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub state_labels: BTreeMap<String, String>,
    pub agent_status: PaneStatus,
}

impl Pane {
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
