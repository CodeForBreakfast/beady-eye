//! The lines the forest is made of, and the shape of the tree behind them.
//!
//! Every question here is asked of a `Tree` and its nodes alone: which node is
//! whose child, what rests open, what a run stands for, how far along a branch
//! is. None of it knows the fold state, the selection, or which screen it is
//! drawn on, which is what lets the state machine next door be about nothing
//! else.

use crate::model::join::{BeadKey, Conflict};
use crate::model::snapshot::{
    Counts, FailedProject, HiddenTree, LoosePane, Node, TrackerState, Tree, UnconfiguredPane,
};
use crate::model::types::Status;
use crate::view::row::{Progress, Row};

/// How many finished siblings it takes before a count reads better than their
/// names. Under it they are drawn, and a finished branch is one line whatever
/// it holds, so the run saves one row per member past the first.
const MANY: usize = 3;

pub(crate) const OPEN: &str = "▾ ";
pub(crate) const SHUT: &str = "▸ ";
/// A tree's children start under its header's marker, not under its project.
const INDENT: &str = "  ";
const BRANCH: &str = "├── ";
const LAST: &str = "└── ";
/// The same elbows with the shut marker drawn into them. A marker appended
/// after an elbow would cost its own two columns, and a line's content would
/// then start further right for having something folded under it.
const BRANCH_SHUT: &str = "├─▸ ";
const LAST_SHUT: &str = "└─▸ ";
const TRUNK: &str = "│   ";
const GAP: &str = "    ";

/// Where a line sits in the walk that drew it: the tree it was drawn in, and
/// the beads stepped through below that tree's root to reach it.
///
/// A bead reachable more than once is drawn once for each way down to it, and
/// every copy carries the same key. Only the way down tells them apart, which
/// is why a fold and a selection are held by this rather than by the bead the
/// line sits on.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Place {
    /// The tree, by its root.
    pub tree: BeadKey,
    /// The beads stepped through below that root to reach the line, the last
    /// of which is the bead the line stands for. Empty on the tree's own
    /// header, which is the root itself.
    pub steps: Vec<BeadKey>,
}

impl Place {
    /// A tree's header, where every walk down it starts.
    pub(crate) fn root(tree: BeadKey) -> Self {
        Self {
            tree,
            steps: Vec::new(),
        }
    }

    /// One step further down, onto a child of the bead this place names.
    pub(crate) fn step_to(&self, key: BeadKey) -> Self {
        let mut stepped = self.clone();
        stepped.steps.push(key);
        stepped
    }

    /// The bead this place names.
    pub(crate) fn key(&self) -> &BeadKey {
        self.steps.last().unwrap_or(&self.tree)
    }

    /// Every place above this one in its tree, nearest first, ending at the
    /// tree's own header.
    pub(crate) fn forebears(&self) -> impl Iterator<Item = Place> + '_ {
        (0..self.steps.len()).rev().map(|kept| Self {
            tree: self.tree.clone(),
            steps: self.steps[..kept].to_vec(),
        })
    }
}

/// One line of the forest, in the order the screen draws them.
///
/// A line is exactly one screen row. `selected_line` is an index into these,
/// and the renderer derives its scroll offset by arithmetic on that index, so
/// a line that wrapped would put the selection and the row out of step with
/// nothing to say so. Anything that wants two rows is two lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// The line's leading text: its fold marker, or the box-drawing that
    /// places it under its parent. Drawn here rather than by the renderer
    /// because only the flattening knows which ancestors still have siblings
    /// below them, which is what decides where a `│` runs.
    pub prefix: String,
    /// How far under its tree's header this line sits. Zero for a header, for
    /// a group, and for the lines beneath a group.
    pub depth: u16,
    /// Whether this line's fold is open, where it has one at all.
    pub folded: Option<bool>,
    /// Where this line was drawn, where it stands for a bead at all: a bead's
    /// own row, and a tree header's root. One field rather than a key beside
    /// a position, so the two can never disagree about which copy this is.
    pub place: Option<Place>,
    pub content: Content,
}

impl Line {
    /// The bead this line stands for, for a caller that wants the bead and
    /// not the copy.
    pub fn bead(&self) -> Option<&BeadKey> {
        self.place.as_ref().map(Place::key)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    /// A root: the line a whole tree collapses to.
    Tree(Header),
    Bead(Row),
    /// A run of closed siblings nobody is working, said as a count.
    Elided {
        count: usize,
        /// Where the bead whose children the run stands for was drawn. A run
        /// is not a bead, so it has no `Line::place` of its own; this is what
        /// the fold is known by.
        under: Place,
    },
    /// Something true of the tree above rather than of any one bead in it.
    Note(Note),
    /// One of the groups below the trees.
    Group(Group),
    /// One thing in such a group.
    Item(Item),
}

/// A tree's own line, with the panes `bdi` could recover for it where its
/// tracker could not be read at all.
///
/// It holds the tree's own facts rather than the tree itself: a line is
/// compared whole on every keystroke, and none of a tree's nodes are drawn on
/// its header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub project: String,
    pub root: String,
    pub title: String,
    pub counts: Counts,
    pub tracker: TrackerState,
    /// The root's own status. A root is a bead like any other and a reader
    /// asks the same question of it, but it is the one bead whose line is a
    /// header, so its status has to be carried here to be drawn at all.
    ///
    /// Absent on a tree whose tracker never answered: there are no nodes, so
    /// there is no status to show, and the header says why instead.
    pub status: Option<Status>,
    /// Live panes working in this project, where no bead could be read to
    /// attribute them to. Empty on a tree that was read.
    pub panes: Vec<LoosePane>,
    /// Whether `panes` is all of them. A pane working under no configured
    /// project could belong here and cannot be told, so one of those anywhere
    /// leaves every recovery partial.
    pub panes_complete: bool,
}

/// A finding about a tree rather than about any bead in it, or about the
/// forest rather than about any tree in it.
///
/// The first three say what was in a tree the tracker answered for. A tracker
/// that did not answer is a property of the tree instead, carried on the
/// header, because a child line explaining why a tree has no children is
/// backwards.
///
/// Drawn under the header whether the tree is folded or not: folding is where
/// a finding is easiest to lose, and losing one is the silent partial answer
/// this tool exists to avoid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Note {
    Dangling(usize),
    Unreachable(usize),
    Truncated(usize),
    /// Every tracker answered and none of them had a root to draw, so the
    /// forest is empty. Under no tree, because there is none: it is the only
    /// line on the screen.
    NoRoots,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Group {
    pub kind: GroupKind,
    pub count: usize,
    /// How many of the things this group holds carry findings the screen is
    /// not drawing, because the group holds them rather than showing them.
    ///
    /// Only a hidden tree has any: the filter took its dangling, unreachable
    /// and truncated counts out of the forest with it, and that choice should
    /// hold — but a group that says only how many trees it hides reads like
    /// "nothing to see" when some of them are broken.
    pub with_findings: usize,
}

/// The groups below the trees, in the order they are drawn: the projects with
/// nothing to show first, what the filter chose to hide last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GroupKind {
    FailedProjects,
    Unconfigured,
    Conflicts,
    HiddenTrees,
    Unattributed,
}

impl GroupKind {
    pub const ALL: [GroupKind; 5] = [
        GroupKind::FailedProjects,
        GroupKind::Unconfigured,
        GroupKind::Conflicts,
        GroupKind::HiddenTrees,
        GroupKind::Unattributed,
    ];

    /// Whether what a group holds is live, which is what rests it open. A
    /// count is not a view: a shut group over live panes says they exist and
    /// nothing about which they are or what is on them. What collection and
    /// the filter did is a report about the reading rather than work in
    /// flight, and rests as the report it is.
    pub(crate) fn live(self) -> bool {
        match self {
            GroupKind::Unconfigured | GroupKind::Conflicts | GroupKind::Unattributed => true,
            GroupKind::FailedProjects | GroupKind::HiddenTrees => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Failed(FailedProject),
    Conflict(Conflict),
    Hidden(HiddenTree),
    Loose(LoosePane),
    Unconfigured(UnconfiguredPane),
}

pub(crate) fn root_key(tree: &Tree) -> BeadKey {
    BeadKey {
        project: tree.project.clone(),
        id: tree.root.clone(),
    }
}

/// A header, a group and a shut node say so; an open node says it by drawing
/// its children, and spending a marker on it would only cost the row width.
pub(crate) fn marker(open: bool) -> &'static str {
    if open {
        OPEN
    } else {
        SHUT
    }
}

/// What stands where a marker would, on a line with no fold to draw one for.
/// The column is held so the line starts where every other one of its kind
/// does.
pub(crate) const NO_FOLD: &str = "  ";

/// Where a line sits, in four columns a level of depth. A line resting shut
/// says so inside its own elbow, so the fold state costs no width and every
/// line at a depth starts in the same column.
pub(crate) fn prefix(trunk: &[bool], last: bool, shut: bool) -> String {
    let mut drawn = String::from(INDENT);
    for more in trunk {
        drawn.push_str(if *more { TRUNK } else { GAP });
    }
    drawn.push_str(match (last, shut) {
        (false, false) => BRANCH,
        (false, true) => BRANCH_SHUT,
        (true, false) => LAST,
        (true, true) => LAST_SHUT,
    });
    drawn
}

pub(crate) fn notes_of(tree: &Tree) -> Vec<Note> {
    let mut notes = Vec::new();
    if !tree.dangling.is_empty() {
        notes.push(Note::Dangling(tree.dangling.len()));
    }
    if !tree.unreachable.is_empty() {
        notes.push(Note::Unreachable(tree.unreachable.len()));
    }
    let truncated = tree.nodes.iter().filter(|node| node.truncated).count();
    if truncated > 0 {
        notes.push(Note::Truncated(truncated));
    }
    notes
}

/// Which node is whose child. The model hands over one flat list in render
/// order with an explicit depth, so a node's parent is the last one shallower
/// than it.
pub(crate) fn children_of(nodes: &[Node]) -> Vec<Vec<usize>> {
    let mut children = vec![Vec::new(); nodes.len()];
    let mut ancestors: Vec<usize> = Vec::new();
    for (at, node) in nodes.iter().enumerate() {
        ancestors.truncate(node.depth as usize);
        if let Some(parent) = ancestors.last() {
            children[*parent].push(at);
        }
        ancestors.push(at);
    }
    children
}

/// Whether nothing is happening on this bead: nobody working it, and nothing
/// wrong with it. Says nothing about its children.
pub(crate) fn quiet(node: &Node) -> bool {
    node.agent.is_none() && node.anomalies.is_empty()
}

/// The nodes strictly beneath `at`, in no order worth relying on.
pub(crate) fn beneath(children: &[Vec<usize>], at: usize) -> Vec<usize> {
    let mut found = Vec::new();
    let mut walking = children[at].clone();
    while let Some(node) = walking.pop() {
        found.push(node);
        walking.extend(children[node].iter().copied());
    }
    found
}

/// Whether the line at `at` rests open: whether anything beneath it is work
/// a reader needs on the first screen.
///
/// This is the whole of the fold default. A line rests open exactly when it
/// stands on the spine to such work, so the first screen is that work and the
/// path to it and nothing else.
pub(crate) fn opens_a_fold(tree: &Tree, children: &[Vec<usize>], at: usize) -> bool {
    live_beneath(tree, children, at) || ready_beneath(tree, children, at)
}

/// Whether any bead beneath `at` carries live work: an agent on it, or an
/// anomaly against it.
///
/// No fold `bdi` chose for itself has ever closed over an agent or an
/// anomaly, and this is what holds that.
fn live_beneath(tree: &Tree, children: &[Vec<usize>], at: usize) -> bool {
    beneath(children, at)
        .into_iter()
        .any(|node| !quiet(&tree.nodes[node]))
}

/// Whether any bead beneath `at` is one `bd` would start today.
///
/// Readiness is `bd`'s answer and not a status test: open, blocked and
/// deferred beads are all unfinished, and only `bd` knows which of them has
/// every dependency behind it. Work it will not start is still unfinished
/// work a reader is not looking for, so it earns no fold.
fn ready_beneath(tree: &Tree, children: &[Vec<usize>], at: usize) -> bool {
    beneath(children, at)
        .into_iter()
        .any(|node| tree.nodes[node].ready)
}

/// How many beads beneath `at` are not closed.
///
/// The mirror of `live_beneath`, which asks whether anyone is on the work
/// rather than whether the work is done. `bdi` walks dependents, so a bead's
/// children are the work closing it unblocked and a closed bead over open
/// ones is the ordinary shape of this tree — but with nobody on any of them
/// the branch rests shut, and the row above it says done.
pub(crate) fn unfinished_beneath(tree: &Tree, children: &[Vec<usize>], at: usize) -> usize {
    beneath(children, at)
        .into_iter()
        .filter(|node| !tree.nodes[*node].status.is_closed())
        .count()
}

/// Whether the branch at `at` is finished: every bead in it closed, no agent
/// anywhere in it, no anomaly anywhere in it.
///
/// Asked of the whole branch rather than of its top bead, because that is the
/// set every use of the answer stands for. A bead can be closed and unmanned
/// and still hold a working agent three levels down, and the two mechanisms
/// this feeds — a branch drawn as one finished line, a run drawn as a count —
/// each hide everything beneath it.
fn finished(tree: &Tree, children: &[Vec<usize>], at: usize) -> bool {
    let node = &tree.nodes[at];
    node.status.is_closed()
        && quiet(node)
        && children[at]
            .iter()
            .all(|kid| finished(tree, children, *kid))
}

/// A node's children split into the ones drawn and the run that is not.
///
/// A finished sibling collapses into the run; one holding an agent or an
/// anomaly at any depth does not, because eliding it would hide live work
/// behind a line saying there is none. A run of one is drawn: `… 1 more`
/// costs a line and saves none.
pub(crate) fn split(tree: &Tree, children: &[Vec<usize>], at: usize) -> (Vec<usize>, Vec<usize>) {
    let done: Vec<usize> = children[at]
        .iter()
        .copied()
        .filter(|kid| finished(tree, children, *kid))
        .collect();

    if done.len() < MANY {
        return (children[at].clone(), Vec::new());
    }

    let drawn = children[at]
        .iter()
        .copied()
        .filter(|kid| !done.contains(kid))
        .collect();
    (drawn, done)
}

/// How far along the subtree at `at` is, where it is more than the one bead.
///
/// A leaf gets nothing: it stands for itself alone, and a fraction over one
/// bead would only say again what its glyph says. Everything else is counted
/// with its own bead among the total, which is the rule a root's counts
/// already follow.
pub(crate) fn progress_of(tree: &Tree, children: &[Vec<usize>], at: usize) -> Option<Progress> {
    if children[at].is_empty() {
        return None;
    }

    let mut counted = Progress {
        closed: 0,
        total: 0,
    };
    let mut walking = vec![at];
    while let Some(node) = walking.pop() {
        counted.total += 1;
        if tree.nodes[node].status.is_closed() {
            counted.closed += 1;
        }
        walking.extend(children[node].iter().copied());
    }

    Some(counted)
}

/// What a run stands for: its own beads and everything beneath them.
///
/// Opening it draws those beads and leaves their descendants to the same rules,
/// which for a quiet closed run of their own is another count one level down.
/// Nothing goes missing either way, so the number holds at every depth.
pub(crate) fn run_size(children: &[Vec<usize>], members: &[usize]) -> usize {
    members.iter().map(|kid| subtree_size(children, *kid)).sum()
}

fn subtree_size(children: &[Vec<usize>], at: usize) -> usize {
    1 + children[at]
        .iter()
        .map(|kid| subtree_size(children, *kid))
        .sum::<usize>()
}
