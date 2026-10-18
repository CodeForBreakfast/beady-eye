//! The ready list: what a reader could pick up now, and what is being
//! worked, each bead once, drawn in place of the forest.
//!
//! It covers what the forest would draw — every tree read, shown or held
//! back, or what is at and beneath the beads the forest is rooted at — and
//! takes each bead's readiness from its tree, so the list and the forest
//! cannot disagree about whether a bead is free to start.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use crate::model::join::BeadKey;
use crate::model::snapshot::{Counts, Node as Bead, Snapshot, TrackerState, Tree};
use crate::model::tree::numeric_id_order;
use crate::model::types::Status;
use crate::view::lines::{
    links_below, root_key, Content, Line, Note, Place, ProjectLine, Section, INDENT,
};
use crate::view::row;

use super::drawn::{Drawn, Node};
use super::facts::Facts;
use super::layout::{project_drawn, Rooted};

/// Every line the ready list draws: each project's own line, saying how
/// fresh its rows are, then the two sections, then what the list leaves out.
pub(super) fn lay_out(
    snapshot: &Snapshot,
    facts: &Facts,
    rooted: &[Rooted],
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
        } else if bead.ready {
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
    for (section, mut beads) in [(Section::Ready, ready), (Section::InProgress, in_progress)] {
        beads.sort_by(|a, b| next_first(a.0, b.0));
        lines.push(drawn(said(Content::Section {
            section,
            count: beads.len(),
        })));
        lines.extend(
            beads
                .into_iter()
                .map(|(bead, tree, way)| drawn(row_of(bead, tree, way, facts))),
        );
    }
    let to_check = Counts::over(reached.values().map(|(bead, _, _)| *bead)).anomalies;
    let failed = snapshot.failed_projects.len();
    for (count, note) in [
        (to_check, Note::ToCheck(to_check)),
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
