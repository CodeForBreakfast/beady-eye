//! Each unfinished bead a collection read, once, for a reader picking work
//! rather than drawing a forest.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::model::badges::Badged;
use crate::model::edges::Related;
use crate::model::join::{AgentRef, BeadKey};
use crate::model::tree::{numeric_id_order, OrphanedDependency};
use crate::model::types::{Edge, Status};

use super::{AgentProvider, FailedProject, Node, Snapshot, TrackerState};

/// A snapshot's beads with the forest taken out: every unfinished bead in
/// every tree it read, shown or hidden, keyed `(project, id)` and written
/// once however many ways down reach it.
///
/// What each bead says is what its tree says of it. Readiness especially is
/// the tree's answer and not worked out again, so the listing and the screen
/// cannot disagree about whether a bead is free to start.
#[derive(Debug, Serialize)]
pub struct Listing<'a> {
    generated_at: DateTime<Utc>,
    agents: &'a AgentProvider,
    beads: Vec<Listed<'a>>,
    failed_projects: &'a [FailedProject],
    unread_trees: Vec<UnreadTree<'a>>,
}

/// One bead, as its tree draws it, less what belongs to a place in a tree.
#[derive(Debug, Serialize)]
struct Listed<'a> {
    project: &'a str,
    id: &'a str,
    title: &'a str,
    status: &'a Status,
    issue_type: &'a str,
    priority: u8,
    ready: bool,
    blocked_by: &'a [String],
    orphaned_dependencies: &'a [OrphanedDependency],
    agent: Option<&'a AgentRef>,
    badges: &'a [Badged],
    labels: &'a [String],
    parent: Option<BeadKey>,
    dependencies: Vec<Dependency<'a>>,
    /// Left out where the reader asked for none, and null where the bead
    /// says nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<Option<String>>,
    created_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    defer_until: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    due_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    assignee: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lease_expires_at: Option<DateTime<Utc>>,
    metadata: &'a BTreeMap<String, String>,
}

impl<'a> Listed<'a> {
    /// `node` as the listing writes it, without its description where
    /// `brief`.
    fn of(node: &'a Node, brief: bool) -> Self {
        Listed {
            project: &node.project,
            id: &node.id,
            title: &node.title,
            status: &node.status,
            issue_type: &node.issue_type,
            priority: node.priority,
            ready: node.ready,
            blocked_by: &node.blocked_by,
            orphaned_dependencies: &node.orphaned_dependencies,
            agent: node.agent.as_ref(),
            badges: &node.badges,
            labels: &node.labels,
            parent: node.parent_key(),
            dependencies: node
                .parent
                .iter()
                .chain(&node.depends_on)
                .map(|related| Dependency::of(related, &node.project))
                .collect(),
            description: (!brief).then(|| node.description().filter(|said| !said.is_empty())),
            created_at: node.created_at,
            defer_until: node.defer_until,
            due_at: node.due_at,
            assignee: node.assignee.as_deref(),
            lease_expires_at: node.lease_expires_at,
            metadata: &node.metadata,
        }
    }
}

/// One edge from a listed bead to another, as the bead's tree read it.
#[derive(Debug, Serialize)]
struct Dependency<'a> {
    /// Null where no tracker the run read held the other bead, as `status`
    /// is.
    project: Option<&'a str>,
    id: &'a str,
    #[serde(rename = "type")]
    edge: &'a Edge,
    status: Option<&'a Status>,
}

impl<'a> Dependency<'a> {
    /// `related` as an edge of a bead in `project`.
    fn of(related: &'a Related, project: &'a str) -> Self {
        Dependency {
            project: related
                .status
                .as_ref()
                .map(|_| related.project.as_deref().unwrap_or(project)),
            id: &related.id,
            edge: &related.edge,
            status: related.status.as_ref(),
        }
    }
}

/// A root whose tracker gave no rows for it, so the beads beneath it are
/// missing from the listing.
#[derive(Debug, Serialize)]
struct UnreadTree<'a> {
    project: &'a str,
    root: &'a str,
    tracker: &'a TrackerState,
}

impl<'a> Listing<'a> {
    pub fn of(snapshot: &'a Snapshot, only: &Only, brief: bool) -> Self {
        let mut once: BTreeMap<BeadKey, &Node> = BTreeMap::new();
        for node in snapshot.collected.iter().flat_map(|tree| &tree.beads) {
            if !node.status.is_finished() {
                once.entry(node.key()).or_insert(node);
            }
        }
        let keeps = only.keeping(snapshot);
        let mut beads: Vec<Listed> = once
            .into_values()
            .filter(|node| keeps(node))
            .map(|node| Listed::of(node, brief))
            .collect();
        beads.sort_by(|a, b| {
            a.project
                .cmp(b.project)
                .then_with(|| numeric_id_order(a.id, b.id))
        });
        Listing {
            generated_at: snapshot.generated_at,
            agents: &snapshot.agents,
            beads,
            failed_projects: &snapshot.failed_projects,
            unread_trees: unread_trees(snapshot),
        }
    }
}

/// The ready list with the screen taken out: the beads in progress or with
/// a live agent on them, then the beads ready to start, each as `Listing`
/// writes it and in the order the list draws it.
#[derive(Debug, Serialize)]
pub struct ReadyListing<'a> {
    generated_at: DateTime<Utc>,
    agents: &'a AgentProvider,
    in_progress: Vec<Listed<'a>>,
    ready: Vec<Listed<'a>>,
    failed_projects: &'a [FailedProject],
    unread_trees: Vec<UnreadTree<'a>>,
}

impl<'a> ReadyListing<'a> {
    /// `sections` is the ready list's two sections, as the view picks them.
    pub fn of(
        snapshot: &'a Snapshot,
        sections: [Vec<&'a Node>; 2],
        only: &Only,
        brief: bool,
    ) -> Self {
        let keeps = only.keeping(snapshot);
        let [in_progress, ready] = sections.map(|beads| {
            beads
                .into_iter()
                .filter(|node| keeps(node))
                .map(|node| Listed::of(node, brief))
                .collect()
        });
        ReadyListing {
            generated_at: snapshot.generated_at,
            agents: &snapshot.agents,
            in_progress,
            ready,
            failed_projects: &snapshot.failed_projects,
            unread_trees: unread_trees(snapshot),
        }
    }
}

/// Which beads a listing keeps, as its reader narrowed it: every one where
/// nothing is asked. Each test is the one `bd ready` makes of the option of
/// the same name.
#[derive(Debug, Default)]
pub struct Only {
    /// Every label a kept bead carries.
    pub labels: Vec<String>,
    pub issue_type: Option<String>,
    pub assignee: Option<String>,
    /// The bead a kept bead hangs beneath, by parent links at any depth.
    pub beneath: Option<BeadKey>,
}

impl Only {
    /// Whether a bead of `snapshot` is kept.
    fn keeping<'s>(&'s self, snapshot: &Snapshot) -> impl Fn(&Node) -> bool + 's {
        let parents: BTreeMap<BeadKey, BeadKey> = snapshot
            .collected
            .iter()
            .flat_map(|tree| &tree.beads)
            .filter_map(|node| Some((node.key(), node.parent_key()?)))
            .collect();
        move |node: &Node| {
            self.labels.iter().all(|label| node.labels.contains(label))
                && self
                    .issue_type
                    .as_ref()
                    .is_none_or(|issue_type| *issue_type == node.issue_type)
                && self
                    .assignee
                    .as_deref()
                    .is_none_or(|assignee| node.assignee.as_deref() == Some(assignee))
                && self.beneath.as_ref().is_none_or(|above| {
                    std::iter::successors(parents.get(&node.key()), |at| parents.get(at))
                        .take(parents.len())
                        .any(|ancestor| ancestor == above)
                })
        }
    }
}

fn unread_trees(snapshot: &Snapshot) -> Vec<UnreadTree<'_>> {
    snapshot
        .collected
        .iter()
        .filter(|tree| tree.tracker != TrackerState::Ok)
        .map(|tree| UnreadTree {
            project: &tree.project,
            root: &tree.root,
            tracker: &tree.tracker,
        })
        .collect()
}
