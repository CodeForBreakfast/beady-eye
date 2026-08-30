//! The snapshot, flattened into the lines the screen shows.

use std::collections::BTreeMap;

use crate::model::join::{BeadKey, Conflict};
use crate::model::snapshot::{
    self, FailedProject, Filter, HiddenTree, LoosePane, Node, Snapshot, TrackerState, Tree,
    UnconfiguredPane,
};
use crate::model::types::Status;
use crate::view::row::{self, Progress, Row};
use crate::view::{Action, Motion};

/// How far a half-screen motion moves until the renderer says otherwise.
const HALF_SCREEN: usize = 10;

/// How many finished siblings it takes before a count reads better than their
/// names. Under it they are drawn, and a finished branch is one line whatever
/// it holds, so the run saves one row per member past the first.
const MANY: usize = 3;

const OPEN: &str = "▾ ";
const SHUT: &str = "▸ ";
/// A tree's children start under its header's marker, not under its project.
const INDENT: &str = "  ";
const BRANCH: &str = "├── ";
const LAST: &str = "└── ";
const TRUNK: &str = "│   ";
const GAP: &str = "    ";

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
    /// Whether this is the last line drawn at its depth under its parent.
    pub last_child: bool,
    /// Whether this line's fold is open, where it has one at all.
    pub folded: Option<bool>,
    /// The bead this line stands for, where it stands for one: a bead's own
    /// row, and a tree header's root.
    pub bead: Option<BeadKey>,
    pub content: Content,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    /// A root: the line a whole tree collapses to.
    Tree(Header),
    Bead(Row),
    /// A run of closed siblings nobody is working, said as a count.
    Elided {
        count: usize,
        /// The bead whose children the run stands for. A run is not a bead,
        /// so this is not `Line::bead`; it is what the fold is known by.
        under: BeadKey,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub tree: Tree,
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

/// A finding about a tree rather than about any bead in it.
///
/// Each of these says what was in a tree the tracker answered for. A tracker
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Failed(FailedProject),
    Conflict(Conflict),
    Hidden(HiddenTree),
    Loose(LoosePane),
    Unconfigured(UnconfiguredPane),
}

/// What a line that folds is known by, so both the fold and the selection
/// survive a refresh that reorders or drops lines.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Handle {
    Bead(BeadKey),
    /// The run of quiet closed children under one bead. A bead has at most
    /// one run, so the bead names it.
    Elided(BeadKey),
    Group(GroupKind),
}

/// One entry in a parent's sequence of children, before it becomes a line.
/// Notes and beads share the sequence because they share the box-drawing, and
/// a note is a child of the header exactly as a bead is.
enum Child {
    Note(Note),
    Node(usize),
    /// The children of `under` that a run stands for, in render order.
    Elided {
        under: usize,
        members: Vec<usize>,
    },
}

/// One snapshot's lines in render order, with the fold state and the selection
/// that decide which of them are visible and which one is current.
pub struct Forest {
    snapshot: Snapshot,
    /// The folds the user set by hand, over a default that follows the
    /// selection. Keeping the two apart is what lets a fold outlive moving
    /// away from it without freezing every other root at whatever it was.
    folds: BTreeMap<Handle, bool>,
    cursor: Option<Handle>,
    half_screen: usize,
    lines: Vec<Line>,
    selected: usize,
}

/// Flatten a snapshot into its lines.
pub fn flatten(snapshot: &Snapshot) -> Forest {
    let mut forest = Forest {
        snapshot: snapshot.clone(),
        folds: BTreeMap::new(),
        cursor: None,
        half_screen: HALF_SCREEN,
        lines: Vec::new(),
        selected: 0,
    };
    forest.lay_out();
    forest
}

impl Forest {
    /// The visible lines, in render order.
    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    /// Where the selection sits in `lines`.
    pub fn selected_line(&self) -> usize {
        self.selected
    }

    /// The bead the selection sits on, where it sits on one.
    pub fn selected(&self) -> Option<&BeadKey> {
        self.lines.get(self.selected)?.bead.as_ref()
    }

    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    /// How far a half-screen motion moves. The renderer is the only thing that
    /// knows the viewport's height, so it says.
    pub fn set_half_screen(&mut self, rows: usize) {
        self.half_screen = rows.max(1);
    }

    /// Take a freshly collected snapshot, keeping the folds and the selection.
    pub fn refresh(&mut self, snapshot: &Snapshot) {
        // Only the snapshot the cursor was found in knows what stood above
        // it, so where the new one has dropped the bead the cursor falls to
        // the nearest of its forebears that survived.
        let ancestry = self.ancestry();
        self.snapshot = snapshot.clone();
        self.cursor = ancestry.into_iter().find(|handle| self.present(handle));
        self.lay_out();
    }

    /// What the cursor is on, then everything above it in its tree, nearest
    /// first.
    fn ancestry(&self) -> Vec<Handle> {
        let mut chain: Vec<Handle> = self.cursor.iter().cloned().collect();
        let key = match &self.cursor {
            Some(Handle::Bead(key)) => key.clone(),
            // A run is only ever seen from the bead it hangs under, so that
            // bead is the first forebear a lost run falls back to.
            Some(Handle::Elided(key)) => {
                chain.push(Handle::Bead(key.clone()));
                key.clone()
            }
            _ => return chain,
        };

        for tree in self
            .snapshot
            .trees
            .iter()
            .filter(|t| t.project == key.project)
        {
            let Some(at) = tree.nodes.iter().position(|node| node.id == key.id) else {
                continue;
            };
            let mut above = tree.nodes[at].depth;
            for node in tree.nodes[..at].iter().rev() {
                if node.depth < above {
                    above = node.depth;
                    chain.push(Handle::Bead(BeadKey {
                        project: tree.project.clone(),
                        id: node.id.clone(),
                    }));
                }
            }
            break;
        }
        chain
    }

    /// Apply one action, reporting whether it changed anything.
    ///
    /// Focusing a pane, showing the key bindings, re-collecting and quitting
    /// are the loop's to do, and none of them changes what is on screen here.
    pub fn apply(&mut self, action: Action) -> bool {
        let was = (self.lines.clone(), self.selected);
        match action {
            Action::Move(motion) => self.move_to(motion),
            Action::CollapseOrParent => self.collapse_or_parent(),
            Action::ExpandOrChild => self.expand_or_child(),
            Action::ToggleFold => self.toggle_fold(),
            Action::ToggleFilter => self.toggle_filter(),
            Action::Focus | Action::ShowBindings | Action::Refresh | Action::Quit => return false,
        }
        self.lay_out();
        (self.lines.clone(), self.selected) != was
    }

    fn toggle_filter(&mut self) {
        let next = match self.snapshot.filter {
            Filter::LiveAgents => Filter::All,
            Filter::All => Filter::LiveAgents,
        };
        self.snapshot = snapshot::refilter(&self.snapshot, next);
    }

    fn toggle_fold(&mut self) {
        if let (Some(open), Some(handle)) =
            (self.fold_at(self.selected), self.handle_at(self.selected))
        {
            self.folds.insert(handle, !open);
        }
    }

    /// `h`: shut an open node, and step out of one already shut.
    fn collapse_or_parent(&mut self) {
        match (self.fold_at(self.selected), self.handle_at(self.selected)) {
            (Some(true), Some(handle)) => {
                self.folds.insert(handle, false);
            }
            _ => self.step_to(self.parent_of(self.selected)),
        }
    }

    /// `l`: open a shut node, and step into one already open.
    fn expand_or_child(&mut self) {
        match (self.fold_at(self.selected), self.handle_at(self.selected)) {
            (Some(false), Some(handle)) => {
                self.folds.insert(handle, true);
            }
            _ => self.step_to(self.first_child_of(self.selected)),
        }
    }

    fn move_to(&mut self, motion: Motion) {
        let last = self.lines.len().saturating_sub(1);
        let target = match motion {
            Motion::PreviousRow => self.step(self.selected, false),
            Motion::NextRow => self.step(self.selected, true),
            Motion::FirstRow => self.scan(0, true),
            Motion::LastRow => self.scan(last, false),
            Motion::HalfScreenUp => {
                let at = self.selected.saturating_sub(self.half_screen);
                self.scan(at, false).or_else(|| self.scan(at, true))
            }
            Motion::HalfScreenDown => {
                let at = (self.selected + self.half_screen).min(last);
                self.scan(at, true).or_else(|| self.scan(at, false))
            }
        };
        self.step_to(target);
    }

    fn step_to(&mut self, target: Option<usize>) {
        if let Some(target) = target {
            self.selected = target;
            self.cursor = self.handle_at(target);
        }
    }

    /// The first line at or beyond `from` that the selection can sit on.
    fn scan(&self, from: usize, forward: bool) -> Option<usize> {
        let range: Vec<usize> = if forward {
            (from..self.lines.len()).collect()
        } else {
            (0..=from.min(self.lines.len().checked_sub(1)?))
                .rev()
                .collect()
        };
        range.into_iter().find(|i| selectable(&self.lines[*i]))
    }

    /// The next line the selection can sit on, past `from`.
    fn step(&self, from: usize, forward: bool) -> Option<usize> {
        if forward {
            self.scan(from + 1, true)
        } else {
            self.scan(from.checked_sub(1)?, false)
        }
    }

    fn parent_of(&self, at: usize) -> Option<usize> {
        let depth = self.lines.get(at)?.depth;
        if depth == 0 {
            return None;
        }
        self.lines[..at]
            .iter()
            .rposition(|line| line.depth < depth && selectable(line))
    }

    fn first_child_of(&self, at: usize) -> Option<usize> {
        let depth = self.lines.get(at)?.depth;
        self.lines
            .iter()
            .enumerate()
            .skip(at + 1)
            .take_while(|(_, line)| line.depth > depth)
            .find(|(_, line)| line.depth == depth + 1 && selectable(line))
            .map(|(at, _)| at)
    }

    fn fold_at(&self, at: usize) -> Option<bool> {
        self.lines.get(at)?.folded
    }

    fn handle_at(&self, at: usize) -> Option<Handle> {
        let line = self.lines.get(at)?;
        match &line.content {
            Content::Group(group) => Some(Handle::Group(group.kind)),
            Content::Elided { under, .. } => Some(Handle::Elided(under.clone())),
            _ => line.bead.clone().map(Handle::Bead),
        }
    }

    /// Redraw, and put the selection back on whatever it was holding.
    fn lay_out(&mut self) {
        self.settle_cursor();
        self.lines = self.draw();
        if self.find_cursor().is_none() {
            // The line the cursor named is no longer drawn — an ancestor was
            // folded over it, or the tracker stopped reporting it. Take the
            // nearest line that is, and redraw: whichever tree that lands in
            // is the one the default now expands.
            self.cursor = self
                .scan(self.selected, false)
                .or_else(|| self.scan(self.selected, true))
                .and_then(|at| self.handle_at(at));
            self.lines = self.draw();
        }
        self.selected = self.find_cursor().unwrap_or(0);
    }

    fn settle_cursor(&mut self) {
        if self.cursor.as_ref().is_some_and(|held| self.present(held)) {
            return;
        }
        self.cursor = self.first_handle();
    }

    fn find_cursor(&self) -> Option<usize> {
        let cursor = self.cursor.as_ref()?;
        (0..self.lines.len()).find(|at| self.handle_at(*at).as_ref() == Some(cursor))
    }

    fn first_handle(&self) -> Option<Handle> {
        if let Some(tree) = self.snapshot.trees.first() {
            return Some(Handle::Bead(root_key(tree)));
        }
        let (_, loose) = self.recovery();
        GroupKind::ALL
            .iter()
            .find(|kind| !self.group_items(**kind, &loose).is_empty())
            .copied()
            .map(Handle::Group)
    }

    /// Whether the snapshot still holds what a handle names.
    fn present(&self, handle: &Handle) -> bool {
        match handle {
            Handle::Bead(key) | Handle::Elided(key) => self.snapshot.trees.iter().any(|tree| {
                tree.project == key.project
                    && (tree.root == key.id || tree.nodes.iter().any(|node| node.id == key.id))
            }),
            Handle::Group(kind) => {
                let (_, loose) = self.recovery();
                !self.group_items(*kind, &loose).is_empty()
            }
        }
    }

    /// Whether a fold is open: what the user set it to, or how it rests when
    /// they have not touched it. Only the caller knows the tree a line came
    /// from, so it says where the line rests rather than being asked to
    /// re-derive it here.
    fn expanded(&self, handle: &Handle, resting: bool) -> bool {
        self.folds.get(handle).copied().unwrap_or(resting)
    }

    fn holds_cursor(&self, root: &BeadKey) -> bool {
        // A run hangs under a bead, so the cursor on one is in that bead's
        // tree exactly as a cursor on the bead itself is.
        let Some(Handle::Bead(cursor) | Handle::Elided(cursor)) = &self.cursor else {
            return false;
        };
        cursor.project == root.project
            && self
                .snapshot
                .trees
                .iter()
                .filter(|tree| tree.project == root.project && tree.root == root.id)
                .any(|tree| {
                    tree.root == cursor.id || tree.nodes.iter().any(|node| node.id == cursor.id)
                })
    }

    /// Give each unreadable tree the live panes working in its project, and
    /// keep the rest loose. A pane is one or the other and never both, so what
    /// the headers show and what the group counts still add up to every pane.
    fn recovery(&self) -> (Vec<Vec<LoosePane>>, Vec<LoosePane>) {
        let mut recovered = vec![Vec::new(); self.snapshot.trees.len()];
        let mut loose = Vec::new();
        for pane in &self.snapshot.unattributed {
            let home =
                self.snapshot.trees.iter().position(|tree| {
                    tree.tracker != TrackerState::Ok && tree.project == pane.project
                });
            match home {
                Some(tree) => recovered[tree].push(pane.clone()),
                None => loose.push(pane.clone()),
            }
        }
        (recovered, loose)
    }

    fn group_items(&self, kind: GroupKind, loose: &[LoosePane]) -> Vec<Item> {
        match kind {
            GroupKind::FailedProjects => self
                .snapshot
                .failed_projects
                .iter()
                .cloned()
                .map(Item::Failed)
                .collect(),
            GroupKind::Conflicts => self
                .snapshot
                .conflicts
                .iter()
                .cloned()
                .map(Item::Conflict)
                .collect(),
            GroupKind::HiddenTrees => self
                .snapshot
                .hidden_trees
                .iter()
                .cloned()
                .map(Item::Hidden)
                .collect(),
            GroupKind::Unattributed => loose.iter().cloned().map(Item::Loose).collect(),
            GroupKind::Unconfigured => self
                .snapshot
                .unconfigured
                .iter()
                .cloned()
                .map(Item::Unconfigured)
                .collect(),
        }
    }

    fn draw(&self) -> Vec<Line> {
        let (recovered, loose) = self.recovery();
        let mut lines = Vec::new();
        for (tree, panes) in self.snapshot.trees.iter().zip(recovered) {
            self.draw_tree(tree, panes, &mut lines);
        }
        self.draw_groups(&loose, &mut lines);
        lines
    }

    fn draw_tree(&self, tree: &Tree, panes: Vec<LoosePane>, lines: &mut Vec<Line>) {
        let root = root_key(tree);
        // A root shows its work while the selection is in it and collapses to
        // its header when it is not.
        let open = self.expanded(&Handle::Bead(root.clone()), self.holds_cursor(&root));
        let complete = tree.tracker == TrackerState::Ok || self.snapshot.unconfigured.is_empty();

        lines.push(Line {
            prefix: marker(open).to_string(),
            depth: 0,
            last_child: false,
            folded: Some(open),
            bead: Some(root),
            content: Content::Tree(Header {
                status: tree.nodes.first().map(|root| root.status.clone()),
                tree: tree.clone(),
                panes,
                panes_complete: complete,
            }),
        });

        let children = children_of(&tree.nodes);
        let mut entries: Vec<Child> = notes_of(tree).into_iter().map(Child::Note).collect();
        if open && !tree.nodes.is_empty() {
            entries.extend(self.children_entries(tree, &children, 0));
        }
        self.draw_children(tree, &children, entries, &mut Vec::new(), lines);
    }

    /// A node's children as they are drawn: the ones worth a line each, then
    /// one line for the run that is not.
    fn children_entries(&self, tree: &Tree, children: &[Vec<usize>], at: usize) -> Vec<Child> {
        let (drawn, elided) = split(tree, children, at);
        let mut entries: Vec<Child> = drawn.into_iter().map(Child::Node).collect();
        if !elided.is_empty() {
            entries.push(Child::Elided {
                under: at,
                members: elided,
            });
        }
        entries
    }

    fn draw_children(
        &self,
        tree: &Tree,
        children: &[Vec<usize>],
        entries: Vec<Child>,
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
                    last_child: last,
                    folded: None,
                    bead: None,
                    content: Content::Note(note),
                }),
                Child::Elided { under, members } => {
                    let key = BeadKey {
                        project: tree.project.clone(),
                        id: tree.nodes[under].id.clone(),
                    };
                    // A run rests as the count it was drawn to be.
                    let open = self.expanded(&Handle::Elided(key.clone()), false);
                    lines.push(Line {
                        prefix: prefix(trunk, last, !open),
                        depth,
                        last_child: last,
                        folded: Some(open),
                        bead: None,
                        content: Content::Elided {
                            count: run_size(children, &members),
                            under: key,
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
                        self.draw_children(tree, children, entries, trunk, lines);
                        trunk.pop();
                    }
                }
                Child::Node(at) => {
                    let node = &tree.nodes[at];
                    let key = BeadKey {
                        project: tree.project.clone(),
                        id: node.id.clone(),
                    };
                    let kids = self.children_entries(tree, children, at);
                    // A finished branch rests shut, said in one line by its
                    // own glyph, its whole fraction and the marker. Anything
                    // still being worked rests open, down to the work.
                    let open = !kids.is_empty()
                        && self.expanded(&Handle::Bead(key.clone()), !finished(tree, children, at));
                    lines.push(Line {
                        prefix: prefix(trunk, last, !kids.is_empty() && !open),
                        depth,
                        last_child: last,
                        folded: (!kids.is_empty()).then_some(open),
                        bead: Some(key),
                        content: Content::Bead(row::cells(
                            node,
                            &tree.root,
                            progress_of(tree, children, at),
                        )),
                    });
                    if open {
                        trunk.push(!last);
                        self.draw_children(tree, children, kids, trunk, lines);
                        trunk.pop();
                    }
                }
            }
        }
    }

    /// The hidden trees whose findings went with them. `collected` still holds
    /// every tree that was read, shown or hidden, so what the filter took out
    /// of the forest is still countable here.
    fn with_findings(&self, items: &[Item]) -> usize {
        items
            .iter()
            .filter(|item| match item {
                Item::Hidden(hidden) => self
                    .snapshot
                    .collected
                    .iter()
                    .filter(|tree| tree.project == hidden.project && tree.root == hidden.root)
                    .any(|tree| !notes_of(tree).is_empty()),
                _ => false,
            })
            .count()
    }

    fn draw_groups(&self, loose: &[LoosePane], lines: &mut Vec<Line>) {
        for kind in GroupKind::ALL {
            let items = self.group_items(kind, loose);
            if items.is_empty() {
                continue;
            }
            let open = self.expanded(&Handle::Group(kind), false);
            lines.push(Line {
                prefix: marker(open).to_string(),
                depth: 0,
                last_child: false,
                folded: Some(open),
                bead: None,
                content: Content::Group(Group {
                    kind,
                    count: items.len(),
                    with_findings: self.with_findings(&items),
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
                    last_child: last,
                    folded: None,
                    bead: None,
                    content: Content::Item(item),
                });
            }
        }
    }
}

fn root_key(tree: &Tree) -> BeadKey {
    BeadKey {
        project: tree.project.clone(),
        id: tree.root.clone(),
    }
}

/// A header, a group and a shut node say so; an open node says it by drawing
/// its children, and spending a marker on it would only cost the row width.
fn marker(open: bool) -> &'static str {
    if open {
        OPEN
    } else {
        SHUT
    }
}

fn selectable(line: &Line) -> bool {
    matches!(
        line.content,
        Content::Tree(_) | Content::Bead(_) | Content::Elided { .. } | Content::Group(_)
    )
}

fn prefix(trunk: &[bool], last: bool, shut: bool) -> String {
    let mut drawn = String::from(INDENT);
    for more in trunk {
        drawn.push_str(if *more { TRUNK } else { GAP });
    }
    drawn.push_str(if last { LAST } else { BRANCH });
    if shut {
        drawn.push_str(SHUT);
    }
    drawn
}

fn notes_of(tree: &Tree) -> Vec<Note> {
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
fn children_of(nodes: &[Node]) -> Vec<Vec<usize>> {
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
        && node.agent.is_none()
        && node.anomalies.is_empty()
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
fn split(tree: &Tree, children: &[Vec<usize>], at: usize) -> (Vec<usize>, Vec<usize>) {
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
fn progress_of(tree: &Tree, children: &[Vec<usize>], at: usize) -> Option<Progress> {
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
fn run_size(children: &[Vec<usize>], members: &[usize]) -> usize {
    members.iter().map(|kid| subtree_size(children, *kid)).sum()
}

fn subtree_size(children: &[Vec<usize>], at: usize) -> usize {
    1 + children[at]
        .iter()
        .map(|kid| subtree_size(children, *kid))
        .sum::<usize>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::bd::parse_dep_tree;
    use crate::collect::herdr::{parse_agent_list, Pane};
    use crate::config::Config;
    use crate::model::join::{self, Joined, ProjectRows};
    use crate::model::snapshot::{build_tree, Collected, HerdrState, Readiness, TrackerFailure};
    use crate::model::tree::{assemble, Assembled};
    use chrono::{DateTime, Utc};
    use pretty_assertions::assert_eq;

    /// Orbital's tree as bd writes it. `orb-7.7` declares a parent no row
    /// holds, so it is re-parented onto the root; `orb-7.1.2` is a node bd
    /// stopped at; `orb-7.4` is closed with a pane still on it, and the other
    /// three closed siblings are finished.
    const ORBITAL: &str = r#"[
      {"id":"orb-7","title":"lift the ground station","status":"in_progress","parent_id":"",
       "priority":1,"issue_type":"epic","updated_at":"2026-08-29T12:00:00Z",
       "metadata":{"agent_pane":"w:p1"}},
      {"id":"orb-7.1","title":"re-point the dish","status":"open","parent_id":"orb-7",
       "priority":2,"issue_type":"task"},
      {"id":"orb-7.1.1","title":"true the mount","status":"open","parent_id":"orb-7.1",
       "priority":2,"issue_type":"task"},
      {"id":"orb-7.1.2","title":"seal the feed horn","status":"open","parent_id":"orb-7.1",
       "priority":3,"issue_type":"task","truncated":true},
      {"id":"orb-7.2","title":"survey the mast","status":"closed","parent_id":"orb-7",
       "priority":2,"issue_type":"task","closed_at":"2026-08-28T09:00:00Z"},
      {"id":"orb-7.3","title":"pour the pad","status":"closed","parent_id":"orb-7",
       "priority":2,"issue_type":"task","closed_at":"2026-08-27T09:00:00Z"},
      {"id":"orb-7.4","title":"clear the access road","status":"closed","parent_id":"orb-7",
       "priority":2,"issue_type":"task","closed_at":"2026-08-26T09:00:00Z",
       "metadata":{"agent_pane":"w:p2"}},
      {"id":"orb-7.5","title":"set the guard rail","status":"closed","parent_id":"orb-7",
       "priority":2,"issue_type":"task","closed_at":"2026-08-25T09:00:00Z"},
      {"id":"orb-7.7","title":"log the survey marks","status":"open","parent_id":"orb-6",
       "priority":2,"issue_type":"task"}
    ]"#;

    /// Harbour's tree. Nobody is working in it, so the live-agent filter hides
    /// it.
    const HARBOUR: &str = r#"[
      {"id":"hbr-3","title":"dredge the channel","status":"open","parent_id":"",
       "priority":2,"issue_type":"epic"},
      {"id":"hbr-3.1","title":"survey the silt","status":"open","parent_id":"hbr-3",
       "priority":2,"issue_type":"task"}
    ]"#;

    /// A tree whose run of finished siblings has a finished run of its own, so
    /// an opened run still has something left to count inside it. Three at each
    /// level, which is what it takes to make a run.
    const DEPOT: &str = r#"[
      {"id":"dep-1","title":"re-lay the sidings","status":"in_progress","parent_id":"",
       "priority":1,"issue_type":"epic"},
      {"id":"dep-1.1","title":"grade the bed","status":"open","parent_id":"dep-1",
       "priority":2,"issue_type":"task"},
      {"id":"dep-1.2","title":"lift the old rail","status":"closed","parent_id":"dep-1",
       "priority":2,"issue_type":"task","closed_at":"2026-08-28T09:00:00Z"},
      {"id":"dep-1.2.1","title":"cut the fishplates","status":"closed","parent_id":"dep-1.2",
       "priority":2,"issue_type":"task","closed_at":"2026-08-27T09:00:00Z"},
      {"id":"dep-1.2.2","title":"stack the chairs","status":"closed","parent_id":"dep-1.2",
       "priority":2,"issue_type":"task","closed_at":"2026-08-27T09:00:00Z"},
      {"id":"dep-1.2.3","title":"draw the spikes","status":"closed","parent_id":"dep-1.2",
       "priority":2,"issue_type":"task","closed_at":"2026-08-27T09:00:00Z"},
      {"id":"dep-1.3","title":"clear the ballast","status":"closed","parent_id":"dep-1",
       "priority":2,"issue_type":"task","closed_at":"2026-08-26T09:00:00Z"},
      {"id":"dep-1.4","title":"burn the sleepers","status":"closed","parent_id":"dep-1",
       "priority":2,"issue_type":"task","closed_at":"2026-08-25T09:00:00Z"}
    ]"#;

    /// The shape `bdi-4av` was raised on, with the stale-pane case beside it.
    /// `rly-2.2` and `rly-2.4` are both closed and unmanned, so a rule that
    /// asks its question of the sibling alone sweeps both into the run —
    /// burying a working agent two levels under one of them and a stale-pane
    /// warning under the other. `rly-2.3`, `rly-2.5` and `rly-2.6` are
    /// finished all the way down, and are what a run may honestly hold.
    const RELAY: &str = r#"[
      {"id":"rly-2","title":"re-site the relay","status":"in_progress","parent_id":"",
       "priority":1,"issue_type":"epic"},
      {"id":"rly-2.1","title":"trench the run","status":"open","parent_id":"rly-2",
       "priority":2,"issue_type":"task"},
      {"id":"rly-2.2","title":"strike the old mast","status":"closed","parent_id":"rly-2",
       "priority":2,"issue_type":"task","closed_at":"2026-08-28T09:00:00Z"},
      {"id":"rly-2.2.1","title":"drop the guys","status":"closed","parent_id":"rly-2.2",
       "priority":2,"issue_type":"task","closed_at":"2026-08-27T09:00:00Z"},
      {"id":"rly-2.2.1.1","title":"cut the stays","status":"in_progress","parent_id":"rly-2.2.1",
       "priority":2,"issue_type":"task","updated_at":"2026-08-29T12:00:00Z",
       "metadata":{"agent_pane":"w:p1"}},
      {"id":"rly-2.3","title":"back-fill the pad","status":"closed","parent_id":"rly-2",
       "priority":2,"issue_type":"task","closed_at":"2026-08-26T09:00:00Z"},
      {"id":"rly-2.4","title":"lift the feeder","status":"closed","parent_id":"rly-2",
       "priority":2,"issue_type":"task","closed_at":"2026-08-26T09:00:00Z"},
      {"id":"rly-2.4.1","title":"coil the heliax","status":"closed","parent_id":"rly-2.4",
       "priority":2,"issue_type":"task","closed_at":"2026-08-25T09:00:00Z",
       "metadata":{"agent_pane":"w:p2"}},
      {"id":"rly-2.5","title":"seed the spoil","status":"closed","parent_id":"rly-2",
       "priority":2,"issue_type":"task","closed_at":"2026-08-25T09:00:00Z"},
      {"id":"rly-2.5.1","title":"rake the batter","status":"closed","parent_id":"rly-2.5",
       "priority":2,"issue_type":"task","closed_at":"2026-08-24T09:00:00Z"},
      {"id":"rly-2.6","title":"sign the handover","status":"closed","parent_id":"rly-2",
       "priority":2,"issue_type":"task","closed_at":"2026-08-24T09:00:00Z"}
    ]"#;

    /// `w:p3` and `w:p4` both name `orb-7.1`, so neither holds it; `w:p9` is
    /// working in the project whose tracker refused; `w:pF` is under no
    /// configured project at all.
    const PANES: &str = r#"{"result":{"agents":[
      {"pane_id":"w:p1","cwd":"/srv/work/orbital","agent_status":"working"},
      {"pane_id":"w:p2","cwd":"/srv/work/orbital","agent_status":"idle"},
      {"pane_id":"w:p3","cwd":"/srv/work/orbital","agent_status":"working",
       "display_agent":"orb-7.1"},
      {"pane_id":"w:p4","cwd":"/srv/work/orbital","agent_status":"idle",
       "display_agent":"orb-7.1"},
      {"pane_id":"w:p9","cwd":"/srv/work/ferry","agent_status":"blocked"},
      {"pane_id":"w:pF","cwd":"/srv/spike","agent_status":"idle"}
    ]}}"#;

    fn cfg() -> Config {
        Config::from_toml(
            r#"
[[projects]]
name = "orbital"
path = "/srv/work/orbital"
credential_command = "secret orbital"

[[projects]]
name = "ferry"
path = "/srv/work/ferry"
credential_command = "secret ferry"

[[projects]]
name = "harbour"
path = "/srv/work/harbour"
credential_command = "secret harbour"
"#,
        )
        .expect("the config parses")
    }

    fn now() -> DateTime<Utc> {
        "2026-08-30T12:00:00Z".parse().expect("the instant parses")
    }

    fn assembled(json: &str) -> Assembled {
        assemble(parse_dep_tree(json).expect("the rows parse")).expect("the rows assemble")
    }

    fn panes() -> Vec<Pane> {
        parse_agent_list(PANES).expect("the panes parse")
    }

    /// One working pane and one idle one, both in Orbital's tree. Enough to
    /// staff a fixture without the conflicting and unconfigured panes the
    /// shared snapshot carries to exercise its groups.
    const TWO_PANES: &str = r#"{"result":{"agents":[
      {"pane_id":"w:p1","cwd":"/srv/work/orbital","agent_status":"working"},
      {"pane_id":"w:p2","cwd":"/srv/work/orbital","agent_status":"idle"}
    ]}}"#;

    fn two_panes() -> Vec<Pane> {
        parse_agent_list(TWO_PANES).expect("the panes parse")
    }

    fn joined(orbital: &Assembled, harbour: &Assembled, panes: &[Pane]) -> Joined {
        let cfg = cfg();
        join::resolve(
            &[
                ProjectRows {
                    project: "orbital",
                    rows: &orbital.rows,
                },
                ProjectRows {
                    project: "harbour",
                    rows: &harbour.rows,
                },
            ],
            panes,
            &cfg.projects,
            &cfg.join,
        )
    }

    fn tree_of(project: &str, json: &str) -> Tree {
        let orbital = assembled(ORBITAL);
        let harbour = assembled(HARBOUR);
        let panes = panes();
        let joined = joined(&orbital, &harbour, &panes);
        build_tree(
            project,
            &assembled(json),
            &joined,
            &Readiness::default(),
            &cfg(),
            now(),
        )
    }

    fn gather(trees: Vec<Tree>, failed: Vec<FailedProject>, filter: Filter) -> Snapshot {
        let orbital = assembled(ORBITAL);
        let harbour = assembled(HARBOUR);
        let panes = panes();
        let joined = joined(&orbital, &harbour, &panes);
        snapshot::build(
            Collected {
                trees,
                failed_projects: failed,
            },
            &panes,
            &joined,
            &cfg(),
            HerdrState::Ok,
            filter,
            now(),
        )
    }

    /// Three roots: one read and staffed, one whose tracker refused, one read
    /// and quiet. Plus a project that failed before its roots were known.
    fn built(filter: Filter) -> Snapshot {
        gather(
            vec![
                tree_of("orbital", ORBITAL),
                Tree::tracker_unreachable("ferry", "fer-2", TrackerFailure::Auth),
                tree_of("harbour", HARBOUR),
            ],
            vec![FailedProject {
                project: "lunar".into(),
                tracker: TrackerFailure::Exec,
            }],
            filter,
        )
    }

    fn snapshot() -> Snapshot {
        built(Filter::LiveAgents)
    }

    /// One line as its prefix plus enough of its content to read the shape.
    fn sketch(forest: &Forest) -> Vec<String> {
        forest
            .lines()
            .iter()
            .map(|line| format!("{}{}", line.prefix, said(&line.content)))
            .collect()
    }

    fn said(content: &Content) -> String {
        match content {
            Content::Tree(header) => format!("{} · {}", header.tree.project, header.tree.root),
            Content::Bead(row) => format!("{} {} {}", row.glyph, row.id, row.title),
            Content::Elided { count, .. } => format!("… {count} more"),
            Content::Note(note) => format!("! {note:?}"),
            Content::Group(group) => format!("[{:?}] {}", group.kind, group.count),
            Content::Item(item) => format!("- {item:?}"),
        }
    }

    /// The drawn row for one bead, found by the whole id its line carries
    /// rather than the abbreviated one it shows.
    fn row_of<'a>(forest: &'a Forest, id: &str) -> &'a Row {
        forest
            .lines()
            .iter()
            .find_map(|line| match (&line.bead, &line.content) {
                (Some(bead), Content::Bead(row)) if bead.id == id => Some(row),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{id} is not drawn"))
    }

    fn key(project: &str, id: &str) -> BeadKey {
        BeadKey {
            project: project.into(),
            id: id.into(),
        }
    }

    fn depot() -> Snapshot {
        gather(vec![tree_of("orbital", DEPOT)], Vec::new(), Filter::All)
    }

    /// One project's tree, joined against its own rows so that a pane the
    /// fixture names lands on the bead that names it. `tree_of` joins every
    /// fixture against Orbital's rows, which is what the shared snapshot
    /// needs and what leaves any other fixture's beads unstaffed.
    fn alone(project: &str, json: &str, panes: &[Pane]) -> Snapshot {
        let rows = assembled(json);
        let cfg = cfg();
        let joined = join::resolve(
            &[ProjectRows {
                project,
                rows: &rows.rows,
            }],
            panes,
            &cfg.projects,
            &cfg.join,
        );
        let tree = build_tree(project, &rows, &joined, &Readiness::default(), &cfg, now());
        snapshot::build(
            Collected {
                trees: vec![tree],
                failed_projects: Vec::new(),
            },
            panes,
            &joined,
            &cfg,
            HerdrState::Ok,
            Filter::All,
            now(),
        )
    }

    /// Put the selection on the first elided run, by moving down to it. It
    /// carries no bead, so `select` cannot reach it.
    fn select_run(forest: &mut Forest) {
        forest.apply(Action::Move(Motion::FirstRow));
        for _ in 0..=forest.lines().len() {
            let line = &forest.lines()[forest.selected_line()];
            if matches!(line.content, Content::Elided { .. }) {
                return;
            }
            forest.apply(Action::Move(Motion::NextRow));
        }
        panic!("no elided run is reachable by moving down");
    }

    fn select(forest: &mut Forest, bead: &BeadKey) {
        forest.apply(Action::Move(Motion::FirstRow));
        for _ in 0..=forest.lines().len() {
            if forest.selected() == Some(bead) {
                return;
            }
            forest.apply(Action::Move(Motion::NextRow));
        }
        panic!("{bead:?} is not reachable by moving down");
    }

    #[test]
    fn a_snapshot_flattens_to_the_lines_the_design_draws() {
        let forest = flatten(&snapshot());

        assert_eq!(
            sketch(&forest),
            vec![
                "▾ orbital · orb-7",
                "  ├── ! Dangling(1)",
                "  ├── ! Truncated(1)",
                "  ├── ○ .1 re-point the dish",
                "  │   ├── ○ .1.1 true the mount",
                "  │   └── ○ .1.2 seal the feed horn",
                "  ├── ○ .7 log the survey marks",
                "  ├── ✓ .4 clear the access road",
                "  └── ▸ … 3 more",
                "▸ ferry · fer-2",
                "▸ [FailedProjects] 1",
                "▸ [Unconfigured] 1",
                "▸ [Conflicts] 1",
                "▸ [HiddenTrees] 1",
                "▸ [Unattributed] 2",
            ]
        );
    }

    /// The selection starts on the first root, which is what expands it.
    #[test]
    fn the_selection_starts_on_the_first_root() {
        let forest = flatten(&snapshot());

        assert_eq!(forest.selected_line(), 0);
        assert_eq!(forest.selected(), Some(&key("orbital", "orb-7")));
    }

    #[test]
    fn a_root_is_expanded_only_while_the_selection_is_inside_it() {
        let mut forest = flatten(&snapshot());
        let opened = forest.lines().len();

        forest.apply(Action::Move(Motion::LastRow));

        assert!(forest.lines().len() < opened, "{:#?}", sketch(&forest));
        assert!(!sketch(&forest).iter().any(|line| line.contains(".1.1")));
    }

    /// Compared on content alone: folding also redraws the header's marker
    /// and the elbow on what is now the last line under it, and neither of
    /// those is a line the fold removed.
    #[test]
    fn folding_a_root_removes_exactly_its_subtree() {
        let mut forest = flatten(&snapshot());
        let before = contents(&forest);

        assert!(forest.apply(Action::ToggleFold));

        let after = contents(&forest);
        let gone: Vec<&String> = before.iter().filter(|said| !after.contains(said)).collect();

        assert_eq!(
            gone,
            vec![
                "○ .1 re-point the dish",
                "○ .1.1 true the mount",
                "○ .1.2 seal the feed horn",
                "○ .7 log the survey marks",
                "✓ .4 clear the access road",
                "… 3 more",
            ]
        );
    }

    fn contents(forest: &Forest) -> Vec<String> {
        forest
            .lines()
            .iter()
            .map(|line| said(&line.content))
            .collect()
    }

    /// Folding is where a finding is easiest to lose, so a folded tree keeps
    /// every one of them.
    #[test]
    fn a_trees_findings_are_drawn_whether_it_is_folded_or_not() {
        let mut forest = flatten(&snapshot());
        forest.apply(Action::ToggleFold);

        assert_eq!(
            sketch(&forest)[..3],
            [
                "▸ orbital · orb-7",
                "  ├── ! Dangling(1)",
                "  └── ! Truncated(1)",
            ]
        );
    }

    #[test]
    fn a_fold_made_by_hand_outlives_moving_away_from_it() {
        let mut forest = flatten(&snapshot());
        forest.apply(Action::ToggleFold);
        forest.apply(Action::Move(Motion::LastRow));
        forest.apply(Action::Move(Motion::FirstRow));

        assert!(
            !sketch(&forest).iter().any(|line| line.contains(".1.1")),
            "{:#?}",
            sketch(&forest)
        );
    }

    #[test]
    fn a_run_of_quiet_closed_siblings_collapses_to_a_count() {
        let forest = flatten(&snapshot());

        assert!(sketch(&forest).contains(&"  └── ▸ … 3 more".to_string()));
    }

    /// The count is the only account the screen gives of the beads it stands
    /// for, so the line has to be reachable to be worth anything.
    #[test]
    fn an_elided_run_can_hold_the_selection() {
        let mut forest = flatten(&snapshot());

        select_run(&mut forest);

        assert_eq!(sketch(&forest)[forest.selected_line()], "  └── ▸ … 3 more");
    }

    #[test]
    fn opening_an_elided_run_draws_the_beads_it_counted() {
        let mut forest = flatten(&snapshot());
        select_run(&mut forest);

        forest.apply(Action::ToggleFold);

        assert_eq!(
            sketch(&forest)[7..],
            [
                "  ├── ✓ .4 clear the access road",
                "  └── … 3 more",
                "      ├── ✓ .2 survey the mast",
                "      ├── ✓ .3 pour the pad",
                "      └── ✓ .5 set the guard rail",
                "▸ ferry · fer-2",
                "▸ [FailedProjects] 1",
                "▸ [Unconfigured] 1",
                "▸ [Conflicts] 1",
                "▸ [HiddenTrees] 1",
                "▸ [Unattributed] 2",
            ]
        );
    }

    #[test]
    fn shutting_an_open_elided_run_puts_the_count_back() {
        let mut forest = flatten(&snapshot());
        let was = sketch(&forest);
        select_run(&mut forest);

        forest.apply(Action::ExpandOrChild);
        forest.apply(Action::CollapseOrParent);

        assert_eq!(sketch(&forest), was);
    }

    /// A line that stands for more than itself says how much of that is done,
    /// which is the question a root's `35/51` answers — asked here one level
    /// down. `orb-7.1` is open and holds two open children, so its subtree is
    /// three beads with none of them closed.
    ///
    /// Counted including the bead's own line, because that is what a root
    /// already does: `snapshot` sets `total` to `nodes.len()`, and the root is
    /// one of those nodes. One rule at every depth.
    #[test]
    fn a_bead_with_children_says_how_much_of_its_own_subtree_is_done() {
        let forest = flatten(&snapshot());

        assert_eq!(
            row_of(&forest, "orb-7.1").progress,
            Some(Progress {
                closed: 0,
                total: 3
            })
        );
    }

    /// Both halves of the fraction have to count. `dep-1.2` is closed with two
    /// closed beads under it, so a subtree that is finished says so — an epic
    /// stuck at `0/3` whatever its children did would be worse than no count.
    #[test]
    fn a_finished_subtree_counts_its_closed_beads_and_not_only_its_size() {
        let tree = tree_of("orbital", DEPOT);
        let children = children_of(&tree.nodes);
        let at = tree
            .nodes
            .iter()
            .position(|node| node.id == "dep-1.2")
            .expect("dep-1.2 is in the tree");

        assert_eq!(
            progress_of(&tree, &children, at),
            Some(Progress {
                closed: 4,
                total: 4
            })
        );
    }

    /// A leaf stands for itself alone, so there is nothing to be part-way
    /// through and a fraction over one bead would only repeat its glyph.
    #[test]
    fn a_bead_with_no_children_has_no_progress_to_report() {
        let forest = flatten(&snapshot());

        assert_eq!(row_of(&forest, "orb-7.1.1").progress, None);
    }

    /// A run is drawn with one status glyph standing for every bead it hides,
    /// which is only honest while a run is closed beads and nothing else.
    /// `dep-1.1` is open beside the two closed siblings that make the run, so
    /// widening the predicate sweeps it in and fails here — rather than
    /// leaving the glyph to say `closed` over a bead that is not.
    #[test]
    fn a_run_holds_closed_beads_and_nothing_else_which_is_what_lets_one_glyph_stand_for_it() {
        let tree = tree_of("orbital", DEPOT);
        let children = children_of(&tree.nodes);

        let mut runs = 0;
        for at in 0..tree.nodes.len() {
            let (_, run) = split(&tree, &children, at);
            runs += usize::from(!run.is_empty());
            for member in run {
                let bead = &tree.nodes[member];
                assert!(
                    bead.status.is_closed(),
                    "{} is in a run and is {:?}",
                    bead.id,
                    bead.status
                );
            }
        }

        assert!(runs > 0, "the fixture built no run to check");
    }

    /// The run rule is a property of the forest, not of a place in it, so it
    /// holds inside an open run as it does everywhere else. Nothing
    /// disappears; it is counted one level down. Two folds to reach it now:
    /// the run, and then the finished branch that rests shut inside it.
    #[test]
    fn an_open_run_elides_again_inside_itself() {
        let mut forest = flatten(&depot());
        select_run(&mut forest);
        forest.apply(Action::ToggleFold);

        select(&mut forest, &key("orbital", "dep-1.2"));
        forest.apply(Action::ToggleFold);

        assert_eq!(
            sketch(&forest)[..7],
            [
                "▾ orbital · dep-1",
                "  ├── ○ .1 grade the bed",
                "  └── … 6 more",
                "      ├── ✓ .2 lift the old rail",
                "      │   └── ▸ … 3 more",
                "      ├── ✓ .3 clear the ballast",
                "      └── ✓ .4 burn the sleepers",
            ]
        );
    }

    /// A run is a fold like any other, so a collection that lands under an
    /// open one leaves it open and leaves the cursor on it.
    #[test]
    fn an_open_elided_run_survives_a_refresh() {
        let mut forest = flatten(&snapshot());
        select_run(&mut forest);
        forest.apply(Action::ToggleFold);

        let reordered = ORBITAL.replace(r#""priority":3"#, r#""priority":1"#);
        forest.refresh(&gather(
            vec![tree_of("orbital", &reordered)],
            Vec::new(),
            Filter::LiveAgents,
        ));

        let drawn = sketch(&forest);
        assert!(
            drawn.contains(&"      └── ✓ .5 set the guard rail".to_string()),
            "{drawn:#?}"
        );
        assert_eq!(drawn[forest.selected_line()], "  └── … 3 more");
    }

    /// A run has no bead of its own, so a line the cursor is holding must not
    /// report one: the loop picks the tail's pane from that field.
    #[test]
    fn a_selected_elided_run_stands_for_no_bead_of_its_own() {
        let mut forest = flatten(&snapshot());

        select_run(&mut forest);

        assert_eq!(forest.selected(), None);
    }

    /// A closed bead with a pane still on it is the stale-pane anomaly, and
    /// eliding it would hide a live agent.
    #[test]
    fn a_closed_bead_with_a_live_agent_is_drawn_rather_than_elided() {
        let forest = flatten(&snapshot());

        assert!(sketch(&forest)
            .iter()
            .any(|line| line.contains("clear the access road")));
    }

    #[test]
    fn a_single_quiet_closed_sibling_is_drawn_rather_than_said_as_a_count() {
        let one_closed = ORBITAL.replace(
            r#"{"id":"orb-7.3","title":"pour the pad","status":"closed"#,
            r#"{"id":"orb-7.3","title":"pour the pad","status":"open"#,
        );
        let snapshot = gather(
            vec![tree_of("orbital", &one_closed)],
            Vec::new(),
            Filter::LiveAgents,
        );

        let drawn = sketch(&flatten(&snapshot));

        assert!(
            drawn.iter().any(|line| line.contains("survey the mast")),
            "{drawn:#?}"
        );
        assert!(
            !drawn.iter().any(|line| line.contains("more")),
            "{drawn:#?}"
        );
    }

    /// The invariant this bead exists to restore: nothing `bdi` folds of its
    /// own accord closes over a live agent or over an anomaly. Asked of the
    /// forest at rest, before any fold is set by hand, because that is the
    /// only state `bdi` chooses for itself.
    #[test]
    fn nothing_the_forest_folds_by_itself_hides_a_live_agent_or_an_anomaly() {
        let mut worth_drawing = 0;
        for json in [ORBITAL, DEPOT, RELAY] {
            let snapshot = alone("orbital", json, &two_panes());
            let forest = flatten(&snapshot);
            let drawn: Vec<&str> = forest
                .lines()
                .iter()
                .filter_map(|line| line.bead.as_ref().map(|key| key.id.as_str()))
                .collect();

            for node in &snapshot.trees[0].nodes {
                if node.agent.is_none() && node.anomalies.is_empty() {
                    continue;
                }
                worth_drawing += 1;
                assert!(
                    drawn.contains(&node.id.as_str()),
                    "{} carries an agent or an anomaly and is not on screen: {:#?}",
                    node.id,
                    sketch(&forest)
                );
            }
        }

        assert!(worth_drawing > 0, "the fixtures staffed nothing to check");
    }

    /// The shape `bdi-4av` was raised on: a closed parent, a closed child, and
    /// a live agent under both of them. The run asked its question of the
    /// child alone, swept it in, and printed a sentence saying nobody was on
    /// the beads it had just hidden the agent among.
    #[test]
    fn a_run_never_closes_over_a_subtree_with_a_live_agent_in_it() {
        let forest = flatten(&alone("orbital", RELAY, &two_panes()));

        assert_eq!(
            sketch(&forest),
            vec![
                "▾ orbital · rly-2",
                "  ├── ○ .1 trench the run",
                "  ├── ✓ .2 strike the old mast",
                "  │   └── ✓ .2.1 drop the guys",
                "  │       └── ● .2.1.1 cut the stays",
                "  ├── ✓ .4 lift the feeder",
                "  │   └── ✓ .4.1 coil the heliax",
                "  └── ▸ … 4 more",
            ]
        );
    }

    /// A run's phrase says nobody is on the beads it counts, so it has to be
    /// true of every bead it counts — not merely of the siblings it names.
    /// The count and the set it describes are checked together, because it was
    /// their disagreement that let the sentence lie.
    #[test]
    fn a_run_counts_exactly_the_beads_its_phrase_is_true_of() {
        let mut runs = 0;
        for json in [ORBITAL, DEPOT, RELAY] {
            let tree = alone("orbital", json, &two_panes()).trees.remove(0);
            let children = children_of(&tree.nodes);

            for at in 0..tree.nodes.len() {
                let (_, run) = split(&tree, &children, at);
                if run.is_empty() {
                    continue;
                }
                runs += 1;

                let mut behind = 0;
                let mut walking = run.clone();
                while let Some(node) = walking.pop() {
                    behind += 1;
                    let bead = &tree.nodes[node];
                    assert!(
                        bead.status.is_closed()
                            && bead.agent.is_none()
                            && bead.anomalies.is_empty(),
                        "{} is behind a run that says nobody is on it",
                        bead.id
                    );
                    walking.extend(children[node].iter().copied());
                }

                assert_eq!(run_size(&children, &run), behind);
            }
        }

        assert!(runs > 0, "the fixtures built no run to check");
    }

    /// A branch that is finished all the way down is one line saying so: the
    /// glyph is its own closed status, the fraction says every bead beneath it
    /// is closed too, and the shut marker says it still holds them.
    #[test]
    fn a_wholly_finished_subtree_rests_as_one_line_that_says_it_is_finished() {
        let forest = flatten(&finished_branches());

        assert_eq!(
            sketch(&forest),
            vec![
                "▾ orbital · dep-1",
                "  ├── ○ .1 grade the bed",
                "  ├── ○ .3 clear the ballast",
                "  ├── ▸ ✓ .2 lift the old rail",
                "  └── ✓ .4 burn the sleepers",
            ]
        );
        assert_eq!(
            row_of(&forest, "dep-1.2").progress,
            Some(Progress {
                closed: 4,
                total: 4
            })
        );
    }

    /// Collapsed, not dropped: it is the existing fold, and opening it draws
    /// what it held under the same rules as anywhere else.
    #[test]
    fn opening_a_finished_subtree_draws_what_it_holds() {
        let mut forest = flatten(&finished_branches());
        select(&mut forest, &key("orbital", "dep-1.2"));

        forest.apply(Action::ToggleFold);

        assert_eq!(
            sketch(&forest),
            vec![
                "▾ orbital · dep-1",
                "  ├── ○ .1 grade the bed",
                "  ├── ○ .3 clear the ballast",
                "  ├── ✓ .2 lift the old rail",
                "  │   └── ▸ … 3 more",
                "  └── ✓ .4 burn the sleepers",
            ]
        );
    }

    /// Depot with one of its closed siblings re-opened, leaving two finished
    /// branches — under the threshold, so each keeps its own name rather than
    /// becoming a share of a count.
    fn finished_branches() -> Snapshot {
        let json = DEPOT.replace(
            r#"{"id":"dep-1.3","title":"clear the ballast","status":"closed"#,
            r#"{"id":"dep-1.3","title":"clear the ballast","status":"open"#,
        );
        alone("orbital", &json, &[])
    }

    #[test]
    fn the_selection_survives_a_refresh_that_reorders_the_nodes() {
        let mut forest = flatten(&snapshot());
        select(&mut forest, &key("orbital", "orb-7.1.2"));
        let was = forest.selected_line();

        let reordered = ORBITAL.replace(r#""priority":3"#, r#""priority":1"#);
        forest.refresh(&gather(
            vec![tree_of("orbital", &reordered)],
            Vec::new(),
            Filter::LiveAgents,
        ));

        assert_eq!(forest.selected(), Some(&key("orbital", "orb-7.1.2")));
        assert_ne!(forest.selected_line(), was);
    }

    /// Closing a bead under the cursor is the ordinary way for one to go, and
    /// the tree it was in is still on screen. The cursor stays in that tree,
    /// on the parent, rather than going back to the top of the forest.
    #[test]
    fn a_refresh_that_drops_the_selected_bead_falls_back_to_its_parent() {
        let mut forest = flatten(&snapshot());
        select(&mut forest, &key("orbital", "orb-7.1.2"));

        let without = ORBITAL.replace(
            r#"{"id":"orb-7.1.2","title":"seal the feed horn","status":"open","parent_id":"orb-7.1",
       "priority":3,"issue_type":"task","truncated":true},"#,
            "",
        );
        forest.refresh(&gather(
            vec![tree_of("orbital", &without)],
            Vec::new(),
            Filter::LiveAgents,
        ));

        assert_eq!(forest.selected(), Some(&key("orbital", "orb-7.1")));
    }

    #[test]
    fn a_refresh_that_drops_the_selected_bead_leaves_the_selection_somewhere_real() {
        let mut forest = flatten(&snapshot());
        select(&mut forest, &key("orbital", "orb-7.1.2"));

        forest.refresh(&gather(
            vec![tree_of("harbour", HARBOUR)],
            Vec::new(),
            Filter::All,
        ));

        assert_eq!(forest.selected(), Some(&key("harbour", "hbr-3")));
        assert!(forest.selected_line() < forest.lines().len());
    }

    /// `a` is a display choice over what was already collected, so what it
    /// renders is the answer the trackers already gave.
    #[test]
    fn dropping_the_filter_re_renders_rather_than_re_collecting() {
        let mut forest = flatten(&snapshot());
        let generated_at = forest.snapshot().generated_at;

        assert!(forest.apply(Action::ToggleFilter));

        assert_eq!(forest.snapshot().filter, Filter::All);
        assert_eq!(forest.snapshot().generated_at, generated_at);
        assert!(sketch(&forest)
            .iter()
            .any(|line| line.contains("harbour · hbr-3")));
        assert!(!sketch(&forest)
            .iter()
            .any(|line| line.contains("[HiddenTrees]")));
    }

    #[test]
    fn collapsing_an_expanded_node_and_then_collapsing_again_moves_to_its_parent() {
        let mut forest = flatten(&snapshot());
        select(&mut forest, &key("orbital", "orb-7.1"));

        assert!(forest.apply(Action::CollapseOrParent));
        assert_eq!(forest.selected(), Some(&key("orbital", "orb-7.1")));
        assert!(!sketch(&forest).iter().any(|line| line.contains(".1.1")));

        assert!(forest.apply(Action::CollapseOrParent));
        assert_eq!(forest.selected(), Some(&key("orbital", "orb-7")));
    }

    #[test]
    fn expanding_a_collapsed_node_and_then_expanding_again_moves_to_its_first_child() {
        let mut forest = flatten(&snapshot());
        select(&mut forest, &key("orbital", "orb-7.1"));
        forest.apply(Action::CollapseOrParent);

        assert!(forest.apply(Action::ExpandOrChild));
        assert_eq!(forest.selected(), Some(&key("orbital", "orb-7.1")));

        assert!(forest.apply(Action::ExpandOrChild));
        assert_eq!(forest.selected(), Some(&key("orbital", "orb-7.1.1")));
    }

    #[test]
    fn a_leaf_has_no_child_to_move_to_and_no_fold_to_collapse() {
        let mut forest = flatten(&snapshot());
        select(&mut forest, &key("orbital", "orb-7.1.1"));

        assert!(!forest.apply(Action::ExpandOrChild));
        assert!(forest.apply(Action::CollapseOrParent));
        assert_eq!(forest.selected(), Some(&key("orbital", "orb-7.1")));
    }

    #[test]
    fn moving_stops_at_the_ends() {
        let mut forest = flatten(&snapshot());

        assert!(!forest.apply(Action::Move(Motion::PreviousRow)));
        assert_eq!(forest.selected_line(), 0);

        forest.apply(Action::Move(Motion::LastRow));
        let last = forest.selected_line();

        assert!(!forest.apply(Action::Move(Motion::NextRow)));
        assert_eq!(forest.selected_line(), last);
    }

    #[test]
    fn a_half_screen_moves_as_far_as_the_renderer_says_it_should() {
        let mut forest = flatten(&snapshot());
        forest.set_half_screen(3);

        forest.apply(Action::Move(Motion::HalfScreenDown));

        assert_eq!(forest.selected_line(), 3);

        forest.apply(Action::Move(Motion::HalfScreenUp));

        assert_eq!(forest.selected_line(), 0);
    }

    /// The forest cannot focus a pane, re-collect or quit; the loop does all
    /// three, and none of them changes what is on screen.
    #[test]
    fn focus_refresh_and_quit_change_nothing_in_the_forest() {
        let mut forest = flatten(&snapshot());
        let before = sketch(&forest);

        for action in [Action::Focus, Action::Refresh, Action::Quit] {
            assert!(!forest.apply(action), "{action:?}");
        }

        assert_eq!(sketch(&forest), before);
    }

    #[test]
    fn the_panes_of_a_tree_whose_tracker_failed_are_drawn_on_its_header() {
        let forest = flatten(&snapshot());
        let header = header_of(&forest, "ferry");

        assert_eq!(
            header
                .panes
                .iter()
                .map(|pane| pane.pane.as_str())
                .collect::<Vec<_>>(),
            vec!["w:p9"]
        );
        assert!(!header.panes_complete, "w:pF could belong here");
    }

    /// A tracker that did not answer is a property of the tree, so it rides
    /// the header rather than a line under it — one place, not two.
    #[test]
    fn a_tracker_that_could_not_be_read_says_so_on_its_header_and_nowhere_else() {
        let forest = flatten(&snapshot());

        assert_eq!(
            header_of(&forest, "ferry").tree.tracker,
            TrackerState::Unreachable(TrackerFailure::Auth)
        );
        let header = forest
            .lines()
            .iter()
            .position(
                |line| matches!(&line.content, Content::Tree(tree) if tree.tree.project == "ferry"),
            )
            .expect("ferry has a header");

        assert!(
            matches!(forest.lines()[header + 1].content, Content::Group(_)),
            "nothing is drawn under it: {:#?}",
            sketch(&forest)
        );
    }

    #[test]
    fn a_tree_that_was_read_recovers_no_panes_and_wants_none() {
        let forest = flatten(&snapshot());
        let header = header_of(&forest, "orbital");

        assert_eq!(header.panes, Vec::new());
        assert!(header.panes_complete);
    }

    fn header_of<'a>(forest: &'a Forest, project: &str) -> &'a Header {
        forest
            .lines()
            .iter()
            .find_map(|line| match &line.content {
                Content::Tree(header) if header.tree.project == project => Some(header),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{project} has a header"))
    }

    #[test]
    fn an_empty_group_draws_nothing() {
        let orbital = assembled(ORBITAL);
        let harbour = assembled(HARBOUR);
        let joined = joined(&orbital, &harbour, &panes());
        let snapshot = snapshot::build(
            Collected {
                trees: vec![tree_of("orbital", ORBITAL)],
                failed_projects: Vec::new(),
            },
            &[],
            &Joined {
                agents: joined.agents,
                conflicts: Vec::new(),
            },
            &cfg(),
            HerdrState::Ok,
            Filter::LiveAgents,
            now(),
        );

        assert!(!sketch(&flatten(&snapshot))
            .iter()
            .any(|line| line.contains('[')));
    }

    #[test]
    fn a_group_draws_one_line_for_each_thing_it_holds_when_it_is_opened() {
        let mut forest = flatten(&snapshot());
        select(&mut forest, &key("orbital", "orb-7"));
        forest.apply(Action::ToggleFold);
        forest.apply(Action::Move(Motion::LastRow));

        assert!(forest.apply(Action::ToggleFold));

        let drawn = sketch(&forest);
        let items = drawn.iter().filter(|line| line.contains("Loose")).count();

        assert_eq!(items, 2, "{drawn:#?}");
    }

    /// The panes under no configured project open into their own directories,
    /// which is the whole use of the group: the line says a `[[projects]]`
    /// entry is missing and opening it says which one.
    #[test]
    fn opening_the_unconfigured_group_names_the_directories() {
        let mut forest = flatten(&snapshot());
        forest
            .folds
            .insert(Handle::Group(GroupKind::Unconfigured), true);
        forest.refresh(&snapshot());

        let drawn = sketch(&forest);

        assert!(
            drawn
                .iter()
                .any(|line| line.contains("Unconfigured") && line.contains("/srv/spike")),
            "{drawn:#?}"
        );
    }

    /// The filter's choice holds — a hidden tree is not drawn — but a group
    /// that says only how many trees it hides reads like "nothing to see"
    /// when one of them has a broken parent chain.
    #[test]
    fn the_hidden_trees_group_says_how_many_of_them_have_findings() {
        let broken = HARBOUR.replace(r#""parent_id":"hbr-3""#, r#""parent_id":"hbr-9""#);
        let snapshot = gather(
            vec![tree_of("orbital", ORBITAL), tree_of("harbour", &broken)],
            Vec::new(),
            Filter::LiveAgents,
        );

        let forest = flatten(&snapshot);
        let group = forest
            .lines()
            .iter()
            .find_map(|line| match line.content {
                Content::Group(group) if group.kind == GroupKind::HiddenTrees => Some(group),
                _ => None,
            })
            .expect("harbour is hidden");

        assert_eq!(group.count, 1);
        assert_eq!(group.with_findings, 1);
    }

    #[test]
    fn a_hidden_tree_with_nothing_wrong_in_it_is_only_counted_as_hidden() {
        let forest = flatten(&snapshot());
        let group = forest
            .lines()
            .iter()
            .find_map(|line| match line.content {
                Content::Group(group) if group.kind == GroupKind::HiddenTrees => Some(group),
                _ => None,
            })
            .expect("harbour is hidden");

        assert_eq!(group.count, 1);
        assert_eq!(group.with_findings, 0);
    }

    /// Only a hidden tree takes findings out of the forest with it. Every
    /// other group holds its own subject in full, so none of them has
    /// anything undrawn to admit to.
    #[test]
    fn no_other_group_claims_to_be_hiding_findings() {
        let forest = flatten(&snapshot());
        let others: Vec<Group> = forest
            .lines()
            .iter()
            .filter_map(|line| match line.content {
                Content::Group(group) if group.kind != GroupKind::HiddenTrees => Some(group),
                _ => None,
            })
            .collect();

        assert_eq!(others.len(), 4);
        assert!(
            others.iter().all(|group| group.with_findings == 0),
            "{others:#?}"
        );
    }

    /// Every fold state over every root, every group and one interior node:
    /// 256 of them, which is small enough to visit rather than sample.
    #[test]
    fn nothing_reported_disappears_under_any_fold_state() {
        let snapshot = snapshot();
        let handles = [
            Handle::Bead(key("orbital", "orb-7")),
            Handle::Bead(key("ferry", "fer-2")),
            Handle::Bead(key("orbital", "orb-7.1")),
            Handle::Group(GroupKind::FailedProjects),
            Handle::Group(GroupKind::Unconfigured),
            Handle::Group(GroupKind::Conflicts),
            Handle::Group(GroupKind::HiddenTrees),
            Handle::Group(GroupKind::Unattributed),
        ];
        let expected = in_the_snapshot(&snapshot);

        for state in 0..1 << handles.len() {
            let mut forest = flatten(&snapshot);
            forest.folds = handles
                .iter()
                .enumerate()
                .map(|(bit, handle)| (handle.clone(), state & (1 << bit) == 0))
                .collect();
            forest.refresh(&snapshot);

            assert_eq!(on_screen(&forest), expected, "fold state {state:b}");
        }
    }

    /// The five degraded kinds, plus the two sorts of loose pane — the ones
    /// the recovery moves about, and the ones no configured project covers.
    #[derive(Debug, Default, PartialEq, Eq)]
    struct Reported {
        dangling: usize,
        unreachable: usize,
        truncated: usize,
        conflicts: usize,
        failed_projects: usize,
        loose_panes: usize,
        unconfigured_panes: usize,
    }

    fn in_the_snapshot(snapshot: &Snapshot) -> Reported {
        Reported {
            dangling: snapshot.trees.iter().map(|t| t.dangling.len()).sum(),
            unreachable: snapshot.trees.iter().map(|t| t.unreachable.len()).sum(),
            truncated: snapshot
                .trees
                .iter()
                .flat_map(|tree| &tree.nodes)
                .filter(|node| node.truncated)
                .count(),
            conflicts: snapshot.conflicts.len(),
            failed_projects: snapshot.failed_projects.len(),
            loose_panes: snapshot.unattributed.len(),
            unconfigured_panes: snapshot.unconfigured.len(),
        }
    }

    fn on_screen(forest: &Forest) -> Reported {
        let mut found = Reported::default();
        for line in forest.lines() {
            match &line.content {
                Content::Note(Note::Dangling(n)) => found.dangling += n,
                Content::Note(Note::Unreachable(n)) => found.unreachable += n,
                Content::Note(Note::Truncated(n)) => found.truncated += n,
                Content::Tree(header) => found.loose_panes += header.panes.len(),
                Content::Group(Group { kind, count, .. }) => match kind {
                    GroupKind::Conflicts => found.conflicts += count,
                    GroupKind::FailedProjects => found.failed_projects += count,
                    GroupKind::Unattributed => found.loose_panes += count,
                    GroupKind::Unconfigured => found.unconfigured_panes += count,
                    GroupKind::HiddenTrees => {}
                },
                _ => {}
            }
        }
        found
    }
}
