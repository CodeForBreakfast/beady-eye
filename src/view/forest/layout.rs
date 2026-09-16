//! The lines one snapshot draws, under the folds set over it.
//!
//! Laying out is a pure function of those two. Nothing here knows where the
//! selection is, how tall the screen is, or which key was pressed, and
//! nothing here moves a fold: it asks which way one points and draws what
//! that says.
//!
//! A fold is asked of by the line it is on, and a line is one way down to a
//! bead. The folds name a few lines, and beneath a line they name nothing at
//! or under, every fold answers from the scope over it alone — so what that
//! subtree draws is a function of the bead, where the line stands on the
//! spine, that scope, and nothing else. Those subtrees are counted from
//! the tree, once per bead, and drawn only where a reader looks into them;
//! the lines the folds name, and everything over them, are drawn here.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use crate::config;
use crate::model::join::BeadKey;
use crate::model::snapshot::{Counts, Node as Bead, Snapshot, Tree};
use crate::model::tree::Link;
use crate::model::types::Edge;
use crate::view::draw::identity_widths;
use crate::view::lines::{
    links_below, marker, notes_of, prefix, root_key, run_size, way_below, BeadFacts, Content,
    Group, GroupKind, Item, Line, Note, Place, ProjectLine, Unread, INDENT,
};
use crate::view::row::{self, Widths};

use super::drawn::{Beneath, Count, Counted, Drawn, Ground, Node, Undrawn};
use super::facts::{Facts, TreeFacts, Uniform};
use super::handle::{item_key, Folds, Handle, ItemKey, Scope};
use super::spine::{self, Stand};

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
/// stands over is then drawn on rows of its own.
///
/// Asked at every depth. A root is a bead row like any other, and the one
/// question a fold raises — what did that just take off the screen — has one
/// answer wherever it is asked.
///
/// The same reading answers a root drawn behind the line the mode puts the
/// rest of the forest behind: the bead the forest is rooted at is beneath
/// that root and drawn at the top of the screen, and this count holds it.
fn shut_over(beneath: Counts, folded: Option<bool>) -> Option<Counts> {
    (folded == Some(false)).then_some(beneath)
}

/// Every line the snapshot draws, in render order. `facts` is what the
/// snapshot answered when the forest took it, and `row` is what each bead's
/// row is drawn by.
pub(super) fn lay_out(
    snapshot: &Snapshot,
    facts: &Arc<Facts>,
    folds: &Folds,
    rooted: Option<&Rooted>,
    row: &row::Layout,
) -> Drawn {
    Layout::new(snapshot, facts, folds, rooted, false, &[], row).draw()
}

/// Every line the snapshot draws as one `Vec`, for a test to read whole.
#[cfg(test)]
pub(super) fn draw(
    snapshot: &Snapshot,
    facts: &Arc<Facts>,
    folds: &Folds,
    rooted: Option<&Rooted>,
) -> Vec<Line> {
    lay_out(snapshot, facts, folds, rooted, &row::Layout::default())
        .iter()
        .cloned()
        .collect()
}

/// Every line the snapshot draws and every line a shut fold hides, each fold
/// still saying which way it points. `also` names lines to draw as the folds'
/// own are drawn, so a key pressed on one finds it however the folds stand.
///
/// What a key that points every fold under a line reads: a fold beneath a
/// shut fold is on no line the screen draws, and pointing only what was
/// drawn cost a draw per level to reach it.
pub(super) fn draw_beneath_every_fold(
    snapshot: &Snapshot,
    facts: &Arc<Facts>,
    folds: &Folds,
    rooted: Option<&Rooted>,
    also: &[Handle],
    row: &row::Layout,
) -> Drawn {
    Layout::new(snapshot, facts, folds, rooted, true, also, row).draw()
}

#[cfg(test)]
thread_local! {
    /// How many times this thread has laid the forest out, for a test to
    /// say what a key cost.
    static DRAWS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(super) fn draws_so_far() -> usize {
    DRAWS.with(|draws| draws.get())
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

/// The trees a project is holding back, in the order its group lists them.
fn hidden_trees<'a>(snapshot: &'a Snapshot, project: Option<&str>) -> Vec<&'a Arc<Tree>> {
    snapshot
        .hidden_trees
        .iter()
        .filter(|hidden| Some(hidden.project.as_str()) == project)
        .filter_map(|hidden| {
            snapshot.shared_tree(&BeadKey {
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
    let roots: Vec<&Arc<Tree>> = match kind {
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
                    .map(|tree| (Arc::as_ref(tree), vec![0])),
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
                .map(|root| (Arc::as_ref(root.tree), vec![0])),
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
    tree: &'a Arc<Tree>,
    /// The bead drawn elsewhere, by its place among the tree's beads.
    without: Option<usize>,
}

impl<'a> Behind<'a> {
    /// The beads this root leaves behind the line: every one it reaches
    /// without stepping onto the bead drawn elsewhere, which is where the
    /// drawing stops as well.
    fn beads(&self) -> Vec<&'a Bead> {
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

/// The lines the folds name, by the way down to each: every line with an
/// entry, every line a scope leaves resting, and whatever the caller asked
/// to have drawn. A line named nowhere in here, with nothing named beneath
/// it, answers from the scope over it alone.
#[derive(Default)]
struct Named(BTreeMap<BeadKey, Mentioned>);

/// The named lines under one line, by the id of the bead each steps onto.
#[derive(Default)]
struct Mentioned {
    under: HashMap<String, Mentioned>,
}

impl Named {
    fn of<'a>(handles: impl IntoIterator<Item = &'a Handle>) -> Self {
        let mut named = Named::default();
        for handle in handles {
            let (Handle::Bead(place) | Handle::Elided(place)) = handle else {
                continue;
            };
            let mut at = named.0.entry(place.tree.clone()).or_default();
            for step in &place.steps {
                at = at.under.entry(step.id.clone()).or_default();
            }
        }
        named
    }

    /// The named lines in a tree, from its root.
    fn tree(&self, root: &BeadKey) -> Option<&Mentioned> {
        self.0.get(root)
    }
}

impl Mentioned {
    fn under(&self, id: &str) -> Option<&Mentioned> {
        self.under.get(id)
    }
}

/// What the layout keeps as it goes: the trees drawn from, by index, and
/// what every subtree left undrawn was counted to.
#[derive(Default)]
struct Kept {
    trees: Vec<Arc<Tree>>,
    beads: HashMap<Counted, Count>,
    runs: HashMap<Counted, Count>,
}

/// A snapshot, what it answered, and the folds set over it, which is all
/// that laying out reads.
struct Layout<'a> {
    snapshot: &'a Snapshot,
    facts: &'a Arc<Facts>,
    folds: &'a Folds,
    /// The bead the forest is rooted at, where the reader has rooted it at
    /// one. Nothing is drawn outside what hangs beneath it.
    rooted: Option<&'a Rooted>,
    /// Whether to draw what a shut fold hides as well.
    beneath_shut: bool,
    /// What each bead's row is drawn by, which says which cells a subtree
    /// left undrawn is measured over.
    row: &'a row::Layout,
    named: Named,
    kept: RefCell<Kept>,
}

impl<'a> Layout<'a> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        snapshot: &'a Snapshot,
        facts: &'a Arc<Facts>,
        folds: &'a Folds,
        rooted: Option<&'a Rooted>,
        beneath_shut: bool,
        also: &[Handle],
        row: &'a row::Layout,
    ) -> Self {
        Layout {
            snapshot,
            facts,
            folds,
            rooted,
            beneath_shut,
            row,
            named: Named::of(folds.mentioned().chain(also)),
            kept: RefCell::default(),
        }
    }

    fn draw(self) -> Drawn {
        #[cfg(test)]
        DRAWS.with(|draws| draws.set(draws.get() + 1));
        let mut lines = Vec::new();
        // Every project the config names, in that order — the ones with rows
        // and the ones no collection has reached, which is every project on
        // the first frame of a run. Walking the projects rather than the
        // trees is what lets one be drawn before it has any.
        for project in &self.snapshot.projects {
            if project_drawn(self.snapshot, project) {
                lines.push(self.draw_project(project));
            }
        }
        self.draw_groups(&mut lines);
        // Asked of the drawn lines rather than of the snapshot's fields, so
        // a later kind of line cannot be left out of the question.
        if lines.is_empty() {
            lines.push(Node::drawn(nothing_to_draw(), Vec::new()));
        }
        self.say_what_the_directory_chose(&mut lines);
        let kept = self.kept.into_inner();
        Drawn::over(
            lines,
            Some(Ground {
                trees: kept.trees,
                facts: Arc::clone(self.facts),
                beads: kept.beads,
                runs: kept.runs,
                beneath_shut: self.beneath_shut,
            }),
            self.row,
        )
    }

    /// The index a tree is drawn from by, the same for every line drawn from
    /// that tree.
    fn tree_index(&self, tree: &Arc<Tree>) -> usize {
        let mut kept = self.kept.borrow_mut();
        match kept.trees.iter().position(|held| Arc::ptr_eq(held, tree)) {
            Some(index) => index,
            None => {
                kept.trees.push(Arc::clone(tree));
                kept.trees.len() - 1
            }
        }
    }

    /// One tree, as it is drawn: from its root, or from the bead the forest
    /// is rooted at.
    fn tree_layout(
        &'a self,
        tree: &'a Arc<Tree>,
        over: Option<&'a Scope>,
        rests_shut: bool,
        rooted: Option<&'a Rooted>,
        without: Option<usize>,
    ) -> TreeLayout<'a> {
        let root = root_key(tree);
        let facts = self.facts.tree(&root);
        TreeLayout {
            layout: self,
            over,
            index: self.tree_index(tree),
            tree,
            facts,
            answers: facts.uniform(),
            named: self.named.tree(&root),
            rests_shut,
            rooted,
            without,
        }
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
    fn draw_project(&'a self, project: &str) -> Node {
        let project = project.to_string();
        let handle = Handle::Project(project.clone());
        // A project rests open: the forest is what is being worked, and a
        // project shut over it says only that it exists.
        let pointed = self.folds.pointed(&handle, None);
        let open = pointed.unwrap_or(true);
        let over = self.folds.beneath(&handle, None);

        let line = Line {
            prefix: marker(open).to_string(),
            depth: 0,
            folded: Some(open),
            place: None,
            content: Content::Project(ProjectLine {
                every_root_read: self.snapshot.every_root_read(&project),
                counts: self.facts.project(&project),
                project: project.clone(),
            }),
        };

        if !open && !self.beneath_shut {
            return Node::drawn(line, Vec::new());
        }
        let trees: Vec<&Arc<Tree>> = match self.rooted {
            // Rooted at one bead, the forest draws the one tree holding it,
            // and draws that tree from the bead rather than from its root.
            // Taken from what was collected rather than from what the filter
            // shows, so a reader who rooted the forest at a bead in a tree the
            // filter is holding back keeps the tree they asked for.
            Some(rooted) if rooted.place.tree.project == project => self
                .snapshot
                .shared_tree(&rooted.place.tree)
                .into_iter()
                .collect(),
            Some(_) => Vec::new(),
            None => self
                .snapshot
                .trees
                .iter()
                .filter(|tree| tree.project == project)
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
        let mut children = Vec::new();
        for tree in trees {
            entries -= 1;
            children.push(
                self.tree_layout(tree, over, false, self.rooted, None)
                    .draw(&mut trunk, entries == 0),
            );
        }
        for group in groups {
            entries -= 1;
            children.push(self.draw_group(group, over, &mut trunk, entries == 0));
        }
        Node::drawn(line, children)
    }

    /// One group, on a line under whatever `trunk` says is above it, and what
    /// it holds beneath that line where it is open.
    ///
    /// A hidden tree is a tree, and the group is only where the filter put it:
    /// each is drawn as its project would draw it, one level further in. The
    /// one thing that differs is where it rests — shut, whatever is beneath
    /// it, because the reader asked for trees with nobody on them to be out
    /// of the way and an open one is not.
    fn draw_group(
        &'a self,
        group: Group,
        over: Option<&'a Scope>,
        trunk: &mut Vec<bool>,
        last: bool,
    ) -> Node {
        let handle = Handle::Group(group.kind, group.project.clone());
        let pointed = self.folds.pointed(&handle, over);
        let open = pointed.unwrap_or(group.kind.live());
        let over = self.folds.beneath(&handle, over);
        let prefix = if trunk.is_empty() && group.project.is_none() {
            marker(open).to_string()
        } else {
            prefix(trunk, last, !open, None)
        };
        let depth = trunk.len() as u16 + u16::from(group.project.is_some());
        let kind = group.kind;
        let project = group.project.clone();
        let line = Line {
            prefix,
            depth,
            folded: Some(open),
            place: None,
            content: Content::Group(group),
        };
        if !open && !self.beneath_shut {
            return Node::drawn(line, Vec::new());
        }
        if project.is_some() {
            trunk.push(!last);
        }
        let children = match kind {
            GroupKind::HiddenTrees | GroupKind::OutOfTheWay => {
                let roots = self.roots_in(kind, project.as_deref());
                let count = roots.len();
                roots
                    .into_iter()
                    .enumerate()
                    .map(|(n, root)| {
                        self.tree_layout(root.tree, over, true, None, root.without)
                            .draw(trunk, n + 1 == count)
                    })
                    .collect()
            }
            _ => self.draw_items(group_items(self.snapshot, kind, project.as_deref()), trunk),
        };
        if project.is_some() {
            trunk.pop();
        }
        Node::drawn(line, children)
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

    fn draw_items(&self, items: Vec<Item>, trunk: &[bool]) -> Vec<Node> {
        let count = items.len();
        let depth = trunk.len() as u16 + 1;
        items
            .into_iter()
            .enumerate()
            .map(|(n, item)| {
                Node::drawn(
                    Line {
                        prefix: prefix(trunk, n + 1 == count, false, None),
                        depth,
                        folded: None,
                        place: None,
                        content: Content::Item(item),
                    },
                    Vec::new(),
                )
            })
            .collect()
    }

    /// The groups below the trees: what has no project line to hang under.
    fn draw_groups(&'a self, lines: &mut Vec<Node>) {
        for kind in GroupKind::BELOW_THE_TREES {
            let Some(group) = group_of(self.snapshot, kind, None, self.rooted) else {
                continue;
            };
            lines.push(self.draw_group(group, None, &mut Vec::new(), true));
        }
    }

    /// The count of a subtree nothing draws, worked out from the tree once
    /// per bead and kept for the lines that are drawn from it later.
    fn count(&self, tree: &Tree, answers: Uniform, counted: Counted) -> Count {
        let mut kept = self.kept.borrow_mut();
        count(
            &mut kept,
            tree,
            answers,
            self.beneath_shut,
            self.row,
            counted,
        )
    }
}

/// One tree being drawn: the tree, what it answered when the snapshot was
/// taken, the folds set over it, and whether its root rests shut whatever is
/// beneath it — which is how a tree the filter is holding back rests, and no
/// other.
struct TreeLayout<'a> {
    layout: &'a Layout<'a>,
    /// The scope over the tree, from the lines it hangs under.
    over: Option<&'a Scope>,
    /// The tree, by the index the lines drawn from it later know it by.
    index: usize,
    tree: &'a Arc<Tree>,
    facts: &'a TreeFacts,
    /// The tree's answers by bead alone, where it has them: what lets a
    /// subtree the folds name nothing in be counted rather than drawn.
    answers: Option<Uniform<'a>>,
    /// The lines the folds name in this tree, from its root.
    named: Option<&'a Mentioned>,
    rests_shut: bool,
    /// The bead to draw this tree from, where the reader has rooted the forest
    /// at one. Its own root otherwise.
    rooted: Option<&'a Rooted>,
    /// The bead to leave out, because the forest is drawing it somewhere
    /// else. Only the root the focused bead stands in, drawn behind the line
    /// the mode holds it back with, has one.
    without: Option<usize>,
}

impl<'a> TreeLayout<'a> {
    /// `trunk` is the way down to whatever this tree hangs under, as the
    /// box-drawing says it: empty for a root directly under its project.
    fn draw(&self, trunk: &mut Vec<bool>, last: bool) -> Node {
        let folds = self.layout.folds;
        // Where the walk starts. The bead the reader rooted the forest at is
        // drawn where its tree's root would be, and keeps the place it has
        // everywhere else, so a fold set on it survives the key that rooted
        // the forest there and the key that puts the forest back.
        let (root, way, over, named) = match self.rooted {
            Some(rooted) => (
                rooted.place.clone(),
                rooted.way.clone(),
                self.scope_over_rooted(rooted),
                self.named_under(&rooted.place),
            ),
            None => (
                Place::root(root_key(self.tree)),
                vec![0],
                self.over,
                self.named,
            ),
        };
        let (at, above) = way.split_last().expect("a way down ends somewhere");
        let at = *at;
        let depth = trunk.len() as u16 + 1;
        let Some(node) = self.tree.beads.get(at) else {
            // No nodes, so no row: the root is named on a line of its own
            // rather than left out, because a root that would not read is the
            // one a reader most needs to see is there.
            return Node::drawn(
                Line {
                    prefix: prefix(trunk, last, false, None),
                    depth,
                    folded: None,
                    place: Some(root),
                    content: Content::Unread(Unread {
                        root: self.tree.root.clone(),
                        tracker: self.tree.tracker.clone(),
                    }),
                },
                Vec::new(),
            );
        };
        // A tree opens because of what is in it, not because the selection
        // is in it: the first screen is meant to be the answer to what is
        // being worked and what could be started.
        let kids = self.children_entries(at, above);
        let bead = self.facts.bead(self.tree, at, above);
        let stand = stand_along(self.tree, &way);
        let handle = Handle::Bead(root.clone());
        let pointed = folds.pointed(&handle, over);
        let open = !kids.is_empty() && pointed.unwrap_or(!self.rests_shut && bead.opens_a_fold);
        let below = folds.beneath(&handle, over);

        let folded = (!kids.is_empty()).then_some(open);
        let line = bead_line(
            node,
            None,
            root.clone(),
            trunk,
            last,
            depth,
            None,
            folded,
            &bead,
        );

        let mut entries: Vec<Child> = notes_of(self.tree).into_iter().map(Child::Note).collect();
        if open || self.layout.beneath_shut {
            entries.extend(kids);
        }
        trunk.push(!last);
        let children = self.draw_children(entries, &root, &way, below, named, stand, trunk);
        trunk.pop();
        Node::drawn(line, children)
    }

    /// The lines the folds name beneath a place in this tree.
    fn named_under(&self, place: &Place) -> Option<&'a Mentioned> {
        place
            .steps
            .iter()
            .fold(self.named, |named, step| named?.under(&step.id))
    }

    /// The scope over the bead the forest is rooted at. The beads above it
    /// are drawn behind the line the mode holds the rest back with, and a
    /// scope set on one of them still stands over it — so they are read on
    /// the way down to it, as the walk that drew them would have read them.
    fn scope_over_rooted(&self, rooted: &Rooted) -> Option<&'a Scope> {
        let folds: &'a Folds = self.layout.folds;
        let mut over = self.over;
        let mut place = Place::root(root_key(self.tree));
        for step in &rooted.way[1..] {
            over = folds.beneath(&Handle::Bead(place.clone()), over);
            place = place.step_to(BeadKey {
                project: self.tree.project.clone(),
                id: self.tree.beads[*step].id.clone(),
            });
        }
        over
    }

    /// `above` is the way down to `parent`, the parent itself included: the
    /// way down to every entry drawn here. `over` is the scope they are
    /// under, `named` the lines the folds name beneath the parent, and
    /// `stand` where the parent stands on the spine.
    #[allow(clippy::too_many_arguments)]
    fn draw_children(
        &self,
        entries: Vec<Child>,
        parent: &Place,
        above: &[usize],
        over: Option<&'a Scope>,
        named: Option<&'a Mentioned>,
        stand: Stand,
        trunk: &mut Vec<bool>,
    ) -> Vec<Node> {
        let folds = self.layout.folds;
        let count = entries.len();
        let depth = trunk.len() as u16 + 1;
        let mut drawn = Vec::with_capacity(count);
        for (n, entry) in entries.into_iter().enumerate() {
            let last = n + 1 == count;
            match entry {
                Child::Note(note) => drawn.push(Node::drawn(
                    Line {
                        prefix: prefix(trunk, last, false, None),
                        depth,
                        folded: None,
                        place: None,
                        content: Content::Note(note),
                    },
                    Vec::new(),
                )),
                Child::Elided(members) => {
                    let handle = Handle::Elided(parent.clone());
                    // A run rests as the count it was drawn to be.
                    let pointed = folds.pointed(&handle, over);
                    let open = pointed.unwrap_or(false);
                    let below = folds.beneath(&handle, over);
                    let line = run_line(
                        parent,
                        self.run_size(&members, above),
                        trunk,
                        last,
                        depth,
                        open,
                    );
                    let mut children = Vec::new();
                    if open || self.layout.beneath_shut {
                        // A run is always the last of its parent's entries, so
                        // its beads hang under it rather than beside the
                        // siblings they belong to: anything drawn after it at
                        // that depth would follow an elbow that had already
                        // said it was the last.
                        trunk.push(!last);
                        let entries = members.into_iter().map(Child::Node).collect();
                        children =
                            self.draw_children(entries, parent, above, below, named, stand, trunk);
                        trunk.pop();
                    }
                    drawn.push(Node::drawn(line, children));
                }
                Child::Node(link) => {
                    let node = &self.tree.beads[link.bead];
                    let named = named.and_then(|named| named.under(&node.id));
                    let stand = spine::beneath(stand, link);
                    drawn.push(match (named, self.answers) {
                        (None, Some(answers)) => {
                            self.undrawn(answers, link, parent, over, stand, trunk, last, depth)
                        }
                        (named, _) => {
                            self.draw_child(link, parent, above, over, named, stand, trunk, last)
                        }
                    });
                }
            }
        }
        drawn
    }

    /// One bead the folds name something at or beneath, drawn here with its
    /// own fold asked for by its line, and every entry beneath it drawn the
    /// same way.
    #[allow(clippy::too_many_arguments)]
    fn draw_child(
        &self,
        link: &Link,
        parent: &Place,
        above: &[usize],
        over: Option<&'a Scope>,
        named: Option<&'a Mentioned>,
        stand: Stand,
        trunk: &mut Vec<bool>,
        last: bool,
    ) -> Node {
        let folds = self.layout.folds;
        let at = link.bead;
        let node = &self.tree.beads[at];
        let place = parent.step_to(BeadKey {
            project: self.tree.project.clone(),
            id: node.id.clone(),
        });
        let kids = self.children_entries(at, above);
        let bead = self.facts.bead(self.tree, at, above);
        let handle = Handle::Bead(place.clone());
        // Open the spine to the work a reader needs next and nothing else. A
        // branch with none rests as one line, its glyph, its fraction and its
        // marker saying what it holds.
        let pointed = folds.pointed(&handle, over);
        let open = !kids.is_empty() && pointed.unwrap_or(spine::rests_open(stand, &bead));
        let below = folds.beneath(&handle, over);
        let folded = (!kids.is_empty()).then_some(open);
        let line = bead_line(
            node,
            Some(&parent.key().id),
            place.clone(),
            trunk,
            last,
            trunk.len() as u16 + 1,
            Some(&link.edge),
            folded,
            &bead,
        );
        let mut children = Vec::new();
        if open || self.layout.beneath_shut {
            trunk.push(!last);
            let way = way_below(above, at);
            children = self.draw_children(kids, &place, &way, below, named, stand, trunk);
            trunk.pop();
        }
        Node::drawn(line, children)
    }

    /// One bead the folds name nothing at or beneath: its line, and what
    /// hangs under it counted rather than drawn.
    #[allow(clippy::too_many_arguments)]
    fn undrawn(
        &self,
        answers: Uniform<'a>,
        link: &Link,
        parent: &Place,
        over: Option<&Scope>,
        stand: Stand,
        trunk: &[bool],
        last: bool,
        depth: u16,
    ) -> Node {
        let counted = Counted {
            tree: self.index,
            at: link.bead,
            stand,
            forced: Folds::forced(over),
            without: self.without,
        };
        let count = self.layout.count(self.tree, answers, counted);
        undrawn_node(
            self.tree,
            answers,
            self.layout.beneath_shut,
            counted,
            count.rows,
            link,
            parent,
            trunk,
            last,
            depth,
        )
    }

    /// A node's children as they are drawn: the ones worth a line each, then
    /// one line for the run that is not.
    fn children_entries<'b>(&'b self, at: usize, above: &[usize]) -> Vec<Child<'b>> {
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

/// A bead's line, from what the walk decided about it.
#[allow(clippy::too_many_arguments)]
fn bead_line(
    node: &Bead,
    parent: Option<&str>,
    place: Place,
    trunk: &[bool],
    last: bool,
    depth: u16,
    edge: Option<&Edge>,
    folded: Option<bool>,
    bead: &BeadFacts,
) -> Line {
    Line {
        prefix: prefix(trunk, last, folded == Some(false), edge),
        depth,
        folded,
        place: Some(place),
        content: Content::Bead(row::cells(
            node,
            parent,
            bead.progress,
            shut_over(bead.beneath.clone(), folded),
        )),
    }
}

/// A run's line, from what the walk decided about it.
fn run_line(
    parent: &Place,
    count: usize,
    trunk: &[bool],
    last: bool,
    depth: u16,
    open: bool,
) -> Line {
    Line {
        prefix: prefix(trunk, last, !open, None),
        depth,
        folded: Some(open),
        place: None,
        content: Content::Elided {
            count,
            under: parent.clone(),
        },
    }
}

/// The children of `at` split into the ones drawn and the run that is not,
/// the bead drawn elsewhere left out of both.
fn split_without<'t>(
    answers: Uniform,
    tree: &'t Tree,
    at: usize,
    without: Option<usize>,
) -> (Vec<&'t Link>, Vec<&'t Link>) {
    let (mut drawn, mut elided) = answers.split(tree, at);
    if let Some(elsewhere) = without {
        drawn.retain(|link| link.bead != elsewhere);
        elided.retain(|link| link.bead != elsewhere);
    }
    (drawn, elided)
}

/// What a run stands for, where its members were counted rather than drawn.
fn run_size_undrawn(
    answers: Uniform,
    tree: &Tree,
    at: usize,
    members: &[&Link],
    without: Option<usize>,
) -> usize {
    match without {
        Some(elsewhere) => run_size(tree, members, &[at, elsewhere]),
        None => answers.run(at),
    }
}

/// Where the bead at the end of `way` stands, the tree's root at its head:
/// the root's stand, taken down every link the way takes.
fn stand_along(tree: &Tree, way: &[usize]) -> Stand {
    way.windows(2).fold(spine::root(), |stand, step| {
        let link = tree.children[step[0]]
            .iter()
            .find(|link| link.bead == step[1])
            .expect("a way down follows the tree's own links");
        spine::beneath(stand, link)
    })
}

/// The rows and the identity's widths a subtree nothing draws adds up to,
/// from the tree alone: which way every fold in it goes is the scope's, or
/// the default's, and neither needs the line.
fn count(
    kept: &mut Kept,
    tree: &Tree,
    answers: Uniform,
    beneath_shut: bool,
    row: &row::Layout,
    counted: Counted,
) -> Count {
    if let Some(count) = kept.beads.get(&counted) {
        return count.clone();
    }
    let Counted {
        at,
        stand,
        forced,
        without,
        ..
    } = counted;
    let (drawn, elided) = split_without(answers, tree, at, without);
    let kids = !drawn.is_empty() || !elided.is_empty();
    let open = kids && forced.unwrap_or(spine::rests_open(stand, answers.bead(at)));
    let mut total = Count {
        rows: 1,
        widths: Widths::default(),
    };
    if open || beneath_shut {
        for link in &drawn {
            total.add(count_child(
                kept,
                tree,
                answers,
                beneath_shut,
                row,
                counted,
                link,
            ));
        }
        if !elided.is_empty() {
            total.add(count_run(
                kept,
                tree,
                answers,
                beneath_shut,
                row,
                counted,
                &elided,
            ));
        }
    }
    kept.beads.insert(counted, total.clone());
    total
}

/// What a run under a counted bead adds up to, its own line included.
fn count_run(
    kept: &mut Kept,
    tree: &Tree,
    answers: Uniform,
    beneath_shut: bool,
    row: &row::Layout,
    under: Counted,
    members: &[&Link],
) -> Count {
    if let Some(count) = kept.runs.get(&under) {
        return count.clone();
    }
    let open = under.forced.unwrap_or(false);
    let mut total = Count {
        rows: 1,
        widths: Widths::default(),
    };
    if open || beneath_shut {
        for link in members {
            total.add(count_child(
                kept,
                tree,
                answers,
                beneath_shut,
                row,
                under,
                link,
            ));
        }
    }
    kept.runs.insert(under, total.clone());
    total
}

/// What one child of a counted bead adds up to, its own line's widths among
/// the widths beneath its parent.
///
/// The line is made to be measured, as it would be to be drawn: what is
/// shut over it is not, because nothing in the identity says so.
fn count_child(
    kept: &mut Kept,
    tree: &Tree,
    answers: Uniform,
    beneath_shut: bool,
    row: &row::Layout,
    parent: Counted,
    link: &Link,
) -> Count {
    let child = Counted {
        at: link.bead,
        stand: spine::beneath(parent.stand, link),
        ..parent
    };
    let mut count = count(kept, tree, answers, beneath_shut, row, child);
    let own = row::cells(
        &tree.beads[link.bead],
        Some(&tree.beads[parent.at].id),
        answers.bead(link.bead).progress,
        None,
    );
    count.widths.merge(&identity_widths(&own, row));
    count
}

impl Count {
    fn add(&mut self, beneath: Count) {
        self.rows += beneath.rows;
        self.widths.merge(&beneath.widths);
    }
}

/// A bead's line where nothing the folds name is at or beneath it, with
/// what hangs under it left to be drawn from the tree.
#[allow(clippy::too_many_arguments)]
fn undrawn_node(
    tree: &Tree,
    answers: Uniform,
    beneath_shut: bool,
    counted: Counted,
    rows: usize,
    link: &Link,
    parent: &Place,
    trunk: &[bool],
    last: bool,
    depth: u16,
) -> Node {
    let at = link.bead;
    let node = &tree.beads[at];
    let place = parent.step_to(BeadKey {
        project: tree.project.clone(),
        id: node.id.clone(),
    });
    let kids = links_below(tree, at, &[])
        .into_iter()
        .any(|link| Some(link.bead) != counted.without);
    let bead = answers.bead(at);
    let rests_open = spine::rests_open(counted.stand, bead);
    let open = kids && counted.forced.unwrap_or(rests_open);
    let folded = kids.then_some(open);
    let line = bead_line(
        node,
        Some(&parent.key().id),
        place,
        trunk,
        last,
        depth,
        Some(&link.edge),
        folded,
        bead,
    );
    let beneath = if open || beneath_shut {
        Beneath::Bead(Undrawn {
            counted,
            trunk: trunk.iter().copied().chain([!last]).collect(),
        })
    } else {
        Beneath::Nothing
    };
    Node::undrawn(line, rows, beneath)
}

/// The tree and its answers a subtree left undrawn is drawn from.
fn ground_of<'g>(ground: &'g Ground, counted: &Counted) -> (&'g Arc<Tree>, Uniform<'g>) {
    let tree = &ground.trees[counted.tree];
    let answers = ground
        .facts
        .tree(&root_key(tree))
        .uniform()
        .expect("a subtree left undrawn is in a tree with one answer per bead");
    (tree, answers)
}

/// The entries under a bead copy nothing drew, drawn now: its children and
/// the run they leave, each counted already.
pub(super) fn beneath_bead(ground: &Ground, node: &Node, undrawn: &Undrawn) -> Vec<Node> {
    let counted = undrawn.counted;
    let (tree, answers) = ground_of(ground, &counted);
    let parent = node.line.place.as_ref().expect("a bead's line has a place");
    let (drawn, elided) = split_without(answers, tree, counted.at, counted.without);
    let count = drawn.len() + usize::from(!elided.is_empty());
    let depth = node.line.depth + 1;
    let mut children: Vec<Node> = drawn
        .iter()
        .enumerate()
        .map(|(n, link)| {
            children_of(
                ground,
                tree,
                answers,
                counted,
                link,
                parent,
                &undrawn.trunk,
                n + 1 == count,
                depth,
            )
        })
        .collect();
    if !elided.is_empty() {
        let open = counted.forced.unwrap_or(false);
        let rows = ground
            .runs
            .get(&counted)
            .expect("a run beneath a counted bead was counted with it")
            .rows;
        let line = run_line(
            parent,
            run_size_undrawn(answers, tree, counted.at, &elided, counted.without),
            &undrawn.trunk,
            true,
            depth,
            open,
        );
        let beneath = if open || ground.beneath_shut {
            Beneath::Run(Undrawn {
                counted,
                trunk: undrawn.trunk.iter().copied().chain([false]).collect(),
            })
        } else {
            Beneath::Nothing
        };
        children.push(Node::undrawn(line, rows, beneath));
    }
    children
}

/// The members of a run nothing drew, drawn now.
pub(super) fn beneath_run(ground: &Ground, node: &Node, undrawn: &Undrawn) -> Vec<Node> {
    let counted = undrawn.counted;
    let (tree, answers) = ground_of(ground, &counted);
    let Content::Elided { under, .. } = &node.line.content else {
        return Vec::new();
    };
    let (_, members) = split_without(answers, tree, counted.at, counted.without);
    let count = members.len();
    let depth = node.line.depth + 1;
    members
        .iter()
        .enumerate()
        .map(|(n, link)| {
            children_of(
                ground,
                tree,
                answers,
                counted,
                link,
                under,
                &undrawn.trunk,
                n + 1 == count,
                depth,
            )
        })
        .collect()
}

/// One child of a counted bead, drawn from the count kept for it.
#[allow(clippy::too_many_arguments)]
fn children_of(
    ground: &Ground,
    tree: &Tree,
    answers: Uniform,
    parent: Counted,
    link: &Link,
    place: &Place,
    trunk: &[bool],
    last: bool,
    depth: u16,
) -> Node {
    let counted = Counted {
        at: link.bead,
        stand: spine::beneath(parent.stand, link),
        ..parent
    };
    let rows = ground
        .beads
        .get(&counted)
        .expect("a bead beneath a counted bead was counted with it")
        .rows;
    undrawn_node(
        tree,
        answers,
        ground.beneath_shut,
        counted,
        rows,
        link,
        place,
        trunk,
        last,
        depth,
    )
}

impl Layout<'_> {
    /// The scope, where the directory chose it. A scope the reader typed is
    /// silent, and a run reading everything has nothing to say.
    fn say_what_the_directory_chose(&self, lines: &mut Vec<Node>) {
        if let config::Scope::Directory { project, .. } = &self.snapshot.scope {
            lines.push(Node::drawn(
                Line {
                    prefix: INDENT.to_string(),
                    depth: 0,
                    folded: None,
                    place: None,
                    content: Content::Scoped {
                        project: project.clone(),
                    },
                },
                Vec::new(),
            ));
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
