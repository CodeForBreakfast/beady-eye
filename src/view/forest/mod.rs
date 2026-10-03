//! The snapshot, flattened into the lines the screen shows.
//!
//! One module for what a line that folds or holds the selection is known by,
//! and one for the lines a snapshot draws under those folds. What is left
//! here is the forest itself: the folds the user has set, and where the
//! selection sits.

mod drawn;
mod facts;
mod handle;
mod layout;
mod search;
mod spine;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use crate::model::join::BeadKey;
use crate::model::snapshot::{Filter, Snapshot, TrackerState, Tree};
use crate::view::lines::{beneath, links_below, quiet, root_key, Content, GroupKind, Line, Place};
use crate::view::row;
use crate::view::{Action, Motion, Notch};

pub use drawn::Drawn;
use drawn::{Beneath, Node};
use facts::{Facts, TreeFacts};
use handle::{handle_of, root_handle, selectable, Folds, Handle};
use layout::Rooted;
use search::{Matches, Sought};
pub use spine::Spine;

/// Where a search came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Landed {
    /// Nothing any tree read holds matches the text searched for, so there
    /// was nowhere to go and the forest is left exactly as it was.
    ///
    /// Carries the text because the answer is about it — the foot says what
    /// found nothing — and because the forest is the only thing that still
    /// knows it once a walk rather than a fresh search has come to nothing.
    Nowhere(String),
    /// The selection is on `key`, the `at`th of `of` beads matching, counted
    /// from one and counted in the order the forest draws them.
    ///
    /// Screen order and not a ranking, so the ordinal is a fact about the
    /// forest rather than about the search: the reader can count it off the
    /// screen, and the same bead is the same number however they reached it.
    On { key: BeadKey, at: usize, of: usize },
}

/// Where a search began: what abandoning it puts back, and where each of
/// its keystrokes counts the matches from.
///
/// A place among the beads rather than a line, so a collection landing while
/// the reader types still leaves each keystroke counting from the same bead.
pub struct Origin {
    cursor: Option<Handle>,
    from: usize,
    opened_by_the_last_step: BTreeSet<Handle>,
    searched: Option<String>,
    standing: Option<(Place, bool)>,
}

/// One snapshot's lines in render order, with the fold state and the selection
/// that decide which of them are visible and which one is current.
pub struct Forest {
    snapshot: Snapshot,
    /// What layout reads of the snapshot, answered when it was taken.
    facts: Arc<Facts>,
    folds: Folds,
    /// Which rule opens the spine where no line beneath the top of the
    /// forest says otherwise.
    spine: Spine,
    /// Which rule opens the spine under each line the reader set one on.
    /// Keyed as a hand fold is, so a rule set on a way down that leaves the
    /// tree names no line and goes with it.
    spines: BTreeMap<Handle, Spine>,
    cursor: Option<Handle>,
    /// The first line the last frame had room to draw, and how many it had
    /// room for.
    ///
    /// The room is the frame's to say and a scroll reads it off the last one
    /// drawn, exactly as the bead window's `Show` does. A frame is always
    /// drawn before a key or a notch is answered, so the first of either
    /// never reads a band nobody has measured.
    from: usize,
    room: usize,
    lines: Drawn,
    selected: usize,
    /// The beads the forest is rooted at, where the reader has asked for any:
    /// the one Shift+F was pressed on, or the ones the command line named.
    ///
    /// The line they asked on rather than the bead standing on it, so a bead
    /// drawn more than once roots the forest at the copy they were on. Held
    /// rather than derived, because the selection moves afterwards and the
    /// mode does not move with it. None is at or beneath another.
    focused: Vec<Place>,
    /// The beads the command line named that no collection has drawn yet,
    /// each focused as one does.
    named: Vec<BeadKey>,
    /// What was last searched for, which `n` and `N` step through.
    ///
    /// The text and not the matches it found. A match set held between
    /// presses would be stale the moment a collection landed, and a place in
    /// one would be wrong the moment the reader moved by hand; the text is
    /// still true after both, and the matches are worked out again from where
    /// the selection has actually got to.
    searched: Option<String>,
    /// Which cells a bead's row draws, and in what order: what the lines'
    /// identity widths are measured over.
    layout: row::Layout,
}

/// Flatten a snapshot into its lines.
pub fn flatten(snapshot: Snapshot) -> Forest {
    let spine = Spine::default();
    let spines = BTreeMap::new();
    let mut forest = Forest {
        facts: Arc::new(Facts::of(&snapshot, spine, &spines)),
        snapshot,
        folds: Folds::default(),
        spine,
        spines,
        cursor: None,
        from: 0,
        room: 0,
        lines: Drawn::default(),
        selected: 0,
        focused: Vec::new(),
        named: Vec::new(),
        searched: None,
        layout: row::Layout::default(),
    };
    forest.lay_out();
    forest.select_first_root();
    forest
}

impl Forest {
    /// The visible lines, in render order.
    pub fn lines(&self) -> &Drawn {
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

    /// The first line the viewport is showing.
    pub fn from(&self) -> usize {
        self.from
    }

    /// Take the measure of the band the last frame gave the forest, and come
    /// back inside the forest where a shorter band has left the viewport past
    /// its end. The renderer is the only thing that knows the band's height,
    /// so it says.
    pub fn fit(&mut self, room: usize) {
        self.room = room;
        self.from = self.from.min(self.furthest());
    }

    /// The furthest the viewport can be scrolled and still be full.
    fn furthest(&self) -> usize {
        self.lines.len().saturating_sub(self.room)
    }

    /// How far a half-screen motion moves: half the band the trees are in,
    /// rather than half a screen the key bar and the tail also sit in. Never
    /// nowhere, so the key does something on the shortest band there is.
    fn half_screen(&self) -> usize {
        (self.room / 2).max(1)
    }

    /// Move the viewport one notch of the wheel, leaving the selection where
    /// the reader put it, and report whether it moved. How far a notch goes
    /// is the config's, so the caller holding one says.
    pub fn scrolled(&mut self, notch: Notch, lines: usize) -> bool {
        let to = match notch {
            Notch::Up => self.from.saturating_sub(lines),
            Notch::Down => (self.from + lines).min(self.furthest()),
        };
        let moved = to != self.from;
        self.from = to;
        moved
    }

    /// Bring the selection inside the viewport, where the last frame left it
    /// above or below what there was room for, and report whether the view
    /// moved.
    ///
    /// By the least it can, rather than by putting the selection back in the
    /// middle. A reader who has wheeled the view somewhere and then steps one
    /// row keeps what they were looking at, and it is what the bead window
    /// already does with a row it has to reveal.
    ///
    /// Reported because a keystroke that moves the view and nothing else is a
    /// keystroke the screen has to be redrawn for: `g` on a selection the
    /// wheel has scrolled away from moves no row and changes every one of
    /// them.
    fn reveal(&mut self) -> bool {
        let was = self.from;
        if self.selected < self.from {
            self.from = self.selected;
        } else if self.room > 0 && self.selected >= self.from + self.room {
            self.from = self.selected + 1 - self.room;
        }
        self.from != was
    }

    /// Measure the identity widths over these cells from now on.
    pub fn laid_out_to(&mut self, layout: row::Layout) {
        if self.layout == layout {
            return;
        }
        self.layout = layout;
        self.lay_out();
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
        // The bead leaving the collection ends the mode, and the place it
        // stood on goes with it. Kept, it would take the next press of the
        // key and spend it putting back a forest that is already back.
        //
        // Found again by the bead rather than by the way down to it, because
        // a tracker that reparented it has moved the bead and not lost it.
        let focused = std::mem::take(&mut self.focused);
        self.focused = outermost(
            focused
                .iter()
                .filter_map(|place| self.rerooted(place))
                .collect(),
        );
        let first_focused = self.focus_the_named();
        if self.focused != focused {
            self.answer();
        }
        self.spend_folds(&folded_over);
        // A root whose tracker stopped reading, or started again, is the
        // same line under the other kind of handle.
        self.cursor = ancestry
            .into_iter()
            .map(|handle| match handle {
                Handle::Bead(place) | Handle::Unread(place) => self.handle_on(&place),
                other => other,
            })
            .find(|handle| self.present(handle));
        if let Some(place) = first_focused {
            self.cursor = Some(Handle::Bead(place));
        }
        self.lay_out();
    }

    /// Start focused on the beads the command line named, as Shift+F on each
    /// would: now for the ones this snapshot draws, and for the rest when a
    /// collection draws them.
    pub fn focus_when_drawn(&mut self, beads: Vec<BeadKey>) {
        self.named = beads;
        if let Some(place) = self.focus_the_named() {
            self.answer();
            self.cursor = Some(Handle::Bead(place));
            self.lay_out();
        }
    }

    /// Focus each named bead the snapshot draws, and give up on one its
    /// tracker has reported missing, where its tree would be. Any other is
    /// waited for: a tracker that failed to answer has not said the bead is
    /// not there.
    ///
    /// Hands back the first bead focused where nothing was, for the
    /// selection to go to, as it would be on the bead Shift+F was pressed on.
    fn focus_the_named(&mut self) -> Option<Place> {
        if self.named.is_empty() {
            return None;
        }
        let was_rooted = !self.focused.is_empty();
        for key in std::mem::take(&mut self.named) {
            match self.place_of(&key) {
                Some(place) => self.focused.push(place),
                None if self.reported_missing(&key) => {}
                None => self.named.push(key),
            }
        }
        self.focused = outermost(std::mem::take(&mut self.focused));
        self.focused.first().filter(|_| !was_rooted).cloned()
    }

    /// Whether the bead's tracker answered without it.
    fn reported_missing(&self, key: &BeadKey) -> bool {
        self.snapshot.collected.iter().any(|tree| {
            tree.project == key.project
                && tree.root == key.id
                && tree.tracker == TrackerState::RootNotFound
        })
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
    ///
    /// A scope that shut is spent along the way down to what arrived and no
    /// further: the folds beside that way were folded away too, and nothing
    /// new is under them. The way down is read off the forest drawn as it
    /// stands unrooted, because the mode draws the bead it is rooted at
    /// apart from the line the scope was set on, and the way down to what
    /// arrived under that bead runs through both.
    fn spend_folds(&mut self, folded_over: &BTreeMap<Handle, BTreeSet<BeadKey>>) {
        let spent: Vec<(Handle, BTreeSet<BeadKey>)> = folded_over
            .iter()
            .filter_map(|(handle, over)| {
                let arrived = &self.live_under(handle) - over;
                (!arrived.is_empty()).then(|| (handle.clone(), arrived))
            })
            .collect();
        if spent.is_empty() {
            return;
        }
        let drawn = layout::draw_beneath_every_fold(
            &self.snapshot,
            &self.facts,
            &self.folds,
            &[],
            &[],
            &self.layout,
        );
        let anywhere = arrived_anywhere(&spent);
        let mut reaching = Reaching::new(&anywhere);
        for (handle, arrived) in spent {
            let path = way_down_to(&drawn, &handle, &arrived, &mut reaching);
            self.folds.spend(&handle, path);
        }
    }

    /// The beads beneath `handle` carrying live work. Every bead in a
    /// project's trees for the project, and for the groups of its own that
    /// hold trees — wider than the group, and a group over trees rests shut
    /// whatever arrives, so spending it early changes nothing on screen.
    /// Empty for a run, which holds only finished branches, and for a group
    /// whose things are not beads at all.
    fn live_under(&self, handle: &Handle) -> BTreeSet<BeadKey> {
        let project = match handle {
            Handle::Bead(place) => return self.live_beneath(place),
            Handle::Project(project) => project,
            Handle::Group(GroupKind::HiddenTrees | GroupKind::OutOfTheWay, Some(project)) => {
                project
            }
            _ => return BTreeSet::new(),
        };
        self.snapshot
            .collected
            .iter()
            .filter(|tree| &tree.project == project)
            .flat_map(|tree| {
                tree.beads
                    .iter()
                    .filter(|bead| !quiet(bead))
                    .map(|bead| bead.key())
            })
            .collect()
    }

    fn live_beneath(&self, place: &Place) -> BTreeSet<BeadKey> {
        let Some((tree, way)) = self.locate(place) else {
            return BTreeSet::new();
        };
        let (at, above) = way.split_last().expect("a way down ends somewhere");
        beneath(tree, *at, above)
            .into_iter()
            .filter(|node| !quiet(&tree.beads[*node]))
            .map(|node| tree.beads[node].key())
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
            Some(Handle::Bead(place) | Handle::Unread(place)) => place,
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
        // A line the mode is holding back is behind the line it put it
        // behind, and the group the filter would have put its tree in is not
        // drawn at all while the forest is rooted at one bead.
        if self.held_back(place) {
            chain.push(Handle::Group(
                GroupKind::OutOfTheWay,
                Some(place.tree.project.clone()),
            ));
        } else if self.hidden(&place.tree) {
            chain.push(Handle::Group(
                GroupKind::HiddenTrees,
                Some(place.tree.project.clone()),
            ));
        }
        chain.push(Handle::Project(place.tree.project.clone()));
        chain
    }

    /// Whether the mode is holding a line back, which it is for every line but
    /// the beads the forest is rooted at and the ones beneath them.
    ///
    /// The rest of such a bead's own root is held back as much as another root
    /// is: the mode draws the bead where a root is drawn and stops there, so
    /// the beads above it are behind the line the other roots are behind.
    fn held_back(&self, place: &Place) -> bool {
        !self.focused.is_empty() && self.focused_over(place).is_none()
    }

    /// The focused bead a place is at or beneath, where it is.
    fn focused_over(&self, place: &Place) -> Option<&Place> {
        self.focused
            .iter()
            .find(|focused| at_or_beneath(place, focused))
    }

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
    ///
    /// `links_below` here rather than `Facts::split`, and deliberately. The
    /// walks that decide *which* copy — `children_entries` drawing it,
    /// a search counting it, `stepped_to` resolving into it — all read
    /// `split`, because for them first means first drawn. This one is handed
    /// the copy already, as the ids in `place.steps`, and finds each by name:
    /// same links, same bead, whatever order they arrive in. Putting it on
    /// `split` would couple a resolver to `Facts` and buy no property.
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
    ///
    /// The lines are drawn from the folds, the root and the filter, so an
    /// action that touched none of them would draw the lines it already
    /// holds, and the draw is skipped: a key that moves only the selection
    /// answers at once on a forest of any size.
    pub fn apply(&mut self, action: Action) -> bool {
        self.keep_what_is_open();
        let selected = self.selected;
        // The rule in force is on the screen as well as in the rows, and a
        // forest whose every fold is set by hand draws the same rows under
        // every rule — so a press that moves the rule and no row still has
        // something new to say.
        let spine = self.spine();
        let redraw = match action {
            Action::Move(motion) => {
                self.move_to(motion);
                false
            }
            Action::CollapseOrParent => self.collapse_or_parent(),
            Action::ExpandOrChild => self.expand_or_child(),
            Action::ToggleFold => {
                self.toggle_fold();
                true
            }
            Action::ExpandSubtree => {
                self.fold_subtree(true);
                true
            }
            Action::CollapseSubtree => {
                self.fold_subtree(false);
                true
            }
            Action::RestoreSubtree => {
                self.restore_subtree();
                true
            }
            Action::ExpandForest => {
                self.fold_in(None, true);
                true
            }
            Action::CollapseForest => {
                self.fold_in(None, false);
                true
            }
            Action::RestoreDefault => {
                self.folds.clear();
                true
            }
            Action::CycleSpine => {
                self.cycle_spine();
                true
            }
            Action::CycleSpineForest => {
                self.cycle_spine_forest();
                true
            }
            Action::ToggleFilter => {
                self.toggle_filter();
                true
            }
            Action::FocusForest => {
                self.focus_forest();
                true
            }
            Action::Focus
            | Action::ShowBead
            | Action::NextRelated
            | Action::Back
            | Action::CopyId
            | Action::ShowBindings
            | Action::Search
            | Action::NextMatch
            | Action::PreviousMatch
            | Action::Refresh
            | Action::Quit => return false,
        };
        let redrawn = if redraw {
            let was = self.lay_out();
            self.lines != was
        } else {
            false
        };
        let revealed = self.reveal();
        self.selected != selected || redrawn || revealed || self.spine() != spine
    }

    /// Root the forest at the selected bead, or put it back where it is
    /// already rooted.
    ///
    /// Putting it back leaves the selection on the bead the reader rooted it
    /// at, wherever they had walked to under the mode: they asked to finish
    /// that bead, and the forest they came back to is the one they left.
    /// Rooted at several, that is the one the selection is under, or the
    /// first where it is under none.
    ///
    /// A line carrying no bead roots the forest at nothing. A root whose
    /// tracker refused holds a place but no node, and there is no tree to draw
    /// from a bead that is not there.
    ///
    /// Either way the beads the command line named and no collection has
    /// drawn yet are let go of: the reader has taken the mode in hand.
    fn focus_forest(&mut self) {
        self.named.clear();
        if !self.focused.is_empty() {
            let under = match &self.cursor {
                Some(Handle::Bead(on)) => self.focused_over(on).cloned(),
                _ => None,
            };
            let focused = std::mem::take(&mut self.focused);
            let place = under.unwrap_or_else(|| focused[0].clone());
            self.answer();
            let on = Handle::Bead(place.clone());
            // Drawn again first, so what is shut over the bead is asked of
            // the forest the reader is coming back to rather than of the one
            // they are leaving. The rows it drew go back afterwards, because
            // the press is read for whether the screen changed and these are
            // the rows it changed from.
            let rooted = self.lay_out();
            let on_screen = self.lines.row_of(&on).is_some();
            self.lines = rooted;
            if !on_screen {
                // They shut something over it from inside the mode, and a
                // forest that comes back with the selection somewhere else
                // has not put them back where they were.
                self.open_over(&place);
            }
            self.cursor = Some(on);
            return;
        }
        let Some(Handle::Bead(place)) = self.cursor.clone() else {
            return;
        };
        if self.locate(&place).is_some() {
            self.focused = vec![place];
            self.answer();
        }
    }

    /// Where the bead a place stood on is now, which is the place itself
    /// while nothing has moved. Nothing at all once the collection no longer
    /// holds that bead, which is what ends the mode.
    ///
    /// A place with no steps stands on the root of its tree, and a root filed
    /// under another root has moved as much as any other bead: it is looked
    /// for by the same name, which for that place is the tree's own.
    ///
    /// Asked for the bead rather than for the row: a root whose tracker has
    /// since refused keeps a row saying so, and there is no tree to draw from
    /// a bead that is not there.
    fn rerooted(&self, place: &Place) -> Option<Place> {
        if self.locate(place).is_some() {
            return Some(place.clone());
        }
        self.place_of(place.steps.last().unwrap_or(&place.tree))
    }

    /// Where the forest is rooted, at each focused bead the snapshot in hand
    /// still draws.
    ///
    /// Resolved against that snapshot on every layout rather than kept, so a
    /// collection that moved a bead is followed and one that dropped it
    /// lets it go.
    fn rooted(&self) -> Vec<Rooted> {
        self.focused
            .iter()
            .filter_map(|place| {
                let (_, way) = self.locate(place)?;
                Some(Rooted {
                    place: place.clone(),
                    way,
                })
            })
            .collect()
    }

    /// `s`: put the rule after the one in force at the selection in force
    /// under the selected node, and leave every hand fold alone.
    ///
    /// Whatever rule the reader had set beneath the node goes, as a scope
    /// that points folds takes what was set beneath it: the node is what
    /// they are asking about now. A line that is not a bead's stands for no
    /// way down and answers to neither key.
    fn cycle_spine(&mut self) {
        let Some(Handle::Bead(place)) = self.handle_at(self.selected) else {
            return;
        };
        let next = self.spine_on(&place).next();
        self.spines
            .retain(|handle, _| !beneath_the_line(&place, handle));
        self.spines.insert(Handle::Bead(place), next);
        self.answer();
    }

    /// `S`: the same for the whole forest, which is every line at the top of
    /// it and everything under them.
    fn cycle_spine_forest(&mut self) {
        self.spine = self.spine.next();
        self.spines.clear();
        self.answer();
    }

    /// The rule in force at the selection, which is what the screen says it
    /// is. The forest's own where the selection is not on a bead's line: no
    /// way down stands there for a rule to have been set on.
    pub fn spine(&self) -> Spine {
        match self.handle_at(self.selected) {
            Some(Handle::Bead(place)) => self.spine_on(&place),
            _ => self.spine,
        }
    }

    /// Whether the forest is rooted at the beads the reader focused, rather
    /// than drawn whole.
    pub fn is_focused(&self) -> bool {
        !self.focused.is_empty()
    }

    /// The rule in force on one line: the one set on it, the one set on the
    /// nearest line above it, or the forest's.
    fn spine_on(&self, place: &Place) -> Spine {
        std::iter::once(place.clone())
            .chain(place.forebears())
            .find_map(|above| self.spines.get(&Handle::Bead(above)).copied())
            .unwrap_or(self.spine)
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
    ///
    /// Each bead the forest is rooted at is where the rule in force over it
    /// begins, as a rule set on its line would.
    fn answer(&mut self) {
        let mut spines = self.spines.clone();
        for place in &self.focused {
            spines
                .entry(Handle::Bead(place.clone()))
                .or_insert_with(|| self.spine_on(place));
        }
        self.facts = Arc::new(Facts::of(&self.snapshot, self.spine, &spines));
    }

    /// `e` and `c`: point every fold in the selected node's subtree, at every
    /// depth, one way. `E` and `C` are the same walk with no scope.
    ///
    /// The scope is a handle rather than a bead, so a bead reachable more
    /// than once puts only the way down the selection took inside the scope.
    ///
    /// The walk draws without laying out, because `apply` tells the loop
    /// whether the screen moved by comparing against the lines its own
    /// lay-out displaced — one in here would leave it comparing the new lines
    /// with themselves.
    fn fold_subtree(&mut self, open: bool) {
        let Some(scope) = self.handle_at(self.selected) else {
            return;
        };
        self.fold_in(Some(&scope), open);
    }

    /// `d`: let go of every hand fold on the selected node and everything
    /// under it, and no other.
    ///
    /// A hand fold under a shut node is out of sight and still a hand fold,
    /// so the lines are drawn beneath every fold to reach it there.
    fn restore_subtree(&mut self) {
        let Some(scope) = self.handle_at(self.selected) else {
            return;
        };
        let drawn = self.drawn_beneath_every_fold(std::slice::from_ref(&scope));
        let Some(within) = drawn.node_of(&scope) else {
            return;
        };
        if self.points_by_the_line(&scope) {
            let mut lines = Vec::new();
            drawn.visit(within, &mut |node| {
                lines.extend(handle_of(&node.line));
                true
            });
            for handle in lines {
                self.folds.put_back(handle);
            }
            return;
        }
        let beneath = named_beneath(&drawn, within);
        self.folds.let_go(scope, &beneath);
    }

    /// Point every fold in `scope`'s subtree, or in the whole forest where
    /// there is no scope, at `open`.
    ///
    /// A fold the reader cannot see is still a fold, so the lines are drawn
    /// beneath every fold and the walk reads that one draw. Every fold
    /// beneath is pointed, the ones resting that way already included, so
    /// what the key set holds across a refresh: one left open under a shut
    /// parent would spring its subtree back the moment that parent was
    /// opened again, and one left resting open would shut when what held
    /// it open moved on.
    ///
    /// The whole forest is every line at the top of it, each taken as a
    /// scope of its own.
    fn fold_in(&mut self, scope: Option<&Handle>, open: bool) {
        let drawn = self.drawn_beneath_every_fold(scope.cloned().as_slice());
        let scopes: Vec<Handle> = match scope {
            Some(scope) => vec![scope.clone()],
            None => drawn
                .top()
                .iter()
                .filter_map(|node| handle_of(&node.line))
                .collect(),
        };
        for scope in scopes {
            let Some(within) = drawn.node_of(&scope) else {
                continue;
            };
            if within.line.folded.is_none() {
                continue;
            }
            if self.points_by_the_line(&scope) {
                let mut pointed = Vec::new();
                drawn.visit(within, &mut |node| {
                    if node.line.folded.is_some_and(|was| !open || !was) {
                        pointed.extend(handle_of(&node.line));
                    }
                    true
                });
                for handle in pointed {
                    self.folds.set(handle, open);
                }
                continue;
            }
            let beneath = named_beneath(&drawn, within);
            self.folds.set_over(scope, open, &beneath);
        }
    }

    /// Whether a key that points a subtree from `scope` points each fold
    /// beneath it by itself, as every line did once, rather than writing
    /// one entry that everything beneath answers from. It does where the
    /// line is passing, and where the mode is holding the line back: the
    /// bead the forest is rooted at is drawn apart from the root above it,
    /// so a scope set on that root would reach the bead the key never drew.
    fn points_by_the_line(&self, scope: &Handle) -> bool {
        passing(scope) || matches!(scope, Handle::Bead(place) if self.held_back(place))
    }

    /// The lines with every fold opened over, each fold still saying which
    /// way it points, and the lines `also` names drawn as the folds' own
    /// are, so a key pressed on one finds it however the folds stand.
    fn drawn_beneath_every_fold(&self, also: &[Handle]) -> Drawn {
        let rooted = self.rooted();
        layout::draw_beneath_every_fold(
            &self.snapshot,
            &self.facts,
            &self.folds,
            &rooted,
            also,
            &self.layout,
        )
    }

    fn toggle_fold(&mut self) {
        if let (Some(open), Some(handle)) =
            (self.fold_at(self.selected), self.handle_at(self.selected))
        {
            self.folds.set(handle, !open);
        }
    }

    /// `h`: shut an open node, and step out of one already shut. Reports
    /// whether it shut one.
    fn collapse_or_parent(&mut self) -> bool {
        match (self.fold_at(self.selected), self.handle_at(self.selected)) {
            (Some(true), Some(handle)) => {
                self.folds.set(handle, false);
                true
            }
            _ => {
                self.step_to(self.parent_of(self.selected));
                false
            }
        }
    }

    /// `l`: open a shut node, and step into one already open. Reports
    /// whether it opened one.
    fn expand_or_child(&mut self) -> bool {
        match (self.fold_at(self.selected), self.handle_at(self.selected)) {
            (Some(false), Some(handle)) => {
                self.folds.set(handle, true);
                true
            }
            _ => {
                self.step_to(self.first_child_of(self.selected));
                false
            }
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
                let at = self.selected.saturating_sub(self.half_screen());
                self.scan(at, false).or_else(|| self.scan(at, true))
            }
            Motion::HalfScreenDown => {
                let at = (self.selected + self.half_screen()).min(last);
                self.scan(at, true).or_else(|| self.scan(at, false))
            }
        };
        self.step_to(target);
    }

    /// Put the selection on the line at `at`, reporting whether the screen
    /// has changed.
    ///
    /// Named rather than stepped to, and that is the whole difference from a
    /// motion: a line the selection cannot rest on keeps none, because the
    /// pointer named that line and not the one below it. Which lines those
    /// are is the keyboard's question, asked here in the keyboard's words.
    pub fn select_line(&mut self, at: usize) -> bool {
        self.keep_what_is_open();
        let was = self.selected;
        let mut revealed = false;
        if self.lines.get(at).is_some_and(selectable) {
            self.step_to(Some(at));
            revealed = self.reveal();
        }
        self.selected != was || revealed
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
        self.keep_what_is_open();
        if !self.drawn(place) {
            return false;
        }
        self.open_over(place);
        self.select_place(place)
    }

    /// Put the selection on a match for a search step, shutting what the
    /// last step opened before opening what this one needs. Reports whether
    /// the selection is on it now.
    fn step_to_place(&mut self, place: &Place) -> bool {
        if !self.drawn(place) {
            return false;
        }
        self.folds.shut_what_the_last_step_opened();
        for over in self.folds_over(place) {
            self.folds.open_for_a_step(over);
        }
        self.select_place(place)
    }

    /// Make whatever the last search step opened the reader's, so no later
    /// step shuts it. Every act of the reader's but a search step does.
    pub fn keep_what_is_open(&mut self) {
        self.folds.keep_what_the_last_step_opened();
    }

    fn select_place(&mut self, place: &Place) -> bool {
        let handle = self.handle_on(place);
        self.cursor = Some(handle.clone());
        self.lay_out();
        self.reveal();
        self.cursor == Some(handle)
    }

    /// What the line a place stands for is known by: a bead's, or the row
    /// of a root whose tracker would not read.
    fn handle_on(&self, place: &Place) -> Handle {
        match self.tree_of(place) {
            Some(tree) if place.steps.is_empty() => root_handle(tree),
            _ => Handle::Bead(place.clone()),
        }
    }

    /// Put the selection on a bead matching what the reader typed, wherever
    /// the forest draws it, and report which of the matches it is. Nowhere
    /// where nothing matches, and the forest is left as it was.
    ///
    /// A match is a bead holding the text in its id or in its title, letter
    /// case aside. Part of either and not the whole of one, because a
    /// fragment is what the reader has: the forest row draws a *shortened*
    /// id and `row::abbreviate` is the only thing in `bdi` that draws one, so
    /// on a long screen it is the only spelling of a bead they have been
    /// shown — and a title is prose they are quoting a word out of.
    ///
    /// So a search no longer supplies the half of a key the reader did not
    /// type. It answers with beads, each already a whole `(project, id)`,
    /// and the reader steps through them.
    ///
    /// **Where it lands** is on the bead whose id is exactly what was typed
    /// if one matched, and otherwise on the first match after `origin`, as
    /// `n` would step from there. An id is the one thing a reader can have
    /// meant exactly, and a row merely *titled* after a bead must not shadow
    /// it — that promise is older than this widening. The numbering below is
    /// not touched by it.
    ///
    /// Counted from where the search began rather than from where the
    /// selection is, because the reader's last keystroke has moved it: each
    /// keystroke is the whole search again, from the same place. Where
    /// nothing matches, the forest goes back to how it stood there.
    pub fn seek(&mut self, query: &str, origin: &Origin) -> Landed {
        let mut matched = self.matches(Sought::holding(query));
        let of = matched.len();
        let at = self
            .matches(Sought::named(query))
            .nth(0)
            .and_then(|named| matched.before(&named))
            .map(|(before, _)| before)
            .or_else(|| (of > 0).then(|| Self::past(origin.standing.clone(), &mut matched, true)));
        let found = at.and_then(|at| matched.nth(at));
        let landed = self.land_on(found, at.unwrap_or(0), of, query);
        if let Landed::Nowhere(_) = landed {
            self.restore(origin);
        }
        self.searched = Some(query.to_string());
        landed
    }

    /// Where a search beginning now would count from, and what abandoning it
    /// would put back.
    pub fn origin(&self) -> Origin {
        Origin {
            cursor: self.cursor.clone(),
            from: self.from,
            opened_by_the_last_step: self.folds.opened_by_the_last_step(),
            searched: self.searched.clone(),
            standing: self.standing_at(),
        }
    }

    /// Put the selection, the scroll, what the last search step opened and
    /// what was last searched for back as they stood at `origin`.
    ///
    /// The step's opens come back as that step's, so the step after this
    /// still shuts them.
    pub fn restore(&mut self, origin: &Origin) {
        self.folds
            .reopen_for_the_last_step(origin.opened_by_the_last_step.clone());
        self.cursor = origin.cursor.clone();
        self.searched = origin.searched.clone();
        self.lay_out();
        self.from = origin.from.min(self.furthest());
    }

    /// Step to the next bead matching what was last searched for, or to the
    /// one before it, coming round at either end.
    ///
    /// `None` before anything has been searched for: there is nothing to step
    /// through and nothing has gone wrong, which is the same answer `Focus`
    /// gives a row with no pane.
    pub fn next_match(&mut self, forward: bool) -> Option<Landed> {
        let query = self.searched.clone()?;
        let mut matched = self.matches(Sought::holding(&query));
        let of = matched.len();
        if of == 0 {
            return Some(Landed::Nowhere(query));
        }
        let at = Self::past(self.standing_at(), &mut matched, forward);
        let found = matched.nth(at);
        Some(self.land_on(found, at, of, &query))
    }

    /// Which match to step to from where the reader stands, as `standing_at`
    /// answers it: the first one drawn after them, or the last one drawn
    /// before them, coming round at either end.
    ///
    /// `n` asks it of where the selection is *now* rather than of where the
    /// last step left it. So a reader who has moved by hand between presses
    /// carries on from where they are standing, and a collection that has
    /// moved every bead under them costs the walk nothing — there is no
    /// place in a list to have gone stale, only a question asked again.
    ///
    /// A selection on a row that is not a bead carries on from there too, and
    /// the bead it stands above is *included* going forwards: the reader is
    /// above it, not on it, so it is a match after them rather than the one
    /// they are already looking at. Going back needs no such care — a match
    /// drawn at that bead is below the selection either way.
    ///
    /// *After* is a copy's place in the order and the row the reader is on is
    /// exactly that copy, since `bdi-7ao.136`: a match is a row and `of`
    /// counts rows, so a bead the tree reaches twice is two places in the
    /// order and a reader standing on the second is past the first without
    /// being past the second. `standing_at` finds the one the selection is
    /// actually on, so this never has to guess which copy that was.
    fn past(standing: Option<(Place, bool)>, matched: &mut Matches, forward: bool) -> usize {
        let of = matched.len();
        let standing = standing.and_then(|(place, past_it)| {
            let (before, on_a_match) = matched.before(&place)?;
            Some((before, on_a_match && !past_it))
        });
        let Some((before, on_a_match)) = standing else {
            return if forward { 0 } else { of - 1 };
        };
        if forward {
            let after = before.saturating_add(usize::from(on_a_match));
            if after < of {
                after
            } else {
                0
            }
        } else {
            before.checked_sub(1).unwrap_or(of - 1)
        }
    }

    /// Where the selection stands among the beads drawn, and whether it is
    /// standing *above* that bead rather than on it.
    ///
    /// A project's own line, a group, and a pane in one are not beads, and a
    /// reader can rest on any of them. Each still has a place in the order —
    /// the first bead drawn below it — so stepping from one carries on from
    /// where the reader is instead of starting the walk again.
    ///
    /// Found by place and not by key, since `bdi-7ao.136`: a bead the tree
    /// reaches twice is two places in the order, and only the exact one the
    /// selection sits on says which of them the reader is standing at.
    ///
    /// `None` where there is no such bead, and the walk comes round rather
    /// than carrying on. That is a reader resting under the last bead on the
    /// screen.
    fn standing_at(&self) -> Option<(Place, bool)> {
        let on_a_bead = self
            .lines
            .get(self.selected)
            .and_then(Self::place_of_a_bead);
        match on_a_bead {
            Some(place) => Some((place, false)),
            None => Some((self.first_bead_under()?, true)),
        }
    }

    /// The first bead a selection resting on a row that is not one stands
    /// above.
    ///
    /// Read off the rendered lines wherever there are lines to read. Every
    /// ordering defect this search has had came from deriving the screen's
    /// shape somewhere other than where it is drawn, and the lines are that
    /// shape rather than a second account of it.
    ///
    /// A group resting shut is where there are none: its trees' beads are in
    /// the order, because a search goes by the trees the screen would
    /// draw rather than by the folds set over them, but the fold has taken
    /// every one of them off the screen. The lines below the group are the
    /// rows *after* it, so reading them would step over everything it hides
    /// — so a shut group is asked what it holds instead. One that holds no
    /// beads has nothing to say and the lines answer as they always did.
    /// Nothing under it is drawn, so `place_of` is asked for the copy the
    /// walk would reach first, which is the only one there is a place for.
    fn first_bead_under(&self) -> Option<Place> {
        let resting_on = self.lines.get(self.selected)?;
        let shut_over = match &resting_on.content {
            Content::Group(group) if resting_on.folded == Some(false) => layout::first_bead_of(
                &self.snapshot,
                group.kind,
                group.project.as_deref(),
                &self.rooted(),
            )
            .and_then(|key| self.place_of(&key)),
            _ => None,
        };
        shut_over.or_else(|| {
            (self.selected..self.lines.len())
                .find_map(|row| Self::place_of_a_bead(&self.lines[row]))
        })
    }

    /// The place a line is drawn at, where the line is a bead. A root whose
    /// tracker refused has a place too, but no bead is drawn there for a
    /// match to be counted at.
    fn place_of_a_bead(line: &Line) -> Option<Place> {
        matches!(line.content, Content::Bead(_))
            .then(|| line.place.clone())
            .flatten()
    }

    /// Put the selection on the `at`th match, counting from zero, and say
    /// where it went and how many there were. Nowhere where there is no such
    /// match, or where the forest cannot take the reader to it.
    ///
    /// `found` is the place the match is drawn, which is gone to rather than
    /// whatever `go_to` would find of its key: a bead the tree reaches twice
    /// is two places since `bdi-7ao.136`, and the one counted is the one
    /// landed on.
    fn land_on(&mut self, found: Option<Place>, at: usize, of: usize, query: &str) -> Landed {
        let Some(place) = found.filter(|place| self.step_to_place(place)) else {
            return Landed::Nowhere(query.to_string());
        };
        Landed::On {
            key: place.key().clone(),
            at: at + 1,
            of,
        }
    }

    /// Every drawn copy of a bead a search is after, in the order the screen
    /// draws them.
    ///
    /// The order of the *trees* is asked of `layout`, which is what draws
    /// them, rather than worked out here.
    ///
    /// `place_of`'s rule — the trees the filter shows before the ones it hid
    /// — is not the whole of the screen's order, and reading it as if it were
    /// is what this used to get wrong. It is an ordering *within* a project:
    /// `place_of` takes a key, which carries its project, so it never has to
    /// sequence one project against another. This has to, and the screen goes
    /// project by project, putting each project's hidden trees under that
    /// project rather than after every visible one. The rule is true where it
    /// is written and silent about the question asked here.
    ///
    /// A bead reachable more than once is drawn more than once and counted
    /// once per way down to it.
    fn matches(&self, sought: Sought) -> Matches<'_> {
        Matches::of(&self.snapshot, &self.facts, &self.rooted(), sought)
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
    /// reader scanning down the screen would have found themselves — which
    /// means first *drawn*, and a parent with enough finished branches draws
    /// its children in an order their sort does not give. `way_to` is handed
    /// the tree's facts for that reason, and it is the same order the search
    /// enumerates in.
    ///
    /// A search lands on the exact copy it counted rather than through this,
    /// since `bdi-7ao.136`: every drawn copy is its own match now, and a key
    /// alone cannot say which one a reader meant. This answers a narrower
    /// question — which bead a bare key names first — and `go_to` is the one
    /// caller still asking it.
    ///
    /// What the forest is rooted at is asked first, because a way down the
    /// tree answers with can be a row the mode draws nowhere. Then the
    /// bead's own project's trees, and only then the trees of the projects
    /// that reach it through a bead waiting on it.
    fn place_of(&self, key: &BeadKey) -> Option<Place> {
        self.drawn_at_the_root(key).or_else(|| {
            let trees = || self.snapshot.trees.iter().chain(&self.snapshot.collected);
            trees()
                .filter(|tree| tree.project == key.project)
                .chain(trees().filter(|tree| tree.project != key.project))
                .find_map(|tree| way_to(tree, self.facts.tree(&root_key(tree)), key))
        })
    }

    /// Where the forest draws a bead at the bead it is rooted at or beneath
    /// it, which is where a reader comes to it first: those rows are drawn
    /// above every root the mode is holding back.
    ///
    /// A bead its own tree reaches twice is drawn at the copy the reader
    /// pressed the key on, and every copy of it is left out of the root behind
    /// the line — so the first way down to that bead, and to everything only
    /// it reaches, is a way down to a row nothing draws.
    fn drawn_at_the_root(&self, key: &BeadKey) -> Option<Place> {
        self.focused.iter().find_map(|focused| {
            if focused.steps.last().unwrap_or(&focused.tree) == key {
                return Some(focused.clone());
            }
            let (tree, way) = self.locate(focused)?;
            let (at, above) = way.split_last()?;
            stepped_to(
                tree,
                self.facts.tree(&root_key(tree)),
                key,
                *at,
                above,
                focused,
            )
        })
    }

    /// Open everything shut over a line: everything the line hangs under, and
    /// the run of quiet children the way down to it goes through.
    ///
    /// The runs are asked for beside the ancestry rather than left to it,
    /// which answers for a line that is drawn — and a bead inside a run is
    /// not drawn at all, so nothing above it can name it.
    ///
    /// Set rather than let go of, because a fold the reader shut by hand
    /// stays shut until something asks otherwise, and asking to be taken to a
    /// bead underneath it is asking.
    fn open_over(&mut self, place: &Place) {
        for over in self.folds_over(place) {
            self.folds.set(over, true);
        }
    }

    /// Every fold that has to be open for a line to be drawn.
    fn folds_over(&self, place: &Place) -> Vec<Handle> {
        let runs = self.runs_over(place).into_iter().map(Handle::Elided);
        let over = self.ancestry_of(Some(&self.handle_on(place)));
        // Past the line itself, whose own fold is about the children under it
        // rather than about reaching it.
        runs.chain(over.into_iter().skip(1)).collect()
    }

    /// The forebears of a line whose run of quiet children counts the next
    /// bead on the way down to it.
    fn runs_over(&self, place: &Place) -> Vec<Place> {
        let Some((tree, way)) = self.locate(place) else {
            return Vec::new();
        };
        let facts = self.facts.tree(&root_key(tree));
        place
            .forebears()
            .filter(|forebear| {
                let depth = forebear.steps.len();
                let (_, elided) = facts.split(tree, way[depth], &way[..depth]);
                elided.iter().any(|link| link.bead == way[depth + 1])
            })
            .collect()
    }

    fn step_to(&mut self, target: Option<usize>) {
        if let Some(target) = target {
            self.selected = target;
            self.cursor = self.handle_at(target);
        }
    }

    /// The first line at or beyond `from` that the selection can sit on.
    fn scan(&self, from: usize, forward: bool) -> Option<usize> {
        if forward {
            (from..self.lines.len()).find(|i| selectable(&self.lines[*i]))
        } else {
            (0..=from.min(self.lines.len().checked_sub(1)?))
                .rev()
                .find(|i| selectable(&self.lines[*i]))
        }
    }

    /// The next line the selection can sit on, past `from`.
    fn step(&self, from: usize, forward: bool) -> Option<usize> {
        if forward {
            self.scan(from + 1, true)
        } else {
            self.scan(from.checked_sub(1)?, false)
        }
    }

    /// The line the one at `at` hangs under. Every line with lines under it
    /// is one the selection can sit on.
    fn parent_of(&self, at: usize) -> Option<usize> {
        self.lines.parent_of(at)
    }

    fn first_child_of(&self, at: usize) -> Option<usize> {
        let depth = self.lines.get(at)?.depth;
        (at + 1..self.lines.len())
            .map(|row| (row, &self.lines[row]))
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
    fn lay_out(&mut self) -> Drawn {
        self.settle_cursor();
        let rooted = self.rooted();
        let drawn = layout::lay_out(
            &self.snapshot,
            &self.facts,
            &self.folds,
            &rooted,
            &self.layout,
        );
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
            .find(|forebear| self.lines.row_of(forebear).is_some())
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
        self.lines.row_of(self.cursor.as_ref()?)
    }

    fn first_handle(&self) -> Option<Handle> {
        if let Some(tree) = self.snapshot.trees.first() {
            return Some(root_handle(tree));
        }
        let rooted = self.rooted();
        layout::every_group(&self.snapshot)
            .find(|(kind, project)| {
                layout::group_drawn(&self.snapshot, *kind, project.as_deref(), &rooted)
            })
            .map(|(kind, project)| Handle::Group(kind, project))
    }

    /// Whether the snapshot still holds what a handle names.
    fn present(&self, handle: &Handle) -> bool {
        match handle {
            Handle::Bead(place) | Handle::Unread(place) | Handle::Elided(place) => {
                self.drawn(place)
            }
            Handle::Group(kind, project) => {
                layout::group_drawn(&self.snapshot, *kind, project.as_deref(), &self.rooted())
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
fn way_to(tree: &Tree, facts: &TreeFacts, key: &BeadKey) -> Option<Place> {
    let root = Place::root(root_key(tree));
    if tree.beads.first()?.is(key) {
        return Some(root);
    }
    stepped_to(tree, facts, key, 0, &[], &root)
}

/// The first way down from `at` that reaches a bead, given the beads stepped
/// through to reach `at` and the place it stands on.
///
/// The children come from `Facts::split` and not from `links_below`, for the
/// same reason a search takes them from there: a parent with enough
/// finished branches draws the rest of its children first and the run after,
/// so *first* means first drawn rather than first sorted. Without it the copy
/// this lands on and the copy a search counted at could be two different
/// lines of the same bead.
fn stepped_to(
    tree: &Tree,
    facts: &TreeFacts,
    key: &BeadKey,
    at: usize,
    above: &[usize],
    place: &Place,
) -> Option<Place> {
    let mut way = above.to_vec();
    way.push(at);
    let (shown, elided) = facts.split(tree, at, above);
    shown.into_iter().chain(elided).find_map(|link| {
        let stepped = place.step_to(tree.beads[link.bead].key());
        if tree.beads[link.bead].is(key) {
            return Some(stepped);
        }
        stepped_to(tree, facts, key, link.bead, &way, &stepped)
    })
}

/// The line `scope` names, and everything drawn beneath it, read off lines
/// a test has drawn whole.
///
/// Beneath is depth: the lines after it, up to the first one standing at its
/// own depth or shallower. A project is depth zero, the roots under it are
/// one, and a group's things are one under a group that is also zero, so the
/// scope of a project stops at the next project or the first group.
#[cfg(test)]
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

/// Whether a line stands over what is beneath it only for now: a run of
/// quiet children, which goes when its siblings drop below three, or a
/// group of trees the filter or the mode holds back, which goes when the
/// trees are shown or the forest is put back. A scope set on one would go
/// with it and leave what it pointed resting, so a key pressed on one
/// points each fold beneath it by itself, as every line did once.
fn passing(handle: &Handle) -> bool {
    matches!(
        handle,
        Handle::Elided(_) | Handle::Group(GroupKind::HiddenTrees | GroupKind::OutOfTheWay, _)
    )
}

/// Every line the folds could name under `within`, that line itself left
/// out: the lines drawn because the folds name something at or beneath
/// them. Nothing beneath any other line has an entry, so this is every entry
/// under `within` that a key over it lets go of.
fn named_beneath(drawn: &Drawn, within: &Node) -> Vec<Handle> {
    let mut beneath = Vec::new();
    drawn.visit(within, &mut |node| {
        if !std::ptr::eq(node, within) {
            beneath.extend(handle_of(&node.line));
        }
        matches!(node.beneath, Beneath::Nothing)
    });
    beneath
}

/// Every bead that arrived under any spent fold, which is what the walk to
/// each fold's own arrivals is cut short by.
fn arrived_anywhere(spent: &[(Handle, BTreeSet<BeadKey>)]) -> BTreeSet<BeadKey> {
    spent
        .iter()
        .flat_map(|(_, arrived)| arrived.iter().cloned())
        .collect()
}

/// Whether a bead reaches one that arrived, answered once per bead of each
/// tree the walk steps into. Read of the tree rather than of the lines, so a
/// subtree with no arrival beneath it is stepped over rather than drawn.
struct Reaching<'a> {
    arrived: &'a BTreeSet<BeadKey>,
    trees: HashMap<usize, Vec<bool>>,
}

impl<'a> Reaching<'a> {
    fn new(arrived: &'a BTreeSet<BeadKey>) -> Self {
        Reaching {
            arrived,
            trees: HashMap::new(),
        }
    }

    /// Whether the subtree a line was counted rather than drawn from holds
    /// a line standing on an arrival.
    fn beneath(&mut self, drawn: &Drawn, beneath: &Beneath) -> bool {
        let (Beneath::Bead(undrawn) | Beneath::Run(undrawn)) = beneath else {
            return true;
        };
        let Some(tree) = drawn.tree(undrawn.counted.tree) else {
            return true;
        };
        let reaches = self
            .trees
            .entry(undrawn.counted.tree)
            .or_insert_with(|| reaching(tree, self.arrived));
        reaches[undrawn.counted.at]
    }
}

/// Which beads of a tree with no loop in it reach one of `arrived`, each
/// bead counting itself.
fn reaching(tree: &Tree, arrived: &BTreeSet<BeadKey>) -> Vec<bool> {
    fn walk(tree: &Tree, at: usize, reaches: &mut [Option<bool>]) -> bool {
        if let Some(known) = reaches[at] {
            return known;
        }
        let found = tree.children[at]
            .iter()
            .any(|link| link.bead != at && walk(tree, link.bead, reaches));
        reaches[at] = Some(found);
        found
    }
    let mut reaches: Vec<Option<bool>> = tree
        .beads
        .iter()
        .map(|bead| arrived.contains(&bead.key()).then_some(true))
        .collect();
    (0..tree.beads.len())
        .map(|at| walk(tree, at, &mut reaches))
        .collect()
}

/// The shut folds on the way down from the line `scope` names to every line
/// standing on one of `arrived`, the scope's own left out. A fold open on
/// the way holds nothing back, so nothing arriving spends it.
///
/// Read off the lines by depth, the way the lines beneath a line are read,
/// so the way down is the one that was drawn — through the run a bead hangs
/// in, and by whichever fold the drawing gave each line.
fn way_down_to(
    drawn: &Drawn,
    scope: &Handle,
    arrived: &BTreeSet<BeadKey>,
    reaching: &mut Reaching,
) -> BTreeSet<Handle> {
    let Some(within) = drawn.node_of(scope) else {
        return BTreeSet::new();
    };
    let mut above: Vec<&Line> = Vec::new();
    let mut path = BTreeSet::new();
    drawn.visit(within, &mut |node| {
        if std::ptr::eq(node, within) {
            return true;
        }
        let line = &node.line;
        while above.last().is_some_and(|over| over.depth >= line.depth) {
            above.pop();
        }
        if line
            .place
            .as_ref()
            .is_some_and(|place| arrived.contains(place.key()))
        {
            path.extend(
                above
                    .iter()
                    .filter(|over| over.folded == Some(false))
                    .filter_map(|over| handle_of(over)),
            );
        }
        above.push(line);
        reaching.beneath(drawn, &node.beneath)
    });
    path
}

/// Whether `handle` names a line strictly beneath the one at `place`.
///
/// A place is a way down, so what is beneath a line is what the way down to
/// it is a proper prefix of.
fn beneath_the_line(place: &Place, handle: &Handle) -> bool {
    let (Handle::Bead(under) | Handle::Elided(under)) = handle else {
        return false;
    };
    under.tree == place.tree
        && under.steps.len() > place.steps.len()
        && under.steps.starts_with(&place.steps)
}

/// Whether the line at `place` is the one at `over` or beneath it.
fn at_or_beneath(place: &Place, over: &Place) -> bool {
    place.tree == over.tree && place.steps.starts_with(&over.steps)
}

/// The places given, less any at or beneath another: rooted at a bead, the
/// forest already draws everything under it there.
fn outermost(places: Vec<Place>) -> Vec<Place> {
    let mut kept: Vec<Place> = Vec::with_capacity(places.len());
    for place in places {
        if kept.iter().any(|over| at_or_beneath(&place, over)) {
            continue;
        }
        kept.retain(|under| !at_or_beneath(under, &place));
        kept.push(place);
    }
    kept
}

#[cfg(test)]
mod tests;
