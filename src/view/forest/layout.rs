//! The lines one snapshot draws, under the folds set over it.
//!
//! Laying out is a pure function of those two. Nothing here knows where the
//! selection is, how tall the screen is, or which key was pressed, and
//! nothing here moves a fold: it asks which way one points and draws what
//! that says.

use std::sync::Arc;

use crate::config::Scope;
use crate::model::join::BeadKey;
use crate::model::snapshot::{Counts, Snapshot, Tree};
use crate::model::tree::Link;
use crate::view::lines::{
    first_copy, marker, notes_of, prefix, root_key, way_below, Content, Group, GroupKind, Item,
    Line, Note, Place, ProjectLine, Unread, INDENT,
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
pub(super) fn group_drawn(snapshot: &Snapshot, kind: GroupKind, project: Option<&str>) -> bool {
    group_of(snapshot, kind, project).is_some()
}

/// Whether a project's line is drawn: where anything hangs under it — a tree
/// shown or hidden, or a pane working in its paths that no bead claims — or
/// where nothing has read it yet, which is every project on the first frame
/// of a run.
///
/// A project with nothing beneath it is drawn only where nothing has read
/// it. A tracker that answered and held nothing, and one that refused, are
/// both read: the first has nothing to draw and the second is reported among
/// the failed projects, and a line here would say of either that its rows
/// were still coming.
pub(super) fn project_drawn(snapshot: &Snapshot, project: &str) -> bool {
    snapshot.trees.iter().any(|tree| tree.project == project)
        || snapshot
            .hidden_trees
            .iter()
            .any(|hidden| hidden.project == project)
        || snapshot
            .unattributed
            .iter()
            .any(|pane| pane.project == project)
        || !snapshot.read_at.contains_key(project)
}

/// What a group's own line says: how many things it holds, and how many of
/// them carry findings the screen is not drawing. Nothing where it holds
/// nothing, which is a group not drawn.
///
/// `project` is the project the group is one of, for the two kinds drawn
/// under a project's line, and nothing for the kinds below the trees.
///
/// Only a hidden tree has findings to admit to: the filter took its
/// dangling and looping counts and its anomalies off the screen with it,
/// and a group that said only how many trees it hides would read as
/// "nothing to see" when some of them are broken.
/// The trees a project is holding back, in the order its group lists them.
fn hidden_trees<'a>(snapshot: &'a Snapshot, project: Option<&str>) -> Vec<&'a Tree> {
    snapshot
        .hidden_trees
        .iter()
        .filter(|hidden| Some(hidden.project.as_str()) == project)
        .filter_map(|hidden| {
            snapshot.tree(&BeadKey {
                project: hidden.project.clone(),
                id: hidden.root.clone(),
            })
        })
        .collect()
}

/// The first bead a group's own contents hold, where it holds any.
///
/// A group resting shut draws none of its contents, so a reader standing on
/// its line has nothing below it saying where they are among the beads. This
/// is read from `hidden_trees`, which is the source `trees_drawn` reads, so
/// the anchor and the order it points into cannot disagree.
///
/// Only the hidden trees hold beads. The panes, failed projects and conflicts
/// the other groups hold are in no ordering of beads and have nothing here to
/// answer with.
pub(super) fn first_bead_of(
    snapshot: &Snapshot,
    kind: GroupKind,
    project: Option<&str>,
) -> Option<BeadKey> {
    match kind {
        GroupKind::HiddenTrees => {
            let tree = hidden_trees(snapshot, project).into_iter().next()?;
            Some(BeadKey {
                project: tree.project.clone(),
                id: tree.beads.first()?.id.clone(),
            })
        }
        _ => None,
    }
}

/// Every tree the forest draws, in the order it draws them: project by
/// project as the config names them, and within a project the trees the
/// filter shows before the ones it hid, which is where `draw_project` puts
/// the hidden-trees group.
///
/// `snapshot.trees` cannot answer this. It is in the order the projects were
/// read, which is why `snapshot.projects` exists at all — a project drawn
/// before its collection returns has no tree to be implied by.
///
/// Folds are not consulted, so the set is wider than what is on screen at the
/// moment of asking: this is the order the rows *would* be drawn in with
/// everything open. `draw_project` stops at a project the reader has shut and
/// this does not, which is the same asymmetry `place_of` has always had — a
/// bead's `ancestry_of` ends at its project, and `open_over` sets every handle
/// in that chain, so going to a bead opens the project over it exactly as it
/// opens the folds. An order that left those trees out would be an order a
/// search could not use.
pub(super) fn trees_drawn(snapshot: &Snapshot) -> Vec<&Tree> {
    let mut drawn = Vec::new();
    for project in &snapshot.projects {
        if !project_drawn(snapshot, project) {
            continue;
        }
        drawn.extend(
            snapshot
                .trees
                .iter()
                .filter(|tree| tree.project == *project)
                .map(Arc::as_ref),
        );
        drawn.extend(hidden_trees(snapshot, Some(project)));
    }
    drawn
}

fn group_of(snapshot: &Snapshot, kind: GroupKind, project: Option<&str>) -> Option<Group> {
    let (count, with_findings) = match kind {
        GroupKind::HiddenTrees => {
            let hidden = snapshot
                .hidden_trees
                .iter()
                .filter(|hidden| Some(hidden.project.as_str()) == project);
            (
                hidden.clone().count(),
                hidden.filter(|hidden| hidden.findings).count(),
            )
        }
        _ => (group_items(snapshot, kind, project).len(), 0),
    };
    (count > 0).then_some(Group {
        kind,
        project: project.map(str::to_string),
        count,
        with_findings,
    })
}

/// Every group the snapshot could draw, in the order it draws them: each
/// project's own, then the ones below the trees.
pub(super) fn every_group(
    snapshot: &Snapshot,
) -> impl Iterator<Item = (GroupKind, Option<String>)> + '_ {
    snapshot
        .projects
        .iter()
        .flat_map(|project| {
            GroupKind::UNDER_A_PROJECT
                .into_iter()
                .map(move |kind| (kind, Some(project.clone())))
        })
        .chain(
            GroupKind::BELOW_THE_TREES
                .into_iter()
                .map(|kind| (kind, None)),
        )
}

/// The group one thing sits in, where the snapshot still holds it.
pub(super) fn group_holding(snapshot: &Snapshot, key: &ItemKey) -> Option<Handle> {
    every_group(snapshot)
        .find(|(kind, project)| {
            group_items(snapshot, *kind, project.as_deref())
                .iter()
                .any(|item| item_key(item).as_ref() == Some(key))
        })
        .map(|(kind, project)| Handle::Group(kind, project))
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
        let mut lines = Vec::new();
        // Every project the config names, in that order — the ones with rows
        // and the ones no collection has reached, which is every project on
        // the first frame of a run. Walking the projects rather than the
        // trees is what lets one be drawn before it has any.
        for project in &self.snapshot.projects {
            if project_drawn(self.snapshot, project) {
                self.draw_project(project, &mut lines);
            }
        }
        self.draw_groups(&mut lines);
        // Asked of the drawn lines rather than of the snapshot's fields, so
        // a later kind of line cannot be left out of the question.
        if lines.is_empty() {
            lines.push(nothing_to_draw());
        }
        self.say_what_the_directory_chose(&mut lines);
        lines
    }

    /// A project's own line, and everything that hangs under it: the roots
    /// the filter shows, then the project's own groups — the trees the
    /// filter is holding back, and the panes in its paths that no bead
    /// claims.
    ///
    /// The project line says what is the project's — its name and how much
    /// work it holds — and every root below it is a bead row like any other.
    /// A root is a bead, and a reader asks a bead's questions of it: what is
    /// its status, who is on it, what is it doing. A line that answered those
    /// in a project's terms answered none of them.
    ///
    /// The project is named rather than taken from its first tree, because a
    /// project waiting on the collection that will fill it in has no tree to
    /// take it from and is exactly what the first frame of a run is made of.
    /// Everything the line says of one falls out: no root refused, no work
    /// counted, and nothing under it until the rows arrive. Its fold is kept
    /// on the same handle as ever, so a reader who shuts a project while it
    /// is still being read finds it shut when its rows land.
    fn draw_project(&self, project: &str, lines: &mut Vec<Line>) {
        let project = project.to_string();
        // A project rests open: the forest is what is being worked, and a
        // project shut over it says only that it exists.
        let open = self.folds.expanded(&Handle::Project(project.clone()), true);

        lines.push(Line {
            prefix: marker(open).to_string(),
            depth: 0,
            folded: Some(open),
            place: None,
            content: Content::Project(ProjectLine {
                every_root_read: self.snapshot.every_root_read(&project),
                counts: self.facts.project(&project),
                project: project.clone(),
            }),
        });

        if !open {
            return;
        }
        let trees: Vec<&Tree> = self
            .snapshot
            .trees
            .iter()
            .filter(|tree| tree.project == project)
            .map(Arc::as_ref)
            .collect();
        let groups: Vec<Group> = GroupKind::UNDER_A_PROJECT
            .into_iter()
            .filter_map(|kind| group_of(self.snapshot, kind, Some(&project)))
            .collect();
        let mut entries = trees.len() + groups.len();
        let mut trunk = Vec::new();
        for tree in trees {
            entries -= 1;
            TreeLayout {
                folds: self.folds,
                tree,
                facts: self.facts.tree(&root_key(tree)),
                rests_shut: false,
            }
            .draw(&mut trunk, entries == 0, lines);
        }
        for group in groups {
            entries -= 1;
            self.draw_group(group, &mut trunk, entries == 0, lines);
        }
    }

    /// One group, on a line under whatever `trunk` says is above it, and what
    /// it holds beneath that line where it is open.
    ///
    /// A hidden tree is a tree, and the group is only where the filter put it:
    /// each is drawn as its project would draw it, one level further in. The
    /// one thing that differs is where it rests — shut, whatever is beneath
    /// it, because the reader asked for trees with nobody on them to be out
    /// of the way and an open one is not.
    fn draw_group(&self, group: Group, trunk: &mut Vec<bool>, last: bool, lines: &mut Vec<Line>) {
        let open = self.folds.expanded(
            &Handle::Group(group.kind, group.project.clone()),
            group.kind.live(),
        );
        let prefix = if trunk.is_empty() && group.project.is_none() {
            marker(open).to_string()
        } else {
            prefix(trunk, last, !open, None)
        };
        let depth = trunk.len() as u16 + u16::from(group.project.is_some());
        let kind = group.kind;
        let project = group.project.clone();
        lines.push(Line {
            prefix,
            depth,
            folded: Some(open),
            place: None,
            content: Content::Group(group),
        });
        if !open {
            return;
        }
        if project.is_some() {
            trunk.push(!last);
        }
        match kind {
            GroupKind::HiddenTrees => {
                let hidden = self.hidden_trees(project.as_deref());
                let count = hidden.len();
                for (n, tree) in hidden.into_iter().enumerate() {
                    TreeLayout {
                        folds: self.folds,
                        tree,
                        facts: self.facts.tree(&root_key(tree)),
                        rests_shut: true,
                    }
                    .draw(trunk, n + 1 == count, lines);
                }
            }
            _ => self.draw_items(
                group_items(self.snapshot, kind, project.as_deref()),
                trunk,
                lines,
            ),
        }
        if project.is_some() {
            trunk.pop();
        }
    }

    /// The trees the filter hid from one project, which the snapshot still
    /// holds.
    fn hidden_trees(&self, project: Option<&str>) -> Vec<&Tree> {
        hidden_trees(self.snapshot, project)
    }

    fn draw_items(&self, items: Vec<Item>, trunk: &[bool], lines: &mut Vec<Line>) {
        let count = items.len();
        let depth = trunk.len() as u16 + 1;
        for (n, item) in items.into_iter().enumerate() {
            lines.push(Line {
                prefix: prefix(trunk, n + 1 == count, false, None),
                depth,
                folded: None,
                place: None,
                content: Content::Item(item),
            });
        }
    }

    /// The groups below the trees: what has no project line to hang under.
    fn draw_groups(&self, lines: &mut Vec<Line>) {
        for kind in GroupKind::BELOW_THE_TREES {
            let Some(group) = group_of(self.snapshot, kind, None) else {
                continue;
            };
            self.draw_group(group, &mut Vec::new(), true, lines);
        }
    }
}

/// One tree being drawn: the tree, what it answered when the snapshot was
/// taken, the folds set over it, and whether its root rests shut whatever is
/// beneath it — which is how a tree the filter is holding back rests, and no
/// other.
struct TreeLayout<'a> {
    folds: &'a Folds,
    tree: &'a Tree,
    facts: &'a TreeFacts,
    rests_shut: bool,
}

impl TreeLayout<'_> {
    /// `trunk` is the way down to whatever this tree hangs under, as the
    /// box-drawing says it: empty for a root directly under its project.
    fn draw(&self, trunk: &mut Vec<bool>, last: bool, lines: &mut Vec<Line>) {
        let root = Place::root(root_key(self.tree));
        let depth = trunk.len() as u16 + 1;
        let Some(node) = self.tree.beads.first() else {
            // No nodes, so no row: the root is named on a line of its own
            // rather than left out, because a root that would not read is the
            // one a reader most needs to see is there.
            lines.push(Line {
                prefix: prefix(trunk, last, false, None),
                depth,
                folded: None,
                place: Some(root),
                content: Content::Unread(Unread {
                    root: self.tree.root.clone(),
                    tracker: self.tree.tracker.clone(),
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
            && self.folds.expanded(
                &Handle::Bead(root.clone()),
                !self.rests_shut && bead.opens_a_fold,
            );

        let folded = (!kids.is_empty()).then_some(open);
        lines.push(Line {
            prefix: prefix(trunk, last, !kids.is_empty() && !open, None),
            depth,
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
        trunk.push(!last);
        self.draw_children(entries, &root, &[0], trunk, lines);
        trunk.pop();
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

/// The things one group holds. `project` is the project the group is one of,
/// where it is a project's own.
fn group_items(snapshot: &Snapshot, kind: GroupKind, project: Option<&str>) -> Vec<Item> {
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
        GroupKind::Unattributed => snapshot
            .unattributed
            .iter()
            .filter(|pane| Some(pane.project.as_str()) == project)
            .cloned()
            .map(Item::Loose)
            .collect(),
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
