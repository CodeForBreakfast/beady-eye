//! The forest's tests, and the fixtures and helpers more than one of them reads.

use super::facts::TreeFacts;
use super::handle::Fold;
use super::*;
use crate::collect::bd::parse_shared_beads;
use crate::collect::herdr::parse_agent_list;
use crate::config::{Config, Scope};
use crate::model::join::{self, Joined, Listed, ProjectRows};
use crate::model::snapshot;
use crate::model::snapshot::{
    a_provider, build_tree, Collected, FailedProject, ProviderState, Readiness, TrackerFailure,
    TrackerState, A_PROVIDER,
};
use crate::model::tree::{self, Assembled, Nesting};
use crate::model::types::testing::{key as pane_key, A_SESSION};
use crate::model::types::{Bead, Pane};
use crate::view::draw::identity_widths;
use crate::view::lines::{
    counts_beneath, facts_of, links_below, marker, prefix, progress_of, run_size, split,
    walks_on_this_thread, way_below, Group, Item, Note, ProjectLine, OPEN, SHUT,
};
use crate::view::phrase;
use crate::view::row::{Cell, Progress, Row, Widths};
use crate::view::walk::{self, Rows};
use chrono::{DateTime, Utc};
use pretty_assertions::assert_eq;
use std::sync::Arc;

mod columns;
mod copies;
mod default_folds;
mod empty;
mod focus;
mod fold_keys;
mod going_to;
mod groups;
mod missing_blocker;
mod motion;
mod other_projects;
mod progress;
mod projects;
mod refused;
mod runs;
mod searching;
mod selecting;
mod shut_over;
mod viewport;

/// Dunwich's tree as bd writes it. `dun-7.7` waits on a bead no row holds,
/// so the tree reports it; `dun-7.1.2` is a node bd stopped at; `dun-7.4`
/// is closed with a pane still on it, and the other three closed siblings
/// are finished.
const DUNWICH: &str = r#"[
  {"id":"dun-7","title":"lift the ground station","status":"in_progress",
   "priority":1,"issue_type":"epic","updated_at":"2026-08-29T12:00:00Z",
   "metadata":{"agent_pane":"w:p1"}},
  {"id":"dun-7.1","title":"re-point the dish","status":"open",
   "dependencies":[{"depends_on_id":"dun-7","type":"parent-child"}],
   "priority":2,"issue_type":"task"},
  {"id":"dun-7.1.1","title":"true the mount","status":"open",
   "dependencies":[{"depends_on_id":"dun-7.1","type":"parent-child"}],
   "priority":2,"issue_type":"task"},
  {"id":"dun-7.1.2","title":"seal the feed horn","status":"open",
   "dependencies":[{"depends_on_id":"dun-7.1","type":"parent-child"}],
   "priority":3,"issue_type":"task"},
  {"id":"dun-7.2","title":"survey the mast","status":"closed",
   "dependencies":[{"depends_on_id":"dun-7","type":"parent-child"}],
   "priority":2,"issue_type":"task","closed_at":"2026-08-28T09:00:00Z"},
  {"id":"dun-7.3","title":"pour the pad","status":"closed",
   "dependencies":[{"depends_on_id":"dun-7","type":"parent-child"}],
   "priority":2,"issue_type":"task","closed_at":"2026-08-27T09:00:00Z"},
  {"id":"dun-7.4","title":"clear the access road","status":"closed",
   "dependencies":[{"depends_on_id":"dun-7","type":"parent-child"}],
   "priority":2,"issue_type":"task","closed_at":"2026-08-26T09:00:00Z",
   "metadata":{"agent_pane":"w:p2"}},
  {"id":"dun-7.5","title":"set the guard rail","status":"closed",
   "dependencies":[{"depends_on_id":"dun-7","type":"parent-child"}],
   "priority":2,"issue_type":"task","closed_at":"2026-08-25T09:00:00Z"},
  {"id":"dun-7.7","title":"log the survey marks","status":"open",
   "priority":2,"issue_type":"task",
   "dependencies":[{"depends_on_id":"dun-7","type":"parent-child"},
                   {"depends_on_id":"dun-6","type":"blocks"}]}
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
/// The harbour and the slipway as one tree, which is what the tracker
/// answers once somebody files the second root as a child of the first.
/// Two roots become one, and the bead a reader was finishing is where it
/// always was in their head and somewhere else in the answer.
const SLIPWAY_UNDER_HARBOUR: &str = r#"[
  {"id":"hbr-3","title":"dredge the channel","status":"open",
   "priority":2,"issue_type":"epic"},
  {"id":"hbr-3.1","title":"survey the silt","status":"open",
   "dependencies":[{"depends_on_id":"hbr-3","type":"parent-child"}],
   "priority":2,"issue_type":"task"},
  {"id":"hbr-9","title":"re-deck the slipway","status":"open",
   "dependencies":[{"depends_on_id":"hbr-3","type":"parent-child"}],
   "priority":2,"issue_type":"epic"},
  {"id":"hbr-9.1","title":"strip the planking","status":"open",
   "dependencies":[{"depends_on_id":"hbr-9","type":"parent-child"},
                   {"depends_on_id":"hbr-4","type":"blocks"}],
   "priority":2,"issue_type":"task"}
]"#;

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

/// A tree whose widest id is two levels down, where nothing the folds
/// name reaches: what an expanded forest counts rather than draws.
const WIDE_BELOW: &str = r#"[
  {"id":"tow-1","title":"raise the tower","status":"open",
   "priority":1,"issue_type":"epic"},
  {"id":"tow-1.1","title":"stand the mast","status":"open",
   "dependencies":[{"depends_on_id":"tow-1","type":"parent-child"}],
   "priority":2,"issue_type":"task"},
  {"id":"tow-1.1.1000000","title":"bolt the sections","status":"open",
   "dependencies":[{"depends_on_id":"tow-1.1","type":"parent-child"}],
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
const KADATH: &str = r#"[
  {"id":"bcn-6","title":"re-lamp the kadath","status":"in_progress",
   "priority":1,"issue_type":"task","updated_at":"2026-08-29T12:00:00Z"}
]"#;

/// One epic whose two halves are each held up by the same survey. Under
/// the rule that a bead's descendants are what must finish before it,
/// `dun-9` is drawn beneath both of them: under `dun-8.1` as its child,
/// and under `dun-8.2` as what it waits on.
const TWICE: &str = r#"[
  {"id":"dun-8","title":"lift the gantry","status":"in_progress",
   "priority":1,"issue_type":"epic"},
  {"id":"dun-8.1","title":"pour the pad","status":"in_progress",
   "dependencies":[{"depends_on_id":"dun-8","type":"parent-child"}],
   "priority":2,"issue_type":"task"},
  {"id":"dun-8.2","title":"rail the crane","status":"in_progress",
   "dependencies":[{"depends_on_id":"dun-8","type":"parent-child"},
                   {"depends_on_id":"dun-9","type":"blocks"}],
   "priority":2,"issue_type":"task"},
  {"id":"dun-9","title":"survey the ground","status":"in_progress",
   "dependencies":[{"depends_on_id":"dun-8.1","type":"parent-child"}],
   "priority":2,"issue_type":"task"},
  {"id":"dun-9.1","title":"drill the cores","status":"in_progress",
   "dependencies":[{"depends_on_id":"dun-9","type":"parent-child"}],
   "priority":2,"issue_type":"task"}
]"#;

/// A closed bead holding unfinished work, drawn twice in one tree: under
/// `dun-5.1` as its child, and under `dun-5.2` as what it waits on. Both
/// copies rest shut, because nothing under either is live or ready, so
/// what each of them says about the work it is shut over is all that
/// tells them apart.
///
/// `dun-5.1.1` and `dun-5.2.1` are here to be worked on: they are what
/// holds the two halves open, so both copies of `dun-4` are on the
/// screen at once.
const CLOSED_TWICE: &str = r#"[
  {"id":"dun-5","title":"re-deck the bridge","status":"in_progress",
   "priority":1,"issue_type":"epic"},
  {"id":"dun-5.1","title":"strip the north span","status":"in_progress",
   "dependencies":[{"depends_on_id":"dun-5","type":"parent-child"}],
   "priority":2,"issue_type":"task"},
  {"id":"dun-5.1.1","title":"cut the north deck","status":"in_progress",
   "dependencies":[{"depends_on_id":"dun-5.1","type":"parent-child"}],
   "priority":2,"issue_type":"task"},
  {"id":"dun-4","title":"close the towpath","status":"closed",
   "dependencies":[{"depends_on_id":"dun-5.1","type":"parent-child"}],
   "priority":2,"issue_type":"task","closed_at":"2026-08-28T09:00:00Z"},
  {"id":"dun-4.1","title":"post the diversion","status":"open",
   "dependencies":[{"depends_on_id":"dun-4","type":"parent-child"}],
   "priority":2,"issue_type":"task"},
  {"id":"dun-5.2","title":"strip the south span","status":"in_progress",
   "dependencies":[{"depends_on_id":"dun-5","type":"parent-child"},
                   {"depends_on_id":"dun-4","type":"blocks"}],
   "priority":2,"issue_type":"task"},
  {"id":"dun-5.2.1","title":"cut the south deck","status":"in_progress",
   "dependencies":[{"depends_on_id":"dun-5.2","type":"parent-child"}],
   "priority":2,"issue_type":"task"}
]"#;

/// The same shape again, over a blocker deep enough to show what the way
/// down to a copy says about the copy's own children: `dun-6` is drawn
/// under `dun-3.1` as its child and under `dun-3.2` as what it waits on,
/// and the work is two levels below it rather than one.
const DEEP_TWICE: &str = r#"[
  {"id":"dun-3","title":"raise the mast","status":"in_progress",
   "priority":1,"issue_type":"epic"},
  {"id":"dun-3.1","title":"sink the footing","status":"in_progress",
   "dependencies":[{"depends_on_id":"dun-3","type":"parent-child"}],
   "priority":2,"issue_type":"task"},
  {"id":"dun-3.2","title":"guy the mast","status":"in_progress",
   "dependencies":[{"depends_on_id":"dun-3","type":"parent-child"},
                   {"depends_on_id":"dun-6","type":"blocks"}],
   "priority":2,"issue_type":"task"},
  {"id":"dun-6","title":"cast the collar","status":"in_progress",
   "dependencies":[{"depends_on_id":"dun-3.1","type":"parent-child"}],
   "priority":2,"issue_type":"task"},
  {"id":"dun-6.1","title":"mill the collar","status":"in_progress",
   "dependencies":[{"depends_on_id":"dun-6","type":"parent-child"}],
   "priority":2,"issue_type":"task"},
  {"id":"dun-6.1.1","title":"bore the bolt holes","status":"in_progress",
   "dependencies":[{"depends_on_id":"dun-6.1","type":"parent-child"}],
   "priority":2,"issue_type":"task"}
]"#;

/// One bead both halves of an epic wait on, with work of its own beneath
/// it, and one half with work of its own as well.
///
/// The shape the one-copy rules were written for: `dun-1` is drawn under
/// `dun-2.1` as what it waits on and under `dun-2.2` for the same reason,
/// and both ways down to it are the same length.
///
/// Every bead is open and none is ready, so the only work a reader needs
/// in it is the pane a test staffs it with — and moving that pane is the
/// whole of what moves the spine.
const BLOCKS_BOTH: &str = r#"[
  {"id":"dun-2","title":"step the derrick","status":"open",
   "priority":1,"issue_type":"epic"},
  {"id":"dun-2.1","title":"seat the shoe","status":"open",
   "dependencies":[{"depends_on_id":"dun-2","type":"parent-child"},
                   {"depends_on_id":"dun-1","type":"blocks"}],
   "priority":2,"issue_type":"task"},
  {"id":"dun-2.2","title":"trim the stay","status":"open",
   "dependencies":[{"depends_on_id":"dun-2","type":"parent-child"},
                   {"depends_on_id":"dun-1","type":"blocks"}],
   "priority":2,"issue_type":"task"},
  {"id":"dun-2.2.1","title":"swage the stay","status":"open",
   "dependencies":[{"depends_on_id":"dun-2.2","type":"parent-child"}],
   "priority":2,"issue_type":"task"},
  {"id":"dun-1","title":"turn the pintle","status":"open",
   "priority":2,"issue_type":"task"},
  {"id":"dun-1.1","title":"ream the pintle","status":"open",
   "dependencies":[{"depends_on_id":"dun-1","type":"parent-child"}],
   "priority":2,"issue_type":"task"}
]"#;

/// That tree with one agent, on the bead named.
fn a_blocker_both_halves_wait_on(staffed: &str) -> Snapshot {
    alone("dunwich", BLOCKS_BOTH, &panes_on(&[staffed]))
}

/// One bead reached four ways down of four different lengths, so the four
/// one-copy rules choose four different ways to it and draw four
/// different screens.
///
/// `bel-1.1.2.1` is the bead with the agent, and it is a child of one of
/// `bel-1.1`'s children and a blocker of another. `bel-1.1` waits on it
/// as well, which is the way of fewest steps; `bel-1.1.1`, the earlier of
/// the two siblings, waits on it; `bel-1.1.2` is its parent; and
/// `bel-1.1.3.1` waits on it furthest down. It has the lowest priority of
/// `bel-1.1`'s children, so it sorts last among them and the walk reaches
/// it under `bel-1.1.1` before it reaches `bel-1.1`'s own way down to it.
///
/// Every bead is open and none is ready, so the pane is the only work a
/// reader needs in the tree and the way down to it is the whole spine.
const FOUR_WAYS_DOWN: &str = r#"[
  {"id":"bel-1","title":"raise the belfry","status":"open",
   "priority":1,"issue_type":"epic"},
  {"id":"bel-1.1","title":"hang the bell","status":"open",
   "dependencies":[{"depends_on_id":"bel-1","type":"parent-child"},
                   {"depends_on_id":"bel-1.1.2.1","type":"blocks"}],
   "priority":1,"issue_type":"epic"},
  {"id":"bel-1.1.1","title":"dress the stone","status":"open",
   "dependencies":[{"depends_on_id":"bel-1.1","type":"parent-child"},
                   {"depends_on_id":"bel-1.1.2.1","type":"blocks"}],
   "priority":2,"issue_type":"task"},
  {"id":"bel-1.1.2","title":"found the mould","status":"open",
   "dependencies":[{"depends_on_id":"bel-1.1","type":"parent-child"}],
   "priority":2,"issue_type":"task"},
  {"id":"bel-1.1.2.1","title":"cast the bell","status":"open",
   "dependencies":[{"depends_on_id":"bel-1.1.2","type":"parent-child"}],
   "priority":3,"issue_type":"task"},
  {"id":"bel-1.1.3","title":"cut the louvres","status":"open",
   "dependencies":[{"depends_on_id":"bel-1.1","type":"parent-child"}],
   "priority":2,"issue_type":"task"},
  {"id":"bel-1.1.3.1","title":"pin the louvres","status":"open",
   "dependencies":[{"depends_on_id":"bel-1.1.3","type":"parent-child"},
                   {"depends_on_id":"bel-1.1.2.1","type":"blocks"}],
   "priority":2,"issue_type":"task"}
]"#;

/// That tree, with the agent on the bead every rule chooses a way to.
fn four_ways_to_the_agent() -> Snapshot {
    alone("dunwich", FOUR_WAYS_DOWN, &panes_on(&["bel-1.1.2.1"]))
}

/// A tree in which a bead's parent hangs beneath the bead. `cyc-2.2`
/// waits on `cyc-2.3` and is its child, so each is drawn under the other,
/// and `cyc-2.1` waits on `cyc-2.2` so the pair hang under the root.
///
/// Hanging `cyc-2.2` back under its parent would leave the two of them a
/// ring nothing above reaches, and the whole way down would rest shut
/// over the agent.
const PARENT_BENEATH: &str = r#"[
  {"id":"cyc-2","title":"sink the shaft","status":"open",
   "priority":1,"issue_type":"epic"},
  {"id":"cyc-2.1","title":"line the shaft","status":"open",
   "dependencies":[{"depends_on_id":"cyc-2","type":"parent-child"},
                   {"depends_on_id":"cyc-2.2","type":"blocks"}],
   "priority":2,"issue_type":"task"},
  {"id":"cyc-2.2","title":"hang the cage","status":"open",
   "dependencies":[{"depends_on_id":"cyc-2.3","type":"parent-child"},
                   {"depends_on_id":"cyc-2.3","type":"blocks"}],
   "priority":2,"issue_type":"task"},
  {"id":"cyc-2.3","title":"splice the rope","status":"open",
   "priority":2,"issue_type":"task"}
]"#;

/// One tree drawing one bead twice, which is the shape a blocker nested
/// under each bead it holds up gives: same root, same key, two lines.
fn drawn_twice_in_one_tree() -> Snapshot {
    alone("dunwich", TWICE, &panes_on(&["dun-9.1"]))
}

/// `TWICE`'s shape stacked `depth` deep: under each `dun-50.<n>` its two
/// halves, and under both halves `dun-50.<n+1>`, so the last of them is
/// drawn once per way down, which is two to the power of `depth`.
/// Nothing is live or ready, so every fold rests shut and only a search
/// opens one.
fn nested_diamonds(depth: usize) -> Snapshot {
    let mut rows = vec![
        r#"{"id":"dun-50.0","title":"span the valley","status":"open",
         "priority":1,"issue_type":"epic"}"#
            .to_string(),
    ];
    for n in 0..depth {
        let below = n + 1;
        let title = if below == depth {
            "set the keystone"
        } else {
            "turn the arch"
        };
        rows.push(format!(
            r#"{{"id":"dun-50.{n}.1","title":"raise the east pier","status":"open",
              "dependencies":[{{"depends_on_id":"dun-50.{n}","type":"parent-child"}}],
              "priority":2,"issue_type":"task"}}"#
        ));
        rows.push(format!(
            r#"{{"id":"dun-50.{n}.2","title":"raise the west pier","status":"open",
              "dependencies":[{{"depends_on_id":"dun-50.{n}","type":"parent-child"}},
                              {{"depends_on_id":"dun-50.{below}","type":"blocks"}}],
              "priority":2,"issue_type":"task"}}"#
        ));
        rows.push(format!(
            r#"{{"id":"dun-50.{below}","title":"{title}","status":"open",
              "dependencies":[{{"depends_on_id":"dun-50.{n}.1","type":"parent-child"}}],
              "priority":2,"issue_type":"task"}}"#
        ));
    }
    alone("dunwich", &format!("[{}]", rows.join(",")), &[])
}

/// The same, over a blocker with two levels of work beneath it.
fn deep_bead_drawn_twice_in_one_tree() -> Snapshot {
    alone("dunwich", DEEP_TWICE, &panes_on(&["dun-6.1.1"]))
}

/// The same, over a closed bead that still holds unfinished work.
fn closed_bead_drawn_twice_in_one_tree() -> Snapshot {
    alone(
        "dunwich",
        CLOSED_TWICE,
        &panes_on(&["dun-5.1.1", "dun-5.2.1"]),
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

/// `w:p3` and `w:p4` both name `dun-7.1`, so neither holds it; `w:p9` is
/// working in the project whose tracker refused; `w:pF` is under no
/// configured project at all.
const PANES: &str = r#"{"result":{"agents":[
  {"pane_id":"w:p1","cwd":"/srv/work/dunwich","agent_status":"working"},
  {"pane_id":"w:p2","cwd":"/srv/work/dunwich","agent_status":"idle"},
  {"pane_id":"w:p3","cwd":"/srv/work/dunwich","agent_status":"working",
   "display_agent":"dun-7.1"},
  {"pane_id":"w:p4","cwd":"/srv/work/dunwich","agent_status":"idle",
   "display_agent":"dun-7.1"},
  {"pane_id":"w:p9","cwd":"/srv/work/ferry","agent_status":"blocked"},
  {"pane_id":"w:pF","cwd":"/srv/spike","agent_status":"idle"}
]}}"#;

fn cfg() -> Config {
    Config::from_toml(
        r#"
[[projects]]
name = "dunwich"
path = "/srv/work/dunwich"
credential_command = "secret dunwich"

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
fn root_row(beads: &[Arc<crate::model::types::Bead>]) -> String {
    beads
        .iter()
        .find(|b| b.dependencies.is_empty())
        .expect("a root row")
        .id
        .clone()
}

fn assembled(json: &str) -> Assembled {
    let beads = parse_shared_beads(json).expect("the rows parse");
    let root = root_row(&beads);
    Nesting::of(&beads)
        .assemble(&root)
        .expect("the rows assemble")
}

fn panes() -> Vec<Pane> {
    parse_agent_list(A_SESSION, PANES).expect("the panes parse")
}

/// One working pane and one idle one, both in Dunwich's tree. Enough to
/// staff a fixture without the conflicting and unconfigured panes the
/// shared snapshot carries to exercise its groups.
const TWO_PANES: &str = r#"{"result":{"agents":[
  {"pane_id":"w:p1","cwd":"/srv/work/dunwich","agent_status":"working"},
  {"pane_id":"w:p2","cwd":"/srv/work/dunwich","agent_status":"idle"}
]}}"#;

fn two_panes() -> Vec<Pane> {
    parse_agent_list(A_SESSION, TWO_PANES).expect("the panes parse")
}

fn joined(dunwich: &Assembled, harbour: &Assembled, panes: &[Pane]) -> Joined {
    let cfg = cfg();
    join::resolve(
        &[
            ProjectRows {
                project: "dunwich",
                rows: &dunwich.beads,
            },
            ProjectRows {
                project: "harbour",
                rows: &harbour.beads,
            },
        ],
        Listed::all(panes),
        &cfg,
    )
}

fn tree_of(project: &str, json: &str) -> Tree {
    let dunwich = assembled(DUNWICH);
    let harbour = assembled(HARBOUR);
    let panes = panes();
    let joined = joined(&dunwich, &harbour, &panes);
    build_tree(
        project,
        &assembled(json),
        &joined,
        &crate::model::snapshot::said_by(project, &Readiness::default(), &BTreeMap::new()),
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
    let dunwich = assembled(DUNWICH);
    let harbour = assembled(HARBOUR);
    let panes = panes();
    let joined = joined(&dunwich, &harbour, &panes);
    snapshot::build(
        Collected {
            trees,
            failed_projects: failed,
            read_at: every_project_read(),
            speaks_until: BTreeMap::new(),
            read_for_reach: BTreeSet::new(),
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
            tree_of("dunwich", DUNWICH),
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
        Content::Absent(absent) if absent.tracker == TrackerState::RootNotFound => {
            format!("⚠ {} gone", absent.root)
        }
        Content::Absent(absent) => format!("⚠ {} unread", absent.root),
        Content::Bead(row) => format!("{} {} {}", row.glyph, row.id, row.title),
        Content::Elided { count, .. } => format!("… {count} more"),
        Content::Orphaned(orphaned) => format!("⚠ {} {:?}", orphaned.id, orphaned.why),
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

/// The bead on every line resting open, in the order they are drawn.
/// What a rule does is choose which of them these are.
fn resting_open(forest: &Forest) -> Vec<String> {
    forest
        .lines()
        .iter()
        .filter(|line| line.folded == Some(true))
        .filter_map(|line| line.bead().map(|bead| bead.id.clone()))
        .collect()
}

/// Press `key` until the rule named is in force, which is what a reader
/// does and what keeps a test off the order of the cycle.
/// A forest with every copy of each bead on the spine: the screen a test
/// of what one copy of a bead does beside another is read against.
fn under_every_copy(snapshot: Snapshot) -> Forest {
    let mut forest = flatten(snapshot);
    put_in_force(&mut forest, Spine::EveryCopy, Action::CycleSpineForest);
    forest
}

fn put_in_force(forest: &mut Forest, rule: Spine, key: Action) {
    for _ in Spine::EVERY {
        if forest.spine() == rule {
            return;
        }
        forest.apply(key);
    }
    panic!(
        "{rule:?} never came round: the cycle stopped at {:?}",
        forest.spine()
    );
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
    alone("dunwich", DEPOT, &panes_on(&["dep-1.1"]))
}

/// One project's tree, joined against its own rows so that a pane the
/// fixture names lands on the bead that names it. `tree_of` joins every
/// fixture against Dunwich's rows, which is what the shared snapshot
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
    let rows: Vec<Arc<Bead>> = roots
        .iter()
        .flat_map(|root| root.beads.iter().cloned())
        .collect();
    let cfg = cfg();
    let joined = join::resolve(
        &[ProjectRows {
            project,
            rows: &rows,
        }],
        Listed::all(panes),
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
                &crate::model::snapshot::said_by(project, &readiness, &BTreeMap::new()),
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
            speaks_until: BTreeMap::new(),
            read_for_reach: BTreeSet::new(),
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

/// A working pane in Dunwich on each of `on`. Each pane names its bead,
/// which is how a bead carrying no configured key gets its agent, so a
/// fixture is staffed by naming the beads someone is on.
fn panes_on(on: &[&str]) -> Vec<Pane> {
    let agents: Vec<String> = on
        .iter()
        .map(|id| {
            format!(
                r#"{{"pane_id":"w:{id}","cwd":"/srv/work/dunwich",
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
    alone("dunwich", TOWER, &panes_on(on))
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
            project: "dunwich",
            rows: &rows,
        }],
        Listed::all(panes),
        &cfg,
    );
    let tree = |rows: &Assembled| {
        build_tree(
            "dunwich",
            rows,
            &joined,
            &crate::model::snapshot::said_by("dunwich", &Readiness::default(), &BTreeMap::new()),
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
            speaks_until: BTreeMap::new(),
            read_for_reach: BTreeSet::new(),
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

fn siding() -> Snapshot {
    alone("dunwich", SIDING, &panes_on(&["sdg-4.3"]))
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
    alone("dunwich", &json, &panes_on(&["dep-1.1"]))
}

/// The same snapshot with the two panes that were fighting over `dun-7.1`
/// gone, which empties the unattributed group of the one the tests hold.
fn built_without_the_conflicting_panes() -> Snapshot {
    let mut snapshot = snapshot();
    snapshot.unattributed.retain(|pane| pane.pane.id == "w:p3");
    snapshot
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

fn on_the_hidden_trees_group(forest: &Forest) -> bool {
    matches!(
        &forest.lines()[forest.selected_line()].content,
        Content::Group(group) if group.kind == GroupKind::HiddenTrees
    )
}

/// The five degraded kinds, plus the two sorts of loose pane — the ones
/// drawn under their project, and the ones no configured project covers.
#[derive(Debug, Default, PartialEq, Eq)]
struct Reported {
    orphaned_dependencies: usize,
    cycles: usize,
    conflicts: usize,
    failed_projects: usize,
    loose_panes: usize,
    unconfigured_panes: usize,
}

fn on_screen(forest: &Forest) -> Reported {
    let mut found = Reported::default();
    for line in forest.lines() {
        match &line.content {
            // Matched variant by variant so a note added later has to
            // be decided here rather than fall through as nothing.
            Content::Note(note) => match note {
                Note::OrphanedDependencies(n) => found.orphaned_dependencies += n,
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
                // Holds no finding of its own: it stands over whole
                // roots, and what is wrong inside one of those is the
                // root's to report when the group is opened.
                GroupKind::HiddenTrees => {}
            },
            _ => {}
        }
    }
    found
}

/// The line drawn for one bead, by the whole id it carries.
fn line_of<'a>(forest: &'a Forest, id: &str) -> &'a Line {
    forest
        .lines()
        .iter()
        .find(|line| line.bead().is_some_and(|bead| bead.id == id))
        .unwrap_or_else(|| panic!("{id} is not drawn"))
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

/// A fixture with one bead taken out of it.
fn without(json: &str, id: &str) -> String {
    let beads: Vec<serde_json::Value> = serde_json::from_str(json).expect("fixture is json");
    let kept: Vec<serde_json::Value> = beads.into_iter().filter(|bead| bead["id"] != id).collect();
    serde_json::Value::Array(kept).to_string()
}

/// Put the selection on a project's own line, the way a reader would with
/// the keys they have.
fn select_project(forest: &mut Forest, project: &str) {
    let at = forest
        .lines()
        .iter()
        .position(|line| matches!(&line.content, Content::Project(line) if line.project == project))
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

impl Forest {
    /// A search begun from wherever the selection is now.
    fn seek_here(&mut self, query: &str) -> Landed {
        let origin = self.origin();
        self.seek(query, &origin)
    }
}

fn went_to(project: &str, id: &str, at: usize, of: usize) -> Landed {
    Landed::On {
        key: key(project, id),
        at,
        of,
    }
}

fn drawn_here(forest: &Forest, said: &str) -> bool {
    sketch(forest).iter().any(|row| row.contains(said))
}

/// Put the selection on a bead and focus the forest there, the way a
/// reader would with the keys they have.
fn focus_on(forest: &mut Forest, id: &str) {
    select_bead(forest, id);
    assert!(
        forest.apply(Action::FocusForest),
        "focusing {id} changed nothing: {:#?}",
        sketch(forest)
    );
}

/// Start the forest as `bdi` naming these beads on the command line does.
fn named_on_the_command_line(beads: &[(&str, &str)]) -> Forest {
    let mut forest = flatten(snapshot());
    forest.focus_when_drawn(beads.iter().map(|(project, id)| key(project, id)).collect());
    forest
}

/// Harbour's rows and dunwich's, drawn as a collection draws both
/// projects, from the roots named and with a pane on each of `on`.
fn harbour_and_dunwich(
    harbour: &str,
    dunwich: &str,
    roots: &[(&str, &str)],
    on: &[&str],
) -> Snapshot {
    harbour_and_dunwich_ready(harbour, dunwich, roots, on, &[])
}

/// The same, with the beads `bd` answers `ready` with named.
fn harbour_and_dunwich_ready(
    harbour: &str,
    dunwich: &str,
    roots: &[(&str, &str)],
    on: &[&str],
    ready: &[&str],
) -> Snapshot {
    let harbour = parse_shared_beads(harbour).expect("the rows parse");
    let dunwich = parse_shared_beads(dunwich).expect("the rows parse");
    let across = tree::Across::of(
        [
            ("harbour", Nesting::of(&harbour)),
            ("dunwich", Nesting::of(&dunwich)),
        ],
        [],
    );
    let panes = panes_on(on);
    let cfg = cfg();
    let joined = join::resolve(
        &[
            ProjectRows {
                project: "harbour",
                rows: &harbour,
            },
            ProjectRows {
                project: "dunwich",
                rows: &dunwich,
            },
        ],
        Listed::all(&panes),
        &cfg,
    );
    let readiness = Readiness {
        ready: ready.iter().map(|id| (*id).to_string()).collect(),
        ..Readiness::default()
    };
    let relations = [
        ("harbour", crate::model::edges::relations(&harbour)),
        ("dunwich", crate::model::edges::relations(&dunwich)),
    ];
    let said = relations
        .iter()
        .map(|(project, relations)| {
            (
                *project,
                snapshot::Said {
                    readiness: &readiness,
                    relations,
                },
            )
        })
        .collect();
    let trees = roots
        .iter()
        .map(|&(project, root)| {
            build_tree(
                project,
                &across.assemble(project, root).expect("the rows assemble"),
                &joined,
                &said,
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
            speaks_until: BTreeMap::new(),
            read_for_reach: BTreeSet::new(),
        },
        &panes,
        &joined,
        &cfg,
        a_provider(ProviderState::Answering),
        Filter::All,
        now(),
    )
}
