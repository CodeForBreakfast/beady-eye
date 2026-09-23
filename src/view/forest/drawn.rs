//! The lines the forest has drawn, addressed by row.
//!
//! Every reader of the forest asks one of a few things of its lines: how many
//! there are, which line is on a row, which row a handle is on, the rows a
//! band has room for, and how wide each identity cell draws. They ask here
//! rather than of a slice, so that what stands behind the questions can
//! change without the questions moving.
//!
//! What stands behind them is a tree of the lines, expanded only as far as a
//! reader has looked. A line whose subtree no fold names is drawn from the
//! tree it came from when a reader first reaches into it, and counted without
//! being drawn until then — so a forest of hundreds of thousands of rows costs
//! a screenful of lines to show, and a key that reshapes it costs the count.

use std::collections::HashMap;
use std::ops::Index;
use std::sync::{Arc, OnceLock};

use crate::model::snapshot::Tree;
use crate::view::draw::identity_widths;
use crate::view::lines::{Content, Line};
use crate::view::row::{self, Widths};

use super::facts::Facts;
use super::handle::{names, Handle};
use super::layout;
use super::spine::Stand;

/// One snapshot's lines in render order.
#[derive(Debug, Clone, Default)]
pub struct Drawn {
    top: Vec<Node>,
    rows: usize,
    widths: Widths,
    /// What a subtree left undrawn is drawn from. Nothing where every line
    /// was handed over drawn.
    ground: Option<Arc<Ground>>,
}

/// What the lines were drawn from, kept for the subtrees not drawn yet.
#[derive(Debug)]
pub(super) struct Ground {
    pub(super) trees: Vec<Arc<Tree>>,
    pub(super) facts: Arc<Facts>,
    /// Every bead copy's subtree counted, by what it was counted from.
    pub(super) beads: HashMap<Counted, Count>,
    /// Every run counted, by the bead copy it hangs under.
    pub(super) runs: HashMap<Counted, Count>,
    /// Whether what a shut fold hides is drawn as well.
    pub(super) beneath_shut: bool,
}

/// What a subtree left undrawn is counted from: a bead copy, where it
/// stands on the spine, which way the scope over it points every fold that
/// rests, and the bead the walk leaves out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct Counted {
    pub(super) tree: usize,
    pub(super) at: usize,
    pub(super) stand: Stand,
    pub(super) forced: Option<bool>,
    pub(super) without: Option<usize>,
}

/// What a subtree adds up to: its rows, and how wide each identity cell
/// draws on the lines strictly beneath its own.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Count {
    pub(super) rows: usize,
    pub(super) widths: Widths,
}

/// One line, and everything drawn beneath it.
#[derive(Debug, Clone)]
pub(super) struct Node {
    pub(super) line: Line,
    /// This line and every line beneath it.
    pub(super) rows: usize,
    pub(super) beneath: Beneath,
    children: OnceLock<Vec<Node>>,
}

/// Where a line's children come from, where they have not been drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Beneath {
    /// Drawn already, or nothing there.
    Nothing,
    /// A bead copy's children, drawn from its tree.
    Bead(Undrawn),
    /// A run's members, drawn from the tree of the bead the run hangs under.
    Run(Undrawn),
}

/// What draws a subtree nothing has drawn: what it is counted from, and the
/// trunk its children hang under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Undrawn {
    pub(super) counted: Counted,
    pub(super) trunk: Vec<bool>,
}

impl Node {
    pub(super) fn drawn(line: Line, children: Vec<Node>) -> Self {
        let rows = 1 + children.iter().map(|child| child.rows).sum::<usize>();
        Node {
            line,
            rows,
            beneath: Beneath::Nothing,
            children: OnceLock::from(children),
        }
    }

    pub(super) fn undrawn(line: Line, rows: usize, beneath: Beneath) -> Self {
        Node {
            line,
            rows,
            beneath,
            children: OnceLock::new(),
        }
    }

    /// The lines directly beneath this one, drawn now if they were not.
    pub(super) fn children(&self, ground: Option<&Ground>) -> &[Node] {
        self.children.get_or_init(|| match (&self.beneath, ground) {
            (Beneath::Nothing, _) | (_, None) => Vec::new(),
            (Beneath::Bead(undrawn), Some(ground)) => layout::beneath_bead(ground, self, undrawn),
            (Beneath::Run(undrawn), Some(ground)) => layout::beneath_run(ground, self, undrawn),
        })
    }

    /// Whether the handle could be drawn beneath this line, read off the
    /// line alone so the subtree is not drawn to find out it is not.
    fn may_hold(&self, handle: &Handle) -> bool {
        match (&self.line.content, handle) {
            (
                Content::Project(line),
                Handle::Bead(place) | Handle::Unread(place) | Handle::Elided(place),
            ) => place.tree.project == line.project,
            (Content::Project(line), Handle::Group(_, Some(project))) => *project == line.project,
            (Content::Project(_), Handle::Item(_)) => true,
            (
                Content::Group(_),
                Handle::Bead(_) | Handle::Unread(_) | Handle::Elided(_) | Handle::Item(_),
            ) => true,
            (Content::Bead(_), Handle::Bead(place)) => {
                self.line.place.as_ref().is_some_and(|own| {
                    place.tree == own.tree
                        && place.steps.len() > own.steps.len()
                        && place.steps.starts_with(&own.steps)
                })
            }
            (Content::Bead(_), Handle::Elided(place)) => {
                self.line.place.as_ref().is_some_and(|own| {
                    place.tree == own.tree && place.steps.starts_with(&own.steps)
                })
            }
            (Content::Elided { under, .. }, Handle::Bead(place) | Handle::Elided(place)) => {
                place.tree == under.tree
                    && place.steps.len() > under.steps.len()
                    && place.steps.starts_with(&under.steps)
            }
            _ => false,
        }
    }
}

#[allow(clippy::len_without_is_empty)]
impl Drawn {
    /// Lines handed over drawn, nested as their depths say.
    #[cfg(test)]
    pub(super) fn new(lines: Vec<Line>) -> Self {
        let mut open: Vec<(Line, Vec<Node>)> = Vec::new();
        let mut top = Vec::new();
        for line in lines {
            while open
                .last()
                .is_some_and(|(over, _)| over.depth >= line.depth)
            {
                let (over, children) = open.pop().expect("an open line");
                let node = Node::drawn(over, children);
                match open.last_mut() {
                    Some((_, siblings)) => siblings.push(node),
                    None => top.push(node),
                }
            }
            open.push((line, Vec::new()));
        }
        while let Some((over, children)) = open.pop() {
            let node = Node::drawn(over, children);
            match open.last_mut() {
                Some((_, siblings)) => siblings.push(node),
                None => top.push(node),
            }
        }
        Self::over(top, None, &row::Layout::default())
    }

    /// `layout` is what each bead's row is drawn by, and says which cells
    /// the identity's widths are measured over.
    pub(super) fn over(top: Vec<Node>, ground: Option<Ground>, layout: &row::Layout) -> Self {
        let rows = top.iter().map(|node| node.rows).sum();
        let mut drawn = Drawn {
            top,
            rows,
            widths: Widths::default(),
            ground: ground.map(Arc::new),
        };
        drawn.widths = drawn.widest(layout);
        drawn
    }

    fn ground(&self) -> Option<&Ground> {
        self.ground.as_deref()
    }

    /// How many rows the forest draws.
    pub fn len(&self) -> usize {
        self.rows
    }

    /// The line on a row, where the forest draws that many.
    pub fn get(&self, row: usize) -> Option<&Line> {
        self.located(row).map(|(node, _)| &node.line)
    }

    /// The row of the line the row's line hangs under, where it hangs under
    /// one.
    pub(super) fn parent_of(&self, row: usize) -> Option<usize> {
        self.located(row).and_then(|(_, parent)| parent)
    }

    /// The node on a row and the row of its parent.
    fn located(&self, row: usize) -> Option<(&Node, Option<usize>)> {
        let mut nodes = self.top.as_slice();
        let mut parent = None;
        let mut at = 0;
        let mut left = row;
        loop {
            let node = nodes.iter().find(|node| {
                if left < node.rows {
                    return true;
                }
                left -= node.rows;
                at += node.rows;
                false
            })?;
            if left == 0 {
                return Some((node, parent));
            }
            left -= 1;
            parent = Some(at);
            at += 1;
            nodes = node.children(self.ground());
        }
    }

    /// Every line, in render order.
    pub fn iter(&self) -> Lines<'_> {
        Lines(self.viewport(0, self.rows))
    }

    /// The rows a band starting at `from` with room for `height` lines shows,
    /// each with its row.
    pub fn viewport(&self, from: usize, height: usize) -> Rows<'_> {
        let mut rows = Rows {
            drawn: self,
            next: Vec::new(),
            row: from,
            end: from.saturating_add(height).min(self.rows),
        };
        rows.seek(from);
        rows
    }

    /// How wide each identity cell draws on the widest line of the forest,
    /// so every title starts in the same column and a reader's eye runs down
    /// one edge rather than a ragged one.
    pub fn widths(&self) -> &Widths {
        &self.widths
    }

    fn widest(&self, layout: &row::Layout) -> Widths {
        let mut widest = Widths::default();
        let mut left: Vec<&Node> = self.top.iter().collect();
        while let Some(node) = left.pop() {
            if let Content::Bead(row) = &node.line.content {
                widest.merge(&identity_widths(row, layout));
            }
            match &node.beneath {
                Beneath::Nothing => left.extend(node.children(self.ground())),
                beneath => {
                    if let Some(count) = self.count_of(beneath) {
                        widest.merge(&count.widths);
                    }
                }
            }
        }
        widest
    }

    /// Which row carries a handle, where one does. A bead reachable more than
    /// once is drawn once per way down to it, and the handle names the way
    /// down, so at most one row carries it.
    pub(super) fn row_of(&self, handle: &Handle) -> Option<usize> {
        self.locate(handle).map(|(row, _)| row)
    }

    /// The line a handle names, where one is drawn.
    pub(super) fn node_of(&self, handle: &Handle) -> Option<&Node> {
        self.locate(handle).map(|(_, node)| node)
    }

    fn locate(&self, handle: &Handle) -> Option<(usize, &Node)> {
        self.locate_in(&self.top, handle, 0)
    }

    fn locate_in<'a>(
        &'a self,
        nodes: &'a [Node],
        handle: &Handle,
        mut row: usize,
    ) -> Option<(usize, &'a Node)> {
        for node in nodes {
            if names(&node.line, handle) {
                return Some((row, node));
            }
            if node.may_hold(handle) {
                if let Some(found) = self.locate_in(node.children(self.ground()), handle, row + 1) {
                    return Some(found);
                }
            }
            row += node.rows;
        }
        None
    }

    /// Every line at the top of the forest, in order.
    pub(super) fn top(&self) -> &[Node] {
        &self.top
    }

    /// Walk a line and everything beneath it in render order, calling `seen`
    /// on each, and going beneath a line only where `seen` says so.
    pub(super) fn visit<'a>(&'a self, node: &'a Node, seen: &mut impl FnMut(&'a Node) -> bool) {
        if seen(node) {
            for child in node.children(self.ground()) {
                self.visit(child, seen);
            }
        }
    }

    /// The count a subtree left undrawn was given.
    pub(super) fn count_of(&self, beneath: &Beneath) -> Option<&Count> {
        let ground = self.ground()?;
        match beneath {
            Beneath::Nothing => None,
            Beneath::Bead(undrawn) => ground.beads.get(&undrawn.counted),
            Beneath::Run(undrawn) => ground.runs.get(&undrawn.counted),
        }
    }

    pub(super) fn tree(&self, index: usize) -> Option<&Arc<Tree>> {
        self.ground().and_then(|ground| ground.trees.get(index))
    }
}

/// The rows of a forest from one row on, in render order, each with its row.
pub struct Rows<'a> {
    drawn: &'a Drawn,
    /// The lists of siblings the walk is inside, outermost first, each with
    /// the sibling to yield next.
    next: Vec<(&'a [Node], usize)>,
    row: usize,
    end: usize,
}

impl<'a> Rows<'a> {
    fn seek(&mut self, row: usize) {
        self.next.clear();
        if row >= self.drawn.rows {
            return;
        }
        let mut nodes = self.drawn.top.as_slice();
        let mut left = row;
        loop {
            let found = nodes.iter().position(|node| {
                if left < node.rows {
                    return true;
                }
                left -= node.rows;
                false
            });
            let Some(at) = found else {
                return;
            };
            if left == 0 {
                self.next.push((nodes, at));
                return;
            }
            self.next.push((nodes, at + 1));
            left -= 1;
            nodes = nodes[at].children(self.drawn.ground());
        }
    }
}

impl<'a> Iterator for Rows<'a> {
    type Item = (usize, &'a Line);

    fn next(&mut self) -> Option<Self::Item> {
        if self.row >= self.end {
            return None;
        }
        loop {
            let (siblings, at) = self.next.last_mut()?;
            if let Some(node) = siblings.get(*at) {
                *at += 1;
                let row = self.row;
                self.row += 1;
                self.next.push((node.children(self.drawn.ground()), 0));
                return Some((row, &node.line));
            }
            self.next.pop();
        }
    }
}

/// Every line of a forest, in render order.
pub struct Lines<'a>(Rows<'a>);

impl<'a> Iterator for Lines<'a> {
    type Item = &'a Line;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|(_, line)| line)
    }
}

impl<'a> IntoIterator for &'a Drawn {
    type Item = &'a Line;
    type IntoIter = Lines<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl Index<usize> for Drawn {
    type Output = Line;

    fn index(&self, row: usize) -> &Line {
        self.get(row)
            .unwrap_or_else(|| panic!("row {row} of {} lines", self.rows))
    }
}

impl PartialEq for Drawn {
    /// Line for line. Two subtrees neither side has drawn are the same where
    /// they would be drawn from the same thing, and are drawn to compare
    /// otherwise.
    fn eq(&self, other: &Self) -> bool {
        self.rows == other.rows
            && Same {
                this: self,
                that: other,
                ground: same_ground(self.ground(), other.ground()),
            }
            .lists(&self.top, &other.top)
    }
}

impl Eq for Drawn {}

fn same_ground(this: Option<&Ground>, that: Option<&Ground>) -> bool {
    match (this, that) {
        (Some(this), Some(that)) => {
            this.beneath_shut == that.beneath_shut
                && (Arc::ptr_eq(&this.facts, &that.facts) || this.facts == that.facts)
        }
        _ => false,
    }
}

struct Same<'a> {
    this: &'a Drawn,
    that: &'a Drawn,
    /// Whether the two were drawn from the same facts, which is what lets
    /// two undrawn subtrees be called the same without drawing them.
    ground: bool,
}

impl Same<'_> {
    fn lists(&self, this: &[Node], that: &[Node]) -> bool {
        this.len() == that.len()
            && this
                .iter()
                .zip(that)
                .all(|(this, that)| self.nodes(this, that))
    }

    fn nodes(&self, this: &Node, that: &Node) -> bool {
        this.line == that.line
            && this.rows == that.rows
            && (self.undrawn_alike(this, that)
                || self.lists(
                    this.children(self.this.ground()),
                    that.children(self.that.ground()),
                ))
    }

    fn undrawn_alike(&self, this: &Node, that: &Node) -> bool {
        if !self.ground {
            return false;
        }
        let (this, that) = match (&this.beneath, &that.beneath) {
            (Beneath::Bead(this), Beneath::Bead(that))
            | (Beneath::Run(this), Beneath::Run(that)) => (this, that),
            _ => return false,
        };
        let (Some(this_tree), Some(that_tree)) = (
            self.this.tree(this.counted.tree),
            self.that.tree(that.counted.tree),
        ) else {
            return false;
        };
        this.trunk == that.trunk
            && Counted {
                tree: 0,
                ..this.counted
            } == Counted {
                tree: 0,
                ..that.counted
            }
            && (Arc::ptr_eq(this_tree, that_tree) || this_tree == that_tree)
    }
}
