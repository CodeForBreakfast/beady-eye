//! The lines one snapshot draws, under the folds set over it.
//!
//! Laying out is a pure function of those two. Nothing here knows where the
//! selection is, how tall the screen is, or which key was pressed, and
//! nothing here moves a fold: it asks which way one points and draws what
//! that says.

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::config::Scope;
use crate::model::join::BeadKey;
use crate::model::snapshot::{Counts, Node, Snapshot, Tree};
use crate::model::tree::Link;
use crate::view::lines::{
    first_copy, marker, notes_of, prefix, root_key, run_size, way_below, Content, Group, GroupKind,
    Item, Line, Note, Place, ProjectLine, Unread, INDENT,
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
///
/// The same reading answers a root drawn behind the line the mode puts the
/// rest of the forest behind: the bead the forest is rooted at is beneath
/// that root and drawn at the top of the screen, and this count holds it, for
/// the same reason two copies of a bead do not add up.
fn shut_over(beneath: Counts, first: bool, folded: Option<bool>) -> Option<Counts> {
    (folded == Some(false) && first).then_some(beneath)
}

/// Every line the snapshot draws, in render order. `facts` is what the
/// snapshot answered when the forest took it.
pub(super) fn draw(
    snapshot: &Snapshot,
    facts: &Facts,
    folds: &Folds,
    rooted: Option<&Rooted>,
) -> Vec<Line> {
    Layout {
        snapshot,
        facts,
        folds,
        rooted,
    }
    .draw()
}

/// The one bead the forest is rooted at, where the reader has asked for that:
/// the line they asked on, and the nodes the way down to it steps through.
///
/// Both, because the place says which line and the way says which nodes, and
/// resolving one into the other needs the snapshot the line was drawn from.
pub(super) struct Rooted {
    pub(super) place: Place,
    /// The way down from the tree's root to the focused bead, that bead last.
    pub(super) way: Vec<usize>,
}

/// Whether a group is drawn at all, which is whether the snapshot has put
/// anything in it.
pub(super) fn group_drawn(
    snapshot: &Snapshot,
    kind: GroupKind,
    project: Option<&str>,
    rooted: Option<&Rooted>,
) -> bool {
    group_of(snapshot, kind, project, rooted).is_some()
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
/// is read from the same two answers `walked` reads, so the anchor and the
/// order it points into cannot disagree.
///
/// Only the two groups over roots hold beads. The panes, failed projects and
/// conflicts the other groups hold are in no ordering of beads and have
/// nothing here to answer with.
pub(super) fn first_bead_of(
    snapshot: &Snapshot,
    kind: GroupKind,
    project: Option<&str>,
    rooted: Option<&Rooted>,
) -> Option<BeadKey> {
    let roots = match kind {
        GroupKind::HiddenTrees => hidden_trees(snapshot, project),
        GroupKind::OutOfTheWay => out_of_the_way(snapshot, project, rooted)
            .into_iter()
            .map(|root| root.tree)
            .collect(),
        _ => return None,
    };
    // A root whose tracker refused has a row and no bead, and it leads its
    // project, so the first root here is the one most likely to hold nothing.
    roots.into_iter().find_map(|tree| {
        Some(BeadKey {
            project: tree.project.clone(),
            id: tree.beads.first()?.id.clone(),
        })
    })
}

/// Every tree the forest draws and the way down it starts drawing from, in
/// the order the rows come out: project by project as the config names them,
/// and within a project the trees in the order `draw_project` puts them.
///
/// Rooted at one bead, that is the bead's own tree walked from the bead, and
/// then every root behind the line in the order the line opens onto them —
/// the bead's own tree among them, walked from its root for the part of it
/// the mode stopped drawing.
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
pub(super) fn walked<'a>(
    snapshot: &'a Snapshot,
    rooted: Option<&Rooted>,
) -> Vec<(&'a Tree, Vec<usize>)> {
    let mut drawn = Vec::new();
    for project in &snapshot.projects {
        if !project_drawn(snapshot, project) {
            continue;
        }
        let Some(rooted) = rooted else {
            drawn.extend(
                snapshot
                    .trees
                    .iter()
                    .filter(|tree| tree.project == *project)
                    .map(|tree| (Arc::as_ref(tree), vec![0])),
            );
            drawn.extend(
                hidden_trees(snapshot, Some(project))
                    .into_iter()
                    .map(|tree| (tree, vec![0])),
            );
            continue;
        };
        if rooted.place.tree.project == *project {
            drawn.extend(
                snapshot
                    .tree(&rooted.place.tree)
                    .map(|tree| (tree, rooted.way.clone())),
            );
        }
        drawn.extend(
            out_of_the_way(snapshot, Some(project), Some(rooted))
                .into_iter()
                .map(|root| (root.tree, vec![0])),
        );
    }
    drawn
}

fn group_of(
    snapshot: &Snapshot,
    kind: GroupKind,
    project: Option<&str>,
    rooted: Option<&Rooted>,
) -> Option<Group> {
    let (count, with_findings, held) = match kind {
        GroupKind::HiddenTrees => {
            let hidden = snapshot
                .hidden_trees
                .iter()
                .filter(|hidden| Some(hidden.project.as_str()) == project);
            (
                hidden.clone().count(),
                hidden.filter(|hidden| hidden.findings).count(),
                None,
            )
        }
        // Counted off the beads behind the line rather than off the roots they
        // came from: this line is all a reader gets of what is behind it, and
        // the bead the forest is rooted at is on the screen already.
        GroupKind::OutOfTheWay => {
            let held = out_of_the_way(snapshot, project, rooted);
            let counts = Counts::over(held.iter().flat_map(Behind::beads));
            (held.len(), 0, Some(counts))
        }
        _ => (group_items(snapshot, kind, project).len(), 0, None),
    };
    (count > 0).then_some(Group {
        kind,
        project: project.map(str::to_string),
        count,
        with_findings,
        held,
    })
}

/// One root drawn behind a group's line, and the bead of it the forest is
/// drawing somewhere else.
///
/// Only the root the focused bead stands in has such a bead. The mode draws
/// that bead where a root is drawn, so what is left behind the line is the
/// beads above it and every branch off them.
struct Behind<'a> {
    tree: &'a Tree,
    /// The bead drawn elsewhere, by its place among the tree's beads.
    without: Option<usize>,
}

impl<'a> Behind<'a> {
    /// The beads this root leaves behind the line: every one it reaches
    /// without stepping onto the bead drawn elsewhere, which is where the
    /// drawing stops as well.
    fn beads(&self) -> Vec<&'a Node> {
        reached(self.tree, [0], self.without)
            .into_iter()
            .filter_map(|at| self.tree.beads.get(at))
            .collect()
    }
}

/// Every bead a walk from `from` reaches, `from` included, without stepping
/// onto the bead the forest is drawing somewhere else.
///
/// Which is what a count of what a line stands over has to be walked with: the
/// drawing stops at that bead, so a count that went past it names work the
/// reader is already looking at.
fn reached(
    tree: &Tree,
    from: impl IntoIterator<Item = usize>,
    without: Option<usize>,
) -> BTreeSet<usize> {
    let mut walked = BTreeSet::new();
    let mut left: Vec<usize> = from.into_iter().collect();
    while let Some(at) = left.pop() {
        if Some(at) == without || !walked.insert(at) {
            continue;
        }
        left.extend(
            tree.children
                .get(at)
                .into_iter()
                .flatten()
                .map(|link| link.bead),
        );
    }
    walked
}

/// What one project put out of the way because the forest is rooted at one
/// bead: every root it collected but the one that bead stands in, and that
/// one for the part of it the mode stopped drawing. None at all where the
/// forest is rooted at no bead.
///
/// Shown and hidden alike. The mode moves what the filter was showing as well
/// as what it was not, and one line standing for both sets is the only line
/// that adds up.
fn out_of_the_way<'a>(
    snapshot: &'a Snapshot,
    project: Option<&str>,
    rooted: Option<&Rooted>,
) -> Vec<Behind<'a>> {
    let Some(rooted) = rooted else {
        return Vec::new();
    };
    let (focused, above) = rooted.way.split_last().expect("a way down ends somewhere");
    snapshot
        .collected
        .iter()
        .map(Arc::as_ref)
        .filter(|tree| Some(tree.project.as_str()) == project)
        .filter_map(|tree| {
            if root_key(tree) != rooted.place.tree {
                return Some(Behind {
                    tree,
                    without: None,
                });
            }
            // The root that bead is itself leaves nothing here: the whole tree
            // hangs beneath it, so the mode stopped drawing none of it.
            (!above.is_empty()).then_some(Behind {
                tree,
                without: Some(*focused),
            })
        })
        .collect()
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
    /// The bead the forest is rooted at, where the reader has rooted it at
    /// one. Nothing is drawn outside what hangs beneath it.
    rooted: Option<&'a Rooted>,
}

impl<'a> Layout<'a> {
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
        let trees: Vec<&Tree> = match self.rooted {
            // Rooted at one bead, the forest draws the one tree holding it,
            // and draws that tree from the bead rather than from its root.
            // Taken from what was collected rather than from what the filter
            // shows, so a reader who rooted the forest at a bead in a tree the
            // filter is holding back keeps the tree they asked for.
            Some(rooted) if rooted.place.tree.project == project => {
                self.snapshot.tree(&rooted.place.tree).into_iter().collect()
            }
            Some(_) => Vec::new(),
            None => self
                .snapshot
                .trees
                .iter()
                .filter(|tree| tree.project == project)
                .map(Arc::as_ref)
                .collect(),
        };
        let groups: Vec<Group> = GroupKind::UNDER_A_PROJECT
            .into_iter()
            // The group of trees the filter is holding back draws whole trees,
            // and one bead is the only root there is while the forest is
            // rooted at one.
            .filter(|kind| self.rooted.is_none() || *kind != GroupKind::HiddenTrees)
            .filter_map(|kind| group_of(self.snapshot, kind, Some(&project), self.rooted))
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
                rooted: self.rooted,
                without: None,
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
            GroupKind::HiddenTrees | GroupKind::OutOfTheWay => {
                let roots = self.roots_in(kind, project.as_deref());
                let count = roots.len();
                for (n, root) in roots.into_iter().enumerate() {
                    TreeLayout {
                        folds: self.folds,
                        tree: root.tree,
                        facts: self.facts.tree(&root_key(root.tree)),
                        rests_shut: true,
                        rooted: None,
                        without: root.without,
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

    /// The roots one of the two groups that stand over roots is standing over,
    /// for the project it is one of.
    fn roots_in(&self, kind: GroupKind, project: Option<&str>) -> Vec<Behind<'a>> {
        match kind {
            GroupKind::OutOfTheWay => out_of_the_way(self.snapshot, project, self.rooted),
            _ => hidden_trees(self.snapshot, project)
                .into_iter()
                .map(|tree| Behind {
                    tree,
                    without: None,
                })
                .collect(),
        }
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
            let Some(group) = group_of(self.snapshot, kind, None, self.rooted) else {
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
    /// The bead to draw this tree from, where the reader has rooted the forest
    /// at one. Its own root otherwise.
    rooted: Option<&'a Rooted>,
    /// The bead to leave out, because the forest is drawing it somewhere
    /// else. Only the root the focused bead stands in, drawn behind the line
    /// the mode holds it back with, has one.
    without: Option<usize>,
}

impl TreeLayout<'_> {
    /// `trunk` is the way down to whatever this tree hangs under, as the
    /// box-drawing says it: empty for a root directly under its project.
    fn draw(&self, trunk: &mut Vec<bool>, last: bool, lines: &mut Vec<Line>) {
        // Where the walk starts. The bead the reader rooted the forest at is
        // drawn where its tree's root would be, and keeps the place it has
        // everywhere else, so a fold set on it survives the key that rooted
        // the forest there and the key that puts the forest back.
        let (root, way) = match self.rooted {
            Some(rooted) => (rooted.place.clone(), rooted.way.clone()),
            None => (Place::root(root_key(self.tree)), vec![0]),
        };
        let (at, above) = way.split_last().expect("a way down ends somewhere");
        let at = *at;
        let depth = trunk.len() as u16 + 1;
        let Some(node) = self.tree.beads.get(at) else {
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
        let kids = self.children_entries(at, above);
        let bead = self.facts.bead(self.tree, at, above);
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
                None,
                bead.progress,
                shut_over(bead.beneath, first_copy(self.tree, at, above), folded),
            )),
        });

        let mut entries: Vec<Child> = notes_of(self.tree).into_iter().map(Child::Note).collect();
        if open {
            entries.extend(kids);
        }
        trunk.push(!last);
        self.draw_children(entries, &root, &way, trunk, lines);
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
                            count: self.run_size(&members, above),
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
                            Some(&parent.key().id),
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
        let mut entries: Vec<Child> = drawn
            .into_iter()
            .filter(|link| self.draws(link))
            .map(Child::Node)
            .collect();
        let elided: Vec<&Link> = elided.into_iter().filter(|link| self.draws(link)).collect();
        if !elided.is_empty() {
            entries.push(Child::Elided(elided));
        }
        entries
    }

    fn draws(&self, link: &Link) -> bool {
        Some(link.bead) != self.without
    }

    /// What a run stands for: its members and everything beneath them. Walked
    /// again where a bead is being left out, because the tree's own answer was
    /// worked out over a run this drawing is not making and reaches beads it
    /// is not drawing.
    ///
    /// The bead left out is put among the beads the way down came through,
    /// which is where the count already stops: a way back to one of those is
    /// a loop the drawing cuts, and the bead drawn elsewhere is cut for the
    /// same reason its rows are.
    fn run_size(&self, members: &[&Link], above: &[usize]) -> usize {
        match self.without {
            Some(elsewhere) => {
                let above: Vec<usize> = above.iter().copied().chain([elsewhere]).collect();
                run_size(self.tree, members, &above)
            }
            None => self.facts.run_size(self.tree, members, above),
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
        // These two hold whole roots, drawn as trees rather than as things in
        // a group.
        GroupKind::HiddenTrees | GroupKind::OutOfTheWay => Vec::new(),
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
