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
    #[serde(default, deserialize_with = "none_if_empty")]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub edge_from_parent: Option<Edge>,
    /// Every bead this one depends on, where the answer named them all.
    /// `bd list` does; `bd dep tree` does not — see `depends_on`.
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
    #[serde(default)]
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
    #[serde(default)]
    pub truncated: bool,
}

impl Bead {
    /// Every bead this one depends on, however the answer said it.
    ///
    /// `bd list` names them all outright. `bd dep tree --direction=up` walks
    /// dependents and dedupes, so it is a spanning tree: each row carries the
    /// one edge the walk first reached it by, as `parent_id` with
    /// `edge_from_parent` for its kind, and every other edge into that bead is
    /// absent from the answer. bd names the kind on every row but the root's,
    /// and the only kind that reaches a row from a bead it depends on without
    /// saying so is parent-child.
    pub fn depends_on(&self) -> Vec<Dependency> {
        if !self.dependencies.is_empty() {
            return self.dependencies.clone();
        }
        self.parent_id
            .iter()
            .map(|on| Dependency {
                on: on.clone(),
                edge: self.edge_from_parent.clone().unwrap_or(Edge::ParentChild),
            })
            .collect()
    }
}

/// bd writes the root's absent parent as `""` rather than omitting the field.
fn none_if_empty<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let raw = Option::<String>::deserialize(d)?;
    Ok(raw.filter(|s| !s.is_empty()))
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
