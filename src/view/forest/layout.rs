//! The lines one snapshot draws, under the folds set over it.
//!
//! Laying out is a pure function of those two. Nothing here knows where the
//! selection is, how tall the screen is, or which key was pressed, and
//! nothing here moves a fold: it asks which way one points and draws what
//! that says.

use crate::model::join::BeadKey;
use crate::model::snapshot::{Counts, LoosePane, Snapshot, TrackerState, Tree};
use crate::view::lines::{
    children_of, first_copy, marker, notes_of, opens_a_fold, prefix, progress_of, root_key,
    run_size, split, unfinished_beneath, Content, Group, GroupKind, Item, Line, Note, Place,
    ProjectLine, Recovery, Unread,
};
use crate::view::row;

use super::handle::{item_key, Folds, Handle, ItemKey};

/// One entry in a parent's sequence of children, before it becomes a line.
/// Notes and beads share the sequence because they share the box-drawing, and
/// a note is a child of the header exactly as a bead is.
enum Child {
    Note(Note),
    Node(usize),
    /// The children a run stands for, in render order. Whose they are is the
    /// parent the entries were drawn under, so it is not repeated here.
    Elided(Vec<usize>),
}

/// Every line the snapshot draws, in render order.
pub(super) fn draw(snapshot: &Snapshot, folds: &Folds) -> Vec<Line> {
    Layout { snapshot, folds }.draw()
}

/// Whether a group is drawn at all, which is whether the snapshot has put
/// anything in it.
pub(super) fn group_drawn(snapshot: &Snapshot, kind: GroupKind) -> bool {
    let (_, loose) = recovery(snapshot);
    !group_items(snapshot, kind, &loose).is_empty()
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

/// A snapshot and the folds set over it, which is all that laying out reads.
struct Layout<'a> {
    snapshot: &'a Snapshot,
    folds: &'a Folds,
}

impl Layout<'_> {
    fn draw(&self) -> Vec<Line> {
        let (recovered, loose) = recovery(self.snapshot);
        let mut lines = Vec::new();
        let mut from = 0;
        // A project's trees arrive together and in the order the config named
        // the projects, so a run of them is a project.
        for run in self.snapshot.trees.chunk_by(|a, b| a.project == b.project) {
            let panes = &recovered[from..from + run.len()];
            self.draw_project(run, panes, &mut lines);
            from += run.len();
        }
        self.draw_groups(&loose, &mut lines);
        // Asked of the drawn lines rather than of the snapshot's fields, so
        // a later kind of line cannot be left out of the question.
        if lines.is_empty() {
            lines.push(nothing_to_draw());
        }
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
    fn draw_project(&self, trees: &[Tree], panes: &[Vec<LoosePane>], lines: &mut Vec<Line>) {
        let project = trees[0].project.clone();
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
                project,
                counts: Counts::over(trees.iter().flat_map(|tree| &tree.nodes)),
                recovery,
            }),
        });

        if !open {
            return;
        }
        let count = trees.len();
        for (n, tree) in trees.iter().enumerate() {
            self.draw_tree(tree, n + 1 == count, lines);
        }
    }

    fn draw_tree(&self, tree: &Tree, last: bool, lines: &mut Vec<Line>) {
        let root = Place::root(root_key(tree));
        let children = children_of(&tree.nodes);
        let Some(node) = tree.nodes.first() else {
            // No nodes, so no row: the root is named on a line of its own
            // rather than left out, because a root that would not read is the
            // one a reader most needs to see is there.
            lines.push(Line {
                prefix: prefix(&[], last, false),
                depth: 1,
                folded: None,
                place: Some(root),
                content: Content::Unread(Unread {
                    root: tree.root.clone(),
                    tracker: tree.tracker,
                }),
            });
            return;
        };
        // A tree opens because of what is in it, not because the selection
        // is in it: the first screen is meant to be the answer to what is
        // being worked and what could be started.
        let kids = children_entries(tree, &children, 0);
        let open = !kids.is_empty()
            && self.folds.expanded(
                &Handle::Bead(root.clone()),
                opens_a_fold(tree, &children, 0),
            );

        lines.push(Line {
            prefix: prefix(&[], last, !kids.is_empty() && !open),
            depth: 1,
            folded: (!kids.is_empty()).then_some(open),
            place: Some(root.clone()),
            content: Content::Bead(row::cells(
                node,
                &tree.root,
                progress_of(tree, &children, 0),
                None,
            )),
        });

        let mut entries: Vec<Child> = notes_of(tree).into_iter().map(Child::Note).collect();
        if open {
            entries.extend(kids);
        }
        self.draw_children(tree, &children, entries, &root, &mut vec![!last], lines);
    }

    fn draw_children(
        &self,
        tree: &Tree,
        children: &[Vec<usize>],
        entries: Vec<Child>,
        parent: &Place,
        trunk: &mut Vec<bool>,
        lines: &mut Vec<Line>,
    ) {
        let count = entries.len();
        let depth = trunk.len() as u16 + 1;
        for (n, entry) in entries.into_iter().enumerate() {
            let last = n + 1 == count;
            match entry {
                Child::Note(note) => lines.push(Line {
                    prefix: prefix(trunk, last, false),
                    depth,
                    folded: None,
                    place: None,
                    content: Content::Note(note),
                }),
                Child::Elided(members) => {
                    // A run rests as the count it was drawn to be.
                    let open = self.folds.expanded(&Handle::Elided(parent.clone()), false);
                    lines.push(Line {
                        prefix: prefix(trunk, last, !open),
                        depth,
                        folded: Some(open),
                        place: None,
                        content: Content::Elided {
                            count: run_size(tree, children, &members),
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
                        self.draw_children(tree, children, entries, parent, trunk, lines);
                        trunk.pop();
                    }
                }
                Child::Node(at) => {
                    let node = &tree.nodes[at];
                    let place = parent.step_to(BeadKey {
                        project: tree.project.clone(),
                        id: node.id.clone(),
                    });
                    let kids = children_entries(tree, children, at);
                    let first = first_copy(tree, at);
                    // Open the spine to the work a reader needs next and
                    // nothing else. A branch with none rests as one line, its
                    // glyph, its fraction and its marker saying what it holds.
                    let open = !kids.is_empty()
                        && self.folds.expanded(
                            &Handle::Bead(place.clone()),
                            first && opens_a_fold(tree, children, at),
                        );
                    // A later line is shut over beads the first line is
                    // already drawing, so counting them here would have a
                    // reader adding up the ways down rather than the work.
                    let holding = (node.status.is_closed() && !open && first)
                        .then(|| unfinished_beneath(tree, children, at))
                        .filter(|unfinished| *unfinished > 0);
                    lines.push(Line {
                        prefix: prefix(trunk, last, !kids.is_empty() && !open),
                        depth,
                        folded: (!kids.is_empty()).then_some(open),
                        place: Some(place.clone()),
                        content: Content::Bead(row::cells(
                            node,
                            &tree.root,
                            progress_of(tree, children, at),
                            holding,
                        )),
                    });
                    if open {
                        trunk.push(!last);
                        self.draw_children(tree, children, kids, &place, trunk, lines);
                        trunk.pop();
                    }
                }
            }
        }
    }

    fn draw_groups(&self, loose: &[LoosePane], lines: &mut Vec<Line>) {
        for kind in GroupKind::ALL {
            let items = group_items(self.snapshot, kind, loose);
            if items.is_empty() {
                continue;
            }
            let open = self.folds.expanded(&Handle::Group(kind), kind.live());
            lines.push(Line {
                prefix: marker(open).to_string(),
                depth: 0,
                folded: Some(open),
                place: None,
                content: Content::Group(Group {
                    kind,
                    count: items.len(),
                    with_findings: with_findings(self.snapshot, &items),
                }),
            });
            if !open {
                continue;
            }
            let count = items.len();
            for (n, item) in items.into_iter().enumerate() {
                let last = n + 1 == count;
                lines.push(Line {
                    prefix: prefix(&[], last, false),
                    depth: 1,
                    folded: None,
                    place: None,
                    content: Content::Item(item),
                });
            }
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
        GroupKind::HiddenTrees => snapshot
            .hidden_trees
            .iter()
            .cloned()
            .map(Item::Hidden)
            .collect(),
        GroupKind::Unattributed => loose.iter().cloned().map(Item::Loose).collect(),
        GroupKind::Unconfigured => snapshot
            .unconfigured
            .iter()
            .cloned()
            .map(Item::Unconfigured)
            .collect(),
    }
}

/// A node's children as they are drawn: the ones worth a line each, then
/// one line for the run that is not.
fn children_entries(tree: &Tree, children: &[Vec<usize>], at: usize) -> Vec<Child> {
    let (drawn, elided) = split(tree, children, at);
    let mut entries: Vec<Child> = drawn.into_iter().map(Child::Node).collect();
    if !elided.is_empty() {
        entries.push(Child::Elided(elided));
    }
    entries
}

/// The hidden trees whose findings went with them. `collected` still holds
/// every tree that was read, shown or hidden, so what the filter took out
/// of the forest is still countable here.
fn with_findings(snapshot: &Snapshot, items: &[Item]) -> usize {
    items
        .iter()
        .filter(|item| match item {
            Item::Hidden(hidden) => snapshot
                .collected
                .iter()
                .filter(|tree| tree.project == hidden.project && tree.root == hidden.root)
                .any(|tree| !notes_of(tree).is_empty()),
            _ => false,
        })
        .count()
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
