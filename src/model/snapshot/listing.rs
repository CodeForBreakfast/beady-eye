//! Each unfinished bead a collection read, once, for a reader picking work
//! rather than drawing a forest.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::model::badges::Badged;
use crate::model::join::{AgentRef, BeadKey};
use crate::model::tree::numeric_id_order;
use crate::model::types::Status;

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
    agent: Option<&'a AgentRef>,
    badges: &'a [Badged],
}

impl<'a> From<&'a Node> for Listed<'a> {
    fn from(node: &'a Node) -> Self {
        Listed {
            project: &node.project,
            id: &node.id,
            title: &node.title,
            status: &node.status,
            issue_type: &node.issue_type,
            priority: node.priority,
            ready: node.ready,
            blocked_by: &node.blocked_by,
            agent: node.agent.as_ref(),
            badges: &node.badges,
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
    pub fn of(snapshot: &'a Snapshot) -> Self {
        let mut once: BTreeMap<BeadKey, &Node> = BTreeMap::new();
        for node in snapshot.collected.iter().flat_map(|tree| &tree.beads) {
            if !node.status.is_finished() {
                once.entry(node.key()).or_insert(node);
            }
        }
        let mut beads: Vec<Listed> = once.into_values().map(Listed::from).collect();
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
            unread_trees: snapshot
                .collected
                .iter()
                .filter(|tree| tree.tracker != TrackerState::Ok)
                .map(|tree| UnreadTree {
                    project: &tree.project,
                    root: &tree.root,
                    tracker: &tree.tracker,
                })
                .collect(),
        }
    }
}
