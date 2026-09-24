//! What a line the selection or a fold can rest on is known by, and the folds
//! held against those names.
//!
//! Both halves of the forest say these words: the loop that reads keys moves
//! a fold, laying out asks which way one points, and neither reaches past the
//! other to do it.

use std::collections::{BTreeMap, BTreeSet};

use crate::model::join::Conflict;
use crate::model::snapshot::Tree;
use crate::model::types::PaneKey;
use crate::view::lines::{root_key, Content, GroupKind, Item, Line, Place};

/// What a line that folds is known by, so both the fold and the selection
/// survive a refresh that reorders or drops lines.
///
/// A bead can be drawn on more than one line, so a handle names the line and
/// not the bead standing on it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Handle {
    Bead(Place),
    /// A root whose tracker would not read, by where it stands: a line with
    /// no bead on it.
    Unread(Place),
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
pub(super) struct Folds {
    lines: BTreeMap<Handle, Fold>,
    /// The folds the last search step opened, whatever their lines say, until
    /// the next step shuts them again or the reader keeps them.
    ///
    /// Apart from the lines rather than written over them, so a fold the
    /// reader shut is still theirs underneath, and live work arriving under
    /// it spends it as it would have with no search in the way.
    stepped_open: BTreeSet<Handle>,
}

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
    /// Every fold points this way.
    Points { open: bool },
    /// Every fold rests, whatever a scope above says.
    Rests,
}

impl Fold {
    fn holds_shut(&self) -> bool {
        self.line == Some(Way::Shut) || matches!(self.scope, Some(Scope::Points { open: false }))
    }
}

impl Folds {
    /// Which way the reader has pointed a fold, from the nearest entry on the
    /// way down to it: the line's own fold, the scope set on the line, or the
    /// scope `over` it. Nothing where the fold is left to rest: only the
    /// caller knows the tree a line came from, so it says where the line
    /// rests rather than being asked to re-derive it here.
    pub(super) fn pointed(&self, handle: &Handle, over: Option<&Scope>) -> Option<bool> {
        if self.stepped_open.contains(handle) {
            return Some(true);
        }
        match self.lines.get(handle).and_then(|fold| fold.line) {
            Some(Way::Open) => return Some(true),
            Some(Way::Shut) => return Some(false),
            Some(Way::Rests) => return None,
            None => {}
        }
        Folds::forced(self.beneath(handle, over))
    }

    /// The scope everything under a line answers from: the one set on the
    /// line where a key set one there, and the one `over` the line otherwise.
    pub(super) fn beneath<'a>(
        &'a self,
        handle: &Handle,
        over: Option<&'a Scope>,
    ) -> Option<&'a Scope> {
        self.lines
            .get(handle)
            .and_then(|fold| fold.scope.as_ref())
            .or(over)
    }

    /// Point one fold the way the user asked.
    pub(super) fn set(&mut self, handle: Handle, open: bool) {
        let way = if open { Way::Open } else { Way::Shut };
        self.lines.entry(handle).or_default().line = Some(way);
    }

    /// Point a line and every fold beneath it one way. Whatever the reader
    /// had set beneath it goes: the scope is what they are asking for now.
    pub(super) fn set_over(&mut self, handle: Handle, open: bool, beneath: &[Handle]) {
        for under in beneath {
            self.lines.remove(under);
        }
        let scope = Some(Scope::Points { open });
        self.lines.insert(handle, Fold { line: None, scope });
    }

    /// Hand a line and everything beneath it back to the default, and hold
    /// it there against whatever scope stands over it.
    pub(super) fn let_go(&mut self, handle: Handle, beneath: &[Handle]) {
        for under in beneath {
            self.lines.remove(under);
        }
        let scope = Some(Scope::Rests);
        self.lines.insert(handle, Fold { line: None, scope });
    }

    /// Hand one line's own fold back to the default, and hold it there
    /// against whatever scope stands over it. A scope set on the line
    /// stays: `d` by the line puts back each line it drew and no more, and
    /// the lines it did not draw go on answering from that scope.
    pub(super) fn put_back(&mut self, handle: Handle) {
        self.lines.entry(handle).or_default().line = Some(Way::Rests);
    }

    /// The folds the user has shut, which are the only ones that can be
    /// spent: a fold left open holds nothing back. A scope that shut is one
    /// of them, on the line it was set on.
    pub(super) fn shut(&self) -> impl Iterator<Item = &Handle> {
        self.lines
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
            self.lines.get(handle),
            Some(Fold {
                scope: Some(Scope::Points { open: false }),
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
        let fold = self.lines.entry(handle).or_default();
        if fold.line != Some(Way::Open) {
            fold.line = Some(Way::Rests);
        }
    }

    /// Let go of every fold at once.
    pub(super) fn clear(&mut self) {
        self.lines.clear();
        self.stepped_open.clear();
    }

    /// Open one fold for a search step, until the next step shuts it again.
    pub(super) fn open_for_a_step(&mut self, handle: Handle) {
        self.stepped_open.insert(handle);
    }

    /// Hand every fold the last search step opened back to its line.
    pub(super) fn shut_what_the_last_step_opened(&mut self) {
        self.stepped_open.clear();
    }

    /// What the last search step opened, as it stands now.
    pub(super) fn opened_by_the_last_step(&self) -> BTreeSet<Handle> {
        self.stepped_open.clone()
    }

    /// Put back what a search step had opened, still that step's to shut.
    pub(super) fn reopen_for_the_last_step(&mut self, opened: BTreeSet<Handle>) {
        self.stepped_open = opened;
    }

    /// Make whatever the last search step opened the reader's, so no later
    /// step shuts it.
    pub(super) fn keep_what_the_last_step_opened(&mut self) {
        for handle in std::mem::take(&mut self.stepped_open) {
            self.set(handle, true);
        }
    }

    /// Every line the folds name: the ones with an entry, and the ones a
    /// search step opened. A line named nowhere here answers from the scope
    /// over it alone, and so does everything beneath it.
    pub(super) fn mentioned(&self) -> impl Iterator<Item = &Handle> {
        self.lines.keys().chain(&self.stepped_open)
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
        self.lines.iter()
    }
}

/// What a line is known by, where it is one the selection can hold.
///
/// A note stands for a finding rather than for a thing, so it has none — and
/// a line the forest cannot name could not be put back after a refresh, which
/// is why this is the same question as whether the selection may sit there.
pub(super) fn handle_of(line: &Line) -> Option<Handle> {
    match &line.content {
        Content::Bead(_) => line.place.clone().map(Handle::Bead),
        Content::Unread(_) => line.place.clone().map(Handle::Unread),
        Content::Project(line) => Some(Handle::Project(line.project.clone())),
        Content::Elided { under, .. } => Some(Handle::Elided(under.clone())),
        Content::Group(group) => Some(Handle::Group(group.kind, group.project.clone())),
        Content::Item(item) => item_key(item).map(Handle::Item),
        Content::Note(_) | Content::Scoped { .. } => None,
    }
}

/// What a tree's root line is known by. A tree with no nodes is one whose
/// tracker would not read, and its root is a row with no bead on it.
pub(super) fn root_handle(tree: &Tree) -> Handle {
    let place = Place::root(root_key(tree));
    if tree.beads.is_empty() {
        Handle::Unread(place)
    } else {
        Handle::Bead(place)
    }
}

pub(super) fn selectable(line: &Line) -> bool {
    handle_of(line).is_some()
}

/// Whether a line is the one a handle names, asked without spelling the
/// line's own handle out.
pub(super) fn names(line: &Line, handle: &Handle) -> bool {
    match (handle, &line.content) {
        (Handle::Bead(place), Content::Bead(_)) | (Handle::Unread(place), Content::Unread(_)) => {
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
