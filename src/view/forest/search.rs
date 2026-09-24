//! A search's matches, counted beneath each bead rather than listed once per
//! way down to it.
//!
//! Every drawn copy of a bead is a match of its own, and a bead the tree
//! reaches several ways is drawn once per way, with its children under each
//! copy — so where shared dependencies nest, the copies grow exponentially
//! with the nesting. Listing them costs as much. So what a search asks is
//! answered by counting instead: how many matches a bead's subtree draws is
//! worked out once per bead and added up, the way layout counts a subtree
//! nothing draws, and a match is found by counting down to it.

use crate::model::snapshot::{Snapshot, Tree};
use crate::view::lines::{root_key, Place};

use super::facts::{Facts, TreeFacts};
use super::layout::{self, Rooted};

/// What a search is after in a bead's row.
///
/// Letter case is neither side's: a title is prose, and a reader retyping a
/// word off a row is not reproducing the capitals it happened to carry.
pub(super) enum Sought {
    /// A bead holding the text in its id or its title.
    Holding(String),
    /// The bead whose id is the text.
    Named(String),
}

impl Sought {
    pub(super) fn holding(text: &str) -> Self {
        Sought::Holding(text.to_lowercase())
    }

    pub(super) fn named(text: &str) -> Self {
        Sought::Named(text.to_lowercase())
    }

    fn finds(&self, tree: &Tree, at: usize) -> bool {
        let bead = &tree.beads[at];
        match self {
            Sought::Holding(text) => {
                bead.id.to_lowercase().contains(text) || bead.title.to_lowercase().contains(text)
            }
            Sought::Named(text) => bead.id.to_lowercase() == *text,
        }
    }
}

/// Every drawn copy of a bead a search is after, numbered in the order the
/// screen draws them, from zero.
pub(super) struct Matches<'a> {
    sought: Sought,
    walks: Vec<Walk<'a>>,
    /// How many matches each walk draws, beside it.
    totals: Vec<usize>,
}

impl<'a> Matches<'a> {
    /// Over every tree the forest draws, in the order `layout::walked` gives
    /// them: the order the screen draws them in with every fold open.
    pub(super) fn of(
        snapshot: &'a Snapshot,
        facts: &'a Facts,
        rooted: Option<&Rooted>,
        sought: Sought,
    ) -> Self {
        let mut walks: Vec<Walk> = layout::walked(snapshot, rooted)
            .into_iter()
            .map(|(tree, way, without)| Walk::new(tree, facts.tree(&root_key(tree)), way, without))
            .collect();
        let totals = walks.iter_mut().map(|walk| walk.total(&sought)).collect();
        Matches {
            sought,
            walks,
            totals,
        }
    }

    pub(super) fn len(&self) -> usize {
        self.totals
            .iter()
            .fold(0, |sum, total| sum.saturating_add(*total))
    }

    /// Where the `n`th match is drawn. Nothing past the last.
    pub(super) fn nth(&mut self, mut n: usize) -> Option<Place> {
        for (walk, total) in self.walks.iter_mut().zip(&self.totals) {
            if n < *total {
                return Some(walk.nth(&self.sought, n));
            }
            n -= total;
        }
        None
    }

    /// How many matches are drawn before a place, and whether the place is
    /// one itself. Nothing where the forest draws no bead there.
    pub(super) fn before(&mut self, place: &Place) -> Option<(usize, bool)> {
        let mut passed = 0usize;
        for (walk, total) in self.walks.iter_mut().zip(&self.totals) {
            if let Some((before, found)) = walk.before(&self.sought, place) {
                return Some((passed.saturating_add(before), found));
            }
            passed = passed.saturating_add(*total);
        }
        None
    }
}

/// One tree the forest draws, from the way down it starts drawing at, and
/// leaving out the bead `layout::walked` says the mode draws elsewhere.
struct Walk<'a> {
    tree: &'a Tree,
    facts: &'a TreeFacts,
    way: Vec<usize>,
    without: Option<usize>,
    start: Place,
    /// The matches beneath each bead, once counted, in a tree with no loop in
    /// it. A tree with one cuts it by the way down, so there two copies of a
    /// bead can hold different things and each is counted as it is reached.
    // ponytail: a tree with a loop is still counted per copy, exponential in
    // shared dependencies nested inside it; key this on the way down within
    // the loop if such trees turn up large.
    kept: Option<Vec<Option<usize>>>,
}

impl<'a> Walk<'a> {
    fn new(tree: &'a Tree, facts: &'a TreeFacts, way: Vec<usize>, without: Option<usize>) -> Self {
        let mut start = Place::root(root_key(tree));
        for &at in way.iter().skip(1) {
            start = start.step_to(tree.beads[at].key());
        }
        Walk {
            tree,
            facts,
            way,
            without,
            start,
            kept: tree.cycles.is_empty().then(|| vec![None; tree.beads.len()]),
        }
    }

    /// The bead the walk starts at and the way down to it, where the tree
    /// has a bead there. A tree whose tracker would not read has none.
    fn begins(&self) -> Option<(usize, Vec<usize>)> {
        let (at, above) = self.way.split_last().expect("a way down ends somewhere");
        (*at < self.tree.beads.len()).then(|| (*at, above.to_vec()))
    }

    fn total(&mut self, sought: &Sought) -> usize {
        let Some((at, mut above)) = self.begins() else {
            return 0;
        };
        self.beneath(sought, at, &mut above)
    }

    /// The beads drawn under `at` in the order they are drawn, runs
    /// included: a search opens a run to reach a bead in it.
    ///
    /// From `Facts::split` and not `links_below`, because a parent with
    /// enough finished children to make a run draws the rest first and the
    /// run after, so the tracker's order is not the screen's.
    fn children(&self, at: usize, above: &[usize]) -> Vec<usize> {
        let (shown, elided) = self.facts.split(self.tree, at, above);
        shown
            .into_iter()
            .chain(elided)
            .map(|link| link.bead)
            .filter(|bead| Some(*bead) != self.without)
            .collect()
    }

    /// The matches drawn at `at` and beneath it, `above` being the way down
    /// to it.
    fn beneath(&mut self, sought: &Sought, at: usize, above: &mut Vec<usize>) -> usize {
        if let Some(found) = self.kept.as_ref().and_then(|kept| kept[at]) {
            return found;
        }
        let mut found = usize::from(sought.finds(self.tree, at));
        let children = self.children(at, above);
        above.push(at);
        for child in children {
            found = found.saturating_add(self.beneath(sought, child, above));
        }
        above.pop();
        if let Some(kept) = &mut self.kept {
            kept[at] = Some(found);
        }
        found
    }

    /// Where the `n`th match this walk draws is: counted down to, skipping
    /// every child that holds too few.
    fn nth(&mut self, sought: &Sought, mut n: usize) -> Place {
        let (mut at, mut above) = self.begins().expect("a walk drawing a match has a bead");
        let mut place = self.start.clone();
        loop {
            if sought.finds(self.tree, at) {
                if n == 0 {
                    return place;
                }
                n -= 1;
            }
            let children = self.children(at, &above);
            above.push(at);
            let mut under = None;
            for child in children {
                let found = self.beneath(sought, child, &mut above);
                if n < found {
                    under = Some(child);
                    break;
                }
                n -= found;
            }
            at = under.expect("the matches beneath a bead are beneath one of its children");
            place = place.step_to(self.tree.beads[at].key());
        }
    }

    /// How many matches this walk draws before `place`, and whether `place`
    /// is one. Nothing where the walk never reaches it.
    fn before(&mut self, sought: &Sought, place: &Place) -> Option<(usize, bool)> {
        if place.tree != self.start.tree || !place.steps.starts_with(&self.start.steps) {
            return None;
        }
        let (mut at, mut above) = self.begins()?;
        let mut before = 0usize;
        for step in &place.steps[self.start.steps.len()..] {
            before = before.saturating_add(usize::from(sought.finds(self.tree, at)));
            let children = self.children(at, &above);
            above.push(at);
            let mut next = None;
            for child in children {
                if self.tree.beads[child].key() == *step {
                    next = Some(child);
                    break;
                }
                before = before.saturating_add(self.beneath(sought, child, &mut above));
            }
            at = next?;
        }
        Some((before, sought.finds(self.tree, at)))
    }
}
