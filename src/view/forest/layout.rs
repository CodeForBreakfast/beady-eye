//! The lines one snapshot draws, under the folds set over it.
//!
//! Laying out is a pure function of those two. Nothing here knows where the
//! selection is, how tall the screen is, or which key was pressed, and
//! nothing here moves a fold: it asks which way one points and draws what
//! that says.

use std::sync::Arc;

use crate::config::Scope;
use crate::model::join::BeadKey;
use crate::model::snapshot::{Counts, LoosePane, Snapshot, TrackerState, Tree};
use crate::model::tree::Link;
use crate::view::lines::{
    first_copy, marker, notes_of, prefix, root_key, way_below, Content, Group, GroupKind, Item,
    Line, Note, Place, ProjectLine, Recovery, Unread, INDENT,
};
use crate::view::row;

use super::facts::{Facts, TreeFacts};
use super::handle::{item_key, Folds, Handle, ItemKey};

/// One entry in a parent's sequence of children, before it becomes a line.
/// Notes and beads share the sequence because they share the box-drawing, and
/// a note is a child of the header exactly as a bead is.
enum Child<'a> {
    Note(Note),
    /// One way down from the parent to a bead beneath it.
    Node(&'a Link),
    /// The children a run stands for, in render order. Whose they are is the
    /// parent the entries were drawn under, so it is not repeated here.
    Elided(Vec<&'a Link>),
}

/// The work a line resting shut is hiding: `beneath` it, the beads under it
/// that its fold keeps off the screen, counted once each.
///
/// Nothing where the line is open or has nothing under it, because what it
/// stands over is then drawn on rows of its own. Nothing either on a later
/// copy of a bead, whose first line is already saying it — a reader adding up
/// what two copies hide is adding the ways down rather than the work.
///
/// Asked at every depth. A root is a bead row like any other, and the one
/// question a fold raises — what did that just take off the screen — has one
/// answer wherever it is asked.
fn shut_over(beneath: Counts, first: bool, folded: Option<bool>) -> Option<Counts> {
    (folded == Some(false) && first).then_some(beneath)
}

/// Every line the snapshot draws, in render order. `facts` is what the
/// snapshot answered when the forest took it.
pub(super) fn draw(snapshot: &Snapshot, facts: &Facts, folds: &Folds) -> Vec<Line> {
    Layout {
        snapshot,
        facts,
        folds,
    }
    .draw()
}

/// Whether a group is drawn at all, which is whether the snapshot has put
/// anything in it.
pub(super) fn group_drawn(snapshot: &Snapshot, kind: GroupKind) -> bool {
    let (_, loose) = recovery(snapshot);
    group_of(snapshot, kind, &loose).is_some()
}

/// What a group's own line says: how many things it holds, and how many of
/// them carry findings the screen is not drawing. Nothing where it holds
/// nothing, which is a group not drawn.
///
/// Only a hidden tree has findings to admit to: the filter took its
/// dangling and looping counts and its anomalies off the screen with it,
/// and a group that said only how many trees it hides would read as
/// "nothing to see" when some of them are broken.
fn group_of(snapshot: &Snapshot, kind: GroupKind, loose: &[LoosePane]) -> Option<Group> {
    let (count, with_findings) = match kind {
        GroupKind::HiddenTrees => (
            snapshot.hidden_trees.len(),
            snapshot
                .hidden_trees
                .iter()
                .filter(|hidden| hidden.findings)
                .count(),
        ),
        _ => (group_items(snapshot, kind, loose).len(), 0),
    };
    (count > 0).then_some(Group {
        kind,
        count,
        with_findings,
    })
}

/// The group one thing sits in, where the snapshot still holds it.
pub(super) fn group_holding(snapshot: &Snapshot, key: &ItemKey) -> Option<GroupKind> {
    let (_, loose) = recovery(snapshot);
    GroupKind::ALL.into_iter().find(|kind| {
        group_items(snapshot, *kind, &loose)
            .iter()
            .any(|item| item_key(item).as_ref() == Some(key))
    })
}

/// A snapshot, what it answered, and the folds set over it, which is all
/// that laying out reads.
struct Layout<'a> {
    snapshot: &'a Snapshot,
    facts: &'a Facts,
    folds: &'a Folds,
}

impl Layout<'_> {
    fn draw(&self) -> Vec<Line> {
        let (recovered, loose) = recovery(self.snapshot);
        let mut lines = Vec::new();
        let mut from = 0;
        // Every project the config names, in that order — the ones with rows
        // and the ones no collection has reached, which is every project on
        // the first frame of a run. Walking the projects rather than the
        // trees is what lets one be drawn before it has any: a project's
        // trees arrive together, so the run of them at `from` is that
        // project's, and so is the same run of the panes recovered for them.
        //
        // A project with no trees is drawn only where nothing has read it. A
        // tracker that answered and held nothing, and one that refused, are
        // both read: the first has nothing to draw and the second is reported
        // among the failed projects, and a line here would say of either that
        // its rows were still coming.
        for project in &self.snapshot.projects {
            let run = &self.snapshot.trees[from..];
            let held = run
                .iter()
                .take_while(|tree| tree.project == *project)
                .count();
            if held > 0 || !self.snapshot.read_at.contains_key(project) {
                self.draw_project(
                    project,
                    &run[..held],
                    &recovered[from..from + held],
                    &mut lines,
                );
                from += held;
            }
        }
        self.draw_groups(&loose, &mut lines);
        // Asked of the drawn lines rather than of the snapshot's fields, so
        // a later kind of line cannot be left out of the question.
        if lines.is_empty() {
            lines.push(nothing_to_draw());
        }
        self.say_what_the_directory_chose(&mut lines);
        lines
    }

    /// A project's own line, and the roots that hang under it.
    ///
    /// The project line says what is the project's — its name, how much work
    /// it holds, and the panes recovered where a root would not read — and
    /// every root below it is a bead row like any other. A root is a bead, and
    /// a reader asks a bead's questions of it: what is its status, who is on
    /// it, what is it doing. A line that answered those in a project's terms
    /// answered none of them.
    ///
    /// The project is named rather than taken from its first tree, because a
    /// project waiting on the collection that will fill it in has no tree to
    /// take it from and is exactly what the first frame of a run is made of.
    /// Everything the line says of one falls out: no root refused, no work
    /// counted, and nothing under it until the rows arrive. Its fold is kept
    /// on the same handle as ever, so a reader who shuts a project while it
    /// is still being read finds it shut when its rows land.
    fn draw_project(
        &self,
        project: &str,
        trees: &[Arc<Tree>],
        panes: &[Vec<LoosePane>],
        lines: &mut Vec<Line>,
    ) {
        let project = project.to_string();
        let unread = trees.iter().any(|tree| tree.tracker != TrackerState::Ok);
        // A project rests open: the forest is what is being worked, and a
        // project shut over it says only that it exists.
        let open = self.folds.expanded(&Handle::Project(project.clone()), true);
        let recovery = unread.then(|| Recovery {
            panes: panes.iter().flatten().cloned().collect(),
            complete: self.snapshot.unconfigured.is_empty(),
        });

        lines.push(Line {
            prefix: marker(open).to_string(),
            depth: 0,
            folded: Some(open),
            place: None,
            content: Content::Project(ProjectLine {
                every_root_read: self.snapshot.every_root_read(&project),
                counts: self.facts.project(&project),
                project,
                recovery,
            }),
        });

        if !open {
            return;
        }
        self.draw_trees(trees.iter().map(Arc::as_ref).collect(), lines);
    }

    /// Trees in a row under one line, each with what the snapshot answered
    /// for it.
    fn draw_trees(&self, trees: Vec<&Tree>, lines: &mut Vec<Line>) {
        let count = trees.len();
        for (n, tree) in trees.into_iter().enumerate() {
            TreeLayout {
                folds: self.folds,
                tree,
                facts: self.facts.tree(&root_key(tree)),
            }
            .draw(n + 1 == count, lines);
        }
    }
}

/// One tree being drawn: the tree, what it answered when the snapshot was
/// taken, and the folds set over it.
struct TreeLayout<'a> {
    folds: &'a Folds,
    tree: &'a Tree,
    facts: &'a TreeFacts,
}

impl TreeLayout<'_> {
    fn draw(&self, last: bool, lines: &mut Vec<Line>) {
        let root = Place::root(root_key(self.tree));
        let Some(node) = self.tree.beads.first() else {
            // No nodes, so no row: the root is named on a line of its own
            // rather than left out, because a root that would not read is the
            // one a reader most needs to see is there.
            lines.push(Line {
                prefix: prefix(&[], last, false, None),
                depth: 1,
                folded: None,
                place: Some(root),
                content: Content::Unread(Unread {
                    root: self.tree.root.clone(),
                    tracker: self.tree.tracker,
                }),
            });
            return;
        };
        // A tree opens because of what is in it, not because the selection
        // is in it: the first screen is meant to be the answer to what is
        // being worked and what could be started.
        let kids = self.children_entries(0, &[]);
        let bead = self.facts.bead(self.tree, 0, &[]);
        let open = !kids.is_empty()
            && self
                .folds
                .expanded(&Handle::Bead(root.clone()), bead.opens_a_fold);

        let folded = (!kids.is_empty()).then_some(open);
        lines.push(Line {
            prefix: prefix(&[], last, !kids.is_empty() && !open, None),
            depth: 1,
            folded,
            place: Some(root.clone()),
            content: Content::Bead(row::cells(
                node,
                &self.tree.root,
                bead.progress,
                shut_over(bead.beneath, first_copy(self.tree, 0, &[]), folded),
            )),
        });

        let mut entries: Vec<Child> = notes_of(self.tree).into_iter().map(Child::Note).collect();
        if open {
            entries.extend(kids);
        }
        self.draw_children(entries, &root, &[0], &mut vec![!last], lines);
    }

    /// `above` is the way down to `parent`, the parent itself included: the
    /// way down to every entry drawn here.
    fn draw_children(
        &self,
        entries: Vec<Child>,
        parent: &Place,
        above: &[usize],
        trunk: &mut Vec<bool>,
        lines: &mut Vec<Line>,
    ) {
        let count = entries.len();
        let depth = trunk.len() as u16 + 1;
        for (n, entry) in entries.into_iter().enumerate() {
            let last = n + 1 == count;
            match entry {
                Child::Note(note) => lines.push(Line {
                    prefix: prefix(trunk, last, false, None),
                    depth,
                    folded: None,
                    place: None,
                    content: Content::Note(note),
                }),
                Child::Elided(members) => {
                    // A run rests as the count it was drawn to be.
                    let open = self.folds.expanded(&Handle::Elided(parent.clone()), false);
                    lines.push(Line {
                        prefix: prefix(trunk, last, !open, None),
                        depth,
                        folded: Some(open),
                        place: None,
                        content: Content::Elided {
                            count: self.facts.run_size(self.tree, &members, above),
                            under: parent.clone(),
                        },
                    });
                    if open {
                        // A run is always the last of its parent's entries, so
                        // its beads hang under it rather than beside the
                        // siblings they belong to: anything drawn after it at
                        // that depth would follow an elbow that had already
                        // said it was the last.
                        trunk.push(!last);
                        let entries = members.into_iter().map(Child::Node).collect();
                        self.draw_children(entries, parent, above, trunk, lines);
                        trunk.pop();
                    }
                }
                Child::Node(link) => {
                    let at = link.bead;
                    let node = &self.tree.beads[at];
                    let place = parent.step_to(BeadKey {
                        project: self.tree.project.clone(),
                        id: node.id.clone(),
                    });
                    let kids = self.children_entries(at, above);
                    let bead = self.facts.bead(self.tree, at, above);
                    let first = first_copy(self.tree, at, above);
                    // Open the spine to the work a reader needs next and
                    // nothing else. A branch with none rests as one line, its
                    // glyph, its fraction and its marker saying what it holds.
                    let open = !kids.is_empty()
                        && self
                            .folds
                            .expanded(&Handle::Bead(place.clone()), first && bead.opens_a_fold);
                    let folded = (!kids.is_empty()).then_some(open);
                    lines.push(Line {
                        prefix: prefix(trunk, last, !kids.is_empty() && !open, Some(&link.edge)),
                        depth,
                        folded,
                        place: Some(place.clone()),
                        content: Content::Bead(row::cells(
                            node,
                            &self.tree.root,
                            bead.progress,
                            shut_over(bead.beneath, first, folded),
                        )),
                    });
                    if open {
                        trunk.push(!last);
                        let below = way_below(above, at);
                        self.draw_children(kids, &place, &below, trunk, lines);
                        trunk.pop();
                    }
                }
            }
        }
    }

    /// A node's children as they are drawn: the ones worth a line each, then
    /// one line for the run that is not.
    fn children_entries<'a>(&'a self, at: usize, above: &[usize]) -> Vec<Child<'a>> {
        let (drawn, elided) = self.facts.split(self.tree, at, above);
        let mut entries: Vec<Child> = drawn.into_iter().map(Child::Node).collect();
        if !elided.is_empty() {
            entries.push(Child::Elided(elided));
        }
        entries
    }
}

impl Layout<'_> {
    fn draw_groups(&self, loose: &[LoosePane], lines: &mut Vec<Line>) {
        for kind in GroupKind::ALL {
            let Some(group) = group_of(self.snapshot, kind, loose) else {
                continue;
            };
            let open = self.folds.expanded(&Handle::Group(kind), kind.live());
            lines.push(Line {
                prefix: marker(open).to_string(),
                depth: 0,
                folded: Some(open),
                place: None,
                content: Content::Group(group),
            });
            if !open {
                continue;
            }
            match kind {
                GroupKind::HiddenTrees => self.draw_trees(self.hidden_trees(), lines),
                _ => self.draw_items(group_items(self.snapshot, kind, loose), lines),
            }
        }
    }

    /// The trees the filter hid, which the snapshot still holds: a hidden
    /// tree is a tree, and the group is only where the filter put it, so
    /// each is drawn as its project would draw it.
    fn hidden_trees(&self) -> Vec<&Tree> {
        self.snapshot
            .hidden_trees
            .iter()
            .filter_map(|hidden| {
                self.snapshot.tree(&BeadKey {
                    project: hidden.project.clone(),
                    id: hidden.root.clone(),
                })
            })
            .collect()
    }

    fn draw_items(&self, items: Vec<Item>, lines: &mut Vec<Line>) {
        let count = items.len();
        for (n, item) in items.into_iter().enumerate() {
            lines.push(Line {
                prefix: prefix(&[], n + 1 == count, false, None),
                depth: 1,
                folded: None,
                place: None,
                content: Content::Item(item),
            });
        }
    }
}

impl Layout<'_> {
    /// The scope, where the directory chose it. A scope the reader typed is
    /// silent, and a run reading everything has nothing to say.
    fn say_what_the_directory_chose(&self, lines: &mut Vec<Line>) {
        if let Scope::Directory { project, .. } = &self.snapshot.scope {
            lines.push(Line {
                prefix: INDENT.to_string(),
                depth: 0,
                folded: None,
                place: None,
                content: Content::Scoped {
                    project: project.clone(),
                },
            });
        }
    }
}

/// Give each unreadable tree the live panes working in its project, and
/// keep the rest loose. A pane is one or the other and never both, so what
/// the headers show and what the group counts still add up to every pane.
fn recovery(snapshot: &Snapshot) -> (Vec<Vec<LoosePane>>, Vec<LoosePane>) {
    let mut recovered = vec![Vec::new(); snapshot.trees.len()];
    let mut loose = Vec::new();
    for pane in &snapshot.unattributed {
        let home = snapshot
            .trees
            .iter()
            .position(|tree| tree.tracker != TrackerState::Ok && tree.project == pane.project);
        match home {
            Some(tree) => recovered[tree].push(pane.clone()),
            None => loose.push(pane.clone()),
        }
    }
    (recovered, loose)
}

fn group_items(snapshot: &Snapshot, kind: GroupKind, loose: &[LoosePane]) -> Vec<Item> {
    match kind {
        GroupKind::FailedProjects => snapshot
            .failed_projects
            .iter()
            .cloned()
            .map(Item::Failed)
            .collect(),
        GroupKind::Conflicts => snapshot
            .conflicts
            .iter()
            .cloned()
            .map(Item::Conflict)
            .collect(),
        // Hidden trees are drawn as trees rather than as things in a group.
        GroupKind::HiddenTrees => Vec::new(),
        GroupKind::Unattributed => loose.iter().cloned().map(Item::Loose).collect(),
        GroupKind::Unconfigured => snapshot
            .unconfigured
            .iter()
            .cloned()
            .map(Item::Unconfigured)
            .collect(),
    }
}

/// The one line of a forest with nothing in it. Under no tree and in no
/// group, because there is neither: it stands for the whole screen.
fn nothing_to_draw() -> Line {
    Line {
        prefix: String::new(),
        depth: 0,
        folded: None,
        place: None,
        content: Content::Note(Note::NoRoots),
    }
}
