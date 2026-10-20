//! Each unfinished bead a collection read, once, for a reader picking work
//! rather than drawing a forest.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::model::badges::Badged;
use crate::model::join::{AgentRef, BeadKey};
use crate::model::tree::numeric_id_order;
use crate::model::types::{Edge, Status};

use super::{AgentProvider, FailedProject, Node, Snapshot, TrackerState, Tree};

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
    labels: &'a [String],
    description: Option<String>,
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
            labels: &node.labels,
            description: node.description().filter(|said| !said.is_empty()),
            created_at: node.created_at,
            defer_until: node.defer_until,
            due_at: node.due_at,
            assignee: node.assignee.as_deref(),
            lease_expires_at: node.lease_expires_at,
            metadata: &node.metadata,
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
    pub fn of(snapshot: &'a Snapshot, sections: [Vec<&'a Node>; 2]) -> Self {
        let [in_progress, ready] =
            sections.map(|beads| beads.into_iter().map(Listed::from).collect());
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

/// The readiness of the dependency trees of the beads a reader named: each
/// named bead and everything it depends on, followed into other projects'
/// trackers and never down a parent link, every unfinished bead in them
/// written once.
///
/// A gate free to close is a thing to wait for rather than to start, so it
/// is listed apart from the ready beads. Every other bead waits, and says
/// why: its status where that is not open, and each dependency still
/// unfinished.
#[derive(Debug, Serialize)]
pub struct DependencyListing<'a> {
    generated_at: DateTime<Utc>,
    agents: &'a AgentProvider,
    ready: Vec<InTree<'a>>,
    ready_gates: Vec<InTree<'a>>,
    waiting: Vec<Waiting<'a>>,
    failed_projects: &'a [FailedProject],
    unread_trees: Vec<UnreadTree<'a>>,
}

/// One bead in a dependency tree, with what it awaits where it is a gate.
#[derive(Debug, Serialize)]
struct InTree<'a> {
    #[serde(flatten)]
    bead: Listed<'a>,
    #[serde(flatten)]
    gate: Option<Gate<'a>>,
}

/// A bead in a dependency tree that is not ready, with the dependencies it
/// still waits on.
#[derive(Debug, Serialize)]
struct Waiting<'a> {
    #[serde(flatten)]
    bead: InTree<'a>,
    unmet_dependencies: Vec<Unmet<'a>>,
}

/// A dependency not yet finished, keyed on its own project.
#[derive(Debug, Serialize)]
struct Unmet<'a> {
    project: &'a str,
    id: &'a str,
    title: &'a str,
    status: &'a Status,
    issue_type: &'a str,
    #[serde(flatten)]
    gate: Option<Gate<'a>>,
}

/// What a gate waits on, as bd holds it.
#[derive(Debug, Serialize)]
struct Gate<'a> {
    await_type: Option<String>,
    await_id: Option<String>,
    repo: Option<&'a str>,
}

impl<'a> Gate<'a> {
    fn of(node: &'a Node) -> Option<Self> {
        (node.issue_type == "gate").then(|| Gate {
            await_type: node.row.text("await_type"),
            await_id: node.row.text("await_id"),
            repo: node.metadata.get("repo").map(String::as_str),
        })
    }
}

impl<'a> InTree<'a> {
    fn of(node: &'a Node) -> Self {
        InTree {
            bead: Listed::from(node),
            gate: Gate::of(node),
        }
    }
}

impl<'a> DependencyListing<'a> {
    pub fn of(snapshot: &'a Snapshot, named: &[BeadKey]) -> Self {
        let mut ready = Vec::new();
        let mut ready_gates = Vec::new();
        let mut waiting = Vec::new();
        let mut reached: Vec<_> = dependency_trees(snapshot, named).into_values().collect();
        reached.sort_by(|(a, ..), (b, ..)| {
            a.project
                .cmp(&b.project)
                .then_with(|| numeric_id_order(&a.id, &b.id))
        });
        for (node, tree, at) in reached {
            if node.status.is_finished() {
                continue;
            }
            if node.ready && node.status == Status::Open {
                if node.issue_type == "gate" {
                    ready_gates.push(InTree::of(node));
                } else {
                    ready.push(InTree::of(node));
                }
                continue;
            }
            let mut unmet: Vec<Unmet> = tree.children[at]
                .iter()
                .filter(|link| link.edge == Edge::Blocks)
                .map(|link| &tree.beads[link.bead])
                .filter(|blocker| !blocker.status.is_finished())
                .map(|blocker| Unmet {
                    project: &blocker.project,
                    id: &blocker.id,
                    title: &blocker.title,
                    status: &blocker.status,
                    issue_type: &blocker.issue_type,
                    gate: Gate::of(blocker),
                })
                .collect();
            unmet.sort_by(|a, b| {
                a.project
                    .cmp(b.project)
                    .then_with(|| numeric_id_order(a.id, b.id))
            });
            waiting.push(Waiting {
                bead: InTree::of(node),
                unmet_dependencies: unmet,
            });
        }
        DependencyListing {
            generated_at: snapshot.generated_at,
            agents: &snapshot.agents,
            ready,
            ready_gates,
            waiting,
            failed_projects: &snapshot.failed_projects,
            unread_trees: unread_trees(snapshot)
                .into_iter()
                .filter(|tree| {
                    named
                        .iter()
                        .any(|key| key.project == tree.project && key.id == tree.root)
                })
                .collect(),
        }
    }
}

/// Every bead the named beads depend on, each named bead included, by
/// blocking edges alone, keyed `(project, id)` so each is held once. Each
/// comes with the tree and the place in it where it was first
/// reached, whose links down are its dependencies.
fn dependency_trees<'a>(
    snapshot: &'a Snapshot,
    named: &[BeadKey],
) -> BTreeMap<BeadKey, (&'a Node, &'a Tree, usize)> {
    let mut reached = BTreeMap::new();
    for key in named {
        let Some((tree, at)) = snapshot.collected.iter().find_map(|tree| {
            let at = tree.beads.iter().position(|bead| bead.is(key))?;
            Some((&**tree, at))
        }) else {
            continue;
        };
        let mut going = vec![at];
        while let Some(at) = going.pop() {
            let node = &tree.beads[at];
            if reached.contains_key(&node.key()) {
                continue;
            }
            reached.insert(node.key(), (node, tree, at));
            going.extend(
                tree.children[at]
                    .iter()
                    .filter(|link| link.edge == Edge::Blocks)
                    .map(|link| link.bead),
            );
        }
    }
    reached
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
