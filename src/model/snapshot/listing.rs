//! Each unfinished bead a collection read, once, for a reader picking work
//! rather than drawing a forest.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::config::Delivery;
use crate::model::badges::Badged;
use crate::model::edges::Related;
use crate::model::gate;
use crate::model::join::{AgentRef, BeadKey};
use crate::model::tree::{numeric_id_order, OrphanedDependency};
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

/// The readiness of the dependency trees of the beads a reader named: each
/// named bead and everything it depends on, followed into other projects'
/// trackers and never down a parent link, every unfinished bead in them
/// written once.
///
/// A gate free to close is a thing to wait for rather than to start, so it
/// is listed apart from the ready beads, and so is a ready bead whose
/// delivery has already landed, which is a thing to close. Every other bead
/// waits, and says why: its status where that is not open, and each
/// dependency still unfinished. A waiting bead whose pull request is open
/// is in delivery, and listed apart from the rest.
#[derive(Debug, Serialize)]
pub struct DependencyListing<'a> {
    generated_at: DateTime<Utc>,
    agents: &'a AgentProvider,
    ready: Vec<InTree<'a>>,
    ready_gates: Vec<InTree<'a>>,
    delivered: Vec<Delivering<'a, InTree<'a>>>,
    in_delivery: Vec<Delivering<'a, Waiting<'a>>>,
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
    fn of(node: &'a Node, brief: bool) -> Self {
        InTree {
            bead: Listed::of(node, brief),
            gate: Gate::of(node),
        }
    }
}

/// A bead whose work goes through pull requests, with the gate on each.
#[derive(Debug, Serialize)]
struct Delivering<'a, B> {
    #[serde(flatten)]
    bead: B,
    pull_requests: Vec<PullRequest<'a>>,
}

/// A pull request a `gh:pr` gate awaits.
#[derive(Debug, Serialize)]
struct PullRequest<'a> {
    project: &'a str,
    gate: &'a str,
    repo: Option<&'a str>,
    number: Option<String>,
}

impl<'a> PullRequest<'a> {
    fn of(gate: &'a Node) -> Self {
        PullRequest {
            project: &gate.project,
            gate: &gate.id,
            repo: gate.metadata.get("repo").map(String::as_str),
            number: gate.row.text("await_id"),
        }
    }
}

/// The pull requests the bead at `at` is delivered through: the `gh:pr`
/// gates awaiting a merge that it waits on, and those each outcome step it
/// waits on waits on in turn. A gate or a step counts only where its status
/// is `wanted` and it is not marked outside. Also whether the bead waits on
/// an outcome step that counts, since a closed one delivers a bead with no
/// gate beneath it.
fn delivered_through<'a>(
    tree: &'a Tree,
    at: usize,
    delivery: &Delivery,
    wanted: impl Fn(&Status) -> bool,
) -> (Vec<PullRequest<'a>>, bool) {
    let counts = |node: &Node| {
        wanted(&node.status)
            && !delivery
                .outside_key
                .as_ref()
                .is_some_and(|key| node.metadata.get(key).map(String::as_str) == Some("true"))
    };
    let pull_request = |node: &Node| {
        counts(node)
            && node.row.text("await_type").as_deref() == Some(gate::PULL_REQUEST)
            && !node.metadata.contains_key(gate::AWAITS)
    };
    let mut gates = Vec::new();
    let mut through_a_step = false;
    for below in dependency_links(tree, at) {
        let dependency = &tree.beads[below];
        if pull_request(dependency) {
            gates.push(PullRequest::of(dependency));
        } else if counts(dependency) && is_outcome_step(dependency, delivery) {
            through_a_step = true;
            gates.extend(
                dependencies(tree, below)
                    .filter(|gate| pull_request(gate))
                    .map(PullRequest::of),
            );
        }
    }
    (gates, through_a_step)
}

/// Whether `node` is a molecule's outcome step, as the setup labels one. A
/// gate can be one.
fn is_outcome_step(node: &Node, delivery: &Delivery) -> bool {
    delivery
        .outcome_label
        .as_ref()
        .is_some_and(|label| node.labels.contains(label))
}

impl<'a> DependencyListing<'a> {
    /// Each bead in the trees is kept as `only` says, and written without
    /// its description where `brief`. Its unmet dependencies are written
    /// whether or not they are kept. `delivery` says which gates and steps
    /// deliver a bead.
    pub fn of(
        snapshot: &'a Snapshot,
        named: &[BeadKey],
        only: &Only,
        brief: bool,
        delivery: &Delivery,
    ) -> Self {
        let mut ready = Vec::new();
        let mut ready_gates = Vec::new();
        let mut delivered = Vec::new();
        let mut in_delivery = Vec::new();
        let mut waiting = Vec::new();
        let keeps = only.keeping(snapshot);
        let mut reached: Vec<_> = dependency_trees(snapshot, named)
            .into_values()
            .filter(|(node, ..)| keeps(node))
            .collect();
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
                    ready_gates.push(InTree::of(node, brief));
                    continue;
                }
                match delivered_through(tree, at, delivery, Status::is_closed) {
                    (pull_requests, through_a_step)
                        if through_a_step || !pull_requests.is_empty() =>
                    {
                        delivered.push(Delivering {
                            bead: InTree::of(node, brief),
                            pull_requests,
                        })
                    }
                    _ => ready.push(InTree::of(node, brief)),
                }
                continue;
            }
            let mut unmet: Vec<Unmet> = dependencies(tree, at)
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
            let bead = Waiting {
                bead: InTree::of(node, brief),
                unmet_dependencies: unmet,
            };
            let (pull_requests, _) =
                delivered_through(tree, at, delivery, |status| !status.is_finished());
            if pull_requests.is_empty() || is_outcome_step(node, delivery) {
                waiting.push(bead);
            } else {
                in_delivery.push(Delivering {
                    bead,
                    pull_requests,
                });
            }
        }
        DependencyListing {
            generated_at: snapshot.generated_at,
            agents: &snapshot.agents,
            ready,
            ready_gates,
            delivered,
            in_delivery,
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
                    .map(|link| link.bead)
                    .filter(|&below| waits_on(node, &tree.beads[below])),
            );
        }
    }
    reached
}

/// The beads hung beneath the one at `at` that it waits on by a blocking
/// edge.
fn dependencies(tree: &Tree, at: usize) -> impl Iterator<Item = &Node> {
    dependency_links(tree, at).map(|below| &tree.beads[below])
}

/// Where in `tree` each bead the one at `at` waits on by a blocking edge
/// sits.
fn dependency_links(tree: &Tree, at: usize) -> impl Iterator<Item = usize> + '_ {
    let node = &tree.beads[at];
    tree.children[at]
        .iter()
        .map(|link| link.bead)
        .filter(move |&below| waits_on(node, &tree.beads[below]))
}

/// Whether `node` holds a blocking edge on `below`. A link in the tree
/// carries one edge, and names a child its parent's even where the parent
/// also waits on it, so the bead's own dependencies are what say so.
fn waits_on(node: &Node, below: &Node) -> bool {
    node.depends_on.iter().any(|on| {
        on.edge == Edge::Blocks
            && on.id == below.id
            && on.project.as_deref().unwrap_or(&node.project) == below.project
    })
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
