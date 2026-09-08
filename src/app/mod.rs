//! What each tracker is asked for, and what survives between asks.
//!
//! One module for reading a single project — the calls, and what each failure
//! along them means — and one for the standing set of those reads, which is
//! what lets a collection name one project and still draw every other. It
//! sits between `collect/`, which runs the programs, and `model/`, which
//! joins what they said; it names neither `view/` nor `tui/`.

mod collection;
mod tracker;

pub use collection::{run, Asked, Awaited, Collection, Wanted};

/// The fake trackers and panes both halves read in their tests.
///
/// Every tracker here answers in beads and sets, and every provider in panes,
/// which is what each seam carries — so a test on either side of one is
/// reading the same thing rather than its own idea of it, and none of them
/// knows how a real one is asked.
#[cfg(test)]
mod fixtures {
    use chrono::{DateTime, Utc};

    use crate::collect::agents::testing::{pane, titled, Fake as Provider};
    use crate::collect::bd::parse_beads;
    use crate::collect::run::{FailureKind, RunFailure};
    use crate::collect::tracker::testing::{Fake, Fakes};
    use crate::config::Config;
    use crate::model::snapshot::{Node, Tree};
    use crate::model::types::{Bead, PaneStatus};

    pub(super) const ORBITAL: &str = "/srv/work/orbital";
    pub(super) const FERRY: &str = "/srv/work/ferry";

    /// One project's tracker: an epic over two tasks, one of them naming the
    /// pane working it. Every row carries its own `parent` as well as the
    /// edge, because a tracker writes both.
    pub(super) const ORBITAL_TREE: &str = r#"[
      {"id":"orb-7","title":"lift the ground station","status":"in_progress",
       "priority":1,"issue_type":"epic"},
      {"id":"orb-7.1","title":"re-point the dish","status":"in_progress","parent":"orb-7",
       "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
       "priority":2,"issue_type":"task",
       "metadata":{"agent_pane":"w:p1"}},
      {"id":"orb-7.2","title":"lay the feeder cable","status":"open","parent":"orb-7",
       "dependencies":[{"depends_on_id":"orb-7","type":"parent-child"}],
       "priority":2,"issue_type":"task"}
    ]"#;

    /// Two trackers that chose the same id prefix, which no one coordinates.
    pub(super) const COLLIDING_TREE: &str = r#"[
      {"id":"x-1","title":"the shared prefix","status":"in_progress",
       "priority":1,"issue_type":"epic"},
      {"id":"x-1.1","title":"the colliding id","status":"in_progress","parent":"x-1",
       "dependencies":[{"depends_on_id":"x-1","type":"parent-child"}],
       "priority":2,"issue_type":"task"}
    ]"#;

    /// `w:p1` is on a bead; `w:p9` is a session on none.
    pub(super) fn panes() -> Provider {
        Provider::holding(vec![
            titled(pane("w:p1", ORBITAL, PaneStatus::Working), "the dish"),
            pane("w:p9", ORBITAL, PaneStatus::Idle),
        ])
    }

    /// A provider that is there and holds no pane at all.
    pub(super) fn no_panes() -> Provider {
        Provider::holding(Vec::new())
    }

    /// Rows as a test writes them, read into the beads a tracker answers with.
    pub(super) fn beads(rows: &str) -> Vec<Bead> {
        parse_beads(rows).expect("the rows parse")
    }

    pub(super) fn now() -> DateTime<Utc> {
        "2026-08-30T12:00:00Z".parse().expect("the instant parses")
    }

    pub(super) fn one_project() -> Config {
        Config::from_toml(&format!(
            r#"
[[projects]]
name = "orbital"
path = "{ORBITAL}"
"#
        ))
        .expect("the config parses")
    }

    /// The one project, as the reader has just rewritten its entry: the same
    /// tracker at the same path, reached with a credential command that was
    /// not there before.
    pub(super) fn one_project_reached_with_a_credential() -> Config {
        Config::from_toml(&format!(
            r#"
[[projects]]
name = "orbital"
path = "{ORBITAL}"
credential_command = "pass show orbital"
"#
        ))
        .expect("the config parses")
    }

    /// The one project, as the reader has just written a tree of their own
    /// into `[roots.explicit]`: the same tracker, reached the same way, with
    /// one more root asked of it.
    ///
    /// The root is a bead an edge already places, so what naming it changes
    /// is the forest and nothing about what the tracker is asked — which is
    /// the case a fingerprint cannot see.
    pub(super) fn one_project_with_a_root_named() -> Config {
        Config::from_toml(&format!(
            r#"
[[projects]]
name = "orbital"
path = "{ORBITAL}"

[roots.explicit]
orbital = ["orb-7.1"]
"#
        ))
        .expect("the config parses")
    }

    pub(super) fn two_projects() -> Config {
        Config::from_toml(&format!(
            r#"
[[projects]]
name = "orbital"
path = "{ORBITAL}"

[[projects]]
name = "ferry"
path = "{FERRY}"
"#
        ))
        .expect("the config parses")
    }

    /// Orbital's tracker holding `rows` in place of its usual tree, with the
    /// same task ready and the same task blocked from outside it.
    pub(super) fn orbital_holding(rows: &str) -> Fake {
        Fake::holding(beads(rows))
            .ready(["orb-7.2"])
            .blocked("orb-7.1", &["orb-9"])
    }

    /// Orbital's tracker as a healthy single-project run finds it.
    pub(super) fn orbital_tracker() -> Fake {
        orbital_holding(ORBITAL_TREE)
    }

    /// The one project's trackers, with orbital's staged as `tracker`.
    pub(super) fn orbital_with(tracker: Fake) -> Fakes {
        Fakes::default().with("orbital", tracker)
    }

    /// The one project's trackers as a healthy run finds them.
    pub(super) fn orbital() -> Fakes {
        orbital_with(orbital_tracker())
    }

    /// One of the two trackers that chose the same prefix.
    pub(super) fn colliding_tracker() -> Fake {
        Fake::holding(beads(COLLIDING_TREE))
    }

    /// Both projects' trackers, each holding the colliding tree.
    pub(super) fn colliding_trackers() -> Fakes {
        Fakes::default()
            .with("orbital", colliding_tracker())
            .with("ferry", colliding_tracker())
    }

    pub(super) fn failing(kind: FailureKind) -> RunFailure {
        RunFailure {
            kind,
            program: "bd".to_string(),
            detail: "bd could not read the tracker".to_string(),
            unreadable: None,
        }
    }

    pub(super) fn node<'a>(tree: &'a Tree, id: &str) -> &'a Node {
        tree.beads
            .iter()
            .find(|n| n.id == id)
            .unwrap_or_else(|| panic!("{id} is among the beads"))
    }
}
