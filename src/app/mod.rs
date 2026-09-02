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

/// The fake tracker both halves read in their tests.
///
/// Every canned answer here is one bd call spelled as the runner spells it, so
/// a test on either side of the seam is reading the same tracker rather than
/// its own idea of one.
#[cfg(test)]
mod fixtures {
    use chrono::{DateTime, Utc};

    use crate::collect::run::testing::FakeRunner;
    use crate::collect::run::{FailureKind, RunFailure};
    use crate::config::Config;
    use crate::model::snapshot::{Node, Tree};

    pub(super) const ORBITAL: &str = "/srv/work/orbital";
    pub(super) const FERRY: &str = "/srv/work/ferry";

    /// One project's tracker as bd answers for the root: an epic over two
    /// tasks, one of them naming the pane working it. Every row carries its
    /// own `parent` as well as the edge, because bd writes both.
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

    /// A bd call as the runner spells it: the tracker named outright, and
    /// writes refused. Two projects now differ in their argv as well as their
    /// directory, so each is staged for the tracker it reads.
    pub(super) fn spelled_in(tracker: &str, subcommand: &str) -> String {
        format!("bd -C {tracker} --readonly {subcommand}")
    }

    /// The same, for the single project most of these tests read.
    pub(super) fn spelled(subcommand: &str) -> String {
        spelled_in(ORBITAL, subcommand)
    }

    /// The direnv call that reproduces entering a project's directory.
    pub(super) fn entering(tracker: &str) -> String {
        format!("direnv exec {tracker} env -0")
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

[roots]
metadata_keys = ["working_topic"]
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
credential_command = "secret orbital"

[[projects]]
name = "ferry"
path = "{FERRY}"
credential_command = "secret ferry"
"#
        ))
        .expect("the config parses")
    }

    /// The one question a refresh asks before it decides whether to ask the
    /// other seven, spelled as bd takes it.
    pub(super) const PROBE_CALL: &str = "sql --json SELECT dolt_hashof_db() AS h";

    /// One answer to `PROBE_CALL`, as a tracker that has not moved keeps
    /// giving. A test that needs a tracker to have moved stages `MOVED`
    /// against a second runner and collects again.
    pub(super) const UNMOVED: &str = r#"[{"h":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]"#;
    pub(super) const MOVED: &str = r#"[{"h":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}]"#;

    /// The one call a project's whole forest is drawn from, spelled as bd
    /// takes it.
    pub(super) const TRACKER_CALL: &str = "list --all --limit 0 --json";

    /// The same question asked of bd's ephemeral table, which `bd list` does
    /// not read.
    pub(super) const WISP_CALL: &str = "query ephemeral=true --all --limit 0 --json";

    /// Every call a healthy single-project run makes.
    pub(super) fn orbital() -> FakeRunner {
        FakeRunner::default()
            .with("herdr agent list", PANES)
            .with(&entering(ORBITAL), "")
            .with(&spelled(PROBE_CALL), UNMOVED)
            .with(
                &spelled("ready --limit 0 --json"),
                r#"[{"id":"orb-7.2","title":"lay the feeder cable","status":"open"}]"#,
            )
            .with(
                &spelled("blocked --json"),
                r#"[{"id":"orb-7.1","blocked_by":["orb-9"]}]"#,
            )
            .with(&spelled(TRACKER_CALL), ORBITAL_TREE)
            .with(&spelled(WISP_CALL), "[]")
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

    pub(super) fn colliding_trackers(panes: &str) -> FakeRunner {
        let mut runner = FakeRunner::default()
            .with("herdr agent list", panes)
            .with("sh -c secret orbital", "orbital-password")
            .with("sh -c secret ferry", "ferry-password");
        for tracker in [ORBITAL, FERRY] {
            runner = runner
                .with(&spelled_in(tracker, PROBE_CALL), UNMOVED)
                .with(&spelled_in(tracker, "ready --limit 0 --json"), "[]")
                .with(&spelled_in(tracker, "blocked --json"), "[]")
                .with(&spelled_in(tracker, TRACKER_CALL), COLLIDING_TREE)
                .with(&spelled_in(tracker, WISP_CALL), "[]");
        }
        runner
    }
}
