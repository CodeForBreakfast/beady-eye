//! What a line the selection or a fold can rest on is known by, and the folds
//! held against those names.
//!
//! Both halves of the forest say these words: the loop that reads keys moves
//! a fold, laying out asks which way one points, and neither reaches past the
//! other to do it.

use std::collections::BTreeMap;

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
    Group(GroupKind),
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
    /// A pane, by its id, which is unique in a herdr session. It serves both
    /// groups that hold panes: `recovery` puts a pane in exactly one of them,
    /// and an unconfigured pane is one under no configured project at all.
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
/// The map itself never leaves: a fold is asked which way it points, pointed,
/// or let go of, and nothing else.
#[derive(Default)]
pub(super) struct Folds(BTreeMap<Handle, bool>);

impl Folds {
    /// Whether a fold is open: what the user set it to, or how it rests when
    /// they have not touched it. Only the caller knows the tree a line came
    /// from, so it says where the line rests rather than being asked to
    /// re-derive it here.
    pub(super) fn expanded(&self, handle: &Handle, resting: bool) -> bool {
        self.0.get(handle).copied().unwrap_or(resting)
    }

    /// Point one fold the way the user asked.
    pub(super) fn set(&mut self, handle: Handle, open: bool) {
        self.0.insert(handle, open);
    }

    /// The folds the user has shut, which are the only ones that can be
    /// spent: a fold left open holds nothing back.
    pub(super) fn shut(&self) -> impl Iterator<Item = &Handle> {
        self.0
            .iter()
            .filter(|(_, open)| !**open)
            .map(|(handle, _)| handle)
    }

    /// Let go of the folds named, handing each back to the default it rests
    /// at.
    pub(super) fn spend(&mut self, spent: &[Handle]) {
        for handle in spent {
            self.0.remove(handle);
        }
    }

    /// Let go of every fold at once.
    pub(super) fn clear(&mut self) {
        self.0.clear();
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
        Content::Group(group) => Some(Handle::Group(group.kind)),
        Content::Item(item) => item_key(item).map(Handle::Item),
        Content::Note(_) | Content::Scoped { .. } => None,
    }
}

pub(super) fn selectable(line: &Line) -> bool {
    handle_of(line).is_some()
}
