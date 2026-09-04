//! The snapshot, flattened into the lines the screen shows.
//!
//! One module for what a line that folds or holds the selection is known by,
//! and one for the lines a snapshot draws under those folds. What is left
//! here is the forest itself: the folds the user has set, and where the
//! selection sits.

mod facts;
mod handle;
mod layout;

use std::collections::{BTreeMap, BTreeSet};

use crate::model::join::BeadKey;
use crate::model::snapshot::{Filter, Snapshot, Tree};
use crate::view::lines::{beneath, links_below, quiet, root_key, Content, GroupKind, Line, Place};
use crate::view::{Action, Motion};

use facts::Facts;
use handle::{handle_of, selectable, Folds, Handle};

/// How far a half-screen motion moves until the renderer says otherwise.
const HALF_SCREEN: usize = 10;

/// One snapshot's lines in render order, with the fold state and the selection
/// that decide which of them are visible and which one is current.
pub struct Forest {
    snapshot: Snapshot,
    /// What layout reads of the snapshot, answered when it was taken.
    facts: Facts,
    folds: Folds,
    cursor: Option<Handle>,
    half_screen: usize,
    lines: Vec<Line>,
    selected: usize,
}

/// Flatten a snapshot into its lines.
pub fn flatten(snapshot: Snapshot) -> Forest {
    let mut forest = Forest {
        facts: Facts::of(&snapshot),
        snapshot,
        folds: Folds::default(),
        cursor: None,
        half_screen: HALF_SCREEN,
        lines: Vec::new(),
        selected: 0,
    };
    forest.lay_out();
    forest.select_first_root();
    forest
}

impl Forest {
    /// The visible lines, in render order.
    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    /// Open on the first root rather than on the project above it. A project
    /// is not a bead, so it has no pane, and a screen that opens with its tail
    /// band empty has spent it saying nothing.
    fn select_first_root(&mut self) {
        let first = self
            .lines
            .iter()
            .position(|line| matches!(line.content, Content::Bead(_) | Content::Unread(_)));
        if let Some(at) = first {
            self.selected = at;
            self.cursor = self.handle_at(at);
        }
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

    /// Take a freshly collected snapshot, keeping the folds, the filter and
    /// the selection.
    pub fn refresh(&mut self, mut snapshot: Snapshot) {
        // Only the snapshot the cursor was found in knows what stood above
        // it, so where the new one has dropped the bead the cursor falls to
        // the nearest of its forebears that survived.
        let ancestry = self.ancestry();
        let folded_over = self.folded_over();
        // A collection carries the filter the command line asked for, which
        // is nobody's answer to `a`. So the one in hand goes on the new
        // snapshot, exactly as the folds and the cursor do.
        snapshot.refilter(self.snapshot.filter);
        self.take(snapshot);
        self.spend_folds(&folded_over);
        self.cursor = ancestry.into_iter().find(|handle| self.present(handle));
        self.lay_out();
    }

    /// The live work each fold the user shut is currently shut over.
    fn folded_over(&self) -> BTreeMap<Handle, BTreeSet<BeadKey>> {
        self.folds
            .shut()
            .map(|handle| (handle.clone(), self.live_under(handle)))
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
        self.folds.spend(&spent);
    }

    /// The beads beneath `handle` carrying live work. Empty for anything but
    /// a bead: a run holds only finished branches, and a group's items are
    /// not beads at all.
    fn live_under(&self, handle: &Handle) -> BTreeSet<BeadKey> {
        let Handle::Bead(place) = handle else {
            return BTreeSet::new();
        };
        let Some((tree, way)) = self.locate(place) else {
            return BTreeSet::new();
        };
        let (at, above) = way.split_last().expect("a way down ends somewhere");
        beneath(tree, *at, above)
            .into_iter()
            .filter(|node| !quiet(&tree.beads[*node]))
            .map(|node| BeadKey {
                project: tree.project.clone(),
                id: tree.beads[node].id.clone(),
            })
            .collect()
    }

    /// What the cursor is on, then everything above it, nearest first: the
    /// beads above it in its tree, the group holding the tree where the
    /// filter has put it in one, and the project.
    fn ancestry(&self) -> Vec<Handle> {
        self.ancestry_of(self.cursor.as_ref())
    }

    /// The same chain above any line, so what a lost cursor falls back to and
    /// what has to be opened to reach a line are one answer rather than two
    /// that agree until the forest changes shape.
    fn ancestry_of(&self, from: Option<&Handle>) -> Vec<Handle> {
        let mut chain: Vec<Handle> = from.into_iter().cloned().collect();
        let place = match from {
            Some(Handle::Bead(place)) => place,
            // A run is only ever seen from the bead it hangs under, so that
            // bead is the first forebear a lost run falls back to.
            Some(Handle::Elided(place)) => {
                chain.push(Handle::Bead(place.clone()));
                place
            }
            // Only the snapshot a pane was found in still knows which group
            // held it, so a pane that goes away falls back to that group
            // rather than to the top of the forest — and to the project the
            // group is one of, where it is.
            Some(Handle::Item(key)) => {
                if let Some(group) = layout::group_holding(&self.snapshot, key) {
                    if let Handle::Group(_, Some(project)) = &group {
                        chain.push(Handle::Project(project.clone()));
                    }
                    chain.insert(1, group);
                }
                return chain;
            }
            Some(Handle::Group(_, Some(project))) => {
                chain.push(Handle::Project(project.clone()));
                return chain;
            }
            _ => return chain,
        };
        chain.extend(place.forebears().map(Handle::Bead));
        if self.hidden(&place.tree) {
            chain.push(Handle::Group(
                GroupKind::HiddenTrees,
                Some(place.tree.project.clone()),
            ));
        }
        chain.push(Handle::Project(place.tree.project.clone()));
        chain
    }

    /// Whether the filter is holding the tree a root names back.
    fn hidden(&self, root: &BeadKey) -> bool {
        self.snapshot
            .hidden_trees
            .iter()
            .any(|hidden| hidden.project == root.project && hidden.root == root.id)
    }

    /// The tree a place was drawn in and the way down it, as the beads
    /// stepped through from the root to the one its last step lands on.
    ///
    /// The steps are walked rather than the last of them looked up, because a
    /// bead reachable more than once is drawn more than once and only the way
    /// down to a copy tells it from its twins — and the way down is what
    /// every question about the copy is asked with.
    fn locate(&self, place: &Place) -> Option<(&Tree, Vec<usize>)> {
        let tree = self.tree_of(place)?;
        let mut way = vec![(!tree.beads.is_empty()).then_some(0)?];
        for step in &place.steps {
            let (at, above) = way.split_last().expect("a way down ends somewhere");
            let next = links_below(tree, *at, above)
                .into_iter()
                .find(|link| tree.beads[link.bead].id == step.id)?
                .bead;
            way.push(next);
        }
        Some((tree, way))
    }

    fn tree_of(&self, place: &Place) -> Option<&Tree> {
        self.snapshot.tree(&place.tree)
    }

    /// Apply one action, reporting whether it changed anything.
    ///
    /// Focusing a pane, copying a bead's id, showing a bead or the key
    /// bindings and going back from them, re-collecting and quitting are the
    /// loop's to do, and none of them changes what is on screen here.
    pub fn apply(&mut self, action: Action) -> bool {
        let selected = self.selected;
        match action {
            Action::Move(motion) => self.move_to(motion),
            Action::CollapseOrParent => self.collapse_or_parent(),
            Action::ExpandOrChild => self.expand_or_child(),
            Action::ToggleFold => self.toggle_fold(),
            Action::ExpandSubtree => self.fold_subtree(true),
            Action::CollapseSubtree => self.fold_subtree(false),
            Action::RestoreDefault => self.folds.clear(),
            Action::ToggleFilter => self.toggle_filter(),
            Action::Focus
            | Action::ShowBead
            | Action::NextRelated
            | Action::Back
            | Action::CopyId
            | Action::ShowBindings
            | Action::Refresh
            | Action::Quit => return false,
        }
        let was = self.lay_out();
        self.selected != selected || self.lines != was
    }

    fn toggle_filter(&mut self) {
        let next = match self.snapshot.filter {
            Filter::LiveAgents => Filter::All,
            Filter::All => Filter::LiveAgents,
        };
        self.snapshot.refilter(next);
        self.answer();
    }

    /// Take a snapshot as the one drawn.
    fn take(&mut self, snapshot: Snapshot) {
        self.snapshot = snapshot;
        self.answer();
    }

    /// Answer what layout reads of the snapshot in hand, here and not per
    /// keystroke.
    fn answer(&mut self) {
        self.facts = Facts::of(&self.snapshot);
    }

    /// `E` and `C`: point every fold in the selected node's subtree, at every
    /// depth, one way.
    ///
    /// The subtree and not the forest, because a reader who has navigated to
    /// one project, or to one bead with a deep subtree, is asking about that
    /// and would lose their place among rows they never asked to see. `D` is
    /// the key that still speaks for the whole forest.
    ///
    /// Opening a node draws children that were not there to be enumerated, so
    /// the subtree is opened a level at a time until a draw turns up nothing
    /// left shut. Shutting goes the same way round first: a fold the reader
    /// cannot see is still a fold, and one left open under a shut parent
    /// would spring its subtree back the moment that parent was opened again.
    /// That opening walk is scoped as well as the shutting one, or `C` on a
    /// node would throw the rest of the forest open on the way past.
    ///
    /// The scope is named once and reused, because the walk redraws between
    /// passes and a line's place in the draw moves under it. It is a handle
    /// rather than a bead, so a bead reachable more than once puts only the
    /// way down the selection took inside the scope.
    ///
    /// The walks draw without laying out, because `apply` tells the loop
    /// whether the screen moved by comparing against the lines its own
    /// lay-out displaced — one in here would leave it comparing the new lines
    /// with themselves.
    fn fold_subtree(&mut self, open: bool) {
        let Some(scope) = self.handle_at(self.selected) else {
            return;
        };
        self.fold_subtree_in(&scope, self.rounds_to_settle(), open);
    }

    /// The rounds a forest of this size can need, counted off the snapshot
    /// before the walk starts.
    ///
    /// A round points every fold on a drawn line, so the next one reaches
    /// the folds that round drew and no deeper: the walk spends a round per
    /// level of fold rather than one per fold, and a round that points none
    /// is the last. A fold nests inside another only where a bead hangs
    /// under a bead, with at most the run of quiet children between the two,
    /// so a tree holds two levels per bead it has — and the beads down one
    /// path are all different, because a way down that comes back to a bead
    /// it came through is cut there. The two left over are the fold a
    /// project or a group draws over what hangs beneath it, and the round
    /// that finds nothing left to point.
    ///
    /// Generous on purpose: a level too many costs one draw that finds
    /// nothing, a level too few stops a walk that was still working. Which is
    /// also what lets a walk over one subtree take it — a count for the whole
    /// snapshot is loose there and still an upper bound.
    ///
    /// Counted off the snapshot, and off nothing a fold or a drawn line
    /// says, so it still counts out where the walk is wrong about what it
    /// drew.
    fn rounds_to_settle(&self) -> usize {
        let beads: usize = self
            .snapshot
            .collected
            .iter()
            .map(|tree| tree.beads.len())
            .sum();
        2 * beads + 2
    }

    /// Point every fold in `scope`'s subtree at `open`, in at most `rounds`
    /// of them.
    ///
    /// A walk that runs out has set a fold and drawn it again unchanged,
    /// which is a defect: the subtree is left at the level it reached, and
    /// the folds it never got to are drawn shut like any other, so the
    /// screen still says truthfully which way every fold points.
    fn fold_subtree_in(&mut self, scope: &Handle, rounds: usize, open: bool) {
        for _ in 0..rounds {
            if !self.point_every_drawn_fold(scope, true) {
                break;
            }
        }
        if !open {
            self.point_every_drawn_fold(scope, false);
        }
    }

    /// Point every fold drawn in `scope`'s subtree at `open`, reporting
    /// whether any of them was pointing the other way.
    fn point_every_drawn_fold(&mut self, scope: &Handle, open: bool) -> bool {
        let drawn = layout::draw(&self.snapshot, &self.facts, &self.folds);
        let pointed: Vec<Handle> = subtree_of(&drawn, scope)
            .iter()
            .filter(|line| line.folded == Some(!open))
            .filter_map(handle_of)
            .collect();
        for handle in &pointed {
            self.folds.set(handle.clone(), open);
        }
        !pointed.is_empty()
    }

    fn toggle_fold(&mut self) {
        if let (Some(open), Some(handle)) =
            (self.fold_at(self.selected), self.handle_at(self.selected))
        {
            self.folds.set(handle, !open);
        }
    }

    /// `h`: shut an open node, and step out of one already shut.
    fn collapse_or_parent(&mut self) {
        match (self.fold_at(self.selected), self.handle_at(self.selected)) {
            (Some(true), Some(handle)) => {
                self.folds.set(handle, false);
            }
            _ => self.step_to(self.parent_of(self.selected)),
        }
    }

    /// `l`: open a shut node, and step into one already open.
    fn expand_or_child(&mut self) {
        match (self.fold_at(self.selected), self.handle_at(self.selected)) {
            (Some(false), Some(handle)) => {
                self.folds.set(handle, true);
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

    /// Put the selection on the line at `at`, reporting whether it moved.
    ///
    /// Named rather than stepped to, and that is the whole difference from a
    /// motion: a line the selection cannot rest on keeps none, because the
    /// pointer named that line and not the one below it. Which lines those
    /// are is the keyboard's question, asked here in the keyboard's words.
    pub fn select_line(&mut self, at: usize) -> bool {
        let was = self.selected;
        if self.lines.get(at).is_some_and(selectable) {
            self.step_to(Some(at));
        }
        self.selected != was
    }

    /// Where the selection sits, where it sits on a bead at all.
    ///
    /// A place and not a key, because a bead drawn more than once is drawn
    /// once per way down to it, and a caller keeping this to come back to is
    /// keeping the copy the reader was looking at.
    pub fn place(&self) -> Option<&Place> {
        self.lines.get(self.selected)?.place.as_ref()
    }

    /// Put the selection on a bead the snapshot holds, wherever the forest
    /// draws it, opening whatever is folded over it. Reports whether the
    /// selection is on it now.
    pub fn go_to(&mut self, key: &BeadKey) -> bool {
        let Some(place) = self.place_of(key) else {
            return false;
        };
        self.go_to_place(&place)
    }

    /// Put the selection back on a line it held before, opening whatever has
    /// been folded over it since. Reports whether the selection is on it now,
    /// which it is not where the snapshot has stopped drawing that line.
    pub fn go_to_place(&mut self, place: &Place) -> bool {
        if !self.drawn(place) {
            return false;
        }
        self.open_over(place);
        self.cursor = Some(Handle::Bead(place.clone()));
        self.lay_out();
        self.cursor.as_ref() == Some(&Handle::Bead(place.clone()))
    }

    /// Whether the forest can take the reader to a bead: whether any tree it
    /// holds draws one.
    ///
    /// Asked of the snapshot rather than worked out from how roots are
    /// found, so it goes on being the same question when they are found
    /// another way.
    pub fn draws(&self, key: &BeadKey) -> bool {
        self.snapshot.locate(key).is_some()
    }

    /// Where the forest first draws a bead, going down the trees in the order
    /// the screen draws them: the trees the filter shows before the ones it
    /// hid, and within a tree the first way down that reaches it.
    ///
    /// The first copy rather than the shallowest, because that is the one a
    /// reader scanning down the screen would have found themselves.
    fn place_of(&self, key: &BeadKey) -> Option<Place> {
        self.snapshot
            .trees
            .iter()
            .chain(&self.snapshot.collected)
            .filter(|tree| tree.project == key.project)
            .find_map(|tree| way_to(tree, &key.id))
    }

    /// Open everything shut over a line: everything the line hangs under, and
    /// the run of quiet children each forebear may be counting it in.
    ///
    /// The run is asked for beside each forebear rather than left to the
    /// ancestry, which answers for a line that is drawn — and a bead inside a
    /// run is not drawn at all, so nothing above it can name it.
    ///
    /// Set rather than let go of, because a fold the reader shut by hand
    /// stays shut until something asks otherwise, and asking to be taken to a
    /// bead underneath it is asking.
    fn open_over(&mut self, place: &Place) {
        let over = self.ancestry_of(Some(&Handle::Bead(place.clone())));
        // Past the line itself, whose own fold is about the children under it
        // rather than about reaching it.
        for above in over.into_iter().skip(1) {
            if let Handle::Bead(under) = &above {
                self.folds.set(Handle::Elided(under.clone()), true);
            }
            self.folds.set(above, true);
        }
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

    /// Redraw, and put the selection back on whatever it was holding, handing
    /// back the lines the redraw displaced.
    ///
    /// They are moved out rather than copied. This runs on every keystroke,
    /// and a caller asking whether the screen moved can compare the two sets
    /// without duplicating either.
    fn lay_out(&mut self) -> Vec<Line> {
        self.settle_cursor();
        let drawn = layout::draw(&self.snapshot, &self.facts, &self.folds);
        let was = std::mem::replace(&mut self.lines, drawn);
        if self.find_cursor().is_none() {
            // The line the cursor named is not drawn — an ancestor is folded
            // over it, or the tracker stopped reporting it. Take the nearest
            // line that is. Nothing needs drawing again for it: the fold
            // state no longer turns on where the selection sits.
            self.cursor = self
                .forebear_drawn_over_the_cursors_hidden_tree()
                .or_else(|| {
                    self.scan(self.selected, false)
                        .or_else(|| self.scan(self.selected, true))
                        .and_then(|at| self.handle_at(at))
                });
        }
        self.selected = self.find_cursor().unwrap_or(0);
        was
    }

    /// Where the filter has taken the cursor's tree into the group holding
    /// its project's hidden trees, the nearest of the cursor's forebears that
    /// is drawn: the tree's root, resting shut over the cursor, where the
    /// group is open, and the group's line where it is shut. Either is where
    /// the tree went, which is more than the nearest row can say.
    ///
    /// Only the cursor's own tree going into the group does this. A fold
    /// shutting over the selection in a tree still drawn leaves it on the
    /// nearest row, as it always has.
    fn forebear_drawn_over_the_cursors_hidden_tree(&self) -> Option<Handle> {
        let (Handle::Bead(place) | Handle::Elided(place)) = self.cursor.as_ref()? else {
            return None;
        };
        if !self.hidden(&place.tree) {
            return None;
        }
        self.ancestry()
            .into_iter()
            .skip(1)
            .find(|forebear| self.line_holding(forebear).is_some())
    }

    /// Which line carries a handle, where one does.
    fn line_holding(&self, handle: &Handle) -> Option<usize> {
        (0..self.lines.len()).find(|at| self.handle_at(*at).as_ref() == Some(handle))
    }

    fn settle_cursor(&mut self) {
        if self.cursor.as_ref().is_some_and(|held| self.present(held)) {
            return;
        }
        self.cursor = self.first_handle();
    }

    /// Which line the cursor is on. A bead reachable more than once is drawn
    /// once per way down to it, and the handle names the way down, so exactly
    /// one line carries it — which is what lets a step onto the lower copy
    /// survive the redraw that follows it.
    fn find_cursor(&self) -> Option<usize> {
        self.line_holding(self.cursor.as_ref()?)
    }

    fn first_handle(&self) -> Option<Handle> {
        if let Some(tree) = self.snapshot.trees.first() {
            return Some(Handle::Bead(Place::root(root_key(tree))));
        }
        layout::every_group(&self.snapshot)
            .find(|(kind, project)| layout::group_drawn(&self.snapshot, *kind, project.as_deref()))
            .map(|(kind, project)| Handle::Group(kind, project))
    }

    /// Whether the snapshot still holds what a handle names.
    fn present(&self, handle: &Handle) -> bool {
        match handle {
            Handle::Bead(place) | Handle::Elided(place) => self.drawn(place),
            Handle::Group(kind, project) => {
                layout::group_drawn(&self.snapshot, *kind, project.as_deref())
            }
            Handle::Item(key) => layout::group_holding(&self.snapshot, key).is_some(),
            Handle::Project(project) => layout::project_drawn(&self.snapshot, project),
        }
    }

    /// Whether the snapshot still draws the line a place names.
    fn drawn(&self, place: &Place) -> bool {
        // A tracker that could not be read keeps its root and has no nodes,
        // so its header is drawn with nothing beneath it to walk to.
        if place.steps.is_empty() {
            return self.tree_of(place).is_some();
        }
        self.locate(place).is_some()
    }
}

/// The first way down a tree that reaches a bead, as the place the line at
/// the end of it stands on. Nothing where the tree holds no such bead.
///
/// The walk carries the beads it came through, which is what cuts a loop: a
/// way down that comes back to a bead it came through stops there, so a
/// cyclic tree is walked once rather than for ever.
fn way_to(tree: &Tree, id: &str) -> Option<Place> {
    let root = Place::root(root_key(tree));
    if tree.beads.first()?.id == id {
        return Some(root);
    }
    stepped_to(tree, id, 0, &[], &root)
}

/// The first way down from `at` that reaches a bead, given the beads stepped
/// through to reach `at` and the place it stands on.
fn stepped_to(tree: &Tree, id: &str, at: usize, above: &[usize], place: &Place) -> Option<Place> {
    let mut way = above.to_vec();
    way.push(at);
    links_below(tree, at, above).into_iter().find_map(|link| {
        let stepped = place.step_to(BeadKey {
            project: tree.project.clone(),
            id: tree.beads[link.bead].id.clone(),
        });
        if tree.beads[link.bead].id == id {
            return Some(stepped);
        }
        stepped_to(tree, id, link.bead, &way, &stepped)
    })
}

/// The line `scope` names, and everything drawn beneath it.
///
/// Beneath is depth: the lines after it, up to the first one standing at its
/// own depth or shallower. A project is depth zero, the roots under it are
/// one, and a group's things are one under a group that is also zero, so the
/// scope of a project stops at the next project or the first group.
///
/// Nothing at all where `scope` is not drawn, which leaves the walk with no
/// fold to point and stops it.
fn subtree_of<'a>(drawn: &'a [Line], scope: &Handle) -> &'a [Line] {
    let Some(at) = drawn
        .iter()
        .position(|line| handle_of(line).as_ref() == Some(scope))
    else {
        return &[];
    };
    let depth = drawn[at].depth;
    let end = drawn[at + 1..]
        .iter()
        .position(|line| line.depth <= depth)
        .map_or(drawn.len(), |past| at + 1 + past);
    &drawn[at..end]
}

#[cfg(test)]
mod tests {
    use super::facts::TreeFacts;
    use super::*;
    use crate::collect::bd::parse_beads;
    use crate::collect::herdr::parse_agent_list;
    use crate::config::{Config, Scope};
    use crate::model::join::{self, Joined, ProjectRows};
    use crate::model::snapshot;
    use crate::model::snapshot::{
        a_provider, build_tree, Collected, FailedProject, ProviderState, Readiness, TrackerFailure,
        TrackerState, A_PROVIDER,
    };
    use crate::model::tree::{self, Assembled, Nesting};
    use crate::model::types::testing::{key as pane_key, A_SESSION};
    use crate::model::types::{Bead, Pane};
    use crate::view::lines::{
        counts_beneath, facts_of, links_below, marker, prefix, progress_of, run_size, split,
        walks_on_this_thread, way_below, Group, Item, Note, ProjectLine, OPEN, SHUT,
    };
    use crate::view::phrase;
    use crate::view::row::{Progress, Row};
    use crate::view::walk::{self, Rows};
    use chrono::{DateTime, Utc};
    use pretty_assertions::assert_eq;
    use std::sync::Arc;

    /// Orbital's tree as bd writes it. `orb-7.7` waits on a bead no row holds,
    /// so the tree reports it; `orb-7.1.2` is a node bd stopped at; `orb-7.4`
    /// is closed with a pane still on it, and the other three closed siblings
    /// are finished.
    const ORBITAL: &str = r#"[
      {"id":"orb-7","title":"lift the ground station","status":"in_progress",
       "priority":1,"issue_type":"epic","updated_at":"2026-08-29T12:00:00Z",
       "metadata":{"agent_pane":"w:p1"}},
      {"id":"orb-7.1","title":"re-point the dish","status":"open",
       "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
       "priority":2,"issue_type":"task"},
      {"id":"orb-7.1.1","title":"true the mount","status":"open",
       "dependencies":[{"depends_on_id":"orb-7.1","type":"parent-child"}],
       "priority":2,"issue_type":"task"},
      {"id":"orb-7.1.2","title":"seal the feed horn","status":"open",
       "dependencies":[{"depends_on_id":"orb-7.1","type":"parent-child"}],
       "priority":3,"issue_type":"task"},
      {"id":"orb-7.2","title":"survey the mast","status":"closed",
       "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
       "priority":2,"issue_type":"task","closed_at":"2026-08-28T09:00:00Z"},
      {"id":"orb-7.3","title":"pour the pad","status":"closed",
       "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
       "priority":2,"issue_type":"task","closed_at":"2026-08-27T09:00:00Z"},
      {"id":"orb-7.4","title":"clear the access road","status":"closed",
       "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
       "priority":2,"issue_type":"task","closed_at":"2026-08-26T09:00:00Z",
       "metadata":{"agent_pane":"w:p2"}},
      {"id":"orb-7.5","title":"set the guard rail","status":"closed",
       "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
       "priority":2,"issue_type":"task","closed_at":"2026-08-25T09:00:00Z"},
      {"id":"orb-7.7","title":"log the survey marks","status":"open",
       "priority":2,"issue_type":"task",
       "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"},
                       {"depends_on_id":"orb-6","type":"blocks"}]}
    ]"#;

    /// Harbour's tree. Nobody is working in it, so the live-agent filter hides
    /// it.
    const HARBOUR: &str = r#"[
      {"id":"hbr-3","title":"dredge the channel","status":"open",
       "priority":2,"issue_type":"epic"},
      {"id":"hbr-3.1","title":"survey the silt","status":"open",
       "dependencies":[{"depends_on_id":"hbr-3","type":"parent-child"}],
       "priority":2,"issue_type":"task"}
    ]"#;

    /// A second root, nobody working in it either, holding a bead that waits
    /// on a row the tracker never returned. Whichever project files it, the
    /// tree is hidden with a finding still in it.
    const SLIPWAY: &str = r#"[
      {"id":"hbr-9","title":"re-deck the slipway","status":"open",
       "priority":2,"issue_type":"epic"},
      {"id":"hbr-9.1","title":"strip the planking","status":"open",
       "dependencies":[{"depends_on_id":"hbr-9","type":"parent-child"},
                       {"depends_on_id":"hbr-4","type":"blocks"}],
       "priority":2,"issue_type":"task"}
    ]"#;

    /// A tree whose run of finished siblings has a finished run of its own, so
    /// an opened run still has something left to count inside it. Three at each
    /// level, which is what it takes to make a run.
    const DEPOT: &str = r#"[
      {"id":"dep-1","title":"re-lay the sidings","status":"in_progress",
       "priority":1,"issue_type":"epic"},
      {"id":"dep-1.1","title":"grade the bed","status":"open",
       "dependencies":[{"depends_on_id":"dep-1","type":"parent-child"}],
       "priority":2,"issue_type":"task"},
      {"id":"dep-1.2","title":"lift the old rail","status":"closed",
       "dependencies":[{"depends_on_id":"dep-1","type":"parent-child"}],
       "priority":2,"issue_type":"task","closed_at":"2026-08-28T09:00:00Z"},
      {"id":"dep-1.2.1","title":"cut the fishplates","status":"closed",
       "dependencies":[{"depends_on_id":"dep-1.2","type":"parent-child"}],
       "priority":2,"issue_type":"task","closed_at":"2026-08-27T09:00:00Z"},
      {"id":"dep-1.2.2","title":"stack the chairs","status":"closed",
       "dependencies":[{"depends_on_id":"dep-1.2","type":"parent-child"}],
       "priority":2,"issue_type":"task","closed_at":"2026-08-27T09:00:00Z"},
      {"id":"dep-1.2.3","title":"draw the spikes","status":"closed",
       "dependencies":[{"depends_on_id":"dep-1.2","type":"parent-child"}],
       "priority":2,"issue_type":"task","closed_at":"2026-08-27T09:00:00Z"},
      {"id":"dep-1.3","title":"clear the ballast","status":"closed",
       "dependencies":[{"depends_on_id":"dep-1","type":"parent-child"}],
       "priority":2,"issue_type":"task","closed_at":"2026-08-26T09:00:00Z"},
      {"id":"dep-1.4","title":"burn the sleepers","status":"closed",
       "dependencies":[{"depends_on_id":"dep-1","type":"parent-child"}],
       "priority":2,"issue_type":"task","closed_at":"2026-08-25T09:00:00Z"}
    ]"#;

    /// The shape `bdi-4av` was raised on, with the stale-pane case beside it.
    /// `rly-2.2` and `rly-2.4` are both closed and unmanned, so a rule that
    /// asks its question of the sibling alone sweeps both into the run —
    /// burying a working agent two levels under one of them and a stale-pane
    /// warning under the other. `rly-2.3`, `rly-2.5` and `rly-2.6` are
    /// finished all the way down, and are what a run may honestly hold.
    const RELAY: &str = r#"[
      {"id":"rly-2","title":"re-site the relay","status":"in_progress",
       "priority":1,"issue_type":"epic"},
      {"id":"rly-2.1","title":"trench the run","status":"open",
       "dependencies":[{"depends_on_id":"rly-2","type":"parent-child"}],
       "priority":2,"issue_type":"task"},
      {"id":"rly-2.2","title":"strike the old mast","status":"closed",
       "dependencies":[{"depends_on_id":"rly-2","type":"parent-child"}],
       "priority":2,"issue_type":"task","closed_at":"2026-08-28T09:00:00Z"},
      {"id":"rly-2.2.1","title":"drop the guys","status":"closed",
       "dependencies":[{"depends_on_id":"rly-2.2","type":"parent-child"}],
       "priority":2,"issue_type":"task","closed_at":"2026-08-27T09:00:00Z"},
      {"id":"rly-2.2.1.1","title":"cut the stays","status":"in_progress",
       "dependencies":[{"depends_on_id":"rly-2.2.1","type":"parent-child"}],
       "priority":2,"issue_type":"task","updated_at":"2026-08-29T12:00:00Z",
       "metadata":{"agent_pane":"w:p1"}},
      {"id":"rly-2.3","title":"back-fill the pad","status":"closed",
       "dependencies":[{"depends_on_id":"rly-2","type":"parent-child"}],
       "priority":2,"issue_type":"task","closed_at":"2026-08-26T09:00:00Z"},
      {"id":"rly-2.4","title":"lift the feeder","status":"closed",
       "dependencies":[{"depends_on_id":"rly-2","type":"parent-child"}],
       "priority":2,"issue_type":"task","closed_at":"2026-08-26T09:00:00Z"},
      {"id":"rly-2.4.1","title":"coil the heliax","status":"closed",
       "dependencies":[{"depends_on_id":"rly-2.4","type":"parent-child"}],
       "priority":2,"issue_type":"task","closed_at":"2026-08-25T09:00:00Z",
       "metadata":{"agent_pane":"w:p2"}},
      {"id":"rly-2.5","title":"seed the spoil","status":"closed",
       "dependencies":[{"depends_on_id":"rly-2","type":"parent-child"}],
       "priority":2,"issue_type":"task","closed_at":"2026-08-25T09:00:00Z"},
      {"id":"rly-2.5.1","title":"rake the batter","status":"closed",
       "dependencies":[{"depends_on_id":"rly-2.5","type":"parent-child"}],
       "priority":2,"issue_type":"task","closed_at":"2026-08-24T09:00:00Z"},
      {"id":"rly-2.6","title":"sign the handover","status":"closed",
       "dependencies":[{"depends_on_id":"rly-2","type":"parent-child"}],
       "priority":2,"issue_type":"task","closed_at":"2026-08-24T09:00:00Z"}
    ]"#;

    /// A spine four beads deep, with a quiet branch of its own beside it.
    /// Nothing in it is closed, in progress or staffed, so the only thing
    /// that can open a fold in it is a pane the test puts on a bead.
    const TOWER: &str = r#"[
      {"id":"tow-1","title":"raise the tower","status":"open",
       "priority":1,"issue_type":"epic"},
      {"id":"tow-1.1","title":"stand the mast","status":"open",
       "dependencies":[{"depends_on_id":"tow-1","type":"parent-child"}],
       "priority":2,"issue_type":"task"},
      {"id":"tow-1.1.1","title":"bolt the sections","status":"open",
       "dependencies":[{"depends_on_id":"tow-1.1","type":"parent-child"}],
       "priority":2,"issue_type":"task"},
      {"id":"tow-1.1.1.1","title":"dress the cables","status":"open",
       "dependencies":[{"depends_on_id":"tow-1.1.1","type":"parent-child"}],
       "priority":2,"issue_type":"task"},
      {"id":"tow-1.2","title":"pour the base","status":"open",
       "dependencies":[{"depends_on_id":"tow-1","type":"parent-child"}],
       "priority":2,"issue_type":"task"},
      {"id":"tow-1.2.1","title":"tie the rebar","status":"open",
       "dependencies":[{"depends_on_id":"tow-1.2","type":"parent-child"}],
       "priority":2,"issue_type":"task"}
    ]"#;

    /// A closed bead standing over work that is still to do. A blocker is
    /// drawn beneath the bead it blocks, so `sdg-4.1`'s descendants are the
    /// work closing it unblocked — the ordinary shape of this tree, not a
    /// malformed one. Nobody is on any of them and nothing is wrong with
    /// them, so the branch rests shut under a row whose own glyph says done.
    /// `sdg-4.2` is finished all the way down. `sdg-4.3` carries the only
    /// pane, which is what opens the root, and rests shut over unfinished
    /// work of its own without ever claiming to be done.
    const SIDING: &str = r#"[
      {"id":"sdg-4","title":"re-point the crossover","status":"in_progress",
       "priority":1,"issue_type":"epic"},
      {"id":"sdg-4.1","title":"slew the up line","status":"closed",
       "dependencies":[{"depends_on_id":"sdg-4","type":"parent-child"}],
       "priority":2,"issue_type":"task","closed_at":"2026-08-28T09:00:00Z"},
      {"id":"sdg-4.1.1","title":"key the switch","status":"closed",
       "dependencies":[{"depends_on_id":"sdg-4.1","type":"parent-child"}],
       "priority":2,"issue_type":"task","closed_at":"2026-08-27T09:00:00Z"},
      {"id":"sdg-4.1.1.1","title":"gauge the check rail","status":"open",
       "dependencies":[{"depends_on_id":"sdg-4.1.1","type":"parent-child"}],
       "priority":2,"issue_type":"task"},
      {"id":"sdg-4.1.1.2","title":"pack the timbers","status":"open",
       "dependencies":[{"depends_on_id":"sdg-4.1.1","type":"parent-child"}],
       "priority":2,"issue_type":"task"},
      {"id":"sdg-4.1.2","title":"weld the closure rail","status":"open",
       "dependencies":[{"depends_on_id":"sdg-4.1","type":"parent-child"}],
       "priority":2,"issue_type":"task"},
      {"id":"sdg-4.1.3","title":"lift the old chairs","status":"closed",
       "dependencies":[{"depends_on_id":"sdg-4.1","type":"parent-child"}],
       "priority":2,"issue_type":"task","closed_at":"2026-08-26T09:00:00Z"},
      {"id":"sdg-4.2","title":"clip the down line","status":"closed",
       "dependencies":[{"depends_on_id":"sdg-4","type":"parent-child"}],
       "priority":2,"issue_type":"task","closed_at":"2026-08-26T09:00:00Z"},
      {"id":"sdg-4.2.1","title":"torque the fishbolts","status":"closed",
       "dependencies":[{"depends_on_id":"sdg-4.2","type":"parent-child"}],
       "priority":2,"issue_type":"task","closed_at":"2026-08-25T09:00:00Z"},
      {"id":"sdg-4.3","title":"re-signal the box","status":"in_progress",
       "dependencies":[{"depends_on_id":"sdg-4","type":"parent-child"}],
       "priority":2,"issue_type":"task"},
      {"id":"sdg-4.3.1","title":"prove the interlocking","status":"open",
       "dependencies":[{"depends_on_id":"sdg-4.3","type":"parent-child"}],
       "priority":2,"issue_type":"task"}
    ]"#;

    /// A configured root that is a leaf. `[roots]` names a bead and nothing
    /// requires that bead to have children, so a tracker whose root is one
    /// bead deep draws a line that is a root and holds no fold. It is the
    /// only shape here where being a root and having children come apart,
    /// and every other fixture answers both questions the same way.
    ///
    /// It names no pane, like Depot, Tower and Siding, so a test staffs it by
    /// naming the bead and one that does not gets the quiet shape.
    const BEACON: &str = r#"[
      {"id":"bcn-6","title":"re-lamp the beacon","status":"in_progress",
       "priority":1,"issue_type":"task","updated_at":"2026-08-29T12:00:00Z"}
    ]"#;

    /// One epic whose two halves are each held up by the same survey. Under
    /// the rule that a bead's descendants are what must finish before it,
    /// `orb-9` is drawn beneath both of them: under `orb-8.1` as its child,
    /// and under `orb-8.2` as what it waits on.
    const TWICE: &str = r#"[
      {"id":"orb-8","title":"lift the gantry","status":"in_progress",
       "priority":1,"issue_type":"epic"},
      {"id":"orb-8.1","title":"pour the pad","status":"in_progress",
       "dependencies":[{"depends_on_id":"orb-8","type":"parent-child"}],
       "priority":2,"issue_type":"task"},
      {"id":"orb-8.2","title":"rail the crane","status":"in_progress",
       "dependencies":[{"depends_on_id":"orb-8","type":"parent-child"},
                       {"depends_on_id":"orb-9","type":"blocks"}],
       "priority":2,"issue_type":"task"},
      {"id":"orb-9","title":"survey the ground","status":"in_progress",
       "dependencies":[{"depends_on_id":"orb-8.1","type":"parent-child"}],
       "priority":2,"issue_type":"task"},
      {"id":"orb-9.1","title":"drill the cores","status":"in_progress",
       "dependencies":[{"depends_on_id":"orb-9","type":"parent-child"}],
       "priority":2,"issue_type":"task"}
    ]"#;

    /// A closed bead holding unfinished work, drawn twice in one tree: under
    /// `orb-5.1` as its child, and under `orb-5.2` as what it waits on. Both
    /// copies rest shut, because nothing under either is live or ready, so
    /// what each of them says about the work it is shut over is all that
    /// tells them apart.
    ///
    /// `orb-5.1.1` and `orb-5.2.1` are here to be worked on: they are what
    /// holds the two halves open, so both copies of `orb-4` are on the
    /// screen at once.
    const CLOSED_TWICE: &str = r#"[
      {"id":"orb-5","title":"re-deck the bridge","status":"in_progress",
       "priority":1,"issue_type":"epic"},
      {"id":"orb-5.1","title":"strip the north span","status":"in_progress",
       "dependencies":[{"depends_on_id":"orb-5","type":"parent-child"}],
       "priority":2,"issue_type":"task"},
      {"id":"orb-5.1.1","title":"cut the north deck","status":"in_progress",
       "dependencies":[{"depends_on_id":"orb-5.1","type":"parent-child"}],
       "priority":2,"issue_type":"task"},
      {"id":"orb-4","title":"close the towpath","status":"closed",
       "dependencies":[{"depends_on_id":"orb-5.1","type":"parent-child"}],
       "priority":2,"issue_type":"task","closed_at":"2026-08-28T09:00:00Z"},
      {"id":"orb-4.1","title":"post the diversion","status":"open",
       "dependencies":[{"depends_on_id":"orb-4","type":"parent-child"}],
       "priority":2,"issue_type":"task"},
      {"id":"orb-5.2","title":"strip the south span","status":"in_progress",
       "dependencies":[{"depends_on_id":"orb-5","type":"parent-child"},
                       {"depends_on_id":"orb-4","type":"blocks"}],
       "priority":2,"issue_type":"task"},
      {"id":"orb-5.2.1","title":"cut the south deck","status":"in_progress",
       "dependencies":[{"depends_on_id":"orb-5.2","type":"parent-child"}],
       "priority":2,"issue_type":"task"}
    ]"#;

    /// One tree drawing one bead twice, which is the shape a blocker nested
    /// under each bead it holds up gives: same root, same key, two lines.
    fn drawn_twice_in_one_tree() -> Snapshot {
        alone("orbital", TWICE, &panes_on(&["orb-9.1"]))
    }

    /// The same, over a closed bead that still holds unfinished work.
    fn closed_bead_drawn_twice_in_one_tree() -> Snapshot {
        alone(
            "orbital",
            CLOSED_TWICE,
            &panes_on(&["orb-5.1.1", "orb-5.2.1"]),
        )
    }

    /// A run whose branches share a blocker. `lck-2` holds up both halves of
    /// the refit, so it is drawn beneath each of them, and the four branches
    /// that are closed and unmanned collapse into one run — five beads drawn
    /// on six rows.
    ///
    /// The only shape where counting beads and counting rows disagree: every
    /// other fixture's runs draw each of their beads once. `lck-1.5` is the
    /// work still to do, and is what holds the root open so the run is on
    /// the screen at all.
    const SHARED_IN_A_RUN: &str = r#"[
      {"id":"lck-1","title":"refit the lock gates","status":"in_progress",
       "priority":1,"issue_type":"epic"},
      {"id":"lck-1.5","title":"hang the new gates","status":"in_progress",
       "dependencies":[{"depends_on_id":"lck-1","type":"parent-child"}],
       "priority":2,"issue_type":"task"},
      {"id":"lck-1.1","title":"drain the upper chamber","status":"closed",
       "dependencies":[{"depends_on_id":"lck-1","type":"parent-child"},
                       {"depends_on_id":"lck-2","type":"blocks"}],
       "priority":2,"issue_type":"task","closed_at":"2026-08-28T09:00:00Z"},
      {"id":"lck-1.2","title":"drain the lower chamber","status":"closed",
       "dependencies":[{"depends_on_id":"lck-1","type":"parent-child"},
                       {"depends_on_id":"lck-2","type":"blocks"}],
       "priority":2,"issue_type":"task","closed_at":"2026-08-27T09:00:00Z"},
      {"id":"lck-1.3","title":"scarf the mitre posts","status":"closed",
       "dependencies":[{"depends_on_id":"lck-1","type":"parent-child"}],
       "priority":2,"issue_type":"task","closed_at":"2026-08-26T09:00:00Z"},
      {"id":"lck-1.4","title":"re-seat the paddles","status":"closed",
       "dependencies":[{"depends_on_id":"lck-1","type":"parent-child"}],
       "priority":2,"issue_type":"task","closed_at":"2026-08-25T09:00:00Z"},
      {"id":"lck-2","title":"stop off the pound","status":"closed",
       "priority":2,"issue_type":"task","closed_at":"2026-08-24T09:00:00Z"}
    ]"#;

    /// Two of one project's roots whose trees overlap. `qua-1.2` blocks both
    /// epics, and a blocker is drawn beneath every bead it blocks, so it
    /// comes back under each of them. Roots are found by climbing the parent
    /// chain and trees by walking dependents, so a bead standing in two trees
    /// is the ordinary shape of shared work, not a malformed tracker.
    const QUARRY: &str = r#"[
      {"id":"qua-1","title":"re-open the quarry","status":"in_progress",
       "priority":1,"issue_type":"epic"},
      {"id":"qua-1.2","title":"cut the haul road","status":"in_progress",
       "dependencies":[{"depends_on_id":"qua-1","type":"parent-child"}],
       "priority":2,"issue_type":"task"},
      {"id":"qua-1.2.1","title":"strip the overburden","status":"in_progress",
       "dependencies":[{"depends_on_id":"qua-1.2","type":"parent-child"}],
       "priority":2,"issue_type":"task"}
    ]"#;

    /// The second of the pair, drawn below Quarry, so the shared bead's lower
    /// copy sits here with more rows under it. Those are the bottom of the
    /// whole list, and are what a selection sprung back up to the upper copy
    /// never reaches.
    ///
    /// The shared bead has a child in each tree, and not the same one, so
    /// each copy is a line that folds over a list of its own.
    const WHARF: &str = r#"[
      {"id":"wha-2","title":"re-face the wharf","status":"in_progress",
       "priority":1,"issue_type":"epic"},
      {"id":"wha-2.1","title":"drive the piles","status":"in_progress",
       "dependencies":[{"depends_on_id":"wha-2","type":"parent-child"}],
       "priority":2,"issue_type":"task"},
      {"id":"qua-1.2","title":"cut the haul road","status":"in_progress",
       "dependencies":[{"depends_on_id":"wha-2","type":"parent-child"}],
       "priority":2,"issue_type":"task"},
      {"id":"wha-2.2","title":"grout the cope","status":"in_progress",
       "dependencies":[{"depends_on_id":"qua-1.2","type":"parent-child"}],
       "priority":2,"issue_type":"task"},
      {"id":"wha-2.3","title":"bed the fenders","status":"in_progress",
       "dependencies":[{"depends_on_id":"wha-2","type":"parent-child"}],
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

    /// One tree with a row edited, where the edit is required to land.
    ///
    /// `str::replace` says nothing when it matches nothing, so a pattern that
    /// drifts from the const it edits leaves the test asserting against the
    /// untouched tree and still passing.
    fn edited(json: &str, from: &str, to: &str) -> String {
        let out = json.replace(from, to);
        assert_ne!(out, json, "no row matched {from:?}");
        out
    }

    /// The root of a hand-written tree: the one row that depends on nothing.
    fn root_row(beads: &[crate::model::types::Bead]) -> String {
        beads
            .iter()
            .find(|b| b.dependencies.is_empty())
            .expect("a root row")
            .id
            .clone()
    }

    fn assembled(json: &str) -> Assembled {
        let beads = parse_beads(json).expect("the rows parse");
        let root = root_row(&beads);
        Nesting::of(&beads)
            .assemble(&root)
            .expect("the rows assemble")
    }

    fn panes() -> Vec<Pane> {
        parse_agent_list(A_SESSION, PANES).expect("the panes parse")
    }

    /// One working pane and one idle one, both in Orbital's tree. Enough to
    /// staff a fixture without the conflicting and unconfigured panes the
    /// shared snapshot carries to exercise its groups.
    const TWO_PANES: &str = r#"{"result":{"agents":[
      {"pane_id":"w:p1","cwd":"/srv/work/orbital","agent_status":"working"},
      {"pane_id":"w:p2","cwd":"/srv/work/orbital","agent_status":"idle"}
    ]}}"#;

    fn two_panes() -> Vec<Pane> {
        parse_agent_list(A_SESSION, TWO_PANES).expect("the panes parse")
    }

    fn joined(orbital: &Assembled, harbour: &Assembled, panes: &[Pane]) -> Joined {
        let cfg = cfg();
        join::resolve(
            &[
                ProjectRows {
                    project: "orbital",
                    rows: &orbital.beads,
                },
                ProjectRows {
                    project: "harbour",
                    rows: &harbour.beads,
                },
            ],
            panes,
            &cfg,
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
            &BTreeMap::new(),
            ProviderState::Answering,
            &cfg(),
            now(),
        )
    }

    /// The first way down to bead `at`: the beads above it on the way the
    /// walk first reached it, the root first.
    fn above(tree: &Tree, at: usize) -> Vec<usize> {
        let mut way = Vec::new();
        let mut reached = at;
        while reached != 0 {
            reached = (0..tree.beads.len())
                .find(|from| {
                    tree.children[*from]
                        .iter()
                        .any(|link| link.bead == reached && link.first)
                })
                .expect("every bead but the root was first reached under one");
            way.push(reached);
        }
        way.reverse();
        way
    }

    /// A bead by id, and the first way down to it.
    fn way_to(tree: &Tree, id: &str) -> (usize, Vec<usize>) {
        let at = tree
            .beads
            .iter()
            .position(|bead| bead.id == id)
            .unwrap_or_else(|| panic!("{id} is in the tree"));
        (at, above(tree, at))
    }

    /// Every configured project, read at `now`.
    ///
    /// These fixtures are collections that have come back, so every project
    /// has been read whether or not its tracker had a root to show for it.
    /// That is what tells them from the first frame of a run, where nothing
    /// has been read and every project is still waiting on one.
    fn every_project_read() -> std::collections::BTreeMap<String, chrono::DateTime<chrono::Utc>> {
        cfg()
            .projects
            .iter()
            .map(|project| (project.name.clone(), now()))
            .collect()
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
                read_at: every_project_read(),
            },
            &panes,
            &joined,
            &cfg(),
            a_provider(ProviderState::Answering),
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
                tracker: TrackerFailure::Unstartable,
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
            Content::Project(line) => line.project.clone(),
            Content::Unread(unread) => format!("⚠ {} unread", unread.root),
            Content::Bead(row) => format!("{} {} {}", row.glyph, row.id, row.title),
            Content::Elided { count, .. } => format!("… {count} more"),
            Content::Note(note) => format!("! {note:?}"),
            Content::Group(group) => format!(
                "[{:?}{}] {}",
                group.kind,
                group
                    .project
                    .as_ref()
                    .map(|project| format!(" {project}"))
                    .unwrap_or_default(),
                group.count
            ),
            Content::Item(item) => format!("- {item:?}"),
            Content::Scoped { project } => format!("~ reading {project}"),
        }
    }

    /// The drawn row for one bead, found by the whole id its line carries
    /// rather than the abbreviated one it shows.
    fn row_of<'a>(forest: &'a Forest, id: &str) -> &'a Row {
        forest
            .lines()
            .iter()
            .find_map(|line| match (line.bead(), &line.content) {
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
            .filter(|(_, line)| line.bead().is_some_and(|bead| bead.id == id))
            .map(|(at, _)| at)
            .collect()
    }

    /// Step down from the top to the last row, reporting where the selection
    /// sat at each step.
    fn walk_down(forest: &mut Forest) -> Vec<usize> {
        forest.apply(Action::Move(Motion::FirstRow));
        let mut visited = vec![forest.selected_line()];
        walk::until(
            forest,
            |forest| forest.selected_line() + 1 == forest.rows(),
            |forest| {
                forest.apply(Action::Move(Motion::NextRow));
                visited.push(forest.selected_line());
            },
            |forest| {
                format!(
                    "stepping down stopped at row {} of {}: {:#?}",
                    forest.selected_line(),
                    forest.rows(),
                    sketch(forest)
                )
            },
        );
        visited
    }

    /// A bead reachable from two roots is drawn in both their trees, and the
    /// selection has to be able to sit on either copy. `find_cursor` took the
    /// first line carrying the handle, so the redraw that follows every
    /// action pulled a step onto the lower copy back up to the upper one, and
    /// the list below it could not be walked into at all.
    #[test]
    fn stepping_down_past_a_bead_drawn_twice_reaches_the_bottom_of_the_list() {
        let mut forest = flatten(overlapping(&panes_on(&["qua-1.2", "wha-2.1"])));
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

    /// A tracker that refused keeps its root and reports no nodes, so there
    /// is no way down from its header to walk. The header is a line all the
    /// same, and holding the selection on it across a refresh is what says
    /// the forest knows a tree by its root rather than by a node.
    #[test]
    fn the_selection_holds_the_line_of_a_root_whose_tracker_refused() {
        let mut forest = flatten(snapshot());
        let header = forest
            .lines()
            .iter()
            .position(
                |line| matches!(&line.content, Content::Unread(unread) if unread.root == "fer-2"),
            )
            .expect("the shared snapshot draws a tree whose tracker refused");

        step_onto(&mut forest, header);
        forest.refresh(snapshot());

        assert_eq!(forest.selected_line(), header, "{:#?}", sketch(&forest));
    }

    /// The forest holds the trees it was handed, not copies of them. A
    /// collection landing while the reader moves is a stall on top of the
    /// keystroke it arrives beside, and copying every tree into the forest
    /// was most of that stall.
    #[test]
    fn the_forest_holds_the_trees_it_was_handed_rather_than_copies() {
        let handed = snapshot();
        let trees = handed.trees.clone();
        let mut forest = flatten(handed);
        assert_held_exactly(&forest, &trees);

        let again = snapshot();
        let trees = again.trees.clone();
        forest.refresh(again);
        assert_held_exactly(&forest, &trees);
    }

    fn assert_held_exactly(forest: &Forest, trees: &[Arc<Tree>]) {
        assert_eq!(forest.snapshot().trees.len(), trees.len());
        for (held, was) in forest.snapshot().trees.iter().zip(trees) {
            assert!(Arc::ptr_eq(held, was), "{} was copied", was.root);
            assert_eq!(
                Arc::strong_count(was),
                3,
                "{} is held by collected, trees and this test, and nothing else",
                was.root
            );
        }
    }

    /// Each copy of a bead drawn twice folds over a list of its own, so
    /// shutting one says nothing about the other. Both carried the same
    /// handle, so one keystroke shut them both and the reader lost a list
    /// they had never been looking at.
    #[test]
    fn folding_one_copy_of_a_bead_drawn_twice_leaves_the_other_open() {
        let mut forest = flatten(overlapping(&panes_on(&["qua-1.2", "wha-2.1"])));
        let [upper, lower] = copies_of(&forest, "qua-1.2");

        step_onto(&mut forest, lower);
        forest.apply(Action::ToggleFold);

        assert_eq!(
            forest.lines()[upper].folded,
            Some(true),
            "{:#?}",
            sketch(&forest)
        );
    }

    /// The same bead under two parents in one tree, which is what a `blocks`
    /// edge drawn as nesting gives: the copies share a root as well as a key,
    /// so nothing but the way down to them tells them apart.
    ///
    /// Each copy is opened against the way the other one rests, so a fold
    /// remembered against the bead rather than against the line would have to
    /// give one of them the other's answer.
    #[test]
    fn folding_one_copy_of_a_bead_drawn_twice_in_one_tree_leaves_the_other_alone() {
        let mut forest = flatten(drawn_twice_in_one_tree());
        let [_, lower] = copies_of(&forest, "orb-9");

        step_onto(&mut forest, lower);
        forest.apply(Action::ToggleFold);
        let [upper, _] = copies_of(&forest, "orb-9");
        step_onto(&mut forest, upper);
        forest.apply(Action::ToggleFold);

        let [upper, lower] = copies_of(&forest, "orb-9");
        assert_eq!(
            (forest.lines()[upper].folded, forest.lines()[lower].folded),
            (Some(false), Some(true)),
            "{:#?}",
            sketch(&forest)
        );
    }

    /// A bead reached more than one way down is one piece of work with a line
    /// each, and the first line is the one that stands for it. The rest are
    /// shut, so the subtree is drawn once however many ways there are into it.
    #[test]
    fn a_bead_drawn_twice_in_one_tree_rests_open_on_the_first_line_and_shut_on_the_second() {
        let forest = flatten(drawn_twice_in_one_tree());
        let [upper, lower] = copies_of(&forest, "orb-9");

        assert_eq!(
            (forest.lines()[upper].folded, forest.lines()[lower].folded),
            (Some(true), Some(false)),
            "{:#?}",
            sketch(&forest)
        );
    }

    /// Shut, never absent: the later line is a way into the same subtree, and
    /// opening it draws the subtree there too.
    #[test]
    fn the_second_line_of_a_bead_drawn_twice_opens_onto_the_same_subtree() {
        let mut forest = flatten(drawn_twice_in_one_tree());
        let [_, lower] = copies_of(&forest, "orb-9");
        assert_eq!(
            lines_of(&forest, "orb-9.1").len(),
            1,
            "{:#?}",
            sketch(&forest)
        );

        step_onto(&mut forest, lower);
        forest.apply(Action::ToggleFold);

        assert_eq!(
            lines_of(&forest, "orb-9.1").len(),
            2,
            "{:#?}",
            sketch(&forest)
        );
    }

    /// `D` lets go of every fold set by hand, so a reader who opened a later
    /// line gets it back the way `bdi` would have drawn it.
    #[test]
    fn letting_go_of_the_folds_shuts_a_second_line_a_reader_opened() {
        let mut forest = flatten(drawn_twice_in_one_tree());
        let [_, lower] = copies_of(&forest, "orb-9");
        step_onto(&mut forest, lower);
        forest.apply(Action::ToggleFold);

        forest.apply(Action::RestoreDefault);

        let [upper, lower] = copies_of(&forest, "orb-9");
        assert_eq!(
            (forest.lines()[upper].folded, forest.lines()[lower].folded),
            (Some(true), Some(false)),
            "{:#?}",
            sketch(&forest)
        );
    }

    /// A shut closed line says what unfinished work it is shut over, because
    /// those beads are then nowhere else on the screen. On a later line of the
    /// same bead they are somewhere else on the screen — on the first line —
    /// so it says nothing, and the count a reader reads is the work rather
    /// than the ways down to it.
    #[test]
    fn only_the_first_line_of_a_bead_drawn_twice_says_what_it_is_shut_over() {
        let forest = flatten(closed_bead_drawn_twice_in_one_tree());
        let [upper, lower] = copies_of(&forest, "orb-4");

        assert_eq!(
            (notes_at(&forest, upper), notes_at(&forest, lower)),
            (vec![phrase::unfinished_beneath(1)], Vec::new()),
            "{:#?}",
            sketch(&forest)
        );
    }

    /// What a bead row says beyond its own fields, on one line.
    fn notes_at(forest: &Forest, at: usize) -> Vec<String> {
        match &forest.lines()[at].content {
            Content::Bead(row) => row.notes.clone(),
            content => panic!("line {at} is not a bead row: {content:?}"),
        }
    }

    /// The two lines a bead is drawn on, asserted to be exactly two so a
    /// fixture that stopped overlapping fails here rather than further down.
    fn copies_of(forest: &Forest, id: &str) -> [usize; 2] {
        let copies = lines_of(forest, id);
        let [upper, lower] = copies[..] else {
            panic!(
                "{id} is drawn {} times: {:#?}",
                copies.len(),
                sketch(forest)
            );
        };
        [upper, lower]
    }

    /// Put the selection on a line by stepping down onto it, which is the
    /// only road a reader has to it.
    fn step_onto(forest: &mut Forest, at: usize) {
        forest.apply(Action::Move(Motion::FirstRow));
        walk::until(
            forest,
            |forest| forest.selected_line() == at,
            |forest| {
                forest.apply(Action::Move(Motion::NextRow));
            },
            |forest| {
                format!(
                    "the selection never reached line {at}: {:#?}",
                    sketch(forest)
                )
            },
        );
    }

    /// The tracker is written while the list is being read, so a refresh can
    /// land between any two keystrokes. It re-derives the selection from the
    /// handle it holds, and must settle on the copy the selection was on
    /// rather than on that copy's twin higher up the list.
    #[test]
    fn a_refresh_between_steps_does_not_pull_the_selection_back_to_a_twin() {
        let panes = panes_on(&["qua-1.2", "wha-2.1"]);
        let mut forest = flatten(overlapping(&panes));
        let drawn = forest.lines().len();

        forest.apply(Action::Move(Motion::FirstRow));
        let mut visited = vec![forest.selected_line()];
        for _ in 1..drawn {
            forest.apply(Action::Move(Motion::NextRow));
            forest.refresh(overlapping(&panes));
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
        forest.lines()[forest.selected_line()].bead()
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
        ready_together(project, &[json], panes, ready)
    }

    /// Several roots of one project, joined together so that a pane the
    /// fixture names lands on whichever root's bead names it.
    fn together(project: &str, jsons: &[&str], panes: &[Pane]) -> Snapshot {
        ready_together(project, jsons, panes, &[])
    }

    fn ready_together(project: &str, jsons: &[&str], panes: &[Pane], ready: &[&str]) -> Snapshot {
        let roots: Vec<Assembled> = jsons.iter().map(|json| assembled(json)).collect();
        let rows: Vec<Bead> = roots
            .iter()
            .flat_map(|root| root.beads.iter().cloned())
            .collect();
        let cfg = cfg();
        let joined = join::resolve(
            &[ProjectRows {
                project,
                rows: &rows,
            }],
            panes,
            &cfg,
        );
        let readiness = Readiness {
            ready: ready.iter().map(|id| (*id).to_string()).collect(),
            ..Readiness::default()
        };
        let trees = roots
            .iter()
            .map(|root| {
                build_tree(
                    project,
                    root,
                    &joined,
                    &readiness,
                    &BTreeMap::new(),
                    ProviderState::Answering,
                    &cfg,
                    now(),
                )
            })
            .collect();
        snapshot::build(
            Collected {
                trees,
                failed_projects: Vec::new(),
                read_at: every_project_read(),
            },
            panes,
            &joined,
            &cfg,
            a_provider(ProviderState::Answering),
            Filter::All,
            now(),
        )
    }

    /// Put the selection on the first elided run, by moving down to it. It
    /// carries no bead, so `select` cannot reach it.
    fn select_run(forest: &mut Forest) {
        forest.apply(Action::Move(Motion::FirstRow));
        walk::until(
            forest,
            |forest| {
                matches!(
                    forest.lines()[forest.selected_line()].content,
                    Content::Elided { .. }
                )
            },
            |forest| {
                forest.apply(Action::Move(Motion::NextRow));
            },
            |_| "no elided run is reachable by moving down".to_string(),
        );
    }

    fn select(forest: &mut Forest, bead: &BeadKey) {
        forest.apply(Action::Move(Motion::FirstRow));
        walk::until(
            forest,
            |forest| cursor(forest) == Some(bead),
            |forest| {
                forest.apply(Action::Move(Motion::NextRow));
            },
            |_| format!("{bead:?} is not reachable by moving down"),
        );
    }

    #[test]
    fn a_snapshot_flattens_to_the_lines_the_design_draws() {
        let forest = flatten(snapshot());

        assert_eq!(
            sketch(&forest),
            vec![
                "▾ orbital",
                "  ├── ◐ orb-7 lift the ground station",
                "  │   ├── ! Dangling(1)",
                "  │   ├─▸ ○ .1 re-point the dish",
                "  │   ├── ○ .7 log the survey marks",
                "  │   ├── ✓ .4 clear the access road",
                "  │   └─▸ … 3 more",
                "  └── [Unattributed orbital] 2",
                "      ├── - Loose(LoosePane { pane: PaneKey { session: \"default\", id: \"w:p3\" }, project: \"orbital\", cwd: \"/srv/work/orbital\", pane_status: Working, display_agent: Some(\"orb-7.1\"), title: None })",
                "      └── - Loose(LoosePane { pane: PaneKey { session: \"default\", id: \"w:p4\" }, project: \"orbital\", cwd: \"/srv/work/orbital\", pane_status: Idle, display_agent: Some(\"orb-7.1\"), title: None })",
                "▾ ferry",
                "  ├── ⚠ fer-2 unread",
                "  └── [Unattributed ferry] 1",
                "      └── - Loose(LoosePane { pane: PaneKey { session: \"default\", id: \"w:p9\" }, project: \"ferry\", cwd: \"/srv/work/ferry\", pane_status: Blocked, display_agent: None, title: None })",
                "▾ harbour",
                "  └─▸ [HiddenTrees harbour] 1",
                "▸ [FailedProjects] 1",
                "▾ [Unconfigured] 1",
                "  └── - Unconfigured(UnconfiguredPane { pane: PaneKey { session: \"default\", id: \"w:pF\" }, cwd: \"/srv/spike\", pane_status: Idle })",
                "▾ [Conflicts] 1",
                "  └── - Conflict(SeveralPanesNameOneBead { bead: BeadKey { project: \"orbital\", id: \"orb-7.1\" }, panes: [PaneKey { session: \"default\", id: \"w:p3\" }, PaneKey { session: \"default\", id: \"w:p4\" }] })",
            ]
        );
    }

    /// The selection starts on the first root and not on the project line
    /// above it: a project has no pane, so opening there would spend the tail
    /// band saying there is nothing to show.
    #[test]
    fn the_selection_starts_on_the_first_root() {
        let forest = flatten(snapshot());

        assert_eq!(forest.selected_line(), 1);
        assert_eq!(cursor(&forest), Some(&key("orbital", "orb-7")));
    }

    /// The fold state is the user's and the live work's, and moving is
    /// neither: walking out of a tree leaves it exactly as it was drawn.
    #[test]
    fn a_root_stays_as_it_was_when_the_selection_walks_out_of_it() {
        let mut forest = flatten(snapshot());
        let was = sketch(&forest);

        forest.apply(Action::Move(Motion::LastRow));

        assert_eq!(sketch(&forest), was);
    }

    /// Compared on content alone: folding also redraws the header's marker
    /// and the elbow on what is now the last line under it, and neither of
    /// those is a line the fold removed.
    #[test]
    fn folding_a_root_removes_exactly_its_subtree() {
        let mut forest = flatten(snapshot());
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
        let mut forest = flatten(snapshot());
        forest.apply(Action::ToggleFold);

        assert_eq!(
            sketch(&forest)[..3],
            [
                "▾ orbital",
                "  ├─▸ ◐ orb-7 lift the ground station",
                "  │   └── ! Dangling(1)",
            ]
        );
    }

    #[test]
    fn a_fold_made_by_hand_outlives_moving_away_from_it() {
        let mut forest = flatten(snapshot());
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
        parse_agent_list(
            A_SESSION,
            &format!(r#"{{"result":{{"agents":[{}]}}}}"#, agents.join(",")),
        )
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
        let mut rows = quarry.beads.clone();
        rows.extend(wharf.beads.clone());
        let joined = join::resolve(
            &[ProjectRows {
                project: "orbital",
                rows: &rows,
            }],
            panes,
            &cfg,
        );
        let tree = |rows: &Assembled| {
            build_tree(
                "orbital",
                rows,
                &joined,
                &Readiness::default(),
                &BTreeMap::new(),
                ProviderState::Answering,
                &cfg,
                now(),
            )
        };
        snapshot::build(
            Collected {
                trees: vec![tree(&quarry), tree(&wharf)],
                failed_projects: Vec::new(),
                read_at: every_project_read(),
            },
            panes,
            &joined,
            &cfg,
            a_provider(ProviderState::Answering),
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
            .find(|line| line.bead().is_some_and(|key| key.id == id))
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
            "▾ orbital",
            "  └── ○ tow-1 raise the tower",
            "      ├── ○ .1 stand the mast",
            "      │   └── ○ .1.1 bolt the sections",
            "      │       └── ○ .1.1.1 dress the cables",
            "      └─▸ ○ .2 pour the base",
        ];

        let staffed = flatten(tower_staffed(&["tow-1.1.1.1"]));
        let ready = flatten(tower_ready(&["tow-1.1.1.1"]));

        assert_eq!(sketch(&staffed), opened);
        assert_eq!(sketch(&ready), opened);
        for forest in [&staffed, &ready] {
            for forebear in ["tow-1", "tow-1.1", "tow-1.1.1"] {
                assert_eq!(fold_of(forest, forebear), Some(true), "{forebear} is shut");
            }
            assert_eq!(fold_of(forest, "tow-1.2"), Some(false));
        }
    }

    /// The third case, and the one a claim makes: a seat has taken the bead
    /// and no pane has joined it yet. `bd ready` drops a bead the moment it
    /// goes `in_progress`, so nothing here is ready and nobody is staffed,
    /// and the forebears open anyway.
    ///
    /// They open on the anomaly. A claim with no pane behind it is an
    /// `orphan-claim` from the first collection, and that is what `quiet`
    /// answers to — so the rule keeping a booting seat's bead on screen lives
    /// in `model/anomaly.rs`, not in this file. Narrow that rule and this
    /// goes red, which is the whole reason it is written down here.
    #[test]
    fn the_default_opens_every_forebear_of_a_bead_someone_has_claimed() {
        let claimed = edited(
            TOWER,
            r#"{"id":"tow-1.1.1.1","title":"dress the cables","status":"open","#,
            r#"{"id":"tow-1.1.1.1","title":"dress the cables","status":"in_progress",
       "updated_at":"2026-08-30T11:00:00Z","#,
        );
        let forest = flatten(ready_alone("orbital", &claimed, &[], &[]));

        assert_eq!(
            sketch(&forest),
            vec![
                "▾ orbital",
                "  └── ○ tow-1 raise the tower",
                "      ├── ○ .1 stand the mast",
                "      │   └── ○ .1.1 bolt the sections",
                "      │       └── ◐ .1.1.1 dress the cables",
                "      └─▸ ○ .2 pour the base",
            ]
        );
        for forebear in ["tow-1", "tow-1.1", "tow-1.1.1"] {
            assert_eq!(fold_of(&forest, forebear), Some(true), "{forebear} is shut");
        }
    }

    /// The other half of the same rule. A tree nobody is working holds no
    /// spine to open, so it rests as the one line saying it is there.
    #[test]
    fn a_tree_with_nothing_live_in_it_rests_as_its_header() {
        let forest = flatten(tower_staffed(&[]));

        assert_eq!(
            sketch(&forest),
            vec!["▾ orbital", "  └─▸ ○ tow-1 raise the tower"]
        );
    }

    /// A default, not a lock: the user shuts a node holding an agent and it
    /// stays shut, refresh after refresh, for as long as what is under there
    /// is what they folded away.
    #[test]
    fn a_fold_set_by_hand_survives_a_refresh_that_brings_nothing_new_under_it() {
        let mut forest = flatten(tower_staffed(&["tow-1.1.1.1"]));
        select(&mut forest, &key("orbital", "tow-1.1"));
        forest.apply(Action::ToggleFold);

        forest.refresh(tower_staffed(&["tow-1.1.1.1"]));

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
        let mut forest = flatten(tower_staffed(&["tow-1.1.1.1"]));
        select(&mut forest, &key("orbital", "tow-1.1"));
        forest.apply(Action::ToggleFold);

        forest.refresh(tower_staffed(&["tow-1.2.1"]));

        assert_eq!(fold_of(&forest, "tow-1.1"), Some(false));
    }

    /// The hard half. A fold says *I have seen what is under here and do not
    /// want it*, which stops being true the moment something new is under it,
    /// so an agent arriving on a bead the user never folded away hands the
    /// node back to the default.
    #[test]
    fn a_fold_set_by_hand_is_spent_when_live_work_arrives_beneath_it() {
        let mut forest = flatten(tower_staffed(&["tow-1.1.1.1"]));
        select(&mut forest, &key("orbital", "tow-1.1"));
        forest.apply(Action::ToggleFold);

        forest.refresh(tower_staffed(&["tow-1.1.1.1", "tow-1.1.1"]));

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
        let mut forest = flatten(snapshot());
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
    /// In the order the groups are drawn: orbital's and ferry's loose panes,
    /// harbour's hidden tree, then the failed project, the unconfigured pane
    /// and the conflict below the trees.
    #[test]
    fn a_group_rests_open_when_what_it_holds_is_live() {
        let forest = flatten(snapshot());
        let markers: Vec<&str> = forest
            .lines()
            .iter()
            .filter_map(|line| match &line.content {
                Content::Group(_) => Some(marker(line.folded == Some(true))),
                _ => None,
            })
            .collect();

        assert_eq!(markers, vec![OPEN, OPEN, SHUT, SHUT, OPEN, OPEN]);
    }

    #[test]
    fn a_run_of_quiet_closed_siblings_collapses_to_a_count() {
        let forest = flatten(snapshot());

        assert!(sketch(&forest).contains(&"  │   └─▸ … 3 more".to_string()));
    }

    /// The count is the only account the screen gives of the beads it stands
    /// for, so the line has to be reachable to be worth anything.
    #[test]
    fn an_elided_run_can_hold_the_selection() {
        let mut forest = flatten(snapshot());

        select_run(&mut forest);

        assert_eq!(
            sketch(&forest)[forest.selected_line()],
            "  │   └─▸ … 3 more"
        );
    }

    #[test]
    fn opening_an_elided_run_draws_the_beads_it_counted() {
        let mut forest = flatten(snapshot());
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
                "  │   └── … 3 more",
                "  │       ├── ✓ .2 survey the mast",
                "  │       ├── ✓ .3 pour the pad",
                "  │       └── ✓ .5 set the guard rail",
            ]
        );
    }

    #[test]
    fn shutting_an_open_elided_run_puts_the_count_back() {
        let mut forest = flatten(snapshot());
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
        let forest = flatten(snapshot());

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
        let (at, above) = way_to(&tree, "dep-1.2");

        assert_eq!(
            progress_of(&tree, at, &above),
            Some(Progress {
                closed: 4,
                total: 4
            })
        );
    }

    /// A fraction says how much of what a bead is waiting on is done, and
    /// that is a count of beads. A blocker two of its descendants share is one
    /// piece of work whether it is drawn once or twice.
    #[test]
    fn a_fraction_counts_beads_rather_than_the_rows_they_are_drawn_on() {
        const SHARED: &str = r#"[
          {"id":"shr-1","title":"root","status":"open"},
          {"id":"shr-1.1","title":"one","status":"open",
           "dependencies":[{"depends_on_id":"shr-1","type":"parent-child"},
                           {"depends_on_id":"shr-1.9","type":"blocks"}]},
          {"id":"shr-1.2","title":"two","status":"open",
           "dependencies":[{"depends_on_id":"shr-1","type":"parent-child"},
                           {"depends_on_id":"shr-1.9","type":"blocks"}]},
          {"id":"shr-1.9","title":"what both wait on","status":"closed",
           "dependencies":[{"depends_on_id":"shr-1","type":"parent-child"}]}
        ]"#;
        let tree = tree_of("orbital", SHARED);

        assert_eq!(
            progress_of(&tree, 0, &[]),
            Some(Progress {
                closed: 1,
                total: 4
            })
        );
    }

    /// A closed bead's descendants are what had to finish before it, so beads
    /// that merely waited on it are not among them and cannot be counted into
    /// its fraction.
    #[test]
    fn a_closed_blocker_reports_no_fraction_over_the_beads_that_waited_on_it() {
        const WAITED: &str = r#"[
          {"id":"wtd-1","title":"root","status":"open"},
          {"id":"wtd-1.1","title":"waiting","status":"open",
           "dependencies":[{"depends_on_id":"wtd-1","type":"parent-child"},
                           {"depends_on_id":"wtd-1.9","type":"blocks"}]},
          {"id":"wtd-1.9","title":"done","status":"closed",
           "dependencies":[{"depends_on_id":"wtd-1","type":"parent-child"}]}
        ]"#;
        let tree = tree_of("orbital", WAITED);
        let (at, above) = way_to(&tree, "wtd-1.9");

        assert_eq!(progress_of(&tree, at, &above), None);
    }

    /// A leaf stands for itself alone, so there is nothing to be part-way
    /// through and a fraction over one bead would only repeat its glyph.
    #[test]
    fn a_bead_with_no_children_has_no_progress_to_report() {
        let forest = flatten(snapshot());

        assert_eq!(row_of(&forest, "orb-7.7").progress, None);
    }

    /// Everything a line says of the tree beneath it — how far along it is,
    /// what it is shut over, whether it rests open, what a run stands for —
    /// depends on the snapshot alone. So it is answered once, when the forest
    /// takes the snapshot, and a keystroke asks nothing of the tree.
    #[test]
    fn a_keystroke_walks_no_subtree() {
        let mut forest = flatten(built(Filter::All));
        assert!(
            forest
                .lines()
                .iter()
                .any(|line| matches!(line.content, Content::Elided { .. })),
            "the fixture draws no run, so a keystroke had no run to size"
        );
        let before = walks_on_this_thread();

        forest.apply(Action::Move(Motion::NextRow));

        assert_eq!(walks_on_this_thread() - before, 0);
    }

    /// `cyc-1.1` hangs under `cyc-1` and is blocked by it, so the walk comes
    /// back to `cyc-1` beneath `cyc-1.1` and cuts the loop there.
    const LOOPED: &str = r#"[
      {"id":"cyc-1","title":"root","status":"open"},
      {"id":"cyc-1.1","title":"one","status":"open",
       "dependencies":[{"depends_on_id":"cyc-1","type":"parent-child"},
                       {"depends_on_id":"cyc-1","type":"blocks"}]},
      {"id":"cyc-1.2","title":"two","status":"closed",
       "dependencies":[{"depends_on_id":"cyc-1.1","type":"parent-child"}]}
    ]"#;

    /// What `cyc-1.1` stands over is `cyc-1.2` alone: its forebear is above
    /// it, not beneath, and only the way down to it can say so. A bead the
    /// tree holds once can be answered once only where no way down is cut.
    #[test]
    fn a_bead_on_a_loop_counts_what_the_way_down_leaves_beneath_it() {
        let forest = flatten(alone("orbital", LOOPED, &panes_on(&["cyc-1.1"])));

        assert_eq!(
            row_of(&forest, "cyc-1.1").progress,
            Some(Progress {
                closed: 1,
                total: 2
            })
        );
    }

    /// Where no loop is cut, nothing beneath a bead can be above it, so the
    /// way down changes no answer and every copy of a bead gets the one the
    /// tree keeps for it.
    #[test]
    fn where_no_loop_is_cut_every_way_down_to_a_bead_gets_the_same_answer() {
        let fixtures = [
            ORBITAL,
            DEPOT,
            RELAY,
            SIDING,
            TOWER,
            BEACON,
            SLUICE,
            TWICE,
            CLOSED_TWICE,
            SHARED_IN_A_RUN,
        ];
        let staffed = panes_on(&[
            "orb-7.1", "dep-1.1", "rly-2.1", "sdg-4.3", "tow-1.1", "bcn-6", "slu-1.1",
        ]);
        for json in fixtures {
            let tree = alone("orbital", json, &staffed).collected.remove(0);
            assert!(tree.cycles.is_empty(), "{} has a loop", tree.root);
            let facts = TreeFacts::of(&tree);

            for (at, above) in every_way_down(&tree) {
                assert_eq!(
                    facts.bead(&tree, at, &above),
                    facts_of(&tree, at, &above),
                    "{} reached by {above:?}",
                    tree.beads[at].id
                );
            }
        }
    }

    /// Every way down the walk takes, as the bead it lands on and the beads
    /// above it.
    fn every_way_down(tree: &Tree) -> Vec<(usize, Vec<usize>)> {
        let mut ways = Vec::new();
        let mut going = vec![(0, Vec::new())];
        while let Some((at, above)) = going.pop() {
            let below = way_below(&above, at);
            going.extend(
                links_below(tree, at, &above)
                    .into_iter()
                    .map(|link| (link.bead, below.clone())),
            );
            ways.push((at, above));
        }
        ways
    }

    /// A run is drawn with one status glyph standing for every bead it hides,
    /// which is only honest while a run is closed beads and nothing else.
    /// `dep-1.1` is open beside the two closed siblings that make the run, so
    /// widening the predicate sweeps it in and fails here — rather than
    /// leaving the glyph to say `closed` over a bead that is not.
    #[test]
    fn a_run_holds_closed_beads_and_nothing_else_which_is_what_lets_one_glyph_stand_for_it() {
        let tree = tree_of("orbital", DEPOT);

        let mut runs = 0;
        for at in 0..tree.beads.len() {
            let (_, run) = split(&tree, at, &above(&tree, at));
            runs += usize::from(!run.is_empty());
            for member in run {
                let bead = &tree.beads[member.bead];
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
        let mut forest = flatten(depot());
        select_run(&mut forest);
        forest.apply(Action::ToggleFold);

        select(&mut forest, &key("orbital", "dep-1.2"));
        forest.apply(Action::ToggleFold);

        assert_eq!(
            sketch(&forest)[..7],
            [
                "▾ orbital",
                "  └── ◐ dep-1 re-lay the sidings",
                "      ├── ○ .1 grade the bed",
                "      └── … 6 more",
                "          ├── ✓ .2 lift the old rail",
                "          │   └─▸ … 3 more",
                "          ├── ✓ .3 clear the ballast",
            ]
        );
    }

    /// A run is a fold like any other, so a collection that lands under an
    /// open one leaves it open and leaves the cursor on it.
    #[test]
    fn an_open_elided_run_survives_a_refresh() {
        let mut forest = flatten(snapshot());
        select_run(&mut forest);
        forest.apply(Action::ToggleFold);

        let reordered = edited(ORBITAL, r#""priority":3"#, r#""priority":1"#);
        forest.refresh(gather(
            vec![tree_of("orbital", &reordered)],
            Vec::new(),
            Filter::LiveAgents,
        ));

        let drawn = sketch(&forest);
        assert!(
            drawn.contains(&"  │       └── ✓ .5 set the guard rail".to_string()),
            "{drawn:#?}"
        );
        assert_eq!(drawn[forest.selected_line()], "  │   └── … 3 more");
    }

    /// A run has no bead of its own, so a line the cursor is holding must not
    /// report one: the loop picks the tail's pane from that field.
    #[test]
    fn a_selected_elided_run_stands_for_no_bead_of_its_own() {
        let mut forest = flatten(snapshot());

        select_run(&mut forest);

        let line = &forest.lines()[forest.selected_line()];
        assert!(matches!(line.content, Content::Elided { .. }));
        assert_eq!(line.bead(), None);
    }

    /// A closed bead with a pane still on it is the stale-pane anomaly, and
    /// eliding it would hide a live agent.
    #[test]
    fn a_closed_bead_with_a_live_agent_is_drawn_rather_than_elided() {
        let forest = flatten(snapshot());

        assert!(sketch(&forest)
            .iter()
            .any(|line| line.contains("clear the access road")));
    }

    #[test]
    fn a_single_quiet_closed_sibling_is_drawn_rather_than_said_as_a_count() {
        let one_closed = edited(
            ORBITAL,
            r#"{"id":"orb-7.3","title":"pour the pad","status":"closed"#,
            r#"{"id":"orb-7.3","title":"pour the pad","status":"open"#,
        );
        let snapshot = gather(
            vec![tree_of("orbital", &one_closed)],
            Vec::new(),
            Filter::LiveAgents,
        );

        let drawn = sketch(&flatten(snapshot));

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
            let forest = flatten(snapshot.clone());
            let drawn: Vec<&str> = forest
                .lines()
                .iter()
                .filter_map(|line| line.bead().map(|key| key.id.as_str()))
                .collect();

            for node in &snapshot.trees[0].beads {
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
        for json in [ORBITAL, DEPOT, RELAY, SIDING, TOWER, BEACON] {
            let unstaffed = alone("orbital", json, &[]);
            let unfinished: Vec<String> = unstaffed.trees[0]
                .beads
                .iter()
                .filter(|node| !node.status.is_closed())
                .map(|node| node.id.clone())
                .collect();

            for id in unfinished {
                asked += 1;
                let forest = flatten(ready_alone("orbital", json, &[], &[&id]));
                let drawn: Vec<&str> = forest
                    .lines()
                    .iter()
                    .filter_map(|line| line.bead().map(|key| key.id.as_str()))
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
        let forest = flatten(alone("orbital", RELAY, &two_panes()));

        assert_eq!(
            sketch(&forest),
            vec![
                "▾ orbital",
                "  └── ◐ rly-2 re-site the relay",
                "      ├── ○ .1 trench the run",
                "      ├── ✓ .2 strike the old mast",
                "      │   └── ✓ .2.1 drop the guys",
                "      │       └── ◐ .2.1.1 cut the stays",
                "      ├── ✓ .4 lift the feeder",
                "      │   └── ✓ .4.1 coil the heliax",
                "      └─▸ … 4 more",
            ]
        );
    }

    /// A run's phrase says nobody is on the beads it counts, so it has to be
    /// true of every bead it counts — not merely of the siblings it names.
    /// The count and the set it describes are checked together, because it was
    /// their disagreement that let the sentence lie.
    ///
    /// The set is of beads rather than of rows, so `SHARED_IN_A_RUN` is here:
    /// it is the only fixture whose run reaches one bead two ways, and under
    /// every other one the two answers are the same number.
    #[test]
    fn a_run_counts_exactly_the_beads_its_phrase_is_true_of() {
        let mut runs = 0;
        for json in [ORBITAL, DEPOT, RELAY, SHARED_IN_A_RUN] {
            let tree = alone("orbital", json, &two_panes()).trees.remove(0);

            for at in 0..tree.beads.len() {
                let above = above(&tree, at);
                let (_, run) = split(&tree, at, &above);
                if run.is_empty() {
                    continue;
                }
                runs += 1;

                let below = way_below(&above, at);
                let mut behind = BTreeSet::new();
                let mut walking: Vec<usize> = run.iter().map(|link| link.bead).collect();
                while let Some(node) = walking.pop() {
                    let bead = &tree.beads[node];
                    behind.insert(bead.id.clone());
                    assert!(
                        bead.status.is_closed()
                            && bead.agent.is_none()
                            && bead.anomalies.is_empty(),
                        "{} is behind a run that says nobody is on it",
                        bead.id
                    );
                    walking.extend(
                        tree::links_from(&tree.children, node, &below)
                            .into_iter()
                            .map(|link| link.bead),
                    );
                }

                assert_eq!(run_size(&tree, &run, &below), behind.len());
            }
        }

        assert!(runs > 0, "the fixtures built no run to check");
    }

    /// A run says how many beads it holds, and a blocker two of its branches
    /// share is one bead however many ways down there are to it. The count
    /// stands in for beads that are not on the screen, so counting the rows
    /// it saved would say the run holds work that does not exist.
    ///
    /// `SHARED_IN_A_RUN` draws five beads on six rows, and the expected
    /// number is written out here rather than walked, because a count taken
    /// from the tree the count is about cannot disagree with it.
    #[test]
    fn a_run_counts_a_blocker_two_of_its_branches_share_once() {
        let forest = flatten(alone("orbital", SHARED_IN_A_RUN, &[]));

        assert_eq!(
            sketch(&forest),
            vec![
                "▾ orbital",
                "  └── ◐ lck-1 refit the lock gates",
                "      ├── ◐ .5 hang the new gates",
                "      └─▸ … 5 more",
            ]
        );
    }

    /// A branch that is finished all the way down is one line saying so: the
    /// glyph is its own closed status, the fraction says every bead beneath it
    /// is closed too, and the shut marker says it still holds them.
    #[test]
    fn a_wholly_finished_subtree_rests_as_one_line_that_says_it_is_finished() {
        let forest = flatten(finished_branches());

        assert_eq!(
            sketch(&forest),
            vec![
                "▾ orbital",
                "  └── ◐ dep-1 re-lay the sidings",
                "      ├── ○ .1 grade the bed",
                "      ├── ○ .3 clear the ballast",
                "      ├─▸ ✓ .2 lift the old rail",
                "      └── ✓ .4 burn the sleepers",
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

    /// A blocker is drawn beneath the bead it blocks, so a bead's children
    /// are the work closing it unblocked. A closed bead standing over open ones is
    /// therefore the healthy shape of this tree, and where nobody is on them
    /// and `bd` will start none of them the branch rests shut under a row
    /// whose glyph says done. What it holds is out of sight either way, so
    /// the line says how much.
    #[test]
    fn a_closed_branch_resting_over_unfinished_work_says_how_much_it_holds() {
        let forest = flatten(siding());

        assert_eq!(
            sketch(&forest),
            vec![
                "▾ orbital",
                "  └── ◐ sdg-4 re-point the crossover",
                "      ├─▸ ◐ .3 re-signal the box",
                "      ├─▸ ✓ .1 slew the up line",
                "      └─▸ ✓ .2 clip the down line",
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
        let forest = flatten(siding());

        assert_eq!(row_of(&forest, "sdg-4.2").notes, Vec::<String>::new());
    }

    /// Asked of the branch, not of the bead: `sdg-4.3` is unfinished itself
    /// and holds one unfinished bead, and a walk that counted the bead it was
    /// asked about would say two. Only closed nodes reach the note from the
    /// renderer, where a self that is closed adds nothing and the difference
    /// cannot show — but the same walk answers the agent count, which every
    /// shut line asks whatever its own status is.
    #[test]
    fn what_a_branch_holds_never_counts_the_bead_it_was_asked_about() {
        let tree = tree_of("orbital", SIDING);
        let (at, above) = way_to(&tree, "sdg-4.3");

        assert_eq!(counts_beneath(&tree, at, &above).unfinished(), 1);
    }

    /// Only a line whose own glyph says done. An unfinished bead resting shut
    /// over unfinished work is not hiding anything its status did not already
    /// admit, and a sentence on every such row is the noise that would stop
    /// the closed ones being read.
    #[test]
    fn an_unfinished_branch_resting_shut_over_its_own_work_says_nothing_extra() {
        let forest = flatten(siding());

        assert_eq!(row_of(&forest, "sdg-4.3").notes, Vec::<String>::new());
    }

    /// Counted at every depth. With the open bead directly under `sdg-4.1`
    /// closed, everything unfinished is two levels down, and a count of the
    /// immediate children would leave the row silent over both of them.
    #[test]
    fn unfinished_work_two_levels_under_a_closed_branch_is_still_counted() {
        let deep = edited(
            SIDING,
            r#""status":"open",
       "dependencies":[{"depends_on_id":"sdg-4.1","type":"parent-child"}]"#,
            r#""status":"closed","closed_at":"2026-08-26T09:00:00Z",
       "dependencies":[{"depends_on_id":"sdg-4.1","type":"parent-child"}]"#,
        );
        let forest = flatten(alone("orbital", &deep, &panes_on(&["sdg-4.3"])));

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
        let forest = flatten(siding());
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
        let mut forest = flatten(siding());
        select(&mut forest, &key("orbital", "sdg-4.1"));

        forest.apply(Action::ToggleFold);

        assert_eq!(row_of(&forest, "sdg-4.1").notes, Vec::<String>::new());
    }

    // ---- what a fold hides -----------------------------------------------

    /// The bead this is for. A row shut over a branch is the only thing on
    /// the screen standing for it, and until now it said its own fraction
    /// and its own agent and nothing about the seats inside it.
    ///
    /// A fold `bdi` set itself never closes over an agent, so the row that
    /// needs this is one the reader shut by hand — which is exactly when
    /// they have stopped looking at the branch and most need to be told
    /// somebody is still in it.
    #[test]
    fn a_branch_shut_over_a_working_agent_says_how_many_are_inside_it() {
        let mut forest = flatten(alone("orbital", SIDING, &panes_on(&["sdg-4.3.1"])));
        select(&mut forest, &key("orbital", "sdg-4.3"));

        forest.apply(Action::ToggleFold);

        assert_eq!(fold_of(&forest, "sdg-4.3"), Some(false));
        assert_eq!(
            row_of(&forest, "sdg-4.3")
                .shut_over
                .as_ref()
                .map(|c| c.live_agents),
            Some(1)
        );
    }

    /// One rule at every depth, which is the principle the bead is about. A
    /// root is a bead row like any other since `bdi-2bb.25`, and the aggregate
    /// it lost went to the project line and widened over every root there. So
    /// the root asks the same question a branch three levels down asks, and
    /// gets the same answer about its own tree.
    #[test]
    fn a_root_shut_over_a_working_agent_says_it_exactly_as_a_branch_does() {
        let mut forest = flatten(alone("orbital", SIDING, &panes_on(&["sdg-4.3.1"])));
        select(&mut forest, &key("orbital", "sdg-4"));

        forest.apply(Action::ToggleFold);

        assert_eq!(fold_of(&forest, "sdg-4"), Some(false));
        assert_eq!(
            row_of(&forest, "sdg-4")
                .shut_over
                .as_ref()
                .map(|c| c.live_agents),
            Some(1)
        );
    }

    /// Opened, the seats are on their own rows and counting them again above
    /// would be the same fact twice. What a line says here is what it is
    /// hiding, not a standing property of the bead.
    #[test]
    fn a_branch_opened_over_its_agents_stops_counting_them() {
        let forest = flatten(alone("orbital", SIDING, &panes_on(&["sdg-4.3.1"])));

        assert_eq!(fold_of(&forest, "sdg-4.3"), Some(true));
        assert_eq!(row_of(&forest, "sdg-4.3").shut_over, None);
    }

    /// Counted over what the fold hides and not over the bead asking. The
    /// row already says its own agent by name, and a count taking that one in
    /// would have a reader add the name to the number and come out with one
    /// agent too many.
    #[test]
    fn the_agents_a_line_counts_are_the_ones_it_hides_and_never_its_own() {
        let mut forest = flatten(alone(
            "orbital",
            SIDING,
            &panes_on(&["sdg-4.3", "sdg-4.3.1"]),
        ));
        select(&mut forest, &key("orbital", "sdg-4.3"));

        forest.apply(Action::ToggleFold);

        let row = row_of(&forest, "sdg-4.3");
        assert!(row.agent.is_some(), "the row names its own agent");
        assert_eq!(row.shut_over.as_ref().map(|c| c.live_agents), Some(1));
    }

    /// The other half of what a fold hides, and the reason it is not agents
    /// alone: `lines::live_beneath` — the whole of the fold default — is an
    /// agent on a bead *or* an anomaly against it, so a row saying one and
    /// not the other would leave a fresh exception where two were closed.
    ///
    /// A pane still on a closed bead is the stale-pane anomaly, and it
    /// carries an agent too, so the two counts are read off the one bead and
    /// cannot be answering with each other.
    #[test]
    fn a_branch_shut_over_a_bead_wanting_looking_at_says_how_many_are_inside_it() {
        let stale = edited(
            SIDING,
            r#"{"id":"sdg-4.3.1","title":"prove the interlocking","status":"open"#,
            r#"{"id":"sdg-4.3.1","title":"prove the interlocking","closed_at":"2026-08-28T09:00:00Z","status":"closed"#,
        );
        let mut forest = flatten(alone("orbital", &stale, &panes_on(&["sdg-4.3.1"])));
        select(&mut forest, &key("orbital", "sdg-4.3"));

        forest.apply(Action::ToggleFold);

        let shut_over = row_of(&forest, "sdg-4.3")
            .shut_over
            .clone()
            .expect("a shut branch says what it hides");
        assert_eq!(shut_over.anomalies, 1);
        assert_eq!(shut_over.live_agents, 1);
    }

    /// Work, not rows — the rule every other count on this screen follows. A
    /// blocker two of a branch's descendants share is drawn beneath each of
    /// them and is one seat, and a line adding its rows would send a reader
    /// hunting for a second agent that is not there.
    #[test]
    fn one_agent_reached_two_ways_down_is_counted_once() {
        let mut forest = flatten(alone("orbital", SHARED_IN_A_RUN, &panes_on(&["lck-2"])));
        assert_eq!(
            lines_of(&forest, "lck-2").len(),
            2,
            "{:#?}",
            sketch(&forest)
        );
        select(&mut forest, &key("orbital", "lck-1"));

        forest.apply(Action::ToggleFold);

        assert_eq!(
            row_of(&forest, "lck-1")
                .shut_over
                .as_ref()
                .map(|c| c.live_agents),
            Some(1)
        );
    }

    /// The second depth exception the bead names, in the same place as the
    /// first: a root shut over unfinished work said nothing, while a closed
    /// branch one line down in the same state said how much. One rule, two
    /// answers, and nothing about a root that earns the difference.
    #[test]
    fn a_closed_root_shut_over_unfinished_work_says_how_much_like_any_other_row() {
        // `sdg-4.3` opens too: `in_progress` with no pane is an orphan claim,
        // and no fold `bdi` sets itself closes over one.
        let done = edited(
            &edited(
                SIDING,
                r#"{"id":"sdg-4.3","title":"re-signal the box","status":"in_progress"#,
                r#"{"id":"sdg-4.3","title":"re-signal the box","status":"open"#,
            ),
            r#"{"id":"sdg-4","title":"re-point the crossover","status":"in_progress"#,
            r#"{"id":"sdg-4","title":"re-point the crossover","closed_at":"2026-08-29T09:00:00Z","status":"closed"#,
        );
        let forest = flatten(alone("orbital", &done, &[]));

        assert_eq!(fold_of(&forest, "sdg-4"), Some(false));
        assert_eq!(
            row_of(&forest, "sdg-4").notes,
            vec![phrase::unfinished_beneath(5)]
        );
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
        let forest = flatten(ready_alone(
            "orbital",
            SIDING,
            &panes_on(&["sdg-4.3"]),
            &["sdg-4.1.2"],
        ));

        assert_eq!(
            sketch(&forest),
            vec![
                "▾ orbital",
                "  └── ◐ sdg-4 re-point the crossover",
                "      ├─▸ ◐ .3 re-signal the box",
                "      ├── ✓ .1 slew the up line",
                "      │   ├── ○ .1.2 weld the closure rail",
                "      │   ├─▸ ✓ .1.1 key the switch",
                "      │   └── ✓ .1.3 lift the old chairs",
                "      └─▸ ✓ .2 clip the down line",
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
        let waiting = edited(
            &edited(
                SIDING,
                r#""status":"open",
       "dependencies":[{"depends_on_id":"sdg-4.1.1","type":"parent-child"}]"#,
                r#""status":"blocked",
       "dependencies":[{"depends_on_id":"sdg-4.1.1","type":"parent-child"}]"#,
            ),
            r#""status":"open",
       "dependencies":[{"depends_on_id":"sdg-4.1","type":"parent-child"}]"#,
            r#""status":"deferred",
       "dependencies":[{"depends_on_id":"sdg-4.1","type":"parent-child"}]"#,
        );
        let forest = flatten(alone("orbital", &waiting, &panes_on(&["sdg-4.3"])));

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
        let mut forest = flatten(finished_branches());
        select(&mut forest, &key("orbital", "dep-1.2"));

        forest.apply(Action::ToggleFold);

        assert_eq!(
            sketch(&forest),
            vec![
                "▾ orbital",
                "  └── ◐ dep-1 re-lay the sidings",
                "      ├── ○ .1 grade the bed",
                "      ├── ○ .3 clear the ballast",
                "      ├── ✓ .2 lift the old rail",
                "      │   └─▸ … 3 more",
                "      └── ✓ .4 burn the sleepers",
            ]
        );
    }

    /// Depot with one of its closed siblings re-opened, leaving two finished
    /// branches — under the threshold, so each keeps its own name rather than
    /// becoming a share of a count.
    fn finished_branches() -> Snapshot {
        let json = edited(
            DEPOT,
            r#"{"id":"dep-1.3","title":"clear the ballast","status":"closed"#,
            r#"{"id":"dep-1.3","title":"clear the ballast","status":"open"#,
        );
        alone("orbital", &json, &panes_on(&["dep-1.1"]))
    }

    #[test]
    fn the_selection_survives_a_refresh_that_reorders_the_nodes() {
        let mut forest = flatten(snapshot());
        open(&mut forest, &key("orbital", "orb-7.1"));
        select(&mut forest, &key("orbital", "orb-7.1.2"));
        let was = forest.selected_line();

        let reordered = edited(ORBITAL, r#""priority":3"#, r#""priority":1"#);
        forest.refresh(gather(
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
        let mut forest = flatten(snapshot());
        open(&mut forest, &key("orbital", "orb-7.1"));
        select(&mut forest, &key("orbital", "orb-7.1.2"));

        let without = edited(
            ORBITAL,
            r#"{"id":"orb-7.1.2","title":"seal the feed horn","status":"open",
       "dependencies":[{"depends_on_id":"orb-7.1","type":"parent-child"}],
       "priority":3,"issue_type":"task"},"#,
            "",
        );
        forest.refresh(gather(
            vec![tree_of("orbital", &without)],
            Vec::new(),
            Filter::LiveAgents,
        ));

        assert_eq!(cursor(&forest), Some(&key("orbital", "orb-7.1")));
    }

    /// The bead the selection was on is gone with its whole tree, and the
    /// nearest of its forebears the new snapshot still draws is its
    /// project's line: orbital still has panes working in it that no bead
    /// claims, so the line is still there to fall back to.
    #[test]
    fn a_refresh_that_drops_the_selected_bead_leaves_the_selection_somewhere_real() {
        let mut forest = flatten(snapshot());
        open(&mut forest, &key("orbital", "orb-7.1"));
        select(&mut forest, &key("orbital", "orb-7.1.2"));
        // Harbour has no live agent, so it is a tree only a reader showing
        // every tree can be left standing on.
        forest.apply(Action::ToggleFilter);

        forest.refresh(gather(
            vec![tree_of("harbour", HARBOUR)],
            Vec::new(),
            Filter::LiveAgents,
        ));

        assert!(
            matches!(
                &forest.lines()[forest.selected_line()].content,
                Content::Project(line) if line.project == "orbital"
            ),
            "{:#?}",
            sketch(&forest)
        );
    }

    /// `a` is a display choice over what was already collected, so what it
    /// renders is the answer the trackers already gave.
    #[test]
    fn dropping_the_filter_re_renders_rather_than_re_collecting() {
        let mut forest = flatten(snapshot());
        let generated_at = forest.snapshot().generated_at;

        assert!(forest.apply(Action::ToggleFilter));

        assert_eq!(forest.snapshot().filter, Filter::All);
        assert_eq!(forest.snapshot().generated_at, generated_at);
        assert!(sketch(&forest).iter().any(|line| line == "▾ harbour"));
        assert!(sketch(&forest).iter().any(|line| line.contains("hbr-3")));
        assert!(!sketch(&forest)
            .iter()
            .any(|line| line.contains("[HiddenTrees]")));
    }

    #[test]
    fn collapsing_an_expanded_node_and_then_collapsing_again_moves_to_its_parent() {
        let mut forest = flatten(snapshot());
        open(&mut forest, &key("orbital", "orb-7.1"));

        assert!(forest.apply(Action::CollapseOrParent));
        assert_eq!(cursor(&forest), Some(&key("orbital", "orb-7.1")));
        assert!(!sketch(&forest).iter().any(|line| line.contains(".1.1")));

        assert!(forest.apply(Action::CollapseOrParent));
        assert_eq!(cursor(&forest), Some(&key("orbital", "orb-7")));
    }

    #[test]
    fn expanding_a_collapsed_node_and_then_expanding_again_moves_to_its_first_child() {
        let mut forest = flatten(snapshot());
        open(&mut forest, &key("orbital", "orb-7.1"));
        forest.apply(Action::CollapseOrParent);

        assert!(forest.apply(Action::ExpandOrChild));
        assert_eq!(cursor(&forest), Some(&key("orbital", "orb-7.1")));

        assert!(forest.apply(Action::ExpandOrChild));
        assert_eq!(cursor(&forest), Some(&key("orbital", "orb-7.1.1")));
    }

    #[test]
    fn a_leaf_has_no_child_to_move_to_and_no_fold_to_collapse() {
        let mut forest = flatten(snapshot());
        open(&mut forest, &key("orbital", "orb-7.1"));
        select(&mut forest, &key("orbital", "orb-7.1.1"));

        assert!(!forest.apply(Action::ExpandOrChild));
        assert!(forest.apply(Action::CollapseOrParent));
        assert_eq!(cursor(&forest), Some(&key("orbital", "orb-7.1")));
    }

    #[test]
    fn moving_stops_at_the_ends() {
        let mut forest = flatten(snapshot());
        forest.apply(Action::Move(Motion::FirstRow));

        assert!(!forest.apply(Action::Move(Motion::PreviousRow)));
        assert_eq!(forest.selected_line(), 0);

        forest.apply(Action::Move(Motion::LastRow));
        let last = forest.selected_line();

        assert!(!forest.apply(Action::Move(Motion::NextRow)));
        assert_eq!(forest.selected_line(), last);
    }

    #[test]
    fn a_half_screen_moves_as_far_as_the_renderer_says_it_should() {
        let mut forest = flatten(snapshot());
        forest.set_half_screen(3);

        forest.apply(Action::Move(Motion::HalfScreenDown));

        assert_eq!(forest.selected_line(), 4);

        forest.apply(Action::Move(Motion::HalfScreenUp));

        assert_eq!(forest.selected_line(), 1);
    }

    /// A keystroke asks whether the screen moved by comparing the lines, so a
    /// line that carries anything the screen does not show makes that question
    /// answerable by data no reader can see. Retitling a node no line draws
    /// must leave the lines identical.
    #[test]
    fn a_line_carries_nothing_the_screen_does_not_show() {
        let drawn: BTreeSet<BeadKey> = flatten(snapshot())
            .lines()
            .iter()
            .filter_map(|line| line.bead().cloned())
            .collect();

        let mut altered = snapshot();
        let mut retitled = 0;
        for tree in &mut altered.trees {
            let tree = Arc::make_mut(tree);
            for node in &mut tree.beads {
                let key = BeadKey {
                    project: tree.project.clone(),
                    id: node.id.clone(),
                };
                if !drawn.contains(&key) {
                    node.title = format!("{} (retitled)", node.title);
                    retitled += 1;
                }
            }
        }
        assert!(
            retitled > 0,
            "every node in the fixture is drawn, so nothing here is undrawn to hide"
        );

        assert_eq!(flatten(snapshot()).lines(), flatten(altered).lines());
    }

    /// The forest cannot focus a pane, re-collect or quit; the loop does all
    /// three, and none of them changes what is on screen.
    #[test]
    fn focus_refresh_and_quit_change_nothing_in_the_forest() {
        let mut forest = flatten(snapshot());
        let before = sketch(&forest);

        for action in [Action::Focus, Action::Refresh, Action::Quit] {
            assert!(!forest.apply(action), "{action:?}");
        }

        assert_eq!(sketch(&forest), before);
    }

    /// A pane working in a project's paths that no bead claims is the
    /// project's, so it is drawn under the project's own line rather than in
    /// a group below the trees: one place to look for everything beneath a
    /// project, whether or not its roots read.
    #[test]
    fn every_loose_pane_hangs_under_its_own_projects_line() {
        let forest = flatten(snapshot());
        let loose: Vec<(&str, String)> = forest
            .lines()
            .iter()
            .enumerate()
            .filter_map(|(at, line)| match &line.content {
                Content::Item(Item::Loose(pane)) => {
                    Some((pane.project.as_str(), project_above(&forest, at)))
                }
                _ => None,
            })
            .collect();

        assert_eq!(
            loose.len(),
            forest.snapshot().unattributed.len(),
            "{:#?}",
            sketch(&forest)
        );
        for (project, above) in loose {
            assert_eq!(project, above, "{:#?}", sketch(&forest));
        }
    }

    /// The project whose line is the nearest one above `at`.
    fn project_above(forest: &Forest, at: usize) -> String {
        forest.lines()[..at]
            .iter()
            .rev()
            .find_map(|line| match &line.content {
                Content::Project(line) => Some(line.project.clone()),
                _ => None,
            })
            .expect("a line under a project has one above it")
    }

    /// A root that would not read says so on a line of its own under its
    /// project. The failure is that root's and not the project's — the
    /// project answered, and this one root did not — so it rides where the
    /// root's row would have been rather than on the line above.
    #[test]
    fn a_root_that_could_not_be_read_says_so_where_its_row_would_have_been() {
        let forest = flatten(snapshot());
        let at = forest
            .lines()
            .iter()
            .position(
                |line| matches!(&line.content, Content::Project(line) if line.project == "ferry"),
            )
            .expect("ferry has a line");

        assert!(
            matches!(
                &forest.lines()[at + 1].content,
                Content::Unread(unread)
                    if unread.root == "fer-2"
                        && unread.tracker == TrackerState::Unreachable(TrackerFailure::Auth)
            ),
            "{:#?}",
            sketch(&forest)
        );
    }

    /// A project with no loose pane draws no line saying so: a group's line
    /// on every healthy project buries the one where it matters.
    #[test]
    fn a_project_with_no_loose_pane_draws_no_line_for_them() {
        let forest = flatten(snapshot());

        assert!(
            !forest.lines().iter().any(|line| matches!(
                &line.content,
                Content::Group(group)
                    if group.kind == GroupKind::Unattributed
                        && group.project.as_deref() == Some("harbour")
            )),
            "{:#?}",
            sketch(&forest)
        );
    }

    /// The defect: a root drew as a tree header, which is not a bead row, so
    /// nothing that reads a bead row could see the agent on it. What is the
    /// project's — its name, and the panes recovered for it — is the project's
    /// own line, and the root beneath it is a bead like any other.
    #[test]
    fn a_project_owns_its_own_line_and_its_roots_are_ordinary_bead_rows() {
        let forest = flatten(snapshot());
        let lines = forest.lines();

        assert!(
            matches!(&lines[0].content, Content::Project(line) if line.project == "orbital"),
            "{:#?}",
            sketch(&forest)
        );
        assert!(
            matches!(&lines[1].content, Content::Bead(row) if row.id == "orb-7"),
            "{:#?}",
            sketch(&forest)
        );
    }

    /// `bdi-2bb.25`: the root carries a pane of its own while `orb-7.4`
    /// beneath it carries another, so a count over the subtree and the root's
    /// own agent cannot come out the same by chance.
    #[test]
    fn a_staffed_root_says_what_its_agent_is_doing_like_any_other_row() {
        let forest = flatten(snapshot());

        assert_eq!(
            row_of(&forest, "orb-7").agent.as_deref(),
            Some("◍ w:p1 · working")
        );
    }

    /// A root is a bead row, so the tail reaches its pane by the road every
    /// other bead row takes. Before this it was the one staffed row on the
    /// screen where the tail said there was no bead to show.
    #[test]
    fn the_tail_follows_a_staffed_root_as_it_follows_any_other_bead() {
        let mut forest = flatten(snapshot());
        forest.apply(Action::Move(Motion::FirstRow));
        forest.apply(Action::Move(Motion::NextRow));

        assert_eq!(
            crate::view::tail::target(&forest).pane(),
            Some(&pane_key("w:p1")),
            "{:#?}",
            sketch(&forest)
        );
    }

    /// A project's line counts the beads of its own trees and no other's.
    /// `built` puts a tree under orbital and one under harbour, so a count
    /// taken over the wrong run of trees shows on one line or the other.
    #[test]
    fn a_project_line_counts_its_own_trees_and_no_others() {
        let forest = flatten(built(Filter::All));

        assert_eq!(
            header_of(&forest, "orbital").counts,
            tree_of("orbital", ORBITAL).counts
        );
        assert_eq!(
            header_of(&forest, "harbour").counts,
            tree_of("harbour", HARBOUR).counts
        );
    }

    fn header_of<'a>(forest: &'a Forest, project: &str) -> &'a ProjectLine {
        forest
            .lines()
            .iter()
            .find_map(|line| match &line.content {
                Content::Project(line) if line.project == project => Some(line),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{project} has a line"))
    }

    /// The defect: a group's lines were drawn and could not be reached, so
    /// nothing the forest holds in a group could be looked at or acted on.
    #[test]
    fn the_selection_can_walk_onto_a_line_under_a_group() {
        let mut forest = flatten(snapshot());
        let panes: Vec<usize> = forest
            .lines()
            .iter()
            .enumerate()
            .filter(|(_, line)| matches!(line.content, Content::Item(Item::Loose(_))))
            .map(|(at, _)| at)
            .collect();

        assert_eq!(panes.len(), 3, "{:#?}", sketch(&forest));
        for at in panes {
            forest.apply(Action::Move(Motion::FirstRow));
            walk::until(
                &mut forest,
                |forest| forest.selected_line() >= at,
                |forest| {
                    forest.apply(Action::Move(Motion::NextRow));
                },
                |forest| format!("line {at} cannot be reached: {:#?}", sketch(forest)),
            );
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
        let mut forest = flatten(snapshot());
        select_item(&mut forest, "w:p4");

        forest.refresh(reordered_groups());

        assert_eq!(selected_item(&forest), Some("w:p4".to_string()));
    }

    /// Every group's items, so no kind is selectable by accident and none is
    /// left behind: a group whose lines cannot be reached is the defect.
    #[test]
    fn every_kind_of_thing_a_group_holds_can_hold_the_selection() {
        let mut forest = flatten(built(Filter::LiveAgents));
        let groups: Vec<(GroupKind, Option<String>)> =
            layout::every_group(forest.snapshot()).collect();
        for (kind, project) in groups {
            forest.folds.set(Handle::Group(kind, project), true);
        }
        forest.refresh(built(Filter::LiveAgents));

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
        let mut forest = flatten(snapshot());
        select_item(&mut forest, "w:p4");

        forest.refresh(built_without_the_conflicting_panes());

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

    /// A project's group that empties leaves the selection on the project's
    /// line, which is where the group hung: the project is still there, and
    /// the top of the forest is not where the reader was.
    #[test]
    fn a_selection_on_a_pane_whose_group_empties_falls_back_to_its_project() {
        let mut forest = flatten(snapshot());
        select_item(&mut forest, "w:p9");

        forest.refresh(built_without_ferrys_panes());

        assert!(
            matches!(
                &forest.lines()[forest.selected_line()].content,
                Content::Project(line) if line.project == "ferry"
            ),
            "{:#?}",
            sketch(&forest)
        );
    }

    /// The same from the group's own line.
    #[test]
    fn a_selection_on_a_projects_group_that_empties_falls_back_to_the_project() {
        let mut forest = flatten(snapshot());
        let group = forest
            .lines()
            .iter()
            .position(|line| {
                matches!(&line.content, Content::Group(group)
                    if group.kind == GroupKind::Unattributed
                        && group.project.as_deref() == Some("ferry"))
            })
            .expect("ferry has a pane no bead claims");
        assert!(forest.select_line(group));

        forest.refresh(built_without_ferrys_panes());

        assert!(
            matches!(
                &forest.lines()[forest.selected_line()].content,
                Content::Project(line) if line.project == "ferry"
            ),
            "{:#?}",
            sketch(&forest)
        );
    }

    /// The same snapshot with the one pane working in ferry gone, which
    /// empties the group ferry's line holds it in.
    fn built_without_ferrys_panes() -> Snapshot {
        let mut snapshot = snapshot();
        snapshot.unattributed.retain(|pane| pane.project != "ferry");
        snapshot
    }

    /// The handle a project's group is held by carries the project, so a
    /// reader opening one project's quiet trees opens nobody else's.
    #[test]
    fn opening_one_projects_hidden_trees_leaves_another_projects_shut() {
        let colliding = edited(SLIPWAY, "hbr-9", "hbr-3");
        let mut forest = flatten(gather(
            vec![tree_of("harbour", HARBOUR), tree_of("orbital", &colliding)],
            Vec::new(),
            Filter::LiveAgents,
        ));
        select_hidden_tree(&mut forest);
        assert_eq!(cursor(&forest), Some(&key("orbital", "hbr-3")));

        let harbours = forest
            .lines()
            .iter()
            .find(|line| {
                matches!(&line.content, Content::Group(group)
                    if group.kind == GroupKind::HiddenTrees
                        && group.project.as_deref() == Some("harbour"))
            })
            .expect("harbour's quiet trees have a line");
        assert_eq!(harbours.folded, Some(false), "{:#?}", sketch(&forest));
    }

    /// A project with quiet trees and loose panes both: opening the line over
    /// its quiet trees draws them one level in, and the line over its loose
    /// panes still follows at the depth it started at, hanging under the
    /// project and not under the trees just opened.
    #[test]
    fn a_projects_loose_panes_follow_its_opened_quiet_trees_at_their_own_depth() {
        let mut quiet = alone("orbital", TOWER, &panes_on(&["nobody"]));
        quiet.refilter(Filter::LiveAgents);
        let mut forest = flatten(quiet);
        assert_eq!(
            sketch(&forest)[..4],
            [
                "▾ orbital",
                "  ├─▸ [HiddenTrees orbital] 1",
                "  └── [Unattributed orbital] 1",
                "      └── - Loose(LoosePane { pane: PaneKey { session: \"default\", id: \"w:nobody\" }, project: \"orbital\", cwd: \"/srv/work/orbital\", pane_status: Working, display_agent: Some(\"nobody\"), title: None })",
            ]
        );

        select_hidden_tree(&mut forest);

        assert_eq!(
            sketch(&forest)[..5],
            [
                "▾ orbital",
                "  ├── [HiddenTrees orbital] 1",
                "  │   └─▸ ○ tow-1 raise the tower",
                "  └── [Unattributed orbital] 1",
                "      └── - Loose(LoosePane { pane: PaneKey { session: \"default\", id: \"w:nobody\" }, project: \"orbital\", cwd: \"/srv/work/orbital\", pane_status: Working, display_agent: Some(\"nobody\"), title: None })",
            ]
        );
    }

    /// The filter decides where a project's trees are drawn and not whether
    /// the project holds them, so its line counts every bead of every tree
    /// and `a` moves nothing on it.
    #[test]
    fn a_projects_line_counts_the_trees_the_filter_holds_back() {
        let mut forest = flatten(snapshot());
        let counted = header_of(&forest, "harbour").counts.clone();
        assert_eq!(counted.total, 2, "{:#?}", sketch(&forest));

        forest.apply(Action::ToggleFilter);

        assert_eq!(forest.snapshot().filter, Filter::All);
        assert_eq!(header_of(&forest, "harbour").counts, counted);
    }

    /// A project whose tracker could not be read at all can still have panes
    /// working in its paths, and they are the project's: its line is drawn
    /// for them, and the failure stays where it is reported, in the group
    /// below the trees.
    #[test]
    fn a_failed_projects_loose_panes_hang_under_its_own_line() {
        let forest = flatten(gather(
            vec![tree_of("orbital", ORBITAL)],
            vec![FailedProject {
                project: "ferry".into(),
                tracker: TrackerFailure::Unstartable,
            }],
            Filter::LiveAgents,
        ));
        let drawn = sketch(&forest);
        let ferry = drawn
            .iter()
            .position(|line| line == "▾ ferry")
            .unwrap_or_else(|| panic!("ferry has a line: {drawn:#?}"));

        assert_eq!(
            drawn[ferry..ferry + 4],
            [
                "▾ ferry",
                "  └── [Unattributed ferry] 1",
                "      └── - Loose(LoosePane { pane: PaneKey { session: \"default\", id: \"w:p9\" }, project: \"ferry\", cwd: \"/srv/work/ferry\", pane_status: Blocked, display_agent: None, title: None })",
                "▸ [FailedProjects] 1",
            ]
        );
    }

    /// A forest of projects drawn only for the panes working in their paths
    /// holds no root, so there is nothing for `select_first_root` to open on.
    /// The selection still settles somewhere and stays: `lay_out` runs before
    /// it and leaves the cursor on the first thing the forest holds, so a key
    /// that lays the forest out again and says nothing about the selection
    /// finds it already held.
    #[test]
    fn a_project_drawn_only_for_its_loose_panes_keeps_the_line_it_opened_on() {
        let mut forest = flatten(gather(
            Vec::new(),
            vec![FailedProject {
                project: "ferry".into(),
                tracker: TrackerFailure::Unstartable,
            }],
            Filter::LiveAgents,
        ));
        let drawn = sketch(&forest);
        assert!(
            !drawn
                .iter()
                .any(|line| line.contains('◐') || line.contains('○')),
            "no root is drawn, which is what leaves nothing to open on: {drawn:#?}"
        );
        let opened_on = forest.selected_line();
        assert_eq!(
            drawn[opened_on], "  └── [Unattributed orbital] 2",
            "the selection settles on the first thing drawn, not on nothing: {drawn:#?}"
        );

        for action in [Action::ToggleFilter, Action::RestoreDefault] {
            forest.apply(action);
            assert_eq!(
                sketch(&forest)[forest.selected_line()],
                drawn[opened_on],
                "{action:?}"
            );
        }
    }

    /// The pane id on the line the selection sits on, where it sits on one.
    fn selected_item(forest: &Forest) -> Option<String> {
        match &forest.lines()[forest.selected_line()].content {
            Content::Item(Item::Loose(pane)) => Some(pane.pane.id.clone()),
            Content::Item(Item::Unconfigured(pane)) => Some(pane.pane.id.clone()),
            _ => None,
        }
    }

    /// Put the selection on the line for one pane, by moving down to it.
    fn select_item(forest: &mut Forest, pane: &str) {
        forest.apply(Action::Move(Motion::FirstRow));
        walk::until(
            forest,
            |forest| selected_item(forest).as_deref() == Some(pane),
            |forest| {
                forest.apply(Action::Move(Motion::NextRow));
            },
            |_| format!("{pane} is not reachable by moving down"),
        );
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

    /// A group the collect empties puts the selection back on the first root,
    /// not on whatever line happened to sit above where the group was. The
    /// two answers only differ once the forest is more than a row tall, which
    /// is every forest a reader has.
    ///
    /// This is the only one of `group_drawn`'s two callers that can tell you
    /// it is wrong. `first_handle` cannot: `lay_out` looks the handle it
    /// returns up, does not find it drawn, and repairs the cursor from the
    /// selected index — so a `group_drawn` that said yes to every kind would
    /// go unnoticed down that road.
    #[test]
    fn a_selection_on_a_group_that_empties_goes_back_to_the_first_root() {
        let mut forest = flatten(snapshot());
        let group = forest
            .lines()
            .iter()
            .position(|line| {
                matches!(&line.content, Content::Group(group)
                    if group.kind == GroupKind::FailedProjects)
            })
            .expect("the shared snapshot draws a group for the project that failed");
        step_onto(&mut forest, group);

        forest.refresh(built_without_the_failed_project());

        assert_eq!(
            forest.lines()[forest.selected_line()]
                .bead()
                .map(|bead| bead.id.clone()),
            Some("orb-7".to_string()),
            "{:#?}",
            sketch(&forest)
        );
    }

    /// The same snapshot with the project that failed before its roots were
    /// known now reading, which empties the group it was the only member of.
    fn built_without_the_failed_project() -> Snapshot {
        let mut snapshot = snapshot();
        snapshot.failed_projects.clear();
        snapshot
    }

    /// The same snapshot with the two panes that were fighting over `orb-7.1`
    /// gone, which empties the unattributed group of the one the tests hold.
    fn built_without_the_conflicting_panes() -> Snapshot {
        let mut snapshot = snapshot();
        snapshot.unattributed.retain(|pane| pane.pane.id == "w:p3");
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
                read_at: every_project_read(),
            },
            &[],
            &Joined {
                agents: joined.agents,
                refused: BTreeMap::new(),
                conflicts: Vec::new(),
            },
            &cfg(),
            a_provider(ProviderState::Answering),
            Filter::LiveAgents,
            now(),
        );

        assert!(!sketch(&flatten(snapshot))
            .iter()
            .any(|line| line.contains('[')));
    }

    #[test]
    fn a_group_draws_one_line_for_each_thing_it_holds_when_it_is_opened() {
        let mut forest = flatten(snapshot());
        let loose = |forest: &Forest| {
            sketch(forest)
                .iter()
                .filter(|line| line.contains("Loose"))
                .count()
        };
        let group = forest
            .lines()
            .iter()
            .position(|line| {
                matches!(&line.content, Content::Group(group)
                    if group.kind == GroupKind::Unattributed
                        && group.project.as_deref() == Some("orbital"))
            })
            .expect("orbital has panes no bead claims");
        forest.select_line(group);

        // Ferry's one loose pane is under ferry's own line, and stays.
        assert!(forest.apply(Action::ToggleFold));
        assert_eq!(loose(&forest), 1, "{:#?}", sketch(&forest));

        assert!(forest.apply(Action::ToggleFold));
        assert_eq!(loose(&forest), 3, "{:#?}", sketch(&forest));
    }

    /// The panes under no configured project open into their own directories,
    /// which is the whole use of the group: the line says a `[[projects]]`
    /// entry is missing and opening it says which one.
    #[test]
    fn opening_the_unconfigured_group_names_the_directories() {
        let mut forest = flatten(snapshot());
        forest
            .folds
            .set(Handle::Group(GroupKind::Unconfigured, None), true);
        forest.refresh(snapshot());

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
    /// when one of them is waiting on work bd never returned.
    #[test]
    fn the_hidden_trees_group_says_how_many_of_them_have_findings() {
        let broken = edited(
            HARBOUR,
            r#"{"depends_on_id":"hbr-3","type":"parent-child"}"#,
            r#"{"depends_on_id":"hbr-3","type":"parent-child"},
                       {"depends_on_id":"hbr-9","type":"blocks"}"#,
        );
        let snapshot = gather(
            vec![tree_of("orbital", ORBITAL), tree_of("harbour", &broken)],
            Vec::new(),
            Filter::LiveAgents,
        );

        let group = hidden_trees_group(&flatten(snapshot), "harbour");

        assert_eq!(group.count, 1);
        assert_eq!(group.with_findings, 1);
    }

    #[test]
    fn a_hidden_tree_with_nothing_wrong_in_it_is_only_counted_as_hidden() {
        let group = hidden_trees_group(&flatten(snapshot()), "harbour");

        assert_eq!(group.count, 1);
        assert_eq!(group.with_findings, 0);
    }

    /// What the group says of the trees it hides was settled when the filter
    /// hid them. A count that had to go back to `collected` for it would
    /// cost every press a walk over the whole forest, and a forest of
    /// thousands of hidden trees was paying half its keystroke for that.
    #[test]
    fn the_hidden_trees_group_does_not_go_back_to_the_collected_trees_for_its_count() {
        let broken = edited(
            HARBOUR,
            r#"{"depends_on_id":"hbr-3","type":"parent-child"}"#,
            r#"{"depends_on_id":"hbr-3","type":"parent-child"},
                       {"depends_on_id":"hbr-9","type":"blocks"}"#,
        );
        let mut snapshot = gather(
            vec![tree_of("orbital", ORBITAL), tree_of("harbour", &broken)],
            Vec::new(),
            Filter::LiveAgents,
        );
        snapshot.collected.clear();

        let group = hidden_trees_group(&flatten(snapshot), "harbour");

        assert_eq!(group.count, 1);
        assert_eq!(group.with_findings, 1);
    }

    /// A hidden tree's findings are the ones in its own tree. Harbour hides
    /// two roots and only the slipway has anything wrong in it, so a match
    /// that asked the project alone would report the channel as hiding a
    /// finding that is not in it.
    #[test]
    fn a_hidden_tree_does_not_take_a_finding_from_another_root_in_its_project() {
        let snapshot = gather(
            vec![tree_of("harbour", HARBOUR), tree_of("harbour", SLIPWAY)],
            Vec::new(),
            Filter::LiveAgents,
        );

        let group = hidden_trees_group(&flatten(snapshot), "harbour");

        assert_eq!(group.count, 2);
        assert_eq!(group.with_findings, 1);
    }

    /// Bead ids are numbered per tracker and the trackers do not coordinate,
    /// so two projects can each hold a root called `hbr-3` and they are
    /// different beads. Each is hidden under its own project, and a match
    /// that asked the root alone would report harbour as hiding the finding
    /// that is in orbital's.
    #[test]
    fn a_hidden_tree_does_not_take_a_finding_from_the_same_root_in_another_project() {
        let colliding = edited(SLIPWAY, "hbr-9", "hbr-3");
        let forest = flatten(gather(
            vec![tree_of("harbour", HARBOUR), tree_of("orbital", &colliding)],
            Vec::new(),
            Filter::LiveAgents,
        ));

        let harbour = hidden_trees_group(&forest, "harbour");
        let orbital = hidden_trees_group(&forest, "orbital");

        assert_eq!((harbour.count, harbour.with_findings), (1, 0));
        assert_eq!((orbital.count, orbital.with_findings), (1, 1));
    }

    /// The line over one project's hidden trees.
    fn hidden_trees_group(forest: &Forest, project: &str) -> Group {
        forest
            .lines()
            .iter()
            .find_map(|line| match &line.content {
                Content::Group(group)
                    if group.kind == GroupKind::HiddenTrees
                        && group.project.as_deref() == Some(project) =>
                {
                    Some(group.clone())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("the filter hid a tree of {project}"))
    }

    /// Put the selection on the first hidden tree's row: open the group,
    /// which rests shut, and step into it.
    fn select_hidden_tree(forest: &mut Forest) {
        let group = forest
            .lines()
            .iter()
            .position(|line| {
                matches!(&line.content, Content::Group(group) if group.kind == GroupKind::HiddenTrees)
            })
            .expect("the filter hid a tree");
        forest.select_line(group);
        assert_eq!(forest.selected_line(), group);
        forest.apply(Action::ExpandOrChild);
        forest.apply(Action::ExpandOrChild);
        assert!(
            matches!(
                forest.lines()[forest.selected_line()].content,
                Content::Bead(_)
            ),
            "{:#?}",
            sketch(forest)
        );
    }

    /// The selected line and everything drawn beneath it, sketched.
    fn beneath_the_selection(forest: &Forest) -> Vec<String> {
        let scope = forest
            .handle_at(forest.selected_line())
            .expect("the selection is on a line it can hold");
        subtree_of(forest.lines(), &scope)
            .iter()
            .map(|line| format!("{}{}", line.prefix, said(&line.content)))
            .collect()
    }

    /// A hidden tree's row is its root's row, so it stands for that bead as
    /// a tree's header does: the show key and `y` have a bead to work on.
    #[test]
    fn a_hidden_trees_row_stands_for_its_root() {
        let mut forest = flatten(snapshot());
        select_hidden_tree(&mut forest);

        assert_eq!(cursor(&forest), Some(&key("harbour", "hbr-3")));
    }

    /// A hidden tree is a tree, and the group is only where the filter put
    /// it: it is drawn there as its project would draw it, one level further
    /// in under the group's line, and opens onto the same rows.
    #[test]
    fn a_hidden_tree_is_drawn_in_the_group_as_its_project_would_draw_it() {
        let mut forest = flatten(snapshot());
        select_hidden_tree(&mut forest);
        assert_eq!(
            beneath_the_selection(&forest),
            ["      └─▸ ○ hbr-3 dredge the channel"],
            "a hidden tree rests shut"
        );

        assert!(forest.apply(Action::ExpandOrChild));

        let in_the_group = beneath_the_selection(&forest);
        assert_eq!(
            in_the_group,
            [
                "      └── ○ hbr-3 dredge the channel",
                "          └── ○ .1 survey the silt"
            ]
        );
        forest.apply(Action::ToggleFilter);
        assert_eq!(forest.snapshot().filter, Filter::All);
        select(&mut forest, &key("harbour", "hbr-3"));
        assert_eq!(
            beneath_the_selection(&forest),
            [
                "  └── ○ hbr-3 dredge the channel",
                "      └── ○ .1 survey the silt"
            ],
            "the same rows under its project, the fold the reader opened included"
        );
    }

    /// Graeme: *"the top-level trees with no live agent should not be
    /// expanded by default"*. A hidden tree rests shut whatever is beneath
    /// it, where the same tree shown under its project rests open onto the
    /// work a reader could start; it is the one thing about a hidden tree
    /// that differs from the same tree shown.
    #[test]
    fn a_hidden_tree_rests_shut_even_over_work_that_would_open_it_shown() {
        let ready = ready_alone("orbital", HARBOUR, &[], &["hbr-3.1"]);
        assert_eq!(
            lines_of(&flatten(ready.clone()), "hbr-3.1").len(),
            1,
            "shown, the tree rests open onto its ready work"
        );

        let mut hidden = ready;
        hidden.refilter(Filter::LiveAgents);
        let mut forest = flatten(hidden);
        select_hidden_tree(&mut forest);

        assert_eq!(forest.lines()[forest.selected_line()].folded, Some(false));
        assert_eq!(
            lines_of(&forest, "hbr-3.1").len(),
            0,
            "{:#?}",
            sketch(&forest)
        );
    }

    /// Opening a hidden tree is the reader's choice and not the filter's, so
    /// letting go of the folds shuts it again: the group is shut with the
    /// rest, and opened once more by hand it holds the row shut over its
    /// tree, as the filter left it.
    #[test]
    fn the_default_puts_an_opened_hidden_tree_back() {
        let mut forest = flatten(snapshot());
        select_hidden_tree(&mut forest);
        let shut = beneath_the_selection(&forest);
        forest.apply(Action::ExpandOrChild);
        assert_ne!(beneath_the_selection(&forest), shut, "the tree opened");

        forest.apply(Action::RestoreDefault);

        assert_eq!(
            hidden_trees_group(&forest, "harbour").count,
            1,
            "the group is drawn shut: {:#?}",
            sketch(&forest)
        );
        select_hidden_tree(&mut forest);
        assert_eq!(beneath_the_selection(&forest), shut);
    }

    /// A tree that is only its root has nothing to open onto, and a fold
    /// over nothing would be a key that does nothing.
    #[test]
    fn a_hidden_tree_with_nothing_beneath_its_root_offers_no_fold() {
        let lone = r#"[{"id":"hbr-1","title":"moor the lightship","status":"open"}]"#;
        let mut forest = flatten(gather(
            vec![tree_of("harbour", lone)],
            Vec::new(),
            Filter::LiveAgents,
        ));
        select_hidden_tree(&mut forest);
        let row = forest.selected_line();

        assert_eq!(forest.lines()[row].folded, None);
        assert_eq!(
            beneath_the_selection(&forest),
            ["      └── ○ hbr-1 moor the lightship"]
        );
        assert!(!forest.apply(Action::ExpandOrChild), "nothing to open onto");
    }

    /// A hidden tree's facts are a tree's facts, answered when the forest
    /// takes the snapshot: drawing it, opening it and moving through it ask
    /// nothing of the tree.
    #[test]
    fn a_keystroke_in_the_hidden_trees_group_walks_no_subtree() {
        let mut forest = flatten(snapshot());
        let before = walks_on_this_thread();

        select_hidden_tree(&mut forest);
        forest.apply(Action::ExpandOrChild);
        forest.apply(Action::Move(Motion::NextRow));

        assert_eq!(walks_on_this_thread() - before, 0);
    }

    /// A hidden tree's findings are drawn under its root as any tree's are,
    /// whether the root is folded or not. The group holding it shut is what
    /// keeps them off the screen, and the group's line admits to them.
    #[test]
    fn a_hidden_trees_findings_are_drawn_under_its_root_as_any_trees_are() {
        let mut forest = flatten(gather(
            vec![tree_of("orbital", ORBITAL), tree_of("harbour", SLIPWAY)],
            Vec::new(),
            Filter::LiveAgents,
        ));
        select_hidden_tree(&mut forest);

        assert_eq!(
            beneath_the_selection(&forest),
            [
                "      └─▸ ○ hbr-9 re-deck the slipway",
                "          └── ! Dangling(1)"
            ]
        );
    }

    /// `E` on the hidden-trees group opens every hidden tree to the bottom.
    /// The walk's budget is counted off the beads, and a hidden tree's beads
    /// are as much of the forest as a shown tree's: a budget counted off the
    /// shown trees alone runs out on a forest that shows none.
    #[test]
    fn expanding_the_hidden_trees_group_reaches_the_bottom_of_a_deep_hidden_tree() {
        let chain = r#"[
          {"id":"hbr-5","title":"root","status":"open"},
          {"id":"hbr-5.1","title":"one","status":"open",
           "dependencies":[{"depends_on_id":"hbr-5","type":"parent-child"}]},
          {"id":"hbr-5.1.1","title":"two","status":"open",
           "dependencies":[{"depends_on_id":"hbr-5.1","type":"parent-child"}]},
          {"id":"hbr-5.1.1.1","title":"three","status":"open",
           "dependencies":[{"depends_on_id":"hbr-5.1.1","type":"parent-child"}]}
        ]"#;
        let mut forest = flatten(gather(
            vec![tree_of("harbour", chain)],
            Vec::new(),
            Filter::LiveAgents,
        ));
        assert!(forest.snapshot().trees.is_empty(), "every tree is hidden");
        let group = forest
            .lines()
            .iter()
            .position(|line| {
                matches!(&line.content, Content::Group(group) if group.kind == GroupKind::HiddenTrees)
            })
            .expect("the group is drawn");
        forest.select_line(group);
        assert_eq!(forest.selected_line(), group);

        forest.apply(Action::ExpandSubtree);

        assert_eq!(
            lines_of(&forest, "hbr-5.1.1.1").len(),
            1,
            "{:#?}",
            sketch(&forest)
        );
    }

    fn on_the_hidden_trees_group(forest: &Forest) -> bool {
        matches!(
            &forest.lines()[forest.selected_line()].content,
            Content::Group(group) if group.kind == GroupKind::HiddenTrees
        )
    }

    /// A tree the filter takes from under the selection goes into the
    /// hidden-trees group, and so does the selection: with the group shut
    /// over it, the group's line is where the tree went, which is more than
    /// whatever row happened to be nearest can say.
    #[test]
    fn putting_the_filter_back_over_the_selected_tree_moves_the_selection_to_the_group() {
        let mut forest = flatten(built(Filter::All));
        select(&mut forest, &key("harbour", "hbr-3"));

        forest.apply(Action::ToggleFilter);

        assert_eq!(forest.snapshot().filter, Filter::LiveAgents);
        assert!(on_the_hidden_trees_group(&forest), "{:#?}", sketch(&forest));
    }

    /// The same where a refresh is what hides it: the agent that kept the
    /// tree on the screen has gone, and the selection was on a bead inside.
    /// A pane on no bead keeps a group drawn below the hidden trees, so the
    /// nearest row to where the selection was is not the group's line.
    #[test]
    fn a_refresh_that_hides_the_selected_tree_moves_the_selection_to_the_group() {
        let mut staffed = alone("orbital", TOWER, &panes_on(&["tow-1.1", "nobody"]));
        staffed.refilter(Filter::LiveAgents);
        let mut forest = flatten(staffed);
        select(&mut forest, &key("orbital", "tow-1.1"));

        forest.refresh(alone("orbital", TOWER, &panes_on(&["nobody"])));

        assert_eq!(forest.snapshot().hidden_trees.len(), 1);
        assert_eq!(forest.snapshot().unattributed.len(), 1);
        assert!(on_the_hidden_trees_group(&forest), "{:#?}", sketch(&forest));
    }

    /// With the group open, the tree's root is drawn there under the same
    /// handle, so the selection simply follows the tree into the group.
    #[test]
    fn with_the_group_open_the_selection_follows_the_tree_the_filter_hides() {
        let mut forest = flatten(built(Filter::All));
        forest.folds.set(
            Handle::Group(GroupKind::HiddenTrees, Some("harbour".into())),
            true,
        );
        select(&mut forest, &key("harbour", "hbr-3"));

        forest.apply(Action::ToggleFilter);

        assert_eq!(cursor(&forest), Some(&key("harbour", "hbr-3")));
    }

    /// With the group open and the selection on a bead inside a tree that
    /// rested open on its own account, the root the tree now rests shut
    /// under is where the tree went, which is more than the nearest row can
    /// say. A fold the reader had opened by hand would have kept the bead
    /// drawn, and the selection with it.
    #[test]
    fn with_the_group_open_a_bead_inside_the_hidden_tree_falls_back_to_its_root() {
        let mut forest = flatten(ready_alone("orbital", HARBOUR, &[], &["hbr-3.1"]));
        forest.folds.set(
            Handle::Group(GroupKind::HiddenTrees, Some("orbital".into())),
            true,
        );
        select(&mut forest, &key("orbital", "hbr-3.1"));

        forest.apply(Action::ToggleFilter);

        assert_eq!(forest.snapshot().filter, Filter::LiveAgents);
        assert_eq!(cursor(&forest), Some(&key("orbital", "hbr-3")));
    }

    /// Only the cursor's own tree going into the group takes the selection
    /// there. Letting go of the folds shuts one over a bead in a tree that is
    /// still drawn, and the selection takes the nearest row as it always has,
    /// however many other trees of the same project the group is shut over.
    #[test]
    fn a_fold_shutting_over_the_selection_keeps_it_out_of_the_hidden_trees_group() {
        let mut staffed = together("orbital", &[TOWER, HARBOUR], &panes_on(&["tow-1.1"]));
        staffed.refilter(Filter::LiveAgents);
        let mut forest = flatten(staffed);
        assert_eq!(forest.snapshot().hidden_trees.len(), 1);
        select(&mut forest, &key("orbital", "tow-1.1"));
        forest.apply(Action::ToggleFold);
        forest.apply(Action::Move(Motion::NextRow));
        assert_eq!(cursor(&forest), Some(&key("orbital", "tow-1.1.1")));

        forest.apply(Action::RestoreDefault);

        assert_eq!(
            cursor(&forest),
            Some(&key("orbital", "tow-1.2")),
            "{:#?}",
            sketch(&forest)
        );
    }

    /// Only a hidden tree takes findings out of the forest with it. Every
    /// other group holds its own subject in full, so none of them has
    /// anything undrawn to admit to.
    #[test]
    fn no_other_group_claims_to_be_hiding_findings() {
        let forest = flatten(snapshot());
        let others: Vec<Group> = forest
            .lines()
            .iter()
            .filter_map(|line| match &line.content {
                Content::Group(group) if group.kind != GroupKind::HiddenTrees => {
                    Some(group.clone())
                }
                _ => None,
            })
            .collect();

        assert_eq!(others.len(), 5);
        assert!(
            others.iter().all(|group| group.with_findings == 0),
            "{others:#?}"
        );
    }

    /// Every fold state over every root, every group the snapshot draws and
    /// one interior node: a few hundred of them, which is small enough to
    /// visit rather than sample.
    #[test]
    fn nothing_reported_disappears_under_any_fold_state() {
        let snapshot = snapshot();
        let mut handles = vec![
            Handle::Bead(Place::root(key("orbital", "orb-7"))),
            Handle::Bead(Place::root(key("ferry", "fer-2"))),
            Handle::Bead(Place::root(key("orbital", "orb-7")).step_to(key("orbital", "orb-7.1"))),
        ];
        handles.extend(
            layout::every_group(&snapshot)
                .filter(|(kind, project)| layout::group_drawn(&snapshot, *kind, project.as_deref()))
                .map(|(kind, project)| Handle::Group(kind, project)),
        );
        assert_eq!(handles.len(), 9, "{handles:#?}");
        for state in 0..1 << handles.len() {
            let mut forest = flatten(snapshot.clone());
            for (bit, handle) in handles.iter().enumerate() {
                forest.folds.set(handle.clone(), state & (1 << bit) == 0);
            }
            forest.refresh(snapshot.clone());

            let hidden_trees_open = forest.lines().iter().any(|line| {
                matches!(&line.content, Content::Group(group) if group.kind == GroupKind::HiddenTrees)
                    && line.folded == Some(true)
            });
            assert_eq!(
                on_screen(&forest),
                in_the_snapshot(&snapshot, hidden_trees_open),
                "fold state {state:b}"
            );
        }
    }

    /// The five degraded kinds, plus the two sorts of loose pane — the ones
    /// drawn under their project, and the ones no configured project covers.
    #[derive(Debug, Default, PartialEq, Eq)]
    struct Reported {
        dangling: usize,
        cycles: usize,
        conflicts: usize,
        failed_projects: usize,
        loose_panes: usize,
        unconfigured_panes: usize,
    }

    /// What the screen owes the reader: everything in the shown trees, and
    /// the hidden trees' findings too once the group holding them is open —
    /// shut, that group's line admits to them as a count instead.
    fn in_the_snapshot(snapshot: &Snapshot, hidden_trees_open: bool) -> Reported {
        let drawn: Vec<&Tree> = snapshot
            .trees
            .iter()
            .map(Arc::as_ref)
            .chain(
                snapshot
                    .hidden_trees
                    .iter()
                    .filter(|_| hidden_trees_open)
                    .filter_map(|hidden| snapshot.tree(&key(&hidden.project, &hidden.root))),
            )
            .collect();
        Reported {
            dangling: drawn.iter().map(|t| t.dangling.len()).sum(),
            cycles: drawn.iter().map(|t| t.cycles.len()).sum(),
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
                    Note::Cycle(n) => found.cycles += n,
                    // A property of the drawing rather than a finding in the
                    // snapshot, so there is no count for it to reach.
                    Note::NoRoots => {}
                },
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

    // ---- the column a line's content starts in ----------------------------

    /// How wide a prefix is on screen. Every glyph a prefix is drawn from is
    /// one column, so counting them is the column its content starts in.
    fn columns(prefix: &str) -> usize {
        prefix.chars().count()
    }

    /// The line drawn for one bead, by the whole id it carries.
    fn line_of<'a>(forest: &'a Forest, id: &str) -> &'a Line {
        forest
            .lines()
            .iter()
            .find(|line| line.bead().is_some_and(|bead| bead.id == id))
            .unwrap_or_else(|| panic!("{id} is not drawn"))
    }

    /// `sdg-4.3` rests shut over work of its own and `sdg-4.1` rests open
    /// beside it, both children of the root. A reader runs down the column
    /// the ids are in, and a line pushed right of its siblings is out of the
    /// column that was drawn to be read.
    ///
    /// Asked in columns rather than of a substring: every prefix here holds
    /// the elbow the other one does, so a test that looked for one found it
    /// on both and said nothing about where they started.
    #[test]
    fn a_shut_node_starts_in_the_same_column_as_an_open_sibling() {
        let forest = flatten(ready_alone(
            "orbital",
            SIDING,
            &panes_on(&["sdg-4.3"]),
            &["sdg-4.1.2"],
        ));
        let shut = line_of(&forest, "sdg-4.3");
        let open = line_of(&forest, "sdg-4.1");

        assert_eq!(shut.folded, Some(false), "sdg-4.3 is the one resting shut");
        assert_eq!(open.folded, Some(true), "sdg-4.1 is the one resting open");
        assert_eq!(shut.depth, open.depth, "they are siblings");
        assert_eq!(
            columns(&shut.prefix),
            columns(&open.prefix),
            "a shut node and an open sibling start in different columns:\n{}",
            sketch(&forest).join("\n")
        );
    }

    /// `slu-1.1` is a child of the root and holds up its sibling `slu-1.2`,
    /// so the tree draws it under each: once as part of the root, once as
    /// what `slu-1.2` cannot finish until. The same nesting is saying two
    /// different things, and the arm of the elbow is where it says which —
    /// dashed under the bead it blocks, solid under the bead it is part of.
    /// `slu-1.1.1` hangs under both copies on a solid arm, because it is a
    /// child of `slu-1.1` wherever `slu-1.1` is drawn.
    const SLUICE: &str = r#"[
      {"id":"slu-1","title":"rehang the sluice","status":"in_progress",
       "priority":1,"issue_type":"epic"},
      {"id":"slu-1.1","title":"forge the new pintles","status":"in_progress",
       "dependencies":[{"depends_on_id":"slu-1","type":"parent-child"}],
       "priority":2,"issue_type":"task"},
      {"id":"slu-1.1.1","title":"cast the pintle blanks","status":"open",
       "dependencies":[{"depends_on_id":"slu-1.1","type":"parent-child"}],
       "priority":2,"issue_type":"task"},
      {"id":"slu-1.2","title":"hang the gate","status":"open",
       "dependencies":[{"depends_on_id":"slu-1","type":"parent-child"},
                       {"depends_on_id":"slu-1.1","type":"blocks"}],
       "priority":2,"issue_type":"task"}
    ]"#;

    /// A bead drawn under one it blocks is drawn on a dashed arm, and under
    /// its parent on the solid one every child gets. Otherwise the two copies
    /// are the same row twice, and a reader takes the tree for having
    /// duplicated it.
    ///
    /// The arm is the elbow's, so it holds whether the line rests shut or
    /// open: the fold marker takes the arm's last column exactly as it does
    /// on a child, and the width is the four columns a level every line has.
    #[test]
    fn a_bead_drawn_under_one_it_blocks_hangs_on_a_dashed_arm() {
        let mut forest = flatten(alone("orbital", SLUICE, &panes_on(&["slu-1.1"])));

        assert_eq!(
            sketch(&forest),
            vec![
                "▾ orbital",
                "  └── ◐ slu-1 rehang the sluice",
                "      ├─▸ ◐ .1 forge the new pintles",
                "      └── ○ .2 hang the gate",
                "          └┄▸ ◐ .1 forge the new pintles",
            ]
        );

        forest.apply(Action::ExpandSubtree);

        assert_eq!(
            sketch(&forest),
            vec![
                "▾ orbital",
                "  └── ◐ slu-1 rehang the sluice",
                "      ├── ◐ .1 forge the new pintles",
                "      │   └── ○ .1.1 cast the pintle blanks",
                "      └── ○ .2 hang the gate",
                "          └┄┄ ◐ .1 forge the new pintles",
                "              └── ○ .1.1 cast the pintle blanks",
            ]
        );
    }

    /// Four columns a level, every line in a forest with trees in it,
    /// whatever that line is doing.
    ///
    /// Stated once over whole screens rather than shape by shape. What went
    /// wrong was a span appended to the prefix of one kind of line, and the
    /// next such span will be appended by someone reading a rule about the
    /// kind of line they happen to be drawing.
    ///
    /// The one line outside the rule is the forest with nothing in it, which
    /// stands for the whole screen rather than for a place in a tree and has
    /// no prefix at all.
    #[test]
    fn every_prefix_is_four_columns_a_level() {
        for json in [ORBITAL, DEPOT, RELAY, SIDING, TOWER, BEACON, SLUICE] {
            let staffed = panes_on(&[
                "orb-7.1", "dep-1.1", "rly-2.1", "sdg-4.3", "tow-1.1", "bcn-6", "slu-1.1",
            ]);
            let mut forest = flatten(alone("orbital", json, &staffed));
            four_columns_a_level(&forest);
            forest.apply(Action::ExpandSubtree);
            four_columns_a_level(&forest);
        }
    }

    fn four_columns_a_level(forest: &Forest) {
        for line in forest.lines() {
            assert_eq!(
                columns(&line.prefix),
                2 + 4 * line.depth as usize,
                "a prefix at depth {} is not four columns a level:\n{}",
                line.depth,
                sketch(forest).join("\n")
            );
        }
    }

    /// A root that would not read has nothing under it, so it draws no marker
    /// and holds no fold state either. The two say the same thing about the
    /// same line, and a line offering a fold nothing could act on is how the
    /// marker got there in the first place.
    #[test]
    fn an_unread_root_holds_no_fold_to_set() {
        let forest = flatten(snapshot());
        let header = forest
            .lines()
            .iter()
            .find(|line| matches!(&line.content, Content::Unread(unread) if unread.root == "fer-2"))
            .expect("the shared snapshot draws a tree whose tracker refused");

        assert_eq!(header.folded, None, "{:#?}", sketch(&forest));
        assert_eq!(
            columns(&header.prefix),
            columns(&prefix(&[], true, false, None))
        );
    }

    /// A root that read fine and has nothing under it holds no fold either.
    /// The unread root above takes `draw_tree`'s arm for a tree with no nodes
    /// and never reaches the fold, so this is the only place a bead that is
    /// drawn and has no children is asked whether it offers one.
    ///
    /// The `folded` assertion is the one carrying the weight. A marker is
    /// drawn off `!kids.is_empty() && !open`, which stays false here however
    /// the fold state is decided, so the sketch reads the same whether this
    /// line holds no fold or holds one pointing shut — and a line that holds
    /// one pointing shut is a fold every walk over the forest keeps trying to
    /// open. The screen is where that is invisible, which is why it is asked
    /// of the state instead.
    #[test]
    fn a_root_with_no_children_holds_no_fold_to_set() {
        let forest = flatten(alone("orbital", BEACON, &panes_on(&["bcn-6"])));

        assert_eq!(
            sketch(&forest),
            vec!["▾ orbital", "  └── ◐ bcn-6 re-lamp the beacon"]
        );

        let root = forest
            .lines()
            .iter()
            .find(|line| matches!(&line.content, Content::Bead(row) if row.id == "bcn-6"))
            .expect("the fixture draws its root");

        assert_eq!(root.folded, None, "{:#?}", sketch(&forest));
        assert_eq!(
            columns(&root.prefix),
            columns(&prefix(&[], true, false, None))
        );
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
        parse_agent_list(A_SESSION, json).expect("the panes parse")
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
        let joined = join::resolve(&[], panes, &cfg);
        snapshot::build(
            Collected {
                read_at: every_project_read(),
                ..collected
            },
            panes,
            &joined,
            &cfg,
            a_provider(ProviderState::Answering),
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

    /// The first frame of a run. Nothing has been read, so every project is
    /// drawn from the name the config gave it and holds nothing yet — which
    /// is the point: the reader sees the shape of their work in the time it
    /// takes to draw a frame, rather than a blank terminal for as long as the
    /// trackers take.
    #[test]
    fn a_run_that_has_read_nothing_yet_draws_a_line_for_every_configured_project() {
        let awaiting = Snapshot::awaiting(
            vec!["orbital".to_string(), "ferry".to_string()],
            A_PROVIDER,
            Scope::Everything,
            Filter::LiveAgents,
            now(),
        );

        assert_eq!(sketch(&flatten(awaiting)), vec!["▾ orbital", "▾ ferry"]);
    }

    /// A scope the reader did not type is said on the screen: a reader who
    /// sees one project could think the others vanished. It is the last
    /// line, below the groups, where the hidden trees say what the filter
    /// took away.
    #[test]
    fn a_scope_the_directory_chose_is_said_below_the_groups() {
        let chosen = Snapshot {
            scope: Scope::Directory {
                project: "orbital".to_string(),
                widened: Vec::new(),
            },
            ..built(Filter::LiveAgents)
        };

        let drawn = sketch(&flatten(chosen));

        assert_eq!(
            drawn.last().map(String::as_str),
            Some("  ~ reading orbital")
        );
        let last_group = drawn.iter().rposition(|line| line.contains('['));
        assert!(
            last_group.is_some_and(|at| at + 1 < drawn.len()),
            "the line is not below the groups: {drawn:#?}"
        );
    }

    /// Said from the first frame, before any tracker has answered: the
    /// projects the run is about are on the screen, and so is why.
    #[test]
    fn the_first_frame_already_says_the_directory_chose() {
        let chosen = Snapshot::awaiting(
            vec!["orbital".to_string()],
            A_PROVIDER,
            Scope::Directory {
                project: "orbital".to_string(),
                widened: Vec::new(),
            },
            Filter::LiveAgents,
            now(),
        );

        assert_eq!(
            sketch(&flatten(chosen)),
            vec!["▾ orbital", "  ~ reading orbital"]
        );
    }

    /// Scoping by `--project` is silent because the reader typed it, and a
    /// run reading everything has nothing to say.
    #[test]
    fn a_scope_the_reader_typed_is_silent() {
        for scope in [Scope::Everything, Scope::Asked(vec!["orbital".to_string()])] {
            let awaiting = Snapshot::awaiting(
                vec!["orbital".to_string()],
                A_PROVIDER,
                scope,
                Filter::LiveAgents,
                now(),
            );

            assert_eq!(sketch(&flatten(awaiting)), vec!["▾ orbital"]);
        }
    }

    /// A project whose tracker answered and held nothing draws no line, as it
    /// always has.
    ///
    /// Only a project nothing has read is drawn empty, and `read_at` is the
    /// whole of what tells the two apart. Without it a tracker that answered
    /// with no roots would sit there looking forever about to produce some,
    /// and the forest would never say what it says here instead.
    #[test]
    fn a_project_read_and_holding_nothing_draws_no_line() {
        let read = Snapshot {
            read_at: std::collections::BTreeMap::from([("orbital".to_string(), now())]),
            ..Snapshot::awaiting(
                vec!["orbital".to_string()],
                A_PROVIDER,
                Scope::Everything,
                Filter::LiveAgents,
                now(),
            )
        };

        assert_eq!(sketch(&flatten(read)), vec!["! NoRoots"]);
    }

    /// Every tree a snapshot holds belongs to a project it names.
    ///
    /// The forest walks the projects and takes the trees that follow each
    /// one, so a tree whose project is missing from the list is a tree that
    /// vanishes off the screen with nothing said. Both come from the same
    /// `cfg.projects` inside `build`, which is what makes it true — and this
    /// is what says so, because nothing about the types does.
    #[test]
    fn every_tree_belongs_to_a_project_the_snapshot_names() {
        let snapshot = built(Filter::All);

        for tree in &snapshot.trees {
            assert!(
                snapshot.projects.contains(&tree.project),
                "{} is not among {:?}",
                tree.project,
                snapshot.projects
            );
        }
        // The config's three, and not the fixture's fourth: `lunar` is a
        // failed project no `[[projects]]` entry names, so it is reported
        // among the failures and has no line of its own to be drawn on.
        assert_eq!(snapshot.projects, ["orbital", "ferry", "harbour"]);
    }

    /// Every tracker answered and none of them had a root. The screen has to
    /// carry the reason, because a pane drawn blank reads as a crash.
    #[test]
    fn a_forest_with_nothing_in_it_says_so_rather_than_drawing_nothing() {
        let forest = flatten(only(Collected::default(), &[]));

        assert_eq!(sketch(&forest), vec!["! NoRoots"]);
    }

    /// The fold keys reach a forest that holds nothing, and a walk over it is
    /// given a count of rounds taken from beads there are none of. It has no
    /// fold to point either way, so the screen does not move and still says
    /// what it holds.
    #[test]
    fn the_fold_keys_on_a_forest_with_nothing_in_it_leave_it_saying_so() {
        for action in [Action::ExpandSubtree, Action::CollapseSubtree] {
            let mut forest = flatten(only(Collected::default(), &[]));

            assert!(!forest.apply(action), "{:#?}", sketch(&forest));
            assert_eq!(sketch(&forest), vec!["! NoRoots"]);
        }
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
                        read_at: every_project_read(),
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
                        read_at: every_project_read(),
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
                            tracker: TrackerFailure::Unstartable,
                        }],
                        read_at: every_project_read(),
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
            let forest = flatten(snapshot);

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
        assert!(!says_it_holds_nothing(&flatten(snapshot())));
    }

    // ---- naming a line rather than stepping to it -------------------------

    #[test]
    fn selecting_a_line_puts_the_selection_on_it() {
        let mut forest = flatten(snapshot());
        let reachable = walk_down(&mut forest);

        for at in reachable {
            assert!(
                forest.select_line(at) || forest.selected_line() == at,
                "line {at} would not take the selection: {:#?}",
                sketch(&forest)
            );
            assert_eq!(forest.selected_line(), at, "{:#?}", sketch(&forest));
        }
    }

    /// The keyboard never rests on a note or an elided run, because every
    /// motion filters through the same predicate. A pointer names one line
    /// and no other, so a line the keyboard cannot rest on takes no selection
    /// from a click either — sliding to the neighbour would select something
    /// the reader did not point at.
    #[test]
    fn selecting_a_line_the_selection_cannot_rest_on_leaves_it_where_it_was() {
        let mut forest = flatten(snapshot());
        let unreachable: Vec<usize> = (0..forest.lines().len())
            .filter(|at| !selectable(&forest.lines()[*at]))
            .collect();

        assert!(!unreachable.is_empty(), "{:#?}", sketch(&forest));
        for at in unreachable {
            forest.apply(Action::Move(Motion::FirstRow));
            let was = forest.selected_line();

            assert!(
                !forest.select_line(at),
                "line {at} took the selection: {:#?}",
                sketch(&forest)
            );
            assert_eq!(forest.selected_line(), was);
        }
    }

    #[test]
    fn selecting_a_line_that_is_not_drawn_leaves_the_selection_where_it_was() {
        let mut forest = flatten(snapshot());
        forest.apply(Action::Move(Motion::FirstRow));

        assert!(!forest.select_line(forest.lines().len()));
        assert!(!forest.select_line(usize::MAX));
        assert_eq!(forest.selected_line(), 0);
    }

    #[test]
    fn selecting_the_line_already_selected_changes_nothing() {
        let mut forest = flatten(snapshot());
        forest.apply(Action::Move(Motion::FirstRow));
        let at = forest.selected_line();

        assert!(!forest.select_line(at));
        assert_eq!(forest.selected_line(), at);
    }

    /// A bead reachable from two roots is drawn under each, so a click names
    /// one of two rows carrying the same handle. The refresh that follows
    /// re-derives the selection from that handle, and has to settle on the
    /// copy the pointer landed on rather than on its twin higher up.
    #[test]
    fn selecting_the_lower_copy_of_a_twin_keeps_the_selection_on_it_across_a_refresh() {
        let panes = panes_on(&["qua-1.2", "wha-2.1"]);
        let mut forest = flatten(overlapping(&panes));
        let copies = lines_of(&forest, "qua-1.2");

        assert_eq!(copies.len(), 2, "{:#?}", sketch(&forest));
        let lower = copies[1];

        assert!(forest.select_line(lower), "{:#?}", sketch(&forest));
        forest.refresh(overlapping(&panes));

        assert_eq!(forest.selected_line(), lower, "{:#?}", sketch(&forest));
    }

    // ---- expand all, collapse all, and back to the default ---------------

    /// Every bead on screen, by id, in render order. A bead reachable more
    /// than once is here once per copy drawn.
    fn drawn_beads(forest: &Forest) -> Vec<String> {
        forest
            .lines()
            .iter()
            .filter_map(|line| line.bead().map(|key| key.id.clone()))
            .collect()
    }

    /// Opening a node draws children that were not there to be enumerated
    /// when the key was pressed, so one pass over the lines stops at the
    /// first level it opened. Tower is a spine four deep with nothing live
    /// in it, so every level below the header is a fold no reader could see.
    #[test]
    fn expanding_reaches_a_fold_that_was_not_drawn_when_it_was_pressed() {
        let mut forest = flatten(tower_staffed(&[]));
        assert_eq!(drawn_beads(&forest), ["tow-1"], "{:#?}", sketch(&forest));

        forest.apply(Action::ExpandSubtree);

        assert_eq!(
            drawn_beads(&forest),
            [
                "tow-1",
                "tow-1.1",
                "tow-1.1.1",
                "tow-1.1.1.1",
                "tow-1.2",
                "tow-1.2.1"
            ],
            "{:#?}",
            sketch(&forest)
        );
    }

    /// A walk that runs out of rounds leaves the folds it never reached, so a
    /// subtree that will not settle disagrees with the key that was pressed
    /// instead of never coming back. Tower is a spine four deep, and one
    /// round reaches the level it drew and no further.
    #[test]
    fn a_walk_out_of_rounds_leaves_the_folds_it_did_not_reach() {
        let mut forest = flatten(tower_staffed(&[]));
        assert_eq!(drawn_beads(&forest), ["tow-1"], "{:#?}", sketch(&forest));
        let scope = forest.handle_at(forest.selected).expect("a selected line");

        forest.fold_subtree_in(&scope, 1, true);
        forest.lay_out();

        assert_eq!(
            drawn_beads(&forest),
            ["tow-1", "tow-1.1", "tow-1.2"],
            "{:#?}",
            sketch(&forest)
        );
    }

    /// The lines the selection stands over, and itself: everything from it to
    /// the first line drawn at its own depth or shallower.
    ///
    /// Read off the screen rather than asked of the forest, so a walk that
    /// pointed the folds of the wrong lines is answered by the drawing and
    /// not by the same reckoning that misplaced them.
    fn from_the_selection_down(forest: &Forest) -> impl Iterator<Item = &Line> {
        let at = forest.selected_line();
        let depth = forest.lines()[at].depth;
        forest.lines()[at..].iter().take(1).chain(
            forest.lines()[at + 1..]
                .iter()
                .take_while(move |line| line.depth > depth),
        )
    }

    /// Six shapes of tree, each of them the one tree of its project, so the
    /// root the selection opens on stands over every fold that tree has.
    #[test]
    fn expanding_from_a_root_leaves_no_fold_shut_under_it() {
        for json in [ORBITAL, DEPOT, RELAY, SIDING, TOWER, BEACON] {
            let mut forest = flatten(alone("orbital", json, &two_panes()));
            forest.apply(Action::ExpandSubtree);

            assert!(
                from_the_selection_down(&forest).all(|line| line.folded != Some(false)),
                "{:#?}",
                sketch(&forest)
            );
        }
    }

    /// The mirror, and the project's own line is what says where the scope
    /// stopped: `C` shuts the root it was pressed on and everything that root
    /// stands over, and leaves the project above it as the reader had it.
    #[test]
    fn collapsing_from_a_root_shuts_it_and_leaves_the_project_above_it_open() {
        for json in [ORBITAL, DEPOT, RELAY, SIDING, TOWER, BEACON] {
            let mut forest = flatten(alone("orbital", json, &two_panes()));
            forest.apply(Action::CollapseSubtree);

            assert!(
                from_the_selection_down(&forest).all(|line| line.folded != Some(true)),
                "{:#?}",
                sketch(&forest)
            );
            assert_eq!(
                forest.lines()[0].folded,
                Some(true),
                "{:#?}",
                sketch(&forest)
            );
        }
    }

    /// A run is the one line whose fold draws lines that are not its own
    /// children by any other reckoning, so it is the one where opening the
    /// scope could plausibly lose it. It does not: the run keeps its line
    /// when it opens, and the beads it stood for hang a level under it.
    #[test]
    fn a_run_keeps_its_scope_through_being_opened_and_shut_again() {
        let mut forest = flatten(snapshot());
        select_run(&mut forest);
        let shut = drawn_beads(&forest);

        assert!(
            forest.apply(Action::ExpandSubtree),
            "{:#?}",
            sketch(&forest)
        );
        let opened = drawn_beads(&forest);
        assert!(
            opened.len() > shut.len(),
            "the run drew nothing when it opened: {:#?}",
            sketch(&forest)
        );
        assert!(
            matches!(
                forest.lines()[forest.selected_line()].content,
                Content::Elided { .. }
            ),
            "the selection came off the run: {:#?}",
            sketch(&forest)
        );

        assert!(
            forest.apply(Action::CollapseSubtree),
            "{:#?}",
            sketch(&forest)
        );

        assert_eq!(drawn_beads(&forest), shut, "{:#?}", sketch(&forest));
    }

    /// A project's line takes the scope as a bead's does, and a project is
    /// the widest thing a reader can press these keys on. The others keep
    /// what they had, which is what a reader who navigated to one project is
    /// asking for.
    #[test]
    fn collapsing_from_a_project_leaves_the_other_projects_where_they_were() {
        let mut forest = flatten(built(Filter::All));
        forest.apply(Action::ExpandSubtree);
        select_project(&mut forest, "orbital");
        let elsewhere: Vec<String> = drawn_beads(&forest)
            .into_iter()
            .filter(|id| !id.starts_with("orb-"))
            .collect();
        assert!(
            !elsewhere.is_empty(),
            "nothing outside orbital to be left alone: {:#?}",
            sketch(&forest)
        );

        forest.apply(Action::CollapseSubtree);

        assert_eq!(
            drawn_beads(&forest)
                .into_iter()
                .filter(|id| !id.starts_with("orb-"))
                .collect::<Vec<_>>(),
            elsewhere,
            "{:#?}",
            sketch(&forest)
        );
        assert!(
            !drawn_beads(&forest).iter().any(|id| id.starts_with("orb-")),
            "{:#?}",
            sketch(&forest)
        );
    }

    /// Put the selection on a project's own line, the way a reader would with
    /// the keys they have.
    fn select_project(forest: &mut Forest, project: &str) {
        let at = forest
            .lines()
            .iter()
            .position(
                |line| matches!(&line.content, Content::Project(line) if line.project == project),
            )
            .unwrap_or_else(|| panic!("{project} is not drawn: {:#?}", sketch(forest)));
        step_onto(forest, at);
    }

    /// Put the selection on a bead and fold it, the way a reader would with
    /// the keys they have.
    fn select_bead(forest: &mut Forest, id: &str) {
        let at = *lines_of(forest, id)
            .first()
            .unwrap_or_else(|| panic!("{id} is not drawn: {:#?}", sketch(forest)));
        step_onto(forest, at);
    }

    fn toggle_fold_of(forest: &mut Forest, id: &str) {
        select_bead(forest, id);
        forest.apply(Action::ToggleFold);
    }

    /// `E` folds from the selected node, so a sibling subtree keeps whatever
    /// the reader left it at. `tow-1.2` is shut here and stays shut, which is
    /// the assertion that tells a scoped fold from a global one.
    ///
    /// The scope is a `Handle`, and a handle names the way down to a line
    /// rather than the bead standing on it, so exactly one line carries it —
    /// which is what stops a fold set inside the window from also reaching a
    /// second copy of that bead outside it.
    #[test]
    fn expanding_from_a_node_leaves_a_sibling_subtree_where_it_was() {
        let mut forest = flatten(tower_staffed(&[]));
        toggle_fold_of(&mut forest, "tow-1");
        select_bead(&mut forest, "tow-1.1");

        forest.apply(Action::ExpandSubtree);

        assert_eq!(
            drawn_beads(&forest),
            ["tow-1", "tow-1.1", "tow-1.1.1", "tow-1.1.1.1", "tow-1.2"],
            "{:#?}",
            sketch(&forest)
        );
    }

    /// `C` the same way round: `tow-1.1` shuts over its subtree, `tow-1.2`
    /// keeps the one the reader opened, and the node the selection sits on
    /// is shut rather than left open over shut children.
    #[test]
    fn collapsing_from_a_node_leaves_a_sibling_subtree_where_it_was() {
        let mut forest = flatten(tower_staffed(&[]));
        forest.apply(Action::ExpandSubtree);
        select_bead(&mut forest, "tow-1.1");

        forest.apply(Action::CollapseSubtree);

        assert_eq!(
            drawn_beads(&forest),
            ["tow-1", "tow-1.1", "tow-1.2", "tow-1.2.1"],
            "{:#?}",
            sketch(&forest)
        );
    }

    /// Collapsing opens the subtree first, so that every fold in it lands on
    /// a drawn line and one pass can shut them all. That opening walk is
    /// scoped too: a sibling the reader left shut is not thrown open on the
    /// way past and shut again a frame later.
    ///
    /// `tow-1.2` rests shut here and is never selected, so a walk that opened
    /// the forest and narrowed only the shutting pass would leave `tow-1.2.1`
    /// on screen.
    #[test]
    fn collapsing_from_a_node_does_not_open_a_sibling_on_the_way() {
        let mut forest = flatten(tower_staffed(&[]));
        toggle_fold_of(&mut forest, "tow-1");
        toggle_fold_of(&mut forest, "tow-1.1");
        select_bead(&mut forest, "tow-1.1");

        forest.apply(Action::CollapseSubtree);

        assert_eq!(
            drawn_beads(&forest),
            ["tow-1", "tow-1.1", "tow-1.2"],
            "{:#?}",
            sketch(&forest)
        );
    }

    /// `E` on a shut node opens the node itself and not only what hangs
    /// beneath it, which is the ordinary case for `E`: a key that opened only
    /// the children of a node the reader cannot see inside would leave the
    /// screen exactly as it found it.
    #[test]
    fn expanding_from_a_shut_node_opens_that_node_too() {
        let mut forest = flatten(tower_staffed(&[]));
        toggle_fold_of(&mut forest, "tow-1");
        select_bead(&mut forest, "tow-1.2");
        assert_eq!(
            forest.fold_at(forest.selected_line()),
            Some(false),
            "the node this is about has to start shut: {:#?}",
            sketch(&forest)
        );

        assert!(
            forest.apply(Action::ExpandSubtree),
            "{:#?}",
            sketch(&forest)
        );

        assert_eq!(
            drawn_beads(&forest),
            ["tow-1", "tow-1.1", "tow-1.2", "tow-1.2.1"],
            "{:#?}",
            sketch(&forest)
        );
    }

    /// A row carrying no fold has nothing drawn beneath it — a note, an
    /// unread root, a thing in a group and a leaf bead are all of them — so
    /// the scope taken from such a row is the row alone, and both keys find
    /// nothing to point. The screen does not move and nothing is said.
    ///
    /// The implication runs one way only. A fold with nothing under it is
    /// ordinary: a project waiting on its first collection draws a header
    /// that folds over no trees at all.
    #[test]
    fn the_fold_keys_on_a_row_with_no_fold_leave_the_screen_where_it_was() {
        for action in [Action::ExpandSubtree, Action::CollapseSubtree] {
            let mut forest = flatten(tower_staffed(&[]));
            forest.apply(Action::ExpandSubtree);
            select_bead(&mut forest, "tow-1.1.1.1");
            assert_eq!(
                forest.fold_at(forest.selected_line()),
                None,
                "the row this is about has to carry no fold: {:#?}",
                sketch(&forest)
            );
            let before = sketch(&forest);

            assert!(!forest.apply(action), "{action:?}: {:#?}", sketch(&forest));

            assert_eq!(sketch(&forest), before, "{action:?}");
        }
    }

    /// A fold shut over another fold hides it without settling it, so
    /// shutting only what is on screen leaves that one resting open. The
    /// reader then opens their way back down and a subtree springs at them
    /// from a forest they were told was collapsed.
    ///
    /// `tow-1.1` is shut by hand first, which puts `tow-1.1.1` out of sight
    /// still resting open over the agent beneath it. `C` from the root above
    /// has to reach it there.
    #[test]
    fn collapsing_shuts_a_fold_the_reader_cannot_see() {
        let mut forest = flatten(tower_staffed(&["tow-1.1.1.1"]));
        toggle_fold_of(&mut forest, "tow-1.1");
        select_bead(&mut forest, "tow-1");

        forest.apply(Action::CollapseSubtree);
        toggle_fold_of(&mut forest, "tow-1");
        toggle_fold_of(&mut forest, "tow-1.1");

        assert_eq!(
            drawn_beads(&forest),
            ["tow-1", "tow-1.1", "tow-1.1.1", "tow-1.2"],
            "{:#?}",
            sketch(&forest)
        );
    }

    /// The default is derived from what is live rather than stored, so
    /// restoring it after an agent has gone home opens the spine to the work
    /// that is left and not to where the work was.
    ///
    /// The agent on `tow-1.1.1.1` goes and the one on `tow-1.2.1` stays, so
    /// nothing new arrives under any fold and every fold collapse-all set
    /// survives the refresh. What is restored is therefore the whole of what
    /// the key did, and not something the refresh had already undone.
    #[test]
    fn restoring_the_default_recomputes_it_rather_than_replaying_the_old_one() {
        let mut forest = flatten(tower_staffed(&["tow-1.1.1.1", "tow-1.2.1"]));
        forest.apply(Action::CollapseSubtree);
        forest.refresh(tower_staffed(&["tow-1.2.1"]));

        forest.apply(Action::RestoreDefault);

        assert_eq!(
            sketch(&forest),
            sketch(&flatten(tower_staffed(&["tow-1.2.1"])))
        );
    }

    /// A fold these keys set is a fold the user set, so it keeps the standing
    /// rule: it survives a refresh that brings nothing new beneath it.
    #[test]
    fn the_folds_these_keys_set_survive_a_refresh() {
        for action in [Action::ExpandSubtree, Action::CollapseSubtree] {
            let mut forest = flatten(tower_staffed(&["tow-1.1.1.1"]));
            forest.apply(action);
            let before = sketch(&forest);

            forest.refresh(tower_staffed(&["tow-1.1.1.1"]));

            assert_eq!(sketch(&forest), before, "{action:?}");
        }
    }

    /// No fold `bdi` chooses hides a live agent. `C` is the one place a
    /// reader may override that, because they asked for it by name — and
    /// restoring the default is how they get the agent back.
    #[test]
    fn collapsing_may_shut_a_fold_over_a_live_agent_because_the_reader_asked() {
        let mut forest = flatten(tower_staffed(&["tow-1.1.1.1"]));
        let staffed = "tow-1.1.1.1".to_string();
        assert!(
            drawn_beads(&forest).contains(&staffed),
            "{:#?}",
            sketch(&forest)
        );

        forest.apply(Action::CollapseSubtree);
        assert!(
            !drawn_beads(&forest).contains(&staffed),
            "{:#?}",
            sketch(&forest)
        );

        forest.apply(Action::RestoreDefault);
        assert!(
            drawn_beads(&forest).contains(&staffed),
            "{:#?}",
            sketch(&forest)
        );
    }

    /// These three keys are about folds. Which trees are drawn at all is the
    /// filter's, with its own key and its own word for what it does, so a
    /// reader who pressed `a` deliberately does not lose it to a fold key.
    #[test]
    fn the_fold_keys_leave_the_filter_where_the_reader_put_it() {
        for action in [
            Action::ExpandSubtree,
            Action::CollapseSubtree,
            Action::RestoreDefault,
        ] {
            let mut forest = flatten(built(Filter::LiveAgents));
            forest.apply(Action::ToggleFilter);

            forest.apply(action);

            assert_eq!(forest.snapshot().filter, Filter::All, "{action:?}");
        }
    }

    /// The filter is the reader's too, and a collection landing under them is
    /// not them changing their mind. The one line that offers the key is the
    /// hidden-trees group header, so a refresh that puts the filter back reads
    /// on screen as that group shutting itself.
    #[test]
    fn a_refresh_leaves_the_filter_where_the_reader_put_it() {
        let mut forest = flatten(built(Filter::LiveAgents));
        forest.apply(Action::ToggleFilter);
        let before = sketch(&forest);

        forest.refresh(built(Filter::LiveAgents));

        assert_eq!(forest.snapshot().filter, Filter::All);
        assert_eq!(sketch(&forest), before);
    }

    /// `apply` reports whether the screen moved, and a subtree already open
    /// has nowhere to go. The loop redraws on that answer.
    #[test]
    fn expanding_from_a_node_already_open_moves_nothing() {
        let mut forest = flatten(tower_staffed(&[]));

        assert!(
            forest.apply(Action::ExpandSubtree),
            "{:#?}",
            sketch(&forest)
        );
        assert!(
            !forest.apply(Action::ExpandSubtree),
            "{:#?}",
            sketch(&forest)
        );
    }

    // ---- going to a bead the reader has not walked to --------------------

    /// The forest can say where the selection is, as a place: a bead drawn
    /// under two parents is drawn twice, and a caller keeping this to come
    /// back to is keeping the copy the reader was looking at.
    #[test]
    fn the_forest_says_where_the_selection_is() {
        let forest = flatten(snapshot());

        assert_eq!(
            forest.place().map(|place| place.key().clone()),
            Some(key("orbital", "orb-7"))
        );
    }

    /// Nothing where the selection is not on a bead at all, which is a line
    /// with nowhere to come back to rather than a line with no name.
    #[test]
    fn a_line_that_is_not_a_bead_is_nowhere_to_come_back_to() {
        let mut forest = flatten(snapshot());
        forest.apply(Action::Move(Motion::FirstRow));

        assert_eq!(forest.place(), None, "{:#?}", sketch(&forest));
    }

    /// A bead the trees hold is one the forest can go to, whether or not it
    /// is drawn this instant; one no tree holds is not. The question is asked
    /// of the snapshot, so it goes on being the same question when roots are
    /// found another way.
    #[test]
    fn the_forest_draws_the_beads_its_trees_hold_and_no_others() {
        let forest = flatten(snapshot());

        assert!(forest.draws(&key("orbital", "orb-7.1.1")));
        assert!(!forest.draws(&key("orbital", "orb-404")));
        assert!(
            !forest.draws(&key("ferry", "orb-7")),
            "a bead is (project, id), so one project's id is not another's"
        );
    }

    /// The whole point: `orb-7.1` is drawn shut, so `orb-7.1.1` is on no line
    /// at all. Going to it opens what is over it and lands on it.
    #[test]
    fn going_to_a_bead_under_a_shut_fold_opens_it_and_lands_there() {
        let mut forest = flatten(snapshot());
        assert!(
            !drawn_here(&forest, "true the mount"),
            "the bead is drawn already, so this would test nothing: {:#?}",
            sketch(&forest)
        );

        assert!(forest.go_to(&key("orbital", "orb-7.1.1")));

        assert_eq!(cursor(&forest), Some(&key("orbital", "orb-7.1.1")));
        assert!(
            drawn_here(&forest, "true the mount"),
            "{:#?}",
            sketch(&forest)
        );
    }

    /// A tree the filter took out of the forest is drawn in the group below
    /// its project, and that group has a fold of its own. Both open, and the
    /// project's with them.
    #[test]
    fn going_to_a_bead_in_a_tree_the_filter_hid_opens_the_group_over_it() {
        let mut forest = flatten(snapshot());

        assert!(forest.go_to(&key("harbour", "hbr-3.1")));

        assert_eq!(cursor(&forest), Some(&key("harbour", "hbr-3.1")));
    }

    /// A fold the reader shut by hand stays shut until something asks
    /// otherwise, and asking to be taken to a bead underneath it is asking.
    #[test]
    fn going_to_a_bead_opens_a_fold_the_reader_shut_by_hand() {
        let mut forest = flatten(snapshot());
        forest.go_to(&key("orbital", "orb-7.1"));
        forest.apply(Action::CollapseSubtree);
        assert!(
            !drawn_here(&forest, "true the mount"),
            "{:#?}",
            sketch(&forest)
        );

        assert!(forest.go_to(&key("orbital", "orb-7.1.1")));

        assert_eq!(cursor(&forest), Some(&key("orbital", "orb-7.1.1")));
    }

    /// A bead no tree holds is nowhere to go, and the forest is left exactly
    /// as it was rather than half-opened on the way to nothing.
    #[test]
    fn going_to_a_bead_no_tree_holds_moves_nothing() {
        let mut forest = flatten(snapshot());
        let was = sketch(&forest);
        let selected = forest.selected_line();

        assert!(!forest.go_to(&key("orbital", "orb-404")));

        assert_eq!(sketch(&forest), was);
        assert_eq!(forest.selected_line(), selected);
    }

    /// A bead drawn under two parents is two lines, and coming back means the
    /// one the reader was on. A key could not say which; a place does.
    #[test]
    fn coming_back_to_a_place_lands_on_the_copy_that_was_left() {
        let mut forest = flatten(drawn_twice_in_one_tree());
        forest.apply(Action::ExpandSubtree);
        let twice: Vec<Place> = forest
            .lines()
            .iter()
            .filter_map(|line| line.place.clone())
            .filter(|place| *place.key() == key("orbital", "orb-9.1"))
            .collect();
        assert_eq!(
            twice.len(),
            2,
            "the fixture draws it twice: {:#?}",
            sketch(&forest)
        );

        forest.apply(Action::Move(Motion::FirstRow));
        assert!(forest.go_to_place(&twice[1]));

        assert_eq!(forest.place(), Some(&twice[1]));
        assert_ne!(forest.place(), Some(&twice[0]), "the other copy of it");
    }

    fn drawn_here(forest: &Forest, said: &str) -> bool {
        sketch(forest).iter().any(|row| row.contains(said))
    }
}
