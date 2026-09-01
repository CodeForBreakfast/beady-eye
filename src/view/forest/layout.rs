//! The lines one snapshot draws, under the folds set over it.
//!
//! Laying out is a pure function of those two. Nothing here knows where the
//! selection is, how tall the screen is, or which key was pressed, and
//! nothing here moves a fold: it asks which way one points and draws what
//! that says.

use crate::model::join::BeadKey;
use crate::model::snapshot::{Counts, LoosePane, Snapshot, TrackerState, Tree};
use crate::view::lines::{
    children_of, counts_beneath, first_copy, marker, notes_of, opens_a_fold, prefix, progress_of,
    root_key, run_size, split, Content, Group, GroupKind, Item, Line, Note, Place, ProjectLine,
    Recovery, Unread,
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

/// The work a line resting shut is hiding: the beads under it that its fold
/// keeps off the screen, counted once each.
///
/// Nothing where the line is open or has nothing under it, because what it
/// stands over is then drawn on rows of its own. Nothing either on a later
/// copy of a bead, whose first line is already saying it — a reader adding up
/// what two copies hide is adding the ways down rather than the work.
///
/// Asked at every depth. A root is a bead row like any other, and the one
/// question a fold raises — what did that just take off the screen — has one
/// answer wherever it is asked.
fn shut_over(
    tree: &Tree,
    children: &[Vec<usize>],
    at: usize,
    folded: Option<bool>,
) -> Option<Counts> {
    (folded == Some(false) && first_copy(tree, at)).then(|| counts_beneath(tree, children, at))
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
        trees: &[Tree],
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

        let folded = (!kids.is_empty()).then_some(open);
        lines.push(Line {
            prefix: prefix(&[], last, !kids.is_empty() && !open),
            depth: 1,
            folded,
            place: Some(root.clone()),
            content: Content::Bead(row::cells(
                node,
                &tree.root,
                progress_of(tree, &children, 0),
                shut_over(tree, &children, 0, folded),
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
                    // Open the spine to the work a reader needs next and
                    // nothing else. A branch with none rests as one line, its
                    // glyph, its fraction and its marker saying what it holds.
                    let open = !kids.is_empty()
                        && self.folds.expanded(
                            &Handle::Bead(place.clone()),
                            first_copy(tree, at) && opens_a_fold(tree, children, at),
                        );
                    let folded = (!kids.is_empty()).then_some(open);
                    lines.push(Line {
                        prefix: prefix(trunk, last, !kids.is_empty() && !open),
                        depth,
                        folded,
                        place: Some(place.clone()),
                        content: Content::Bead(row::cells(
                            node,
                            &tree.root,
                            progress_of(tree, children, at),
                            shut_over(tree, children, at, folded),
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
