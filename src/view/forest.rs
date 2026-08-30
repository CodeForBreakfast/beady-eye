//! The snapshot, flattened into the lines the screen shows.

use std::collections::{BTreeMap, BTreeSet};

use crate::model::join::{BeadKey, Conflict};
use crate::model::snapshot::{self, Filter, LoosePane, Snapshot, TrackerState, Tree};
use crate::view::lines::{
    beneath, children_of, marker, notes_of, opens_a_fold, prefix, progress_of, quiet, root_key,
    run_size, split, unfinished_beneath, Content, Group, GroupKind, Header, Item, Line, Note,
};
use crate::view::row;
use crate::view::{Action, Motion};

/// How far a half-screen motion moves until the renderer says otherwise.
const HALF_SCREEN: usize = 10;

/// What a line that folds is known by, so both the fold and the selection
/// survive a refresh that reorders or drops lines.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Handle {
    Bead(BeadKey),
    /// The run of quiet closed children under one bead. A bead has at most
    /// one run, so the bead names it.
    Elided(BeadKey),
    Group(GroupKind),
    Item(ItemKey),
}

/// What one thing in a group is known by.
///
/// A handle has to be an identity the thing still has after the next collect,
/// never where it sat, or a group re-read in another order would move the
/// selection to a neighbour with nothing on screen to say so.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum ItemKey {
    /// A pane, by its id, which is unique in a herdr session. It serves both
    /// groups that hold panes: `recovery` puts a pane in exactly one of them,
    /// and an unconfigured pane is one under no configured project at all.
    Pane(String),
    Project(String),
    /// A hidden tree, by the root it was hidden by.
    Tree(BeadKey),
    /// A disagreement, by the whole of what it says. No one field identifies
    /// every arm — several panes naming one bead in another project make
    /// several conflicts sharing that bead — and the value is made entirely
    /// of pane and bead ids, sorted and deduplicated by the join, so two
    /// collects that saw the same disagreement write the same one.
    Conflict(Conflict),
}

/// What a thing in a group is known by. Every kind has an identity, so every
/// one of them can hold the selection; a kind whose identity were only its
/// place in the group would have to return `None` here and stay unreachable.
fn item_key(item: &Item) -> Option<ItemKey> {
    Some(match item {
        Item::Loose(pane) => ItemKey::Pane(pane.pane.clone()),
        Item::Unconfigured(pane) => ItemKey::Pane(pane.pane.clone()),
        Item::Failed(failed) => ItemKey::Project(failed.project.clone()),
        Item::Hidden(hidden) => ItemKey::Tree(BeadKey {
            project: hidden.project.clone(),
            id: hidden.root.clone(),
        }),
        Item::Conflict(conflict) => ItemKey::Conflict(conflict.clone()),
    })
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
        let folded_over = self.folded_over();
        self.snapshot = snapshot.clone();
        self.spend_folds(&folded_over);
        self.cursor = ancestry.into_iter().find(|handle| self.present(handle));
        self.lay_out();
    }

    /// The live work each fold the user shut is currently shut over.
    fn folded_over(&self) -> BTreeMap<Handle, BTreeSet<BeadKey>> {
        self.folds
            .iter()
            .filter(|(_, open)| !**open)
            .map(|(handle, _)| (handle.clone(), self.live_under(handle)))
            .collect()
    }

    /// Let go of a fold the user set once live work has arrived beneath it
    /// that was not there when they set it.
    ///
    /// A fold says *I have seen what is under here and do not want it*, and
    /// that stops being true the moment something new is under it. So what
    /// they folded away stays folded for as long as it lives, work that dies
    /// down re-opens nothing, and an agent arriving on a bead they never saw
    /// hands the node back to the default.
    fn spend_folds(&mut self, folded_over: &BTreeMap<Handle, BTreeSet<BeadKey>>) {
        let spent: Vec<Handle> = folded_over
            .iter()
            .filter(|(handle, over)| !self.live_under(handle).is_subset(over))
            .map(|(handle, _)| handle.clone())
            .collect();
        for handle in spent {
            self.folds.remove(&handle);
        }
    }

    /// The beads beneath `handle` carrying live work. Empty for anything but
    /// a bead: a run holds only finished branches, and a group's items are
    /// not beads at all.
    fn live_under(&self, handle: &Handle) -> BTreeSet<BeadKey> {
        let Handle::Bead(key) = handle else {
            return BTreeSet::new();
        };
        let Some((tree, at)) = self.snapshot.locate(key) else {
            return BTreeSet::new();
        };
        let children = children_of(&tree.nodes);
        beneath(&children, at)
            .into_iter()
            .filter(|node| !quiet(&tree.nodes[*node]))
            .map(|node| BeadKey {
                project: tree.project.clone(),
                id: tree.nodes[node].id.clone(),
            })
            .collect()
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
            // Only the snapshot a pane was found in still knows which group
            // held it, so a pane that goes away falls back to that group
            // rather than to the top of the forest.
            Some(Handle::Item(key)) => {
                chain.extend(self.group_holding(key).map(Handle::Group));
                return chain;
            }
            _ => return chain,
        };

        if let Some((tree, at)) = self.snapshot.locate(&key) {
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
        handle_of(self.lines.get(at)?)
    }

    /// Redraw, and put the selection back on whatever it was holding.
    fn lay_out(&mut self) {
        self.settle_cursor();
        self.lines = self.draw();
        if self.find_cursor().is_none() {
            // The line the cursor named is not drawn — an ancestor is folded
            // over it, or the tracker stopped reporting it. Take the nearest
            // line that is. Nothing needs drawing again for it: the fold
            // state no longer turns on where the selection sits.
            self.cursor = self
                .scan(self.selected, false)
                .or_else(|| self.scan(self.selected, true))
                .and_then(|at| self.handle_at(at));
        }
        self.selected = self.find_cursor().unwrap_or(0);
    }

    fn settle_cursor(&mut self) {
        if self.cursor.as_ref().is_some_and(|held| self.present(held)) {
            return;
        }
        self.cursor = self.first_handle();
    }

    /// Which line the cursor is on. A bead reachable from two roots is drawn
    /// under each of them, so more than one line can carry the handle, and
    /// the one meant is the one nearest where the selection already sat —
    /// otherwise stepping onto the lower copy is undone by the redraw that
    /// follows it, and the list below that copy cannot be reached.
    fn find_cursor(&self) -> Option<usize> {
        let cursor = self.cursor.as_ref()?;
        (0..self.lines.len())
            .filter(|at| self.handle_at(*at).as_ref() == Some(cursor))
            .min_by_key(|at| at.abs_diff(self.selected))
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
            Handle::Bead(key) | Handle::Elided(key) => self.snapshot.holds(key),
            Handle::Group(kind) => {
                let (_, loose) = self.recovery();
                !self.group_items(*kind, &loose).is_empty()
            }
            Handle::Item(key) => self.group_holding(key).is_some(),
        }
    }

    /// The group one thing sits in, where the snapshot still holds it.
    fn group_holding(&self, key: &ItemKey) -> Option<GroupKind> {
        let (_, loose) = self.recovery();
        GroupKind::ALL.into_iter().find(|kind| {
            self.group_items(*kind, &loose)
                .iter()
                .any(|item| item_key(item).as_ref() == Some(key))
        })
    }

    /// Whether a fold is open: what the user set it to, or how it rests when
    /// they have not touched it. Only the caller knows the tree a line came
    /// from, so it says where the line rests rather than being asked to
    /// re-derive it here.
    fn expanded(&self, handle: &Handle, resting: bool) -> bool {
        self.folds.get(handle).copied().unwrap_or(resting)
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
        // Asked of the drawn lines rather than of the snapshot's fields, so
        // a later kind of line cannot be left out of the question.
        if lines.is_empty() {
            lines.push(nothing_to_draw());
        }
        lines
    }

    fn draw_tree(&self, tree: &Tree, panes: Vec<LoosePane>, lines: &mut Vec<Line>) {
        let root = root_key(tree);
        let children = children_of(&tree.nodes);
        // A tree opens because of what is in it, not because the selection
        // is in it: the first screen is meant to be the answer to what is
        // being worked and what could be started.
        let resting = !tree.nodes.is_empty() && opens_a_fold(tree, &children, 0);
        let open = self.expanded(&Handle::Bead(root.clone()), resting);
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
                    // Open the spine to the work a reader needs next and
                    // nothing else. A branch with none rests as one line, its
                    // glyph, its fraction and its marker saying what it holds.
                    let open = !kids.is_empty()
                        && self
                            .expanded(&Handle::Bead(key.clone()), opens_a_fold(tree, children, at));
                    let holding = (node.status.is_closed() && !open)
                        .then(|| unfinished_beneath(tree, children, at))
                        .filter(|unfinished| *unfinished > 0);
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
                            holding,
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
            let open = self.expanded(&Handle::Group(kind), kind.live());
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

/// The one line of a forest with nothing in it. Under no tree and in no
/// group, because there is neither: it stands for the whole screen.
fn nothing_to_draw() -> Line {
    Line {
        prefix: String::new(),
        depth: 0,
        last_child: false,
        folded: None,
        bead: None,
        content: Content::Note(Note::NoRoots),
    }
}

/// What a line is known by, where it is one the selection can hold.
///
/// A note stands for a finding rather than for a thing, so it has none — and
/// a line the forest cannot name could not be put back after a refresh, which
/// is why this is the same question as whether the selection may sit there.
fn handle_of(line: &Line) -> Option<Handle> {
    match &line.content {
        Content::Tree(_) | Content::Bead(_) => line.bead.clone().map(Handle::Bead),
        Content::Elided { under, .. } => Some(Handle::Elided(under.clone())),
        Content::Group(group) => Some(Handle::Group(group.kind)),
        Content::Item(item) => item_key(item).map(Handle::Item),
        Content::Note(_) => None,
    }
}

fn selectable(line: &Line) -> bool {
    handle_of(line).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::bd::parse_dep_tree;
    use crate::collect::herdr::parse_agent_list;
    use crate::config::Config;
    use crate::model::join::{self, Joined, ProjectRows};
    use crate::model::snapshot::{
        build_tree, Collected, FailedProject, HerdrState, Readiness, TrackerFailure,
    };
    use crate::model::tree::{assemble, Assembled};
    use crate::model::types::Pane;
    use crate::view::lines::{OPEN, SHUT};
    use crate::view::phrase;
    use crate::view::row::{Progress, Row};
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

    /// A spine four beads deep, with a quiet branch of its own beside it.
    /// Nothing in it is closed, in progress or staffed, so the only thing
    /// that can open a fold in it is a pane the test puts on a bead.
    const TOWER: &str = r#"[
      {"id":"tow-1","title":"raise the tower","status":"open","parent_id":"",
       "priority":1,"issue_type":"epic"},
      {"id":"tow-1.1","title":"stand the mast","status":"open","parent_id":"tow-1",
       "priority":2,"issue_type":"task"},
      {"id":"tow-1.1.1","title":"bolt the sections","status":"open","parent_id":"tow-1.1",
       "priority":2,"issue_type":"task"},
      {"id":"tow-1.1.1.1","title":"dress the cables","status":"open","parent_id":"tow-1.1.1",
       "priority":2,"issue_type":"task"},
      {"id":"tow-1.2","title":"pour the base","status":"open","parent_id":"tow-1",
       "priority":2,"issue_type":"task"},
      {"id":"tow-1.2.1","title":"tie the rebar","status":"open","parent_id":"tow-1.2",
       "priority":2,"issue_type":"task"}
    ]"#;

    /// A closed bead standing over work that is still to do. `bdi` reads
    /// `bd dep tree --direction=up`, so `sdg-4.1`'s descendants are the work
    /// closing it unblocked — the ordinary shape of this tree, not a
    /// malformed one. Nobody is on any of them and nothing is wrong with
    /// them, so the branch rests shut under a row whose own glyph says done.
    /// `sdg-4.2` is finished all the way down. `sdg-4.3` carries the only
    /// pane, which is what opens the root, and rests shut over unfinished
    /// work of its own without ever claiming to be done.
    const SIDING: &str = r#"[
      {"id":"sdg-4","title":"re-point the crossover","status":"in_progress","parent_id":"",
       "priority":1,"issue_type":"epic"},
      {"id":"sdg-4.1","title":"slew the up line","status":"closed","parent_id":"sdg-4",
       "priority":2,"issue_type":"task","closed_at":"2026-08-28T09:00:00Z"},
      {"id":"sdg-4.1.1","title":"key the switch","status":"closed","parent_id":"sdg-4.1",
       "priority":2,"issue_type":"task","closed_at":"2026-08-27T09:00:00Z"},
      {"id":"sdg-4.1.1.1","title":"gauge the check rail","status":"open","parent_id":"sdg-4.1.1",
       "priority":2,"issue_type":"task"},
      {"id":"sdg-4.1.1.2","title":"pack the timbers","status":"open","parent_id":"sdg-4.1.1",
       "priority":2,"issue_type":"task"},
      {"id":"sdg-4.1.2","title":"weld the closure rail","status":"open","parent_id":"sdg-4.1",
       "priority":2,"issue_type":"task"},
      {"id":"sdg-4.1.3","title":"lift the old chairs","status":"closed","parent_id":"sdg-4.1",
       "priority":2,"issue_type":"task","closed_at":"2026-08-26T09:00:00Z"},
      {"id":"sdg-4.2","title":"clip the down line","status":"closed","parent_id":"sdg-4",
       "priority":2,"issue_type":"task","closed_at":"2026-08-26T09:00:00Z"},
      {"id":"sdg-4.2.1","title":"torque the fishbolts","status":"closed","parent_id":"sdg-4.2",
       "priority":2,"issue_type":"task","closed_at":"2026-08-25T09:00:00Z"},
      {"id":"sdg-4.3","title":"re-signal the box","status":"in_progress","parent_id":"sdg-4",
       "priority":2,"issue_type":"task"},
      {"id":"sdg-4.3.1","title":"prove the interlocking","status":"open","parent_id":"sdg-4.3",
       "priority":2,"issue_type":"task"}
    ]"#;

    /// Two of one project's roots whose trees overlap. `qua-1.2` blocks both
    /// epics, and `bd dep tree --direction=up` walks dependents, so it comes
    /// back under each of them. Roots are found by climbing the parent chain
    /// and trees by walking dependents, so a bead standing in two trees is
    /// the ordinary shape of shared work, not a malformed tracker.
    const QUARRY: &str = r#"[
      {"id":"qua-1","title":"re-open the quarry","status":"in_progress","parent_id":"",
       "priority":1,"issue_type":"epic"},
      {"id":"qua-1.2","title":"cut the haul road","status":"in_progress","parent_id":"qua-1",
       "priority":2,"issue_type":"task"}
    ]"#;

    /// The second of the pair, drawn below Quarry, so the shared bead's lower
    /// copy sits here with two more rows under it. Those two are the bottom
    /// of the whole list, and are what a selection sprung back up to the
    /// upper copy never reaches.
    const WHARF: &str = r#"[
      {"id":"wha-2","title":"re-face the wharf","status":"in_progress","parent_id":"",
       "priority":1,"issue_type":"epic"},
      {"id":"wha-2.1","title":"drive the piles","status":"in_progress","parent_id":"wha-2",
       "priority":2,"issue_type":"task"},
      {"id":"qua-1.2","title":"cut the haul road","status":"in_progress","parent_id":"wha-2",
       "priority":2,"issue_type":"task"},
      {"id":"wha-2.3","title":"bed the fenders","status":"in_progress","parent_id":"wha-2",
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

    /// Every line drawn for one bead, by index. A bead reachable from two
    /// roots is drawn under each, so this answers with more than one.
    fn lines_of(forest: &Forest, id: &str) -> Vec<usize> {
        forest
            .lines()
            .iter()
            .enumerate()
            .filter(|(_, line)| line.bead.as_ref().is_some_and(|bead| bead.id == id))
            .map(|(at, _)| at)
            .collect()
    }

    /// Step down from the top until the selection stops moving, reporting
    /// where it sat at each step.
    fn walk_down(forest: &mut Forest) -> Vec<usize> {
        forest.apply(Action::Move(Motion::FirstRow));
        let mut visited = vec![forest.selected_line()];
        for _ in 0..forest.lines().len() {
            forest.apply(Action::Move(Motion::NextRow));
            let at = forest.selected_line();
            if visited.last() == Some(&at) {
                break;
            }
            visited.push(at);
        }
        visited
    }

    /// A bead reachable from two roots is drawn in both their trees, and the
    /// selection has to be able to sit on either copy. `find_cursor` took the
    /// first line carrying the handle, so the redraw that follows every
    /// action pulled a step onto the lower copy back up to the upper one, and
    /// the list below it could not be walked into at all.
    #[test]
    fn stepping_down_past_a_bead_drawn_twice_reaches_the_bottom_of_the_list() {
        let mut forest = flatten(&overlapping(&panes_on(&["qua-1.2", "wha-2.1"])));
        assert_eq!(
            lines_of(&forest, "qua-1.2").len(),
            2,
            "{:#?}",
            sketch(&forest)
        );
        let drawn = forest.lines().len();

        assert_eq!(
            walk_down(&mut forest),
            (0..drawn).collect::<Vec<usize>>(),
            "{:#?}",
            sketch(&forest)
        );
    }

    /// The tracker is written while the list is being read, so a refresh can
    /// land between any two keystrokes. It re-derives the selection from the
    /// handle it holds, and must settle on the copy the selection was on
    /// rather than on that copy's twin higher up the list.
    #[test]
    fn a_refresh_between_steps_does_not_pull_the_selection_back_to_a_twin() {
        let panes = panes_on(&["qua-1.2", "wha-2.1"]);
        let mut forest = flatten(&overlapping(&panes));
        let drawn = forest.lines().len();

        forest.apply(Action::Move(Motion::FirstRow));
        let mut visited = vec![forest.selected_line()];
        for _ in 1..drawn {
            forest.apply(Action::Move(Motion::NextRow));
            forest.refresh(&overlapping(&panes));
            visited.push(forest.selected_line());
        }

        assert_eq!(
            visited,
            (0..drawn).collect::<Vec<usize>>(),
            "{:#?}",
            sketch(&forest)
        );
    }

    /// Where the cursor is, by the bead its line carries. A tree's header
    /// line carries its root, so this answers for a header as readily as for
    /// a bead — which is the question these tests ask, and why nothing in
    /// production may ask it this way: `tail::target` has to tell a header
    /// from a bead before it reads the key.
    fn cursor(forest: &Forest) -> Option<&BeadKey> {
        forest.lines()[forest.selected_line()].bead.as_ref()
    }

    fn key(project: &str, id: &str) -> BeadKey {
        BeadKey {
            project: project.into(),
            id: id.into(),
        }
    }

    /// Depot with someone on `dep-1.1`, which is what opens its root: every
    /// other bead in it is finished, and a tree with nothing live in it rests
    /// as its header.
    fn depot() -> Snapshot {
        alone("orbital", DEPOT, &panes_on(&["dep-1.1"]))
    }

    /// One project's tree, joined against its own rows so that a pane the
    /// fixture names lands on the bead that names it. `tree_of` joins every
    /// fixture against Orbital's rows, which is what the shared snapshot
    /// needs and what leaves any other fixture's beads unstaffed.
    fn alone(project: &str, json: &str, panes: &[Pane]) -> Snapshot {
        ready_alone(project, json, panes, &[])
    }

    /// The same tree, with the beads `bd` answers `ready` with named. Every
    /// other fixture is built with an empty ready set, so a default that
    /// opens on readiness draws exactly the same screen under all of them and
    /// a green suite would say nothing about it.
    fn ready_alone(project: &str, json: &str, panes: &[Pane], ready: &[&str]) -> Snapshot {
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
        let readiness = Readiness {
            ready: ready.iter().map(|id| (*id).to_string()).collect(),
            ..Readiness::default()
        };
        let tree = build_tree(project, &rows, &joined, &readiness, &cfg, now());
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
            if cursor(forest) == Some(bead) {
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
                "  ├── ▸ ○ .1 re-point the dish",
                "  ├── ○ .7 log the survey marks",
                "  ├── ✓ .4 clear the access road",
                "  └── ▸ … 3 more",
                "▸ ferry · fer-2",
                "▸ [FailedProjects] 1",
                "▾ [Unconfigured] 1",
                "  └── - Unconfigured(UnconfiguredPane { pane: \"w:pF\", cwd: \"/srv/spike\", pane_status: Idle })",
                "▾ [Conflicts] 1",
                "  └── - Conflict(SeveralPanesNameOneBead { bead: BeadKey { project: \"orbital\", id: \"orb-7.1\" }, panes: [\"w:p3\", \"w:p4\"] })",
                "▸ [HiddenTrees] 1",
                "▾ [Unattributed] 2",
                "  ├── - Loose(LoosePane { pane: \"w:p3\", project: \"orbital\", cwd: \"/srv/work/orbital\", pane_status: Working })",
                "  └── - Loose(LoosePane { pane: \"w:p4\", project: \"orbital\", cwd: \"/srv/work/orbital\", pane_status: Idle })",
            ]
        );
    }

    /// The selection starts on the first root, which is what expands it.
    #[test]
    fn the_selection_starts_on_the_first_root() {
        let forest = flatten(&snapshot());

        assert_eq!(forest.selected_line(), 0);
        assert_eq!(cursor(&forest), Some(&key("orbital", "orb-7")));
    }

    /// The fold state is the user's and the live work's, and moving is
    /// neither: walking out of a tree leaves it exactly as it was drawn.
    #[test]
    fn a_root_stays_as_it_was_when_the_selection_walks_out_of_it() {
        let mut forest = flatten(&snapshot());
        let was = sketch(&forest);

        forest.apply(Action::Move(Motion::LastRow));

        assert_eq!(sketch(&forest), was);
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

    /// A working pane in Orbital on each of `on`. Each pane names its bead,
    /// which is how a bead carrying no configured key gets its agent, so a
    /// fixture is staffed by naming the beads someone is on.
    fn panes_on(on: &[&str]) -> Vec<Pane> {
        let agents: Vec<String> = on
            .iter()
            .map(|id| {
                format!(
                    r#"{{"pane_id":"w:{id}","cwd":"/srv/work/orbital",
                       "agent_status":"working","display_agent":"{id}"}}"#
                )
            })
            .collect();
        parse_agent_list(&format!(
            r#"{{"result":{{"agents":[{}]}}}}"#,
            agents.join(",")
        ))
        .expect("the panes parse")
    }

    fn tower_staffed(on: &[&str]) -> Snapshot {
        alone("orbital", TOWER, &panes_on(on))
    }

    /// The same tree with nobody on it and the named beads ready, so the two
    /// halves of the fold default can be asked the same question.
    fn tower_ready(on: &[&str]) -> Snapshot {
        ready_alone("orbital", TOWER, &[], on)
    }

    /// Two of one project's trees in one snapshot, joined against both so a
    /// pane naming a bead reaches it whichever tree draws it. `alone` takes a
    /// single tree, and the overlap these tests are about needs two.
    fn overlapping(panes: &[Pane]) -> Snapshot {
        let cfg = cfg();
        let quarry = assembled(QUARRY);
        let wharf = assembled(WHARF);
        let mut rows = quarry.rows.clone();
        rows.extend(wharf.rows.clone());
        let joined = join::resolve(
            &[ProjectRows {
                project: "orbital",
                rows: &rows,
            }],
            panes,
            &cfg.projects,
            &cfg.join,
        );
        let tree = |rows: &Assembled| {
            build_tree("orbital", rows, &joined, &Readiness::default(), &cfg, now())
        };
        snapshot::build(
            Collected {
                trees: vec![tree(&quarry), tree(&wharf)],
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

    /// Open one node by hand, the way a user reaching past the default does.
    fn open(forest: &mut Forest, bead: &BeadKey) {
        select(forest, bead);
        if fold_of(forest, &bead.id) == Some(false) {
            forest.apply(Action::ToggleFold);
        }
    }

    /// Whether the line for one bead is open, shut, or has no fold at all.
    fn fold_of(forest: &Forest, id: &str) -> Option<bool> {
        forest
            .lines()
            .iter()
            .find(|line| line.bead.as_ref().is_some_and(|key| key.id == id))
            .unwrap_or_else(|| panic!("{id} is not drawn"))
            .folded
    }

    /// The default the bead is about: the first screen is the work a reader
    /// needs next and the path down to it. Four quiet forebears open because
    /// of one bead at the bottom; the branch beside them, holding neither an
    /// agent nor ready work, stays shut.
    ///
    /// Two kinds of bead earn that opening and no third does. An agent on one
    /// says the work is happening; `bd` calling one ready says it can start.
    /// Both are asked of the same tree here, because the claim is that the
    /// screen cannot tell them apart — and either way the bead that earned
    /// the fold does not open its own, and the unfinished work `bd` will not
    /// start is still folded away.
    #[test]
    fn the_default_opens_every_forebear_of_a_live_agent_or_of_ready_work_and_nothing_else() {
        let opened = vec![
            "▾ orbital · tow-1",
            "  ├── ○ .1 stand the mast",
            "  │   └── ○ .1.1 bolt the sections",
            "  │       └── ○ .1.1.1 dress the cables",
            "  └── ▸ ○ .2 pour the base",
        ];

        let staffed = flatten(&tower_staffed(&["tow-1.1.1.1"]));
        let ready = flatten(&tower_ready(&["tow-1.1.1.1"]));

        assert_eq!(sketch(&staffed), opened);
        assert_eq!(sketch(&ready), opened);
        for forest in [&staffed, &ready] {
            for forebear in ["tow-1", "tow-1.1", "tow-1.1.1"] {
                assert_eq!(fold_of(forest, forebear), Some(true), "{forebear} is shut");
            }
            assert_eq!(fold_of(forest, "tow-1.2"), Some(false));
        }
    }

    /// The other half of the same rule. A tree nobody is working holds no
    /// spine to open, so it rests as the one line saying it is there.
    #[test]
    fn a_tree_with_nothing_live_in_it_rests_as_its_header() {
        let forest = flatten(&tower_staffed(&[]));

        assert_eq!(sketch(&forest), vec!["▸ orbital · tow-1"]);
    }

    /// A default, not a lock: the user shuts a node holding an agent and it
    /// stays shut, refresh after refresh, for as long as what is under there
    /// is what they folded away.
    #[test]
    fn a_fold_set_by_hand_survives_a_refresh_that_brings_nothing_new_under_it() {
        let mut forest = flatten(&tower_staffed(&["tow-1.1.1.1"]));
        select(&mut forest, &key("orbital", "tow-1.1"));
        forest.apply(Action::ToggleFold);

        forest.refresh(&tower_staffed(&["tow-1.1.1.1"]));

        assert_eq!(fold_of(&forest, "tow-1.1"), Some(false));
        assert!(
            !sketch(&forest).iter().any(|line| line.contains(".1.1")),
            "{:#?}",
            sketch(&forest)
        );
    }

    /// Work dying down is not news, so it re-opens nothing the user shut.
    /// The agent moves to the other branch, which is what keeps the tree
    /// open for the shut one to still be drawn under.
    #[test]
    fn a_fold_set_by_hand_outlives_the_work_it_was_shut_over_going_away() {
        let mut forest = flatten(&tower_staffed(&["tow-1.1.1.1"]));
        select(&mut forest, &key("orbital", "tow-1.1"));
        forest.apply(Action::ToggleFold);

        forest.refresh(&tower_staffed(&["tow-1.2.1"]));

        assert_eq!(fold_of(&forest, "tow-1.1"), Some(false));
    }

    /// The hard half. A fold says *I have seen what is under here and do not
    /// want it*, which stops being true the moment something new is under it,
    /// so an agent arriving on a bead the user never folded away hands the
    /// node back to the default.
    #[test]
    fn a_fold_set_by_hand_is_spent_when_live_work_arrives_beneath_it() {
        let mut forest = flatten(&tower_staffed(&["tow-1.1.1.1"]));
        select(&mut forest, &key("orbital", "tow-1.1"));
        forest.apply(Action::ToggleFold);

        forest.refresh(&tower_staffed(&["tow-1.1.1.1", "tow-1.1.1"]));

        assert_eq!(fold_of(&forest, "tow-1.1"), Some(true));
        assert!(
            sketch(&forest)
                .iter()
                .any(|line| line.contains(".1.1.1 dress the cables")),
            "{:#?}",
            sketch(&forest)
        );
    }

    /// The default reads the agents, not which trees are drawn, so dropping
    /// the filter adds trees below and changes no fold above.
    #[test]
    fn dropping_the_filter_leaves_the_default_fold_state_alone() {
        let mut forest = flatten(&snapshot());
        let staffed: Vec<String> = sketch(&forest)
            .into_iter()
            .take_while(|line| !line.contains("ferry"))
            .collect();

        forest.apply(Action::ToggleFilter);

        assert_eq!(sketch(&forest)[..staffed.len()], staffed[..]);
    }

    /// A group over live panes is a fold `bdi` chose, and a count is not a
    /// view of what it holds: it says they exist and nothing about which they
    /// are. What collection and the filter did is a report, and rests shut.
    #[test]
    fn a_group_rests_open_when_what_it_holds_is_live() {
        let forest = flatten(&snapshot());
        let markers: Vec<&str> = forest
            .lines()
            .iter()
            .filter_map(|line| match &line.content {
                Content::Group(_) => Some(marker(line.folded == Some(true))),
                _ => None,
            })
            .collect();

        assert_eq!(markers, vec![SHUT, OPEN, OPEN, SHUT, OPEN]);
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

        let drawn = sketch(&forest);
        let from_the_run: Vec<&String> = drawn
            .iter()
            .skip_while(|line| !line.contains("… 3 more"))
            .collect();

        assert_eq!(
            from_the_run[..4],
            [
                "  └── … 3 more",
                "      ├── ✓ .2 survey the mast",
                "      ├── ✓ .3 pour the pad",
                "      └── ✓ .5 set the guard rail",
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

        assert_eq!(row_of(&forest, "orb-7.7").progress, None);
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

        let line = &forest.lines()[forest.selected_line()];
        assert!(matches!(line.content, Content::Elided { .. }));
        assert_eq!(line.bead, None);
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

    /// The same invariant for the half `bdi-wt0` added, asked one bead at a
    /// time so a screen that happened to be open cannot answer for a rule
    /// that is not there. Every unfinished bead in every fixture takes its
    /// turn as the only ready one, and each turn is a whole forest whose
    /// default has to reach it.
    ///
    /// Position is the point. A bead behind three closed forebears, or in the
    /// run a branch collapses to, is where a fold that opens one level would
    /// still lose it.
    #[test]
    fn nothing_the_forest_folds_by_itself_hides_work_bd_would_start() {
        let mut asked = 0;
        for json in [ORBITAL, DEPOT, RELAY, SIDING, TOWER] {
            let unstaffed = alone("orbital", json, &[]);
            let unfinished: Vec<String> = unstaffed.trees[0]
                .nodes
                .iter()
                .filter(|node| !node.status.is_closed())
                .map(|node| node.id.clone())
                .collect();

            for id in unfinished {
                asked += 1;
                let forest = flatten(&ready_alone("orbital", json, &[], &[&id]));
                let drawn: Vec<&str> = forest
                    .lines()
                    .iter()
                    .filter_map(|line| line.bead.as_ref().map(|key| key.id.as_str()))
                    .collect();

                assert!(
                    drawn.contains(&id.as_str()),
                    "{id} is the one bead bd would start and is not on screen: {:#?}",
                    sketch(&forest)
                );
            }
        }

        assert!(asked > 0, "the fixtures held no unfinished bead to ready");
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
                "  │       └── ◐ .2.1.1 cut the stays",
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

    /// `bdi` reads `bd dep tree --direction=up`, so a bead's children are the
    /// work closing it unblocked. A closed bead standing over open ones is
    /// therefore the healthy shape of this tree, and where nobody is on them
    /// and `bd` will start none of them the branch rests shut under a row
    /// whose glyph says done. What it holds is out of sight either way, so
    /// the line says how much.
    #[test]
    fn a_closed_branch_resting_over_unfinished_work_says_how_much_it_holds() {
        let forest = flatten(&siding());

        assert_eq!(
            sketch(&forest),
            vec![
                "▾ orbital · sdg-4",
                "  ├── ▸ ◐ .3 re-signal the box",
                "  ├── ▸ ✓ .1 slew the up line",
                "  └── ▸ ✓ .2 clip the down line",
            ]
        );
        assert_eq!(
            row_of(&forest, "sdg-4.1").notes,
            vec![phrase::unfinished_beneath(3)]
        );
    }

    /// The common case, and the reason the sentence is not on every closed
    /// row: a branch that is done all the way down has nothing further to
    /// say, and a count on it would be noise wherever the eye landed.
    #[test]
    fn a_closed_branch_that_is_finished_all_the_way_down_says_nothing_extra() {
        let forest = flatten(&siding());

        assert_eq!(row_of(&forest, "sdg-4.2").notes, Vec::<String>::new());
    }

    /// Asked of the branch, not of the bead: `sdg-4.3` is unfinished itself
    /// and holds one unfinished bead, and a walk that counted the bead it was
    /// asked about would say two. Only closed nodes reach it from the
    /// renderer today, where a self that is closed adds nothing and the
    /// difference cannot show.
    #[test]
    fn what_a_branch_holds_never_counts_the_bead_it_was_asked_about() {
        let tree = tree_of("orbital", SIDING);
        let children = children_of(&tree.nodes);
        let at = tree
            .nodes
            .iter()
            .position(|node| node.id == "sdg-4.3")
            .expect("the fixture has an unfinished branch");

        assert_eq!(unfinished_beneath(&tree, &children, at), 1);
    }

    /// Only a line whose own glyph says done. An unfinished bead resting shut
    /// over unfinished work is not hiding anything its status did not already
    /// admit, and a sentence on every such row is the noise that would stop
    /// the closed ones being read.
    #[test]
    fn an_unfinished_branch_resting_shut_over_its_own_work_says_nothing_extra() {
        let forest = flatten(&siding());

        assert_eq!(row_of(&forest, "sdg-4.3").notes, Vec::<String>::new());
    }

    /// Counted at every depth. With the open bead directly under `sdg-4.1`
    /// closed, everything unfinished is two levels down, and a count of the
    /// immediate children would leave the row silent over both of them.
    #[test]
    fn unfinished_work_two_levels_under_a_closed_branch_is_still_counted() {
        let deep = SIDING.replace(
            r#""status":"open","parent_id":"sdg-4.1""#,
            r#""status":"closed","closed_at":"2026-08-26T09:00:00Z","parent_id":"sdg-4.1""#,
        );
        let forest = flatten(&alone("orbital", &deep, &panes_on(&["sdg-4.3"])));

        assert_eq!(
            row_of(&forest, "sdg-4.1").notes,
            vec![phrase::unfinished_beneath(2)]
        );
    }

    /// The sentence and the fraction are two readings of one walk, so they
    /// can never disagree: a row saying `2/5` and `3 unfinished beads` is the
    /// same fact twice, once as arithmetic and once in words.
    #[test]
    fn the_count_a_closed_branch_gives_is_the_remainder_of_its_own_fraction() {
        let forest = flatten(&siding());
        let row = row_of(&forest, "sdg-4.1");
        let progress = row.progress.expect("a branch has a fraction");

        assert_eq!(
            row.notes,
            vec![phrase::unfinished_beneath(progress.total - progress.closed)]
        );
    }

    /// Opened, the beads are on screen and counting them again above would be
    /// noise. The sentence is what the shut line is hiding, not a standing
    /// property of the bead.
    #[test]
    fn a_closed_branch_opened_over_its_work_stops_counting_it() {
        let mut forest = flatten(&siding());
        select(&mut forest, &key("orbital", "sdg-4.1"));

        forest.apply(Action::ToggleFold);

        assert_eq!(row_of(&forest, "sdg-4.1").notes, Vec::<String>::new());
    }

    fn siding() -> Snapshot {
        alone("orbital", SIDING, &panes_on(&["sdg-4.3"]))
    }

    /// The case Graeme chose this default for. `sdg-4.1` is closed and its
    /// glyph says so, but `bd` will start `sdg-4.1.2` today, and a reader
    /// looking for what to pick up should not have to press a key to find it.
    ///
    /// Down the spine and no wider: `sdg-4.1` opens because the ready bead is
    /// under it, `sdg-4.1.1` stays shut because none is under that, and the
    /// count moves down to the line that is now the one doing the hiding.
    #[test]
    fn a_closed_branch_over_ready_work_rests_open_down_the_spine_to_it() {
        let forest = flatten(&ready_alone(
            "orbital",
            SIDING,
            &panes_on(&["sdg-4.3"]),
            &["sdg-4.1.2"],
        ));

        assert_eq!(
            sketch(&forest),
            vec![
                "▾ orbital · sdg-4",
                "  ├── ▸ ◐ .3 re-signal the box",
                "  ├── ✓ .1 slew the up line",
                "  │   ├── ○ .1.2 weld the closure rail",
                "  │   ├── ▸ ✓ .1.1 key the switch",
                "  │   └── ✓ .1.3 lift the old chairs",
                "  └── ▸ ✓ .2 clip the down line",
            ]
        );
        assert_eq!(row_of(&forest, "sdg-4.1").notes, Vec::<String>::new());
        assert_eq!(
            row_of(&forest, "sdg-4.1.1").notes,
            vec![phrase::unfinished_beneath(2)]
        );
    }

    /// The other half of the widened rule, and the reason it is `bd ready`
    /// and not a status test: work that is unfinished but blocked or deferred
    /// is not what a reader needs next, so it earns no fold. The statuses
    /// here are the ones that most look like work in hand, and the branch
    /// rests shut over all three exactly as it does over open ones.
    #[test]
    fn a_closed_branch_over_work_bd_will_not_start_rests_shut_and_says_how_much() {
        let waiting = SIDING
            .replace(
                r#""status":"open","parent_id":"sdg-4.1.1""#,
                r#""status":"blocked","parent_id":"sdg-4.1.1""#,
            )
            .replace(
                r#""status":"open","parent_id":"sdg-4.1""#,
                r#""status":"deferred","parent_id":"sdg-4.1""#,
            );
        let forest = flatten(&alone("orbital", &waiting, &panes_on(&["sdg-4.3"])));

        assert_eq!(fold_of(&forest, "sdg-4.1"), Some(false));
        assert_eq!(
            row_of(&forest, "sdg-4.1").notes,
            vec![phrase::unfinished_beneath(3)]
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
        alone("orbital", &json, &panes_on(&["dep-1.1"]))
    }

    #[test]
    fn the_selection_survives_a_refresh_that_reorders_the_nodes() {
        let mut forest = flatten(&snapshot());
        open(&mut forest, &key("orbital", "orb-7.1"));
        select(&mut forest, &key("orbital", "orb-7.1.2"));
        let was = forest.selected_line();

        let reordered = ORBITAL.replace(r#""priority":3"#, r#""priority":1"#);
        forest.refresh(&gather(
            vec![tree_of("orbital", &reordered)],
            Vec::new(),
            Filter::LiveAgents,
        ));

        assert_eq!(cursor(&forest), Some(&key("orbital", "orb-7.1.2")));
        assert_ne!(forest.selected_line(), was);
    }

    /// Closing a bead under the cursor is the ordinary way for one to go, and
    /// the tree it was in is still on screen. The cursor stays in that tree,
    /// on the parent, rather than going back to the top of the forest.
    #[test]
    fn a_refresh_that_drops_the_selected_bead_falls_back_to_its_parent() {
        let mut forest = flatten(&snapshot());
        open(&mut forest, &key("orbital", "orb-7.1"));
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

        assert_eq!(cursor(&forest), Some(&key("orbital", "orb-7.1")));
    }

    #[test]
    fn a_refresh_that_drops_the_selected_bead_leaves_the_selection_somewhere_real() {
        let mut forest = flatten(&snapshot());
        open(&mut forest, &key("orbital", "orb-7.1"));
        select(&mut forest, &key("orbital", "orb-7.1.2"));

        forest.refresh(&gather(
            vec![tree_of("harbour", HARBOUR)],
            Vec::new(),
            Filter::All,
        ));

        assert_eq!(cursor(&forest), Some(&key("harbour", "hbr-3")));
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
        open(&mut forest, &key("orbital", "orb-7.1"));

        assert!(forest.apply(Action::CollapseOrParent));
        assert_eq!(cursor(&forest), Some(&key("orbital", "orb-7.1")));
        assert!(!sketch(&forest).iter().any(|line| line.contains(".1.1")));

        assert!(forest.apply(Action::CollapseOrParent));
        assert_eq!(cursor(&forest), Some(&key("orbital", "orb-7")));
    }

    #[test]
    fn expanding_a_collapsed_node_and_then_expanding_again_moves_to_its_first_child() {
        let mut forest = flatten(&snapshot());
        open(&mut forest, &key("orbital", "orb-7.1"));
        forest.apply(Action::CollapseOrParent);

        assert!(forest.apply(Action::ExpandOrChild));
        assert_eq!(cursor(&forest), Some(&key("orbital", "orb-7.1")));

        assert!(forest.apply(Action::ExpandOrChild));
        assert_eq!(cursor(&forest), Some(&key("orbital", "orb-7.1.1")));
    }

    #[test]
    fn a_leaf_has_no_child_to_move_to_and_no_fold_to_collapse() {
        let mut forest = flatten(&snapshot());
        open(&mut forest, &key("orbital", "orb-7.1"));
        select(&mut forest, &key("orbital", "orb-7.1.1"));

        assert!(!forest.apply(Action::ExpandOrChild));
        assert!(forest.apply(Action::CollapseOrParent));
        assert_eq!(cursor(&forest), Some(&key("orbital", "orb-7.1")));
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

    /// The defect: a group's lines were drawn and could not be reached, so
    /// nothing the forest holds in a group could be looked at or acted on.
    #[test]
    fn the_selection_can_walk_onto_a_line_under_a_group() {
        let mut forest = flatten(&snapshot());
        let panes: Vec<usize> = forest
            .lines()
            .iter()
            .enumerate()
            .filter(|(_, line)| matches!(line.content, Content::Item(Item::Loose(_))))
            .map(|(at, _)| at)
            .collect();

        assert_eq!(panes.len(), 2, "{:#?}", sketch(&forest));
        for at in panes {
            forest.apply(Action::Move(Motion::FirstRow));
            for _ in 0..forest.lines().len() {
                if forest.selected_line() >= at {
                    break;
                }
                forest.apply(Action::Move(Motion::NextRow));
            }
            assert_eq!(
                forest.selected_line(),
                at,
                "line {at} cannot be reached: {:#?}",
                sketch(&forest)
            );
        }
    }

    /// The identity has to be the pane itself and never where it sat, or a
    /// refresh that reorders a group moves the selection to another pane
    /// while everything on screen still looks right.
    #[test]
    fn a_selected_pane_survives_a_refresh_that_reorders_its_group() {
        let mut forest = flatten(&snapshot());
        select_item(&mut forest, "w:p4");

        forest.refresh(&reordered_groups());

        assert_eq!(selected_item(&forest), Some("w:p4".to_string()));
    }

    /// Every group's items, so no kind is selectable by accident and none is
    /// left behind: a group whose lines cannot be reached is the defect.
    #[test]
    fn every_kind_of_thing_a_group_holds_can_hold_the_selection() {
        let mut forest = flatten(&built(Filter::LiveAgents));
        for kind in GroupKind::ALL {
            forest.folds.insert(Handle::Group(kind), true);
        }
        forest.refresh(&built(Filter::LiveAgents));

        let items: Vec<usize> = forest
            .lines()
            .iter()
            .enumerate()
            .filter(|(_, line)| matches!(line.content, Content::Item(_)))
            .map(|(at, _)| at)
            .collect();

        assert_eq!(items.len(), 6, "{:#?}", sketch(&forest));
        for at in items {
            assert!(
                selectable(&forest.lines()[at]),
                "{:?} cannot hold the selection",
                forest.lines()[at].content
            );
        }
    }

    /// A pane that goes away leaves the selection on the group it was in,
    /// rather than at the top of the forest. Work ending should not look
    /// like the screen jumping.
    #[test]
    fn a_selection_on_a_pane_that_goes_away_falls_back_to_its_group() {
        let mut forest = flatten(&snapshot());
        select_item(&mut forest, "w:p4");

        forest.refresh(&built_without_the_conflicting_panes());

        assert!(
            matches!(
                forest.lines()[forest.selected_line()].content,
                Content::Group(Group {
                    kind: GroupKind::Unattributed,
                    ..
                })
            ),
            "{:#?}",
            forest.lines()[forest.selected_line()]
        );
    }

    /// The pane id on the line the selection sits on, where it sits on one.
    fn selected_item(forest: &Forest) -> Option<String> {
        match &forest.lines()[forest.selected_line()].content {
            Content::Item(Item::Loose(pane)) => Some(pane.pane.clone()),
            Content::Item(Item::Unconfigured(pane)) => Some(pane.pane.clone()),
            _ => None,
        }
    }

    /// Put the selection on the line for one pane, by moving down to it.
    fn select_item(forest: &mut Forest, pane: &str) {
        forest.apply(Action::Move(Motion::FirstRow));
        for _ in 0..=forest.lines().len() {
            if selected_item(forest).as_deref() == Some(pane) {
                return;
            }
            forest.apply(Action::Move(Motion::NextRow));
        }
        panic!("{pane} is not reachable by moving down");
    }

    /// The same snapshot with every group's items in the other order, which
    /// is what a collect that re-read them may hand over.
    fn reordered_groups() -> Snapshot {
        let mut snapshot = snapshot();
        snapshot.unattributed.reverse();
        snapshot.conflicts.reverse();
        snapshot.hidden_trees.reverse();
        snapshot
    }

    /// The same snapshot with the two panes that were fighting over `orb-7.1`
    /// gone, which empties the unattributed group of the one the tests hold.
    fn built_without_the_conflicting_panes() -> Snapshot {
        let mut snapshot = snapshot();
        snapshot.unattributed.retain(|pane| pane.pane == "w:p3");
        snapshot
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
                refused: BTreeMap::new(),
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
        let loose = |forest: &Forest| {
            sketch(forest)
                .iter()
                .filter(|line| line.contains("Loose"))
                .count()
        };
        forest.apply(Action::Move(Motion::LastRow));
        // The last line is a pane now that a group's lines can be reached,
        // and a pane has no fold, so `h` steps out to the group holding it.
        forest.apply(Action::CollapseOrParent);

        assert!(forest.apply(Action::ToggleFold));
        assert_eq!(loose(&forest), 0, "{:#?}", sketch(&forest));

        assert!(forest.apply(Action::ToggleFold));
        assert_eq!(loose(&forest), 2, "{:#?}", sketch(&forest));
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
                // Matched variant by variant so a note added later has to
                // be decided here rather than fall through as nothing.
                Content::Note(note) => match note {
                    Note::Dangling(n) => found.dangling += n,
                    Note::Unreachable(n) => found.unreachable += n,
                    Note::Truncated(n) => found.truncated += n,
                    // A property of the drawing rather than a finding in the
                    // snapshot, so there is no count for it to reach.
                    Note::NoRoots => {}
                },
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

    // ---- a forest with nothing in it --------------------------------------

    /// A pane working in a configured project, with no tracker answering for
    /// it, so it reaches the forest as a loose one.
    const WORKING_IN_ORBITAL: &str = r#"{"result":{"agents":[
      {"pane_id":"w:p1","cwd":"/srv/work/orbital","agent_status":"working"}
    ]}}"#;

    /// A pane working in a directory no configured project covers.
    const WORKING_NOWHERE: &str = r#"{"result":{"agents":[
      {"pane_id":"w:pF","cwd":"/srv/spike","agent_status":"idle"}
    ]}}"#;

    fn one_pane(json: &str) -> Vec<Pane> {
        parse_agent_list(json).expect("the panes parse")
    }

    /// A snapshot built out of exactly what it is handed, with no other
    /// project's trees or panes standing behind it.
    ///
    /// Everything the forest draws a line from arrives through one of these:
    /// the trees and failed projects `Collected` carries, and the panes a
    /// join resolves. Handed one of them alone the forest draws that one
    /// thing, which is what it takes to ask whether anything else is quietly
    /// supplying a line.
    fn only(collected: Collected, panes: &[Pane]) -> Snapshot {
        let cfg = cfg();
        let joined = join::resolve(&[], panes, &cfg.projects, &cfg.join);
        snapshot::build(
            collected,
            panes,
            &joined,
            &cfg,
            HerdrState::Ok,
            Filter::LiveAgents,
            now(),
        )
    }

    fn says_it_holds_nothing(forest: &Forest) -> bool {
        forest
            .lines()
            .iter()
            .any(|line| matches!(line.content, Content::Note(Note::NoRoots)))
    }

    /// Every tracker answered and none of them had a root. The screen has to
    /// carry the reason, because a pane drawn blank reads as a crash.
    #[test]
    fn a_forest_with_nothing_in_it_says_so_rather_than_drawing_nothing() {
        let forest = flatten(&only(Collected::default(), &[]));

        assert_eq!(sketch(&forest), vec!["! NoRoots"]);
    }

    /// Everything that can stand alone in a forest. A conflict is not among
    /// them: a pane can only conflict over a bead a tracker answered for, so
    /// it never arrives without the tree that bead is in.
    #[test]
    fn a_forest_holding_any_one_thing_does_not_say_it_holds_nothing() {
        let cases = [
            (
                "a tree",
                only(
                    Collected {
                        trees: vec![tree_of("orbital", ORBITAL)],
                        failed_projects: Vec::new(),
                    },
                    &[],
                ),
            ),
            (
                "a hidden tree",
                only(
                    Collected {
                        trees: vec![tree_of("harbour", HARBOUR)],
                        failed_projects: Vec::new(),
                    },
                    &[],
                ),
            ),
            (
                "a failed project",
                only(
                    Collected {
                        trees: Vec::new(),
                        failed_projects: vec![FailedProject {
                            project: "lunar".into(),
                            tracker: TrackerFailure::Exec,
                        }],
                    },
                    &[],
                ),
            ),
            (
                "a loose pane",
                only(Collected::default(), &one_pane(WORKING_IN_ORBITAL)),
            ),
            (
                "an unconfigured pane",
                only(Collected::default(), &one_pane(WORKING_NOWHERE)),
            ),
        ];

        for (held, snapshot) in cases {
            let forest = flatten(&snapshot);

            assert!(
                !forest.lines().is_empty(),
                "a forest holding {held} drew nothing at all"
            );
            assert!(
                !says_it_holds_nothing(&forest),
                "a forest holding {held} said it holds nothing: {:?}",
                sketch(&forest)
            );
        }
    }

    #[test]
    fn a_forest_holding_trees_and_every_group_does_not_say_it_holds_nothing() {
        assert!(!says_it_holds_nothing(&flatten(&snapshot())));
    }
}
