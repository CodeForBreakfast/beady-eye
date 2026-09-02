//! What each tracker is asked for, and what survives between asks.
//!
//! One module for reading a single project — the calls, and what each failure
//! along them means — and one for the standing set of those reads, which is
//! what lets a collection name one project and still draw every other. It
//! sits between `collect/`, which runs the programs, and `model/`, which
//! joins what they said; it names neither `view/` nor `tui/`.

mod collection;
mod tracker;

pub use collection::{run, Awaited, Collection, Wanted};

/// The fake trackers and panes both halves read in their tests.
///
/// Every tracker here answers in beads and sets, which is what the seam
/// carries, so a test on either side of it is reading the same tracker rather
/// than its own idea of one — and none of them knows how a real one is asked.
#[cfg(test)]
mod fixtures {
    use chrono::{DateTime, Utc};

    use crate::collect::bd::parse_beads;
    use crate::collect::run::testing::FakeRunner;
    use crate::collect::run::{FailureKind, RunFailure};
    use crate::collect::tracker::testing::{Fake, Fakes};
    use crate::config::Config;
    use crate::model::snapshot::{Node, Tree};
    use crate::model::types::Bead;

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
    pub(super) const PANES: &str = r#"{"result":{"agents":[
      {"pane_id":"w:p1","cwd":"/srv/work/orbital","agent_status":"working","title":"the dish"},
      {"pane_id":"w:p9","cwd":"/srv/work/orbital","agent_status":"idle"}
    ]}}"#;

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

    /// The herdr session answering with `agents`.
    pub(super) fn panes_of(agents: &str) -> FakeRunner {
        FakeRunner::default().with("herdr agent list", agents)
    }

    /// The herdr session as a healthy run finds it.
    pub(super) fn panes() -> FakeRunner {
        panes_of(PANES)
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
        }
    }

    pub(super) fn node<'a>(tree: &'a Tree, id: &str) -> &'a Node {
        tree.beads
            .iter()
            .find(|n| n.id == id)
            .unwrap_or_else(|| panic!("{id} is among the beads"))
    }
}
