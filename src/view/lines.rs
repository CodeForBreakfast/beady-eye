//! The lines the forest is made of, and the shape of the tree behind them.
//!
//! Every question here is asked of a `Tree` and its nodes alone: which node is
//! whose child, what rests open, what a run stands for, how far along a branch
//! is. None of it knows the fold state, the selection, or which screen it is
//! drawn on, which is what lets the state machine next door be about nothing
//! else.

use std::collections::BTreeSet;

use crate::model::join::{BeadKey, Conflict};
use crate::model::snapshot::{
    Counts, FailedProject, HiddenTree, LoosePane, Node, TrackerState, Tree, UnconfiguredPane,
};
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
    /// A project: the line its roots hang under, and what collapses them.
    Project(ProjectLine),
    Bead(Row),
    /// A root whose tree would not read. It has no nodes, so it has no row —
    /// and without a line of its own the root would leave the screen, which
    /// is the one way a tree can be lost silently.
    Unread(Unread),
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

/// A project's own line: what it is, how much of it there is, and the panes
/// `bdi` could recover for it where a root would not read.
///
/// It holds the project's own facts rather than its trees: a line is compared
/// whole on every keystroke, and none of a tree's nodes are drawn here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectLine {
    pub project: String,
    /// Every bead in the project's drawn trees, counted once. A bead standing
    /// in several of them is still one bead, which is the rule a tree's own
    /// counts already keep.
    pub counts: Counts,
    /// Whether every root of the project answered the last time it was read.
    ///
    /// One answer for a project whose roots can disagree, resolved to the
    /// worse of them: the rows on the screen are short of a refused root's,
    /// and the mark beside the name is the only thing that says so where the
    /// project is folded shut over its roots.
    pub every_root_read: bool,
    /// What could still be found out about a project one of whose roots would
    /// not read. Absent where every root read: there is nothing to recover,
    /// and saying so on every healthy project would bury the ones where it
    /// matters.
    pub recovery: Option<Recovery>,
}

/// The live panes found working in a project no bead could be read to
/// attribute them to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recovery {
    pub panes: Vec<LoosePane>,
    /// Whether `panes` is all of them. A pane working under no configured
    /// project could belong here and cannot be told, so one of those anywhere
    /// leaves every recovery partial.
    pub complete: bool,
}

/// A root `bdi` was told about and drew no row for, and what its tracker
/// said. The tracker is carried rather than the failure alone so that a root
/// missing for a reason nobody has named is still a root on the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unread {
    pub root: String,
    pub tracker: TrackerState,
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
    Cycle(usize),
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
    /// Only a hidden tree has any: the filter took its dangling, looping
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
    if !tree.cycles.is_empty() {
        notes.push(Note::Cycle(tree.cycles.len()));
    }
    // A bead the tracker stopped at is one bead however many ways down the
    // tree draws it, and the note says beads.
    let truncated: BTreeSet<&str> = tree
        .nodes
        .iter()
        .filter(|node| node.truncated)
        .map(|node| node.id.as_str())
        .collect();
    if !truncated.is_empty() {
        notes.push(Note::Truncated(truncated.len()));
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

/// The beads strictly beneath `at`, one row each.
///
/// A bead reachable more than one way down is drawn once for each, and every
/// question asked of what a branch holds is a question about work rather than
/// about rows: a blocker two of its descendants share is one piece of work
/// however many times it is drawn.
fn beads_beneath(tree: &Tree, children: &[Vec<usize>], at: usize) -> Vec<usize> {
    let mut seen = BTreeSet::new();
    beneath(children, at)
        .into_iter()
        .filter(|node| seen.insert(tree.nodes[*node].id.as_str()))
        .collect()
}

/// Whether the line at `at` is the first this tree draws of its bead.
///
/// A bead reached more than one way down gets a line for each way, and the
/// first of them is the one that stands for the work. Asked of the model's
/// render order rather than of the lines already drawn, so a fold the reader
/// opens elsewhere cannot move which line that is.
pub(crate) fn first_copy(tree: &Tree, at: usize) -> bool {
    let id = &tree.nodes[at].id;
    !tree.nodes[..at].iter().any(|node| node.id == *id)
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

/// What the beads beneath `at` add up to: how many there are, how many are
/// finished, who is on them and how many want looking at.
///
/// Strictly beneath, because every use of this is a line saying what it is
/// shut over rather than what it is. The bead asking is on the screen with
/// its own glyph, its own agent and its own warning already on it.
///
/// Counted as work rather than as rows, like every other statistic here: a
/// blocker two of these branches share is one bead, one seat and one warning
/// however many ways down reach it.
pub(crate) fn counts_beneath(tree: &Tree, children: &[Vec<usize>], at: usize) -> Counts {
    Counts::over(
        beads_beneath(tree, children, at)
            .into_iter()
            .map(|node| &tree.nodes[node]),
    )
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

    let mut counting = vec![at];
    counting.extend(beads_beneath(tree, children, at));
    Some(Progress {
        total: counting.len(),
        closed: counting
            .into_iter()
            .filter(|node| tree.nodes[*node].status.is_closed())
            .count(),
    })
}

/// What a run stands for: its own beads and everything beneath them.
///
/// Opening it draws those beads and leaves their descendants to the same rules,
/// which for a quiet closed run of their own is another count one level down.
/// Nothing goes missing either way, so the number holds at every depth.
///
/// Counted as work rather than as rows, like every other statistic here: a
/// blocker several of the run's branches share is one bead, and the set spans
/// the whole run rather than each member, because the two branches sharing it
/// may be two different members.
pub(crate) fn run_size(tree: &Tree, children: &[Vec<usize>], members: &[usize]) -> usize {
    let mut seen = BTreeSet::new();
    for kid in members {
        for node in std::iter::once(*kid).chain(beneath(children, *kid)) {
            seen.insert(tree.nodes[node].id.as_str());
        }
    }
    seen.len()
}
