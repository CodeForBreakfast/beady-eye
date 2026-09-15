//! What a line the selection or a fold can rest on is known by, and the folds
//! held against those names.
//!
//! Both halves of the forest say these words: the loop that reads keys moves
//! a fold, laying out asks which way one points, and neither reaches past the
//! other to do it.

use std::collections::{BTreeMap, BTreeSet};

use crate::model::join::Conflict;
use crate::model::types::PaneKey;
use crate::view::lines::{Content, GroupKind, Item, Line, Place};

/// What a line that folds is known by, so both the fold and the selection
/// survive a refresh that reorders or drops lines.
///
/// A bead can be drawn on more than one line, so a handle names the line and
/// not the bead standing on it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Handle {
    Bead(Place),
    /// The run of quiet closed children under one drawn bead. A bead has at
    /// most one run per copy of it, so the copy names it.
    Elided(Place),
    /// A group, by its kind and the project it hangs under where it is one
    /// of a project's own.
    Group(GroupKind, Option<String>),
    Item(ItemKey),
    /// A project, by its name, which the config makes unique.
    Project(String),
}

/// What one thing in a group is known by.
///
/// A handle has to be an identity the thing still has after the next collect,
/// never its place in the group, or a group re-read in another order would
/// move the selection to a neighbour with nothing on screen to say so.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum ItemKey {
    /// A pane, by the key naming it across every session on the box — so one
    /// key serves both groups that hold panes, whichever of the two this pane
    /// landed in.
    Pane(PaneKey),
    Project(String),
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
pub(super) fn item_key(item: &Item) -> Option<ItemKey> {
    Some(match item {
        Item::Loose(pane) => ItemKey::Pane(pane.pane.clone()),
        Item::Unconfigured(pane) => ItemKey::Pane(pane.pane.clone()),
        Item::Failed(failed) => ItemKey::Project(failed.project.clone()),
        Item::Conflict(conflict) => ItemKey::Conflict(conflict.clone()),
    })
}

/// The folds the user set by hand, over a default that follows the
/// selection. Keeping the two apart is what lets a fold outlive moving
/// away from it without freezing every other root at whatever it was.
///
/// A key that points one fold writes that fold. A key that points a subtree
/// writes one entry, on the line it was pressed on, and every fold beneath
/// answers from it unless a nearer entry says otherwise — so the map is the
/// size of what the reader pressed, not of what those presses opened.
///
/// The map itself never leaves: a fold is asked which way it points, pointed,
/// or let go of, and nothing else.
#[derive(Default)]
pub(super) struct Folds(BTreeMap<Handle, Fold>);

/// What the reader set on one line.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct Fold {
    /// The line's own fold, where a key that points one fold set it or a
    /// refresh spent it. Wins over any scope.
    pub(super) line: Option<Way>,
    /// What the line and everything beneath it answer from, where a key that
    /// points a subtree set it.
    pub(super) scope: Option<Scope>,
}

/// Which way one line's own fold goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Way {
    Open,
    Shut,
    /// As the default puts it, whatever a scope over the line says: a fold
    /// the reader shut and live work has since arrived under, or one `d`
    /// put back by the line.
    Rests,
}

/// What every fold under one line answers from, unless a nearer entry says
/// otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Scope {
    /// Every fold points this way, but for the ones named as resting — the
    /// folds `e` found resting open — which go on following the default.
    Points {
        open: bool,
        resting: BTreeSet<Handle>,
    },
    /// Every fold rests, whatever a scope above says.
    Rests,
}

impl Fold {
    fn holds_shut(&self) -> bool {
        self.line == Some(Way::Shut)
            || matches!(self.scope, Some(Scope::Points { open: false, .. }))
    }
}

impl Folds {
    /// Which way the reader has pointed a fold, from the nearest entry on the
    /// way down to it: the line's own fold, the scope set on the line, or the
    /// scope `over` it. Nothing where the fold is left to rest: only the
    /// caller knows the tree a line came from, so it says where the line
    /// rests rather than being asked to re-derive it here.
    pub(super) fn pointed(&self, handle: &Handle, over: Option<&Scope>) -> Option<bool> {
        match self.0.get(handle).and_then(|fold| fold.line) {
            Some(Way::Open) => return Some(true),
            Some(Way::Shut) => return Some(false),
            Some(Way::Rests) => return None,
            None => {}
        }
        match self.beneath(handle, over) {
            Some(Scope::Points { open, resting }) if !resting.contains(handle) => Some(*open),
            _ => None,
        }
    }

    /// The scope everything under a line answers from: the one set on the
    /// line where a key set one there, and the one `over` the line otherwise.
    pub(super) fn beneath<'a>(
        &'a self,
        handle: &Handle,
        over: Option<&'a Scope>,
    ) -> Option<&'a Scope> {
        self.0
            .get(handle)
            .and_then(|fold| fold.scope.as_ref())
            .or(over)
    }

    /// Point one fold the way the user asked.
    pub(super) fn set(&mut self, handle: Handle, open: bool) {
        let way = if open { Way::Open } else { Way::Shut };
        self.0.entry(handle).or_default().line = Some(way);
    }

    /// Point a line and every fold beneath it one way, but for `resting`,
    /// which are left to follow the default. Whatever the reader had set
    /// beneath it goes: the scope is what they are asking for now.
    pub(super) fn set_over(
        &mut self,
        handle: Handle,
        open: bool,
        resting: BTreeSet<Handle>,
        beneath: &[Handle],
    ) {
        for under in beneath {
            self.0.remove(under);
        }
        let scope = Some(Scope::Points { open, resting });
        self.0.insert(handle, Fold { line: None, scope });
    }

    /// Hand a line and everything beneath it back to the default, and hold
    /// it there against whatever scope stands over it.
    pub(super) fn let_go(&mut self, handle: Handle, beneath: &[Handle]) {
        for under in beneath {
            self.0.remove(under);
        }
        let scope = Some(Scope::Rests);
        self.0.insert(handle, Fold { line: None, scope });
    }

    /// Hand one line's own fold back to the default, and hold it there
    /// against whatever scope stands over it. A scope set on the line
    /// stays: `d` by the line puts back each line it drew and no more, and
    /// the lines it did not draw go on answering from that scope.
    pub(super) fn put_back(&mut self, handle: Handle) {
        self.0.entry(handle).or_default().line = Some(Way::Rests);
    }

    /// The folds the user has shut, which are the only ones that can be
    /// spent: a fold left open holds nothing back. A scope that shut is one
    /// of them, on the line it was set on.
    pub(super) fn shut(&self) -> impl Iterator<Item = &Handle> {
        self.0
            .iter()
            .filter(|(_, fold)| fold.holds_shut())
            .map(|(handle, _)| handle)
    }

    /// Let go of the shut fold at `handle`, handing it back to the default
    /// whatever scope stands over it. Where the fold is itself a scope that
    /// shut, `path` — the way down to what arrived beneath — is let go of
    /// with it, and the scope goes on standing over the rest.
    pub(super) fn spend(&mut self, handle: &Handle, path: impl IntoIterator<Item = Handle>) {
        let scope_shut = matches!(
            self.0.get(handle),
            Some(Fold {
                scope: Some(Scope::Points { open: false, .. }),
                ..
            })
        );
        self.rest(handle.clone());
        if scope_shut {
            for under in path {
                self.rest(under);
            }
        }
    }

    /// Hand one line's own fold back to the default. A fold the reader
    /// opened by hand stays as they opened it: it holds nothing back, so
    /// nothing arriving beneath it spends it.
    fn rest(&mut self, handle: Handle) {
        let fold = self.0.entry(handle).or_default();
        if fold.line != Some(Way::Open) {
            fold.line = Some(Way::Rests);
        }
    }

    /// Let go of every fold at once.
    pub(super) fn clear(&mut self) {
        self.0.clear();
    }

    /// Every line the folds name: the ones with an entry, and the ones a
    /// scope leaves resting. A line named nowhere here answers from the
    /// scope over it alone, and so does everything beneath it.
    pub(super) fn mentioned(&self) -> impl Iterator<Item = &Handle> {
        self.0.iter().flat_map(|(handle, fold)| {
            let resting = match &fold.scope {
                Some(Scope::Points { resting, .. }) => Some(resting.iter()),
                _ => None,
            };
            std::iter::once(handle).chain(resting.into_iter().flatten())
        })
    }

    /// Which way every fold under a scope answers, where nothing nearer
    /// names it: pointed as the scope points, or left to rest.
    pub(super) fn forced(over: Option<&Scope>) -> Option<bool> {
        match over {
            Some(Scope::Points { open, .. }) => Some(*open),
            Some(Scope::Rests) | None => None,
        }
    }

    #[cfg(test)]
    pub(super) fn entries(&self) -> impl Iterator<Item = (&Handle, &Fold)> {
        self.0.iter()
    }
}

/// What a line is known by, where it is one the selection can hold.
///
/// A note stands for a finding rather than for a thing, so it has none — and
/// a line the forest cannot name could not be put back after a refresh, which
/// is why this is the same question as whether the selection may sit there.
pub(super) fn handle_of(line: &Line) -> Option<Handle> {
    match &line.content {
        Content::Bead(_) | Content::Unread(_) => line.place.clone().map(Handle::Bead),
        Content::Project(line) => Some(Handle::Project(line.project.clone())),
        Content::Elided { under, .. } => Some(Handle::Elided(under.clone())),
        Content::Group(group) => Some(Handle::Group(group.kind, group.project.clone())),
        Content::Item(item) => item_key(item).map(Handle::Item),
        Content::Note(_) | Content::Scoped { .. } => None,
    }
}

pub(super) fn selectable(line: &Line) -> bool {
    handle_of(line).is_some()
}

/// Whether a line is the one a handle names, asked without spelling the
/// line's own handle out.
pub(super) fn names(line: &Line, handle: &Handle) -> bool {
    match (handle, &line.content) {
        (Handle::Bead(place), Content::Bead(_) | Content::Unread(_)) => {
            line.place.as_ref() == Some(place)
        }
        (Handle::Project(project), Content::Project(line)) => line.project == *project,
        (Handle::Elided(place), Content::Elided { under, .. }) => under == place,
        (Handle::Group(kind, project), Content::Group(group)) => {
            group.kind == *kind && group.project == *project
        }
        (Handle::Item(key), Content::Item(item)) => item_key(item).as_ref() == Some(key),
        _ => false,
    }
}
