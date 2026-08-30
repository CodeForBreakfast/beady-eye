//! The snapshot, flattened into the lines the screen shows.

use std::collections::BTreeMap;

use crate::model::join::{BeadKey, Conflict};
use crate::model::snapshot::{
    self, FailedProject, Filter, HiddenTree, LoosePane, Node, Snapshot, TrackerState, Tree,
};
use crate::view::row::{self, Row};
use crate::view::{Action, Motion};

/// How far a half-screen motion moves until the renderer says otherwise.
const HALF_SCREEN: usize = 10;

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

/// The groups below the trees, in the order they are drawn: what could not be
/// read first, what the filter chose to hide last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GroupKind {
    FailedProjects,
    Conflicts,
    HiddenTrees,
    Unattributed,
}

impl GroupKind {
    pub const ALL: [GroupKind; 4] = [
        GroupKind::FailedProjects,
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
}

/// What a line that folds is known by, so both the fold and the selection
/// survive a refresh that reorders or drops lines.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Handle {
    Bead(BeadKey),
    Group(GroupKind),
}

/// One entry in a parent's sequence of children, before it becomes a line.
/// Notes and beads share the sequence because they share the box-drawing, and
/// a note is a child of the header exactly as a bead is.
enum Child {
    Note(Note),
    Node(usize),
    Elided(usize),
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
        let Some(Handle::Bead(key)) = &self.cursor else {
            return chain;
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
            Handle::Bead(key) => self.snapshot.trees.iter().any(|tree| {
                tree.project == key.project
                    && (tree.root == key.id || tree.nodes.iter().any(|node| node.id == key.id))
            }),
            Handle::Group(kind) => {
                let (_, loose) = self.recovery();
                !self.group_items(*kind, &loose).is_empty()
            }
        }
    }

    fn expanded(&self, handle: &Handle) -> bool {
        if let Some(open) = self.folds.get(handle) {
            return *open;
        }
        match handle {
            // A root shows its work while the selection is in it and collapses
            // to its header when it is not; anything below a root that is open
            // is open with it.
            Handle::Bead(key) => !self.is_root(key) || self.holds_cursor(key),
            Handle::Group(_) => false,
        }
    }

    fn is_root(&self, key: &BeadKey) -> bool {
        self.snapshot
            .trees
            .iter()
            .any(|tree| tree.project == key.project && tree.root == key.id)
    }

    fn holds_cursor(&self, root: &BeadKey) -> bool {
        let Some(Handle::Bead(cursor)) = &self.cursor else {
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
            let home = pane.project.as_ref().and_then(|project| {
                self.snapshot
                    .trees
                    .iter()
                    .position(|tree| tree.tracker != TrackerState::Ok && &tree.project == project)
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
        let open = self.expanded(&Handle::Bead(root.clone()));
        let complete = tree.tracker == TrackerState::Ok
            || !self
                .snapshot
                .unattributed
                .iter()
                .any(|pane| pane.project.is_none());

        lines.push(Line {
            prefix: marker(open).to_string(),
            depth: 0,
            last_child: false,
            folded: Some(open),
            bead: Some(root),
            content: Content::Tree(Header {
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
        if elided > 0 {
            entries.push(Child::Elided(elided));
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
                Child::Elided(count) => lines.push(Line {
                    prefix: prefix(trunk, last, false),
                    depth,
                    last_child: last,
                    folded: None,
                    bead: None,
                    content: Content::Elided { count },
                }),
                Child::Node(at) => {
                    let node = &tree.nodes[at];
                    let key = BeadKey {
                        project: tree.project.clone(),
                        id: node.id.clone(),
                    };
                    let kids = self.children_entries(tree, children, at);
                    let open = !kids.is_empty() && self.expanded(&Handle::Bead(key.clone()));
                    lines.push(Line {
                        prefix: prefix(trunk, last, !kids.is_empty() && !open),
                        depth,
                        last_child: last,
                        folded: (!kids.is_empty()).then_some(open),
                        bead: Some(key),
                        content: Content::Bead(row::cells(node, &tree.root)),
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
            let open = self.expanded(&Handle::Group(kind));
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
        Content::Tree(_) | Content::Bead(_) | Content::Group(_)
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

/// A node's children split into the ones drawn and the size of the run that
/// is not.
///
/// A closed sibling nobody is working collapses into the count; one carrying
/// an agent or an anomaly does not, because that is the stale-pane case and
/// eliding it would hide a live agent. A run of one is drawn: `… 1 more`
/// costs a line and saves none.
fn split(tree: &Tree, children: &[Vec<usize>], at: usize) -> (Vec<usize>, usize) {
    let quiet: Vec<usize> = children[at]
        .iter()
        .copied()
        .filter(|kid| {
            let node = &tree.nodes[*kid];
            node.status.is_closed() && node.agent.is_none() && node.anomalies.is_empty()
        })
        .collect();

    if quiet.len() < 2 {
        return (children[at].clone(), 0);
    }

    let elided = quiet
        .iter()
        .map(|kid| subtree_size(children, *kid))
        .sum::<usize>();
    let drawn = children[at]
        .iter()
        .copied()
        .filter(|kid| !quiet.contains(kid))
        .collect();
    (drawn, elided)
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
    /// two closed siblings are quiet.
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
            Content::Elided { count } => format!("… {count} more"),
            Content::Note(note) => format!("! {note:?}"),
            Content::Group(group) => format!("[{:?}] {}", group.kind, group.count),
            Content::Item(item) => format!("- {item:?}"),
        }
    }

    fn key(project: &str, id: &str) -> BeadKey {
        BeadKey {
            project: project.into(),
            id: id.into(),
        }
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
                "  └── … 2 more",
                "▸ ferry · fer-2",
                "▸ [FailedProjects] 1",
                "▸ [Conflicts] 1",
                "▸ [HiddenTrees] 1",
                "▸ [Unattributed] 3",
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
                "… 2 more",
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

        assert!(sketch(&forest).contains(&"  └── … 2 more".to_string()));
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

        assert_eq!(items, 3, "{drawn:#?}");
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

        assert_eq!(others.len(), 3);
        assert!(
            others.iter().all(|group| group.with_findings == 0),
            "{others:#?}"
        );
    }

    /// Every fold state over every root, every group and one interior node:
    /// 128 of them, which is small enough to visit rather than sample.
    #[test]
    fn nothing_reported_disappears_under_any_fold_state() {
        let snapshot = snapshot();
        let handles = [
            Handle::Bead(key("orbital", "orb-7")),
            Handle::Bead(key("ferry", "fer-2")),
            Handle::Bead(key("orbital", "orb-7.1")),
            Handle::Group(GroupKind::FailedProjects),
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

    /// The five degraded kinds, plus the loose panes the recovery moves about.
    #[derive(Debug, Default, PartialEq, Eq)]
    struct Reported {
        dangling: usize,
        unreachable: usize,
        truncated: usize,
        conflicts: usize,
        failed_projects: usize,
        loose_panes: usize,
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
                    GroupKind::HiddenTrees => {}
                },
                _ => {}
            }
        }
        found
    }
}
