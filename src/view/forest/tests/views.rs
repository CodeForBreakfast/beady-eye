//! The views the config names: each a list of the unfinished beads its
//! conditions select, across every project read, drawn in place of the
//! forest.

use super::*;
use pretty_assertions::assert_eq;

/// Harbour's beads. `hbr-1.2` is being signed but waits, so the signing
/// view leaves it out; `hbr-1.4` is a bug, which only harbour's urgent
/// select takes.
const HARBOUR_SIGNING: &str = r#"[
  {"id":"hbr-1","title":"open the harbour","status":"open","priority":2,"issue_type":"epic"},
  {"id":"hbr-1.1","title":"sign the berth lease","status":"open","priority":2,
   "issue_type":"task","labels":["signing"],
   "dependencies":[{"depends_on_id":"hbr-1","type":"parent-child"}]},
  {"id":"hbr-1.2","title":"sign the pilotage terms","status":"open","priority":1,
   "issue_type":"task","labels":["signing","held"],
   "dependencies":[{"depends_on_id":"hbr-1","type":"parent-child"}]},
  {"id":"hbr-1.3","title":"clear the silted channel","status":"open","priority":0,
   "issue_type":"task",
   "dependencies":[{"depends_on_id":"hbr-1","type":"parent-child"}]},
  {"id":"hbr-1.4","title":"fix the swing bridge","status":"open","priority":3,
   "issue_type":"bug",
   "dependencies":[{"depends_on_id":"hbr-1","type":"parent-child"}]}
]"#;

/// Dunwich's beads. `dun-7.4` was signed and is closed, so no view lists
/// it; `dun-7.3` is a bug in a project the urgent view's bug select does
/// not name. `dun-7.2` is in progress with nobody on it, which every list
/// counts at its foot as a bead to check in the forest.
const DUNWICH_SIGNING: &str = r#"[
  {"id":"dun-7","title":"lift the ground station","status":"open","priority":2,
   "issue_type":"epic"},
  {"id":"dun-7.1","title":"sign for the dish","status":"open","priority":1,
   "issue_type":"task","labels":["signing"],
   "dependencies":[{"depends_on_id":"dun-7","type":"parent-child"}]},
  {"id":"dun-7.2","title":"shore up the mast","status":"in_progress","priority":0,
   "issue_type":"task",
   "dependencies":[{"depends_on_id":"dun-7","type":"parent-child"}]},
  {"id":"dun-7.3","title":"fix the feed horn","status":"open","priority":2,
   "issue_type":"bug",
   "dependencies":[{"depends_on_id":"dun-7","type":"parent-child"}]},
  {"id":"dun-7.4","title":"sign off the survey","status":"closed","priority":1,
   "issue_type":"task","labels":["signing"],
   "dependencies":[{"depends_on_id":"dun-7","type":"parent-child"}]}
]"#;

fn signing() -> Snapshot {
    harbour_and_dunwich(
        HARBOUR_SIGNING,
        DUNWICH_SIGNING,
        &[("dunwich", "dun-7"), ("harbour", "hbr-1")],
        &[],
    )
}

const THE_SIGNING_VIEW: &[&str] = &[
    "dunwich",
    "harbour",
    "# View { name: \"signing\", ready: false } 2",
    "▸ ○ dun-7.1 sign for the dish",
    "▸ ○ hbr-1.1 sign the berth lease",
    "! ToCheck(1)",
];

const THE_URGENT_VIEW: &[&str] = &[
    "dunwich",
    "harbour",
    "# View { name: \"urgent\", ready: false } 3",
    "▸ ◐ dun-7.2 shore up the mast",
    "▸ ○ hbr-1.3 clear the silted channel",
    "▸ ○ hbr-1.4 fix the swing bridge",
    "! ToCheck(1)",
];

/// Each view lists the beads its conditions select from both projects, the
/// most urgent first, and none that a condition excludes or that is
/// finished. The key steps through the views in the order the config names
/// them, and past the last puts the forest back.
#[test]
fn the_key_opens_each_view_in_turn_and_then_the_forest() {
    let forest_first = sketch(&flatten(signing()));
    let mut forest = flatten(signing());

    assert!(forest.apply(Action::NextView));
    assert_eq!(sketch(&forest), THE_SIGNING_VIEW);

    assert!(forest.apply(Action::NextView));
    assert_eq!(sketch(&forest), THE_URGENT_VIEW);

    assert!(forest.apply(Action::NextView));
    assert_eq!(sketch(&forest), forest_first);
}

/// A collection landing keeps the reader in the view they had open.
#[test]
fn a_refresh_keeps_the_view() {
    let mut forest = flatten(signing());
    forest.apply(Action::NextView);
    forest.apply(Action::NextView);

    forest.refresh(signing());

    assert_eq!(sketch(&forest), THE_URGENT_VIEW);
}

/// A config read again without the view the reader had open puts the
/// forest back, rather than a list of nothing under a name no config holds.
#[test]
fn a_view_the_config_no_longer_names_gives_way_to_the_forest() {
    let forest_first = sketch(&flatten(signing()));
    let mut forest = flatten(signing());
    forest.apply(Action::NextView);

    let mut reread = signing();
    reread.views.retain(|view| view != "signing");
    forest.refresh(reread);

    assert_eq!(sketch(&forest), forest_first);
}

/// With only ready beads listed, the list that view gives way to is the
/// ready list, as the key past the last view would give.
#[test]
fn a_filtered_view_the_config_no_longer_names_gives_way_to_the_ready_list() {
    let mut forest = flatten(signing());
    forest.apply(Action::ToggleReadyList);
    let ready = sketch(&forest);
    forest.apply(Action::NextView);

    let mut reread = signing();
    reread.views.retain(|view| view != "signing");
    forest.refresh(reread);

    assert_eq!(sketch(&forest), ready);
}

/// With no view configured there is nothing to open, so the key leaves the
/// forest as it is.
#[test]
fn with_no_views_the_key_does_nothing() {
    let mut none = signing();
    none.views.clear();
    let mut forest = flatten(none);
    let before = sketch(&forest);

    assert!(!forest.apply(Action::NextView));
    assert_eq!(sketch(&forest), before);
}

/// Harbour's beads waiting to be signed. `hbr-1` and `hbr-5` are free to
/// sign, since `hbr-5`'s blocker has closed, and `hbr-7` is being signed.
/// `hbr-2` waits on harbour's own open `hbr-3`, and `hbr-4`, which `bd`
/// calls ready, waits on dunwich's open `dun-7`.
const HARBOUR_TO_SIGN: &str = r#"[
  {"id":"hbr-1","title":"sign the berth lease","status":"open","priority":2,
   "labels":["signing"]},
  {"id":"hbr-2","title":"sign the pilotage terms","status":"open","priority":2,
   "labels":["signing"],
   "dependencies":[{"depends_on_id":"hbr-3","type":"blocks"}]},
  {"id":"hbr-3","title":"draft the pilotage terms","status":"open","priority":2},
  {"id":"hbr-4","title":"sign the dish lease","status":"open","priority":2,
   "labels":["signing"],
   "dependencies":[{"depends_on_id":"dun-7","type":"blocks"}]},
  {"id":"hbr-5","title":"sign the mooring terms","status":"open","priority":2,
   "labels":["signing"],
   "dependencies":[{"depends_on_id":"hbr-6","type":"blocks"}]},
  {"id":"hbr-6","title":"draft the mooring terms","status":"closed","priority":2},
  {"id":"hbr-7","title":"sign the crane hire","status":"in_progress","priority":2,
   "labels":["signing"]}
]"#;

fn to_sign() -> Snapshot {
    harbour_and_dunwich_ready(
        HARBOUR_TO_SIGN,
        r#"[{"id":"dun-7","title":"lift the ground station","status":"open"}]"#,
        &[
            ("dunwich", "dun-7"),
            ("harbour", "hbr-1"),
            ("harbour", "hbr-2"),
            ("harbour", "hbr-4"),
            ("harbour", "hbr-5"),
            ("harbour", "hbr-7"),
        ],
        &[],
        &["dun-7", "hbr-1", "hbr-3", "hbr-4", "hbr-5"],
    )
}

const THE_WHOLE_SIGNING_VIEW: &[&str] = &[
    "dunwich",
    "harbour",
    "# View { name: \"signing\", ready: false } 5",
    "  ○ hbr-1 sign the berth lease",
    "  ○ hbr-2 sign the pilotage terms",
    "  ○ hbr-4 sign the dish lease",
    "  ○ hbr-5 sign the mooring terms",
    "  ◐ hbr-7 sign the crane hire",
    "! ToCheck(1)",
];

const THE_READY_SIGNING_VIEW: &[&str] = &[
    "dunwich",
    "harbour",
    "# View { name: \"signing\", ready: true } 3",
    "  ○ hbr-1 sign the berth lease",
    "  ○ hbr-5 sign the mooring terms",
    "  ◐ hbr-7 sign the crane hire",
    "! ToCheck(1)",
];

/// `r` in a view keeps to the view and lists only what the ready list would:
/// a bead whose blockers have all closed, and one being worked. A bead
/// waiting on an open blocker stays out, in its own project or in another.
/// `r` again shows the whole view.
#[test]
fn r_in_a_view_lists_only_the_beads_in_it_that_are_ready() {
    let mut forest = flatten(to_sign());
    forest.apply(Action::NextView);
    assert_eq!(sketch(&forest), THE_WHOLE_SIGNING_VIEW);

    assert!(forest.apply(Action::ToggleReadyList));
    assert_eq!(sketch(&forest), THE_READY_SIGNING_VIEW);

    assert!(forest.apply(Action::ToggleReadyList));
    assert_eq!(sketch(&forest), THE_WHOLE_SIGNING_VIEW);
}

/// The ready filter carries across lists: the key opens the first view
/// filtered from the ready list, keeps the filter as it steps through the
/// views, and past the last puts the ready list back rather than the forest.
#[test]
fn the_key_keeps_the_ready_filter_and_ends_on_the_ready_list() {
    let mut forest = flatten(to_sign());
    assert!(forest.apply(Action::ToggleReadyList));
    let ready = sketch(&forest);

    assert!(forest.apply(Action::NextView));
    assert_eq!(sketch(&forest), THE_READY_SIGNING_VIEW);

    assert!(forest.apply(Action::NextView));
    assert_eq!(
        sketch(&forest)[2],
        "# View { name: \"urgent\", ready: true } 0"
    );

    assert!(forest.apply(Action::NextView));
    assert_eq!(sketch(&forest), ready);
    assert!(forest.is_ready_list());
}
