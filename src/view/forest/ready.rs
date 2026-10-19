//! The ready list: what is being worked, and what a reader could pick up
//! now, each bead once, drawn in place of the forest.
//!
//! It covers what the forest would draw — every tree read, shown or held
//! back, or what is at and beneath the beads the forest is rooted at — and
//! takes each bead's readiness from its tree, so the list and the forest
//! cannot disagree about whether a bead is free to start. The list offers
//! fewer: a parent with a child still open is free to start by its tree's
//! word, but its children are the work left on it.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use crate::model::join::BeadKey;
use crate::model::snapshot::{Counts, Node as Bead, Snapshot, TrackerState, Tree};
use crate::model::tree::numeric_id_order;
use crate::model::types::{Edge, Status};
use crate::view::lines::{
    links_below, marker, root_key, upward_prefix, Content, Line, Note, Place, ProjectLine, Section,
    Waiting, INDENT,
};
use crate::view::row::{self, Step};

use super::drawn::{Drawn, Node};
use super::facts::Facts;
use super::layout::{project_drawn, Rooted};
use super::way_to;

/// Every line the ready list draws: each project's own line, saying how
/// fresh its rows are, then the two sections, then what the list leaves out.
/// A row in `unblocking` is drawn expanded to what its bead holds up.
pub(super) fn lay_out(
    snapshot: &Snapshot,
    facts: &Facts,
    rooted: &[Rooted],
    unblocking: &BTreeSet<BeadKey>,
    layout: &row::Layout,
) -> Drawn {
    let mut unread = 0;
    let mut covered: Vec<(&Tree, Vec<usize>)> = Vec::new();
    if rooted.is_empty() {
        for tree in &snapshot.collected {
            if tree.tracker != TrackerState::Ok {
                unread += 1;
            }
            if !tree.beads.is_empty() {
                covered.push((tree, vec![0]));
            }
        }
    } else {
        for root in rooted {
            match (&root.way, snapshot.tree(&root.place.tree)) {
                (Ok(way), Some(tree)) => covered.push((tree, way.clone())),
                _ => unread += 1,
            }
        }
    }

    let reached = each_once(&covered);
    let mut ready = Vec::new();
    let mut in_progress = Vec::new();
    for (bead, tree, way) in reached.values() {
        if bead.status.is_finished() {
            continue;
        }
        if bead.status == Status::InProgress || bead.agent.is_some() {
            in_progress.push((*bead, *tree, way));
        } else if bead.ready && !waits_on_its_children(tree, way) {
            ready.push((*bead, *tree, way));
        }
    }

    let mut lines: Vec<Node> = snapshot
        .projects
        .iter()
        .filter(|project| project_drawn(snapshot, project, rooted))
        .map(|project| {
            drawn(Line {
                prefix: String::new(),
                depth: 0,
                folded: None,
                place: None,
                content: Content::Project(ProjectLine {
                    every_root_read: snapshot.every_root_read(project),
                    counts: facts.project(project),
                    project: project.clone(),
                }),
            })
        })
        .collect();
    let up = Up {
        snapshot,
        facts,
        waiters: waiters(snapshot),
    };
    for (section, mut beads) in [(Section::InProgress, in_progress), (Section::Ready, ready)] {
        beads.sort_by(|a, b| next_first(a.0, b.0));
        lines.push(drawn(said(Content::Section {
            section,
            count: beads.len(),
        })));
        lines.extend(beads.into_iter().map(|(bead, tree, way)| {
            let mut line = row_of(bead, tree, way, facts);
            let from = [bead.key()];
            if !up.waiting_on(&from).is_empty() {
                let open = unblocking.contains(&from[0]);
                line.folded = Some(open);
                line.prefix = marker(open).to_string();
                if open {
                    let steps = up.steps_above(&from, &[], &mut BTreeSet::new());
                    return Node::drawn(line, steps);
                }
            }
            drawn(line)
        }));
    }
    let to_check = Counts::over(reached.values().map(|(bead, _, _)| *bead)).anomalies;
    let orphaned = reached
        .values()
        .filter(|(bead, tree, _)| tree.orphaned_dependencies.contains(&bead.id))
        .count();
    let looped = reached
        .values()
        .filter(|(bead, tree, _)| tree.cycles.contains(&bead.id))
        .count();
    let failed = snapshot.failed_projects.len();
    for (count, note) in [
        (to_check, Note::ToCheck(to_check)),
        (orphaned, Note::OrphanedDependencies(orphaned)),
        (looped, Note::Cycle(looped)),
        (unread, Note::Unread(unread)),
        (failed, Note::FailedProjects(failed)),
    ] {
        if count > 0 {
            lines.push(drawn(said(Content::Note(note))));
        }
    }
    Drawn::over(lines, None, layout)
}

/// Each bead the covered trees reach, once, with the tree and the way down
/// it was first reached by: the shallowest, in the first tree to reach it.
fn each_once<'a>(
    covered: &[(&'a Tree, Vec<usize>)],
) -> BTreeMap<BeadKey, (&'a Bead, &'a Tree, Vec<usize>)> {
    let mut once = BTreeMap::new();
    for (tree, way) in covered {
        let mut going = std::collections::VecDeque::from([way.clone()]);
        let mut seen = vec![false; tree.beads.len()];
        while let Some(way) = going.pop_front() {
            let (at, above) = way.split_last().expect("a way down ends somewhere");
            if std::mem::replace(&mut seen[*at], true) {
                continue;
            }
            let bead = &tree.beads[*at];
            once.entry(bead.key()).or_insert((bead, *tree, way.clone()));
            for link in links_below(tree, *at, above) {
                let mut further = way.clone();
                further.push(link.bead);
                going.push_back(further);
            }
        }
    }
    once
}

/// Whether a bead has a child still open. bd answers such a parent ready,
/// but nobody can start it, because its children are the work left on it.
fn waits_on_its_children(tree: &Tree, way: &[usize]) -> bool {
    let at = *way.last().expect("a way down ends somewhere");
    tree.children[at]
        .iter()
        .any(|link| link.edge == Edge::ParentChild && !tree.beads[link.bead].status.is_finished())
}

/// The order a section lists its beads in: the most urgent first, and the
/// oldest first among equals, as `bd ready` lists them.
fn next_first(a: &Bead, b: &Bead) -> Ordering {
    a.priority
        .cmp(&b.priority)
        .then_with(|| match (a.created_at, b.created_at) {
            (Some(a), Some(b)) => a.cmp(&b),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        })
        .then_with(|| a.project.cmp(&b.project))
        .then_with(|| numeric_id_order(&a.id, &b.id))
}

/// A bead's row in the list, standing on the way down it was reached by so
/// the selection, the bead window and the tail find it as they do in the
/// forest. Its whole id is drawn, because no parent stands above it here.
fn row_of(bead: &Bead, tree: &Tree, way: &[usize], facts: &Facts) -> Line {
    let (at, above) = way.split_last().expect("a way down ends somewhere");
    let place = Place {
        tree: root_key(tree),
        steps: way[1..].iter().map(|at| tree.beads[*at].key()).collect(),
    };
    Line {
        prefix: INDENT.to_string(),
        depth: 1,
        folded: None,
        place: Some(place),
        content: Content::Bead(row::cells(
            bead,
            None,
            facts.tree(&root_key(tree)).bead(tree, *at, above).progress,
            None,
        )),
    }
}

/// A line of the list's own, standing for no bead.
fn said(content: Content) -> Line {
    Line {
        prefix: String::new(),
        depth: 0,
        folded: None,
        place: None,
        content,
    }
}

fn drawn(line: Line) -> Node {
    Node::drawn(line, Vec::new())
}

/// Every bead a tree hangs another under, by the bead hung, with the edge it
/// hangs by, each waiter once. The forest hangs a child under its parent and
/// a blocker under what it blocks, so this is what each bead holds up.
fn waiters(snapshot: &Snapshot) -> BTreeMap<BeadKey, Vec<(&Bead, &Edge)>> {
    let mut held: BTreeMap<BeadKey, Vec<(&Bead, &Edge)>> = BTreeMap::new();
    for tree in &snapshot.collected {
        for (at, links) in tree.children.iter().enumerate() {
            let waiter = &tree.beads[at];
            for link in links.iter().filter(|link| link.bead != at) {
                let by = held.entry(tree.beads[link.bead].key()).or_default();
                if !by.iter().any(|(already, _)| already.is(&waiter.key())) {
                    by.push((waiter, &link.edge));
                }
            }
        }
    }
    held
}

/// What the ready list walks to draw what a row's bead unblocks.
struct Up<'a> {
    snapshot: &'a Snapshot,
    facts: &'a Facts,
    waiters: BTreeMap<BeadKey, Vec<(&'a Bead, &'a Edge)>>,
}

impl<'a> Up<'a> {
    /// The unfinished beads waiting on the last bead of `up`, the most
    /// urgent first. One already stepped through is left out, which is where
    /// a loop is cut.
    fn waiting_on(&self, up: &[BeadKey]) -> Vec<(&'a Bead, &'a Edge)> {
        let below = up.last().expect("a way up starts at a bead");
        let mut waiting: Vec<(&Bead, &Edge)> = self
            .waiters
            .get(below)
            .into_iter()
            .flatten()
            .filter(|(bead, _)| !bead.status.is_finished())
            .filter(|(bead, _)| !up.iter().any(|stepped| bead.is(stepped)))
            .copied()
            .collect();
        waiting.sort_by(|a, b| next_first(a.0, b.0));
        waiting
    }

    /// A line for each bead waiting on the last bead of `up`, hung on an
    /// arrow up, with the lines for what waits on it beneath it in turn.
    ///
    /// What a bead unblocks is drawn under its first line in the expansion
    /// alone, and `drawn` is the beads that have had theirs. A bead is drawn
    /// once for every way up to it, and diamonds in the dependencies make
    /// the ways up multiply, so drawing each one's steps above again would
    /// grow with the ways up rather than with the edges.
    fn steps_above(
        &self,
        up: &[BeadKey],
        trunk: &[bool],
        drawn: &mut BTreeSet<BeadKey>,
    ) -> Vec<Node> {
        let waiting = self.waiting_on(up);
        let count = waiting.len();
        let mut steps = Vec::with_capacity(count);
        for (nth, (bead, edge)) in waiting.into_iter().enumerate() {
            let last = nth + 1 == count;
            let mut stepped = up.to_vec();
            stepped.push(bead.key());
            let first = drawn.insert(bead.key());
            let mut row = row::cells(bead, None, None, None);
            row.step = Some(Step {
                also_waits_on: self.also_waits_on(&stepped),
                unblocks_as_above: !first && !self.waiting_on(&stepped).is_empty(),
            });
            let line = Line {
                prefix: upward_prefix(trunk, last, edge),
                depth: trunk.len() as u16 + 2,
                folded: None,
                place: self.place_of(&bead.key()),
                content: Content::Waiting(Waiting {
                    row,
                    up: stepped.clone(),
                }),
            };
            let above = if first {
                let mut beneath = trunk.to_vec();
                beneath.push(!last);
                self.steps_above(&stepped, &beneath, drawn)
            } else {
                Vec::new()
            };
            steps.push(Node::drawn(line, above));
        }
        steps
    }

    /// How many unfinished beads the last bead of `up` waits on besides the
    /// beads stepped up through to reach it, counting a blocker no tracker
    /// holds that may be unfinished.
    fn also_waits_on(&self, up: &[BeadKey]) -> usize {
        let step = up.last().expect("a way up ends at a bead");
        let Some((tree, at)) = self.snapshot.locate(step) else {
            return 0;
        };
        let others: BTreeSet<BeadKey> = tree.children[at]
            .iter()
            .map(|link| &tree.beads[link.bead])
            .filter(|bead| !bead.status.is_finished())
            .map(Bead::key)
            .filter(|key| !up.contains(key))
            .collect();
        let missing = tree.beads[at]
            .orphaned_dependencies
            .iter()
            .filter(|orphaned| orphaned.may_block)
            .count();
        others.len() + missing
    }

    /// Where the forest draws a bead, for the bead window and the tail to
    /// find it by.
    fn place_of(&self, key: &BeadKey) -> Option<Place> {
        let (tree, _) = self.snapshot.locate(key)?;
        way_to(tree, self.facts.tree(&root_key(tree)), key)
    }
}
