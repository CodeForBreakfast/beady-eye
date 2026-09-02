//! bd's command line as the way to a project's tracker.
//!
//! The one module that spells `bd -C <path> --readonly …`. Each question the
//! seam asks is one bd invocation or two, answered in bd's own JSON and parsed
//! here and nowhere else. The roots to draw the rows under are read off the
//! rows themselves, in `app::tracker`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use anyhow::Context;
use serde::Deserialize;

use chrono::{DateTime, Utc};
use serde::Deserializer;

use crate::collect::environment;
use crate::collect::run::{Env, RunFailure, Runner};
use crate::collect::tracker::{Tracker, Trackers};
use crate::config::Project;
use crate::model::types::{Bead, Dependency, Edge, Status};

/// Parse a flat array of bd rows, however the answer that carried them was
/// asked for. `bd list`, `bd ready` and `bd query` all write the same row.
pub fn parse_beads(s: &str) -> anyhow::Result<Vec<Bead>> {
    let rows: Vec<Row> =
        serde_json::from_str(s).context("bd --json returned a shape we do not understand")?;
    Ok(rows.into_iter().map(Bead::from).collect())
}

/// One row of a bd listing, in the shape bd writes it, holding only the
/// fields `bdi` reads.
///
/// Unknown fields are ignored; a present field of the wrong type is an error.
/// Every field bd omits when empty is optional here, because bd omits it
/// rather than writing null.
///
/// `depth` is deliberately absent: bd flattens it under `--max-depth`, so the
/// tree recomputes nesting from the dependency edges instead.
#[derive(Deserialize)]
struct Row {
    id: String,
    title: String,
    status: Status,
    #[serde(default)]
    priority: u8,
    #[serde(default)]
    issue_type: String,
    /// bd writes the top of a chain as an empty parent, or leaves the field
    /// out; either reads as none.
    #[serde(default, deserialize_with = "empty_is_none")]
    parent: Option<String>,
    #[serde(default, deserialize_with = "none_is_empty")]
    dependencies: Vec<RowDependency>,
    #[serde(default, deserialize_with = "text_of_each_value")]
    metadata: BTreeMap<String, String>,
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    assignee: Option<String>,
    /// As `bd show` prints it. bd leaves the field out of a row that has
    /// none.
    #[serde(default)]
    description: Option<String>,
    /// Everything `bd note` has added, as one text. Left out the same way.
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    closed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    defer_until: Option<DateTime<Utc>>,
}

/// One dependency as a row carries it.
#[derive(Deserialize)]
struct RowDependency {
    depends_on_id: String,
    #[serde(rename = "type")]
    edge: Edge,
}

impl From<Row> for Bead {
    fn from(row: Row) -> Self {
        Bead {
            id: row.id,
            title: row.title,
            status: row.status,
            priority: row.priority,
            issue_type: row.issue_type,
            parent: row.parent,
            dependencies: row
                .dependencies
                .into_iter()
                .map(|dependency| Dependency {
                    on: dependency.depends_on_id,
                    edge: dependency.edge,
                })
                .collect(),
            metadata: row.metadata,
            owner: row.owner,
            assignee: row.assignee,
            description: row.description,
            notes: row.notes,
            updated_at: row.updated_at,
            started_at: row.started_at,
            closed_at: row.closed_at,
            defer_until: row.defer_until,
        }
    }
}

/// bd omits a field it has nothing for, and `#[serde(default)]` covers that.
/// It does not extend to an explicit null. A tracker is read whole, so a row
/// bd wrote the other way costs not one bead's edges but every bead in that
/// project.
fn none_is_empty<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<RowDependency>, D::Error> {
    Ok(Option::<Vec<RowDependency>>::deserialize(d)?.unwrap_or_default())
}

/// bd spells an absent parent three ways — `""`, `null`, or no field — and
/// they all mean the top of a chain.
fn empty_is_none<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(Option::<String>::deserialize(d)?.filter(|parent| !parent.is_empty()))
}

/// A bead's metadata is whatever JSON was written into it, and bdi draws it
/// as text. So each value is read as the text it prints as, and a value that
/// is not a string costs nothing.
///
/// A tracker is read whole, so the alternative is not a bead without its
/// badge — it is every bead in that project, gone.
fn text_of_each_value<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<String, String>, D::Error> {
    let raw = BTreeMap::<String, serde_json::Value>::deserialize(d)?;
    Ok(raw
        .into_iter()
        .map(|(key, value)| match value {
            serde_json::Value::String(text) => (key, text),
            written => (key, written.to_string()),
        })
        .collect())
}

/// One row of `bd blocked --json`, which carries a blocker set no dep-tree
/// row has.
#[derive(Deserialize)]
struct BlockedRow {
    id: String,
    #[serde(default)]
    blocked_by: Vec<String>,
}

/// bd's CLI, reaching every project's tracker through one runner.
pub struct Cli<'r> {
    runner: &'r dyn Runner,
    /// The credential the shell `bdi` was launched from holds, which a
    /// project configuring none reaches its tracker on. Read once: the shell
    /// `bdi` was launched from does not change while it runs.
    ambient: Option<String>,
}

impl<'r> Cli<'r> {
    pub fn new(runner: &'r dyn Runner) -> Self {
        Self {
            runner,
            ambient: environment::ambient_credential(),
        }
    }
}

impl Trackers for Cli<'_> {
    fn of(&self, project: &Project) -> Result<Box<dyn Tracker + '_>, RunFailure> {
        let env = environment::tracker_env(self.runner, project, self.ambient.as_deref())?;
        Ok(Box::new(Reader {
            runner: self.runner,
            path: project.path.clone(),
            env,
        }))
    }
}

/// One project's tracker as bd reads it: in the project's directory, with the
/// environment its config asked for.
struct Reader<'r> {
    runner: &'r dyn Runner,
    path: PathBuf,
    env: Env,
}

impl Reader<'_> {
    /// One answer out of the tracker.
    ///
    /// `-C` names the tracker outright, and it outranks `BEADS_DIR` in both
    /// directions: a wrong variable still resolves the project, and a wrong
    /// directory is refused rather than resolved to something plausible. That
    /// is what makes entering the directory safe, because direnv can quietly
    /// do nothing. Clearing the inherited variables stays as well; together
    /// they mean a misconfiguration fails loudly.
    ///
    /// `--readonly` has bd refuse the writes `bdi` never makes, so for every
    /// subcommand but one the rule is enforced by bd rather than resting on
    /// `bdi` being well behaved. `sql` is the exception: it is a general
    /// executor, bd's own help for it warns that direct database access
    /// bypasses the storage layer, and `--readonly` does not veto it —
    /// measured against this project's own tracker on 2026-09-01. What holds
    /// there instead is `WORKING_ROOT`, a constant nothing composes, reached
    /// from one method that takes no argument.
    fn asked(&self, subcommand: &[&str]) -> Result<String, RunFailure> {
        let named = self.path.to_string_lossy();
        let mut argv = vec!["-C", named.as_ref(), "--readonly"];
        argv.extend_from_slice(subcommand);
        self.runner.run("bd", &argv, Some(&self.path), &self.env)
    }

    /// The tracker's Dolt working root: one hash over everything the
    /// database holds, committed or not.
    ///
    /// Not the committed head, because `bdi` reads wisps and the head cannot
    /// see them. `wisps` and `wisp_%` are in `dolt_ignore`, so they live in
    /// the working set and never reach `dolt_log` — measured against this
    /// project's own tracker on 2026-09-01, one `bd create --ephemeral` left
    /// `hashof('HEAD')` identical either side of it and moved this. A caller
    /// gating on the head would leave a wisp-only change off the screen until
    /// some unrelated write moved it.
    ///
    /// A read does not move it: three of these with a whole cascade between
    /// them answered the same hash, measured the same day. That is what makes
    /// it worth asking, because a hash that moved on being read would report
    /// a change every time and cost 0.2s to learn nothing.
    fn working_root(&self) -> Result<String, RunFailure> {
        let out = self.asked(&["sql", "--json", WORKING_ROOT])?;
        let rows: Vec<HashRow> =
            serde_json::from_str(&out).map_err(|e| RunFailure::parse("bd", e))?;
        rows.into_iter()
            .next()
            .map(|row| row.h)
            .ok_or_else(|| RunFailure::parse("bd", "bd sql answered no row"))
    }

    /// A tracker's wisps, closed ones included.
    ///
    /// A second call, because bd keeps its ephemeral beads in a table `bd
    /// list` does not read: measured against this project's own tracker on
    /// 2026-08-31, `bd list --all` answered 120 rows both before and after two
    /// wisps were written, and `bd list --wisp-type heartbeat` answered `[]`
    /// against a heartbeat wisp that existed. `bd query` is the one call that
    /// reads them, and it writes the same row `bd list` does.
    fn wisps(&self) -> Result<String, RunFailure> {
        self.asked(&["query", EPHEMERAL, "--all", "--limit", "0", "--json"])
    }
}

impl Tracker for Reader<'_> {
    /// bd over Dolt always has a probe, so this is never `None` here.
    fn fingerprint(&self) -> Option<Result<String, RunFailure>> {
        Some(self.working_root())
    }

    /// One call per project rather than one per root, because a tree is
    /// drawn from dependency edges and `bd dep tree` cannot carry them: it
    /// walks dependents and dedupes, so what comes back is a spanning tree —
    /// each bead with the one edge the walk first reached it by, and every
    /// other edge into it missing. Measured against this project's own
    /// tracker on 2026-08-30, that walk carried 93 of the 176 edges among the
    /// beads it returned. It is also the reason `blocked` is asked for
    /// separately.
    ///
    /// `--all` is load-bearing: without it bd answers about open beads only,
    /// and a smaller correct-looking answer about a different population is
    /// the kind of wrong that reads as right.
    fn all(&self) -> Result<Vec<Bead>, RunFailure> {
        let out = self.asked(&["list", "--all", "--limit", "0", "--json"])?;
        let mut beads = rows(&out)?;
        beads.extend(rows(&self.wisps()?)?);
        Ok(beads)
    }

    /// bd computes readiness itself and treats it as a state of its own, so
    /// it is asked for rather than inferred from status.
    fn ready(&self) -> Result<BTreeSet<String>, RunFailure> {
        let out = self.asked(&["ready", "--limit", "0", "--json"])?;
        Ok(rows(&out)?.into_iter().map(|bead| bead.id).collect())
    }

    /// A dep-tree row carries its tree parent, not its blocker set: a bead
    /// blocked by two others appears once, under one of them, with the second
    /// nowhere in the output. `bd blocked` takes no limit of its own.
    fn blocked(&self) -> Result<BTreeMap<String, Vec<String>>, RunFailure> {
        let out = self.asked(&["blocked", "--json"])?;
        let blocked: Vec<BlockedRow> =
            serde_json::from_str(&out).map_err(|e| RunFailure::parse("bd", e))?;
        Ok(blocked
            .into_iter()
            .map(|row| (row.id, row.blocked_by))
            .collect())
    }
}

/// The whole of the SQL `bdi` writes.
///
/// `dolt_hashof_db()` answers for the database bd is already connected to, as
/// one row and one column. `SHOW VARIABLES LIKE '%_working'` reaches the same
/// hash and is worse three ways: it answers for every attached database at
/// once, `skip_networking` matches that pattern as well, and the `@@` form
/// cannot be quoted through `bd sql` because a database name may hold a
/// hyphen.
const WORKING_ROOT: &str = "SELECT dolt_hashof_db() AS h";

/// The one row `WORKING_ROOT` answers with.
#[derive(Deserialize)]
struct HashRow {
    h: String,
}

/// The `bd query` expression that selects wisps and nothing else.
const EPHEMERAL: &str = "ephemeral=true";

/// `bd list`, `bd ready` and `bd query` all answer with the same rows.
fn rows(out: &str) -> Result<Vec<Bead>, RunFailure> {
    parse_beads(out).map_err(|e| RunFailure::parse("bd", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::types::{Dependency, Edge, Status};

    const FIXTURE: &str = include_str!("../../tests/fixtures/bd_list.json");

    fn fixture() -> Vec<Bead> {
        parse_beads(FIXTURE).expect("the captured rows parse")
    }

    fn row(id: &str) -> Bead {
        fixture()
            .into_iter()
            .find(|b| b.id == id)
            .unwrap_or_else(|| panic!("{id} is in the fixture"))
    }

    #[test]
    fn parses_every_row() {
        assert_eq!(fixture().len(), 7);
    }

    const JOINED: &str = include_str!("../../tests/fixtures/joined_bd_list.json");

    /// The bead as `bd show` gives it is in the rows `bd list` already
    /// writes, so showing one costs no further call. Asserted on a capture
    /// rather than a row typed here, because a key a capture carries is a
    /// measurement of what bd writes.
    #[test]
    fn a_captured_row_carries_the_description_the_notes_and_the_owner() {
        let rows = parse_beads(JOINED).expect("the captured rows parse");
        let bead = rows
            .iter()
            .find(|b| b.id == "bdi-7ao")
            .expect("bdi-7ao is in the capture");

        assert!(
            bead.description
                .as_deref()
                .is_some_and(|said| said.starts_with("`bdi` joins a beads tracker")),
            "{:?}",
            bead.description
        );
        assert!(
            bead.notes
                .as_deref()
                .is_some_and(|said| said.starts_with("Ready and unstaffed at 17:37 BST")),
            "{:?}",
            bead.notes
        );
        assert_eq!(
            bead.owner.as_deref(),
            Some("80714+GraemeF@users.noreply.github.com")
        );
    }

    /// bd leaves both out of a row that has neither, and a row it writes
    /// that way is a bead with nothing to say, not one that will not parse.
    #[test]
    fn a_row_without_a_description_or_notes_parses_with_neither() {
        let bead = row("bdi-2bb.4");

        assert_eq!(bead.description, None);
        assert_eq!(bead.notes, None);
    }

    /// A captured row names every bead it depends on, and the kinds differ
    /// within the one row: the tree cannot be built from the parent edges
    /// alone.
    #[test]
    fn a_captured_row_carries_every_edge_out_of_it() {
        assert_eq!(
            row("bdi-2bb.4").dependencies,
            vec![
                Dependency {
                    on: "bdi-2bb".to_string(),
                    edge: Edge::ParentChild,
                },
                Dependency {
                    on: "bdi-2bb.3".to_string(),
                    edge: Edge::Blocks,
                },
                Dependency {
                    on: "bdi-2bb.9".to_string(),
                    edge: Edge::Blocks,
                },
            ]
        );
    }

    #[test]
    fn statuses_map_onto_the_enum() {
        assert_eq!(row("bdi-r5l").status, Status::InProgress);
        assert_eq!(row("bdi-2bb").status, Status::Open);
        assert_eq!(row("bdi-2bb.9").status, Status::Closed);
    }

    #[test]
    fn every_status_spelling_bd_writes_is_recognised() {
        let spellings = ["open", "in_progress", "blocked", "closed", "deferred"];
        let expected = [
            Status::Open,
            Status::InProgress,
            Status::Blocked,
            Status::Closed,
            Status::Deferred,
        ];

        for (spelling, want) in spellings.iter().zip(expected) {
            let json = format!(r#"[{{"id":"x","title":"t","status":"{spelling}"}}]"#);
            assert_eq!(parse_beads(&json).unwrap()[0].status, want);
        }
    }

    #[test]
    fn a_status_a_later_bd_invents_is_kept_rather_than_rejected() {
        let json = r#"[{"id":"x","title":"t","status":"marinating"}]"#;
        let beads = parse_beads(json).expect("an unknown status still parses");
        assert_eq!(beads[0].status, Status::Other("marinating".to_string()));
    }

    #[test]
    fn an_edge_a_later_bd_invents_is_kept_rather_than_rejected() {
        let json = r#"[{"id":"x","title":"t","status":"open","dependencies":[
          {"depends_on_id":"y","type":"discovered-by"}]}]"#;
        let beads = parse_beads(json).expect("an unknown edge still parses");
        assert_eq!(
            beads[0].dependencies,
            vec![Dependency {
                on: "y".to_string(),
                edge: Edge::Other("discovered-by".to_string()),
            }]
        );
    }

    #[test]
    fn metadata_is_carried_inline_and_absent_metadata_is_an_empty_map() {
        let carrying = row("bdi-r5l");
        assert_eq!(
            carrying.metadata.get("agent_pane").map(String::as_str),
            Some("wCW:p2M")
        );

        assert!(row("bdi-2bb.3").metadata.is_empty());
    }

    /// A tracker's metadata is arbitrary JSON, and bdi draws it as text. A
    /// value that is not a string is read as the text it prints as, because
    /// a tracker is read whole and refusing one value loses every bead in it.
    ///
    /// Measured on summit-works, 2026-08-31: two beads of 1886 carried
    /// `blocks_backstop_removal: true`, and the whole project failed to read.
    #[test]
    fn a_metadata_value_that_is_not_a_string_is_read_as_its_text() {
        let rows = r#"[
            {"id":"a","title":"t","status":"open",
             "metadata":{"blocks_backstop_removal":true}},
            {"id":"b","title":"t","status":"open",
             "metadata":{"attempts":3,"working_topic":"x"}}
        ]"#;

        let beads = parse_beads(rows).expect("one non-string value does not lose a tracker");

        assert_eq!(
            beads[0].metadata.get("blocks_backstop_removal"),
            Some(&"true".to_string())
        );
        assert_eq!(beads[1].metadata.get("attempts"), Some(&"3".to_string()));
        assert_eq!(
            beads[1].metadata.get("working_topic"),
            Some(&"x".to_string()),
            "a string keeps its own text, without the quotes JSON writes it in"
        );
    }

    #[test]
    fn the_timestamps_the_age_rules_need_follow_the_row() {
        let closed = row("bdi-2bb.9");
        assert!(closed.started_at.is_some());
        assert!(closed.closed_at.is_some());

        let open = row("bdi-2bb");
        assert_eq!(open.started_at, None);
        assert_eq!(open.closed_at, None);
        assert!(open.updated_at.is_some());
    }

    #[test]
    fn an_unclaimed_bead_has_no_assignee() {
        assert_eq!(row("bdi-2bb.9").assignee.as_deref(), Some("Graeme Foster"));
        assert_eq!(row("bdi-2bb").assignee, None);
    }

    #[test]
    fn issue_type_distinguishes_the_root_epic_from_its_tasks() {
        assert_eq!(row("bdi-2bb").issue_type, "epic");
        assert_eq!(row("bdi-2bb.9").issue_type, "task");
    }

    #[test]
    fn a_wrongly_typed_field_is_an_error_not_a_default() {
        let bad = r#"[{"id":"x","title":"t","status":"open","priority":"high"}]"#;
        assert!(parse_beads(bad).is_err());
    }

    #[test]
    fn work_in_flight_ranks_ahead_of_work_that_is_finished() {
        let mut statuses = vec![
            Status::Closed,
            Status::Open,
            Status::Other("marinating".to_string()),
            Status::InProgress,
            Status::Deferred,
            Status::Blocked,
        ];
        statuses.sort_by_key(Status::rank);

        assert_eq!(
            statuses,
            vec![
                Status::InProgress,
                Status::Blocked,
                Status::Open,
                Status::Deferred,
                Status::Closed,
                Status::Other("marinating".to_string()),
            ]
        );
        assert!(Status::Closed.is_closed());
        assert!(!Status::Open.is_closed());
    }

    use crate::collect::environment::CREDENTIAL_VAR;
    use crate::collect::run::testing::FakeRunner;
    use crate::collect::run::FailureKind;
    use crate::collect::tracker::Trackers;
    use crate::config::{Environment, Project};
    use std::path::PathBuf;

    fn project_dir() -> PathBuf {
        PathBuf::from("/tmp/proj")
    }

    /// A bd call as the runner spells it: the tracker named outright, and
    /// writes refused. Every call carries both, so the tests name the
    /// subcommand and this holds the invocation round it.
    fn spelled(subcommand: &str) -> String {
        format!("bd -C {} --readonly {subcommand}", project_dir().display())
    }

    fn credentialled() -> Env {
        Env::from([(CREDENTIAL_VAR.to_string(), "hunter2".to_string())])
    }

    /// The tracker under `project_dir()`, opened on its credential.
    fn opened(runner: &FakeRunner) -> Reader<'_> {
        Reader {
            runner,
            path: project_dir(),
            env: credentialled(),
        }
    }

    /// A project entry as the config takes it by default: a path and nothing
    /// else, read in `bdi`'s own environment.
    fn ambient_project() -> Project {
        Project {
            name: "atlas".to_string(),
            path: project_dir(),
            environment: Environment::Ambient,
            credential_command: None,
            poll: true,
            worktrees: Vec::new(),
        }
    }

    /// bd's CLI as `bdi` holds it when launched from a shell holding
    /// `ambient`, or from one holding no credential.
    fn launched_with<'a>(runner: &'a FakeRunner, ambient: Option<&str>) -> Cli<'a> {
        Cli {
            runner,
            ambient: ambient.map(str::to_string),
        }
    }

    /// The direnv call that reproduces entering `project_dir()`.
    fn entering_the_directory() -> String {
        format!("direnv exec {} env -0", project_dir().display())
    }

    /// The seam's promise: a tracker opened for a project is read in the
    /// environment that project's config asks for, so nothing above the
    /// adapter threads an environment through its calls.
    #[test]
    fn a_tracker_opened_for_a_project_is_read_in_the_environment_its_config_asks_for() {
        let runner = FakeRunner::default()
            .with(
                &entering_the_directory(),
                "BEADS_DOLT_PASSWORD=the-projects-own-password",
            )
            .with(&spelled(TRACKER_CALL), FIXTURE)
            .with(&spelled(WISP_CALL), "[]");
        let project = Project {
            environment: Environment::Direnv,
            ..ambient_project()
        };

        let cli = launched_with(&runner, Some("the-launching-shells-password"));
        let tracker = cli.of(&project).expect("the directory can be entered");
        tracker.all().expect("the tracker answers");

        let call = runner.call(&spelled(TRACKER_CALL));
        assert_eq!(call.cwd.as_deref(), Some(project_dir().as_path()));
        assert_eq!(
            call.env,
            Env::from([(
                CREDENTIAL_VAR.to_string(),
                "the-projects-own-password".to_string()
            )]),
            "the credential entering the directory produced is the one bd was given"
        );
    }

    /// A project that configures nothing is read on the credential the shell
    /// `bdi` was launched from holds, which the adapter captured once.
    #[test]
    fn a_project_configuring_nothing_is_read_on_the_ambient_credential() {
        let runner = FakeRunner::default().with(&spelled("ready --limit 0 --json"), "[]");

        let cli = launched_with(&runner, Some("hunter2"));
        let tracker = cli
            .of(&ambient_project())
            .expect("nothing is run to open an ambient project");
        tracker.ready().expect("the tracker answers");

        assert_eq!(
            runner.call(&spelled("ready --limit 0 --json")).env,
            credentialled()
        );
    }

    /// Opening is where the environment capture fails, and a project whose
    /// directory cannot be entered is that project's failure before bd is
    /// asked anything — the fake would panic on a bd call nobody staged.
    #[test]
    fn a_project_whose_directory_cannot_be_entered_fails_before_bd_is_asked_anything() {
        let runner = FakeRunner::default().failing(
            &entering_the_directory(),
            RunFailure::exec("direnv", "No such file or directory"),
        );
        let project = Project {
            environment: Environment::Direnv,
            ..ambient_project()
        };

        let failure = launched_with(&runner, None)
            .of(&project)
            .err()
            .expect("the project cannot be opened");

        assert_eq!(failure.kind, FailureKind::Exec);
        assert_eq!(failure.program, "direnv");
        assert!(
            runner
                .calls()
                .iter()
                .all(|call| !call.argv.starts_with("bd ")),
            "bd was asked something for a project that could not be opened"
        );
    }

    /// The one call a project's whole forest is drawn from, spelled as bd
    /// takes it. `--all` is what makes it the whole tracker rather than its
    /// open beads.
    const TRACKER_CALL: &str = "list --all --limit 0 --json";

    /// The second call the same forest needs, because `bd list` answers
    /// about the permanent table only.
    const WISP_CALL: &str = "query ephemeral=true --all --limit 0 --json";

    const WISPS: &str = include_str!("../../tests/fixtures/bd_wisps.json");

    /// The whole invocation the probe makes, spelled out rather than built
    /// from the constant it asserts about: this is the one place `bdi` writes
    /// SQL, and a change to that statement should have to be made twice.
    const PROBE_CALL: &str = "sql --json SELECT dolt_hashof_db() AS h";

    /// A working root as this tracker's Dolt server answers with one,
    /// captured 2026-09-01.
    const A_WORKING_ROOT: &str = "24eg8eff89bggt3t50ft6lctiu9rlpts";

    #[test]
    fn the_working_root_is_one_hash_out_of_one_statement() {
        let runner = FakeRunner::default().with(
            &spelled(PROBE_CALL),
            &format!(r#"[{{"h":"{A_WORKING_ROOT}"}}]"#),
        );

        let root = opened(&runner)
            .fingerprint()
            .expect("bd has a probe")
            .expect("the tracker answered its working root");

        assert_eq!(root, A_WORKING_ROOT);
        assert_eq!(
            runner.call(&spelled(PROBE_CALL)).env,
            credentialled(),
            "the probe reaches the tracker on the project's own credential"
        );
    }

    /// An answer with no row is a tracker that cannot be compared against,
    /// not a tracker that has not moved — so it fails rather than answering
    /// something a caller would gate on.
    #[test]
    fn a_probe_that_answers_no_row_is_a_failure_rather_than_a_hash() {
        let runner = FakeRunner::default().with(&spelled(PROBE_CALL), "[]");

        let failure = opened(&runner)
            .fingerprint()
            .expect("bd has a probe")
            .expect_err("no row is no answer");

        assert_eq!(failure.kind, FailureKind::Parse);
    }

    /// A tracker with no `dolt_hashof_db` — a SQLite-backed one — answers
    /// with something this cannot read, and that is a failure the caller has
    /// to see rather than a hash it would compare against.
    #[test]
    fn a_probe_answering_something_else_is_a_failure_rather_than_a_hash() {
        let runner = FakeRunner::default().with(&spelled(PROBE_CALL), "no such function");

        let failure = opened(&runner)
            .fingerprint()
            .expect("bd has a probe")
            .expect_err("an answer that is not the row is no answer");

        assert_eq!(failure.kind, FailureKind::Parse);
    }

    #[test]
    fn the_tracker_is_read_in_the_projects_directory_with_its_credential() {
        let runner = FakeRunner::default()
            .with(&spelled(TRACKER_CALL), FIXTURE)
            .with(&spelled(WISP_CALL), "[]");

        let beads = opened(&runner).all().unwrap();

        assert_eq!(beads.len(), 7);
        for subcommand in [TRACKER_CALL, WISP_CALL] {
            let call = runner.call(&spelled(subcommand));
            assert_eq!(call.cwd.as_deref(), Some(project_dir().as_path()));
            assert_eq!(call.env, credentialled());
        }
    }

    /// `bd list` answers about the permanent table, so it returns no wisp at
    /// all — not under `--all`, and not under its own `--wisp-type` filter.
    /// A tracker read only that way draws none of them.
    #[test]
    fn a_tracker_answers_with_its_wisps_as_well_as_its_permanent_beads() {
        let runner = FakeRunner::default()
            .with(&spelled(TRACKER_CALL), FIXTURE)
            .with(&spelled(WISP_CALL), WISPS);

        let beads = opened(&runner).all().unwrap();

        let ids: Vec<&str> = beads.iter().map(|bead| bead.id.as_str()).collect();
        assert!(
            ids.contains(&"bdi-7ao.17.2"),
            "the wisp under a bead: {ids:?}"
        );
        assert!(
            ids.contains(&"bdi-wisp-w3m"),
            "the free-standing wisp: {ids:?}"
        );
        assert_eq!(beads.len(), 9, "both answers, neither replacing the other");
    }

    /// bd omits a field it has nothing for rather than writing it as null:
    /// measured across the 73 ephemeral rows of a tracker on 2026-08-31,
    /// eleven keys are universal and every other one is absent when empty.
    /// So this row is not a shape bd writes today. It is covered because
    /// `#[serde(default)]` does not extend to an explicit null, and a
    /// tracker is read whole — a single row bd wrote differently would cost
    /// every bead in that project rather than its own edges.
    #[test]
    fn a_row_naming_its_dependencies_as_null_still_parses() {
        let json = r#"[{"id":"nix-wisp-gvi","title":"t","status":"open",
                        "issue_type":"molecule","parent":null,"dependencies":null}]"#;

        let bead = &parse_beads(json).expect("the row parses")[0];

        assert_eq!(bead.dependencies, vec![]);
    }

    /// A wisp bd writes under a parent is a child like any other: a dotted id
    /// and a real parent-child edge. One written without a parent has neither,
    /// so nothing but root discovery can place it.
    #[test]
    fn a_wisp_carries_the_edge_that_hangs_it_under_a_bead_where_it_has_one() {
        let wisps = parse_beads(WISPS).expect("the captured wisps parse");
        let by_id = |id: &str| {
            wisps
                .iter()
                .find(|wisp| wisp.id == id)
                .unwrap_or_else(|| panic!("{id} is in the fixture"))
                .clone()
        };

        assert_eq!(
            by_id("bdi-7ao.17.2").dependencies,
            vec![Dependency {
                on: "bdi-7ao.17".to_string(),
                edge: Edge::ParentChild,
            }]
        );
        assert_eq!(by_id("bdi-wisp-w3m").dependencies, vec![]);
    }

    /// One completed molecule and its sixteen steps, from a real wisp run on
    /// another project's tracker. It is here as evidence of what bd writes,
    /// which a constant somebody typed cannot be: the author of a constant
    /// supplies the keys they remember, so a key absent from a capture is a
    /// measurement and a key absent from a constant is authorship.
    ///
    /// Its two-row neighbour is not replaced by it. That one carries a
    /// free-standing wisp under no parent, which a molecule run has none of.
    ///
    /// Every `title` in it is invented, and says so in its own value. The
    /// capture arrived without the key: the redaction dropped `title`,
    /// `description`, `notes`, `owner`, `created_by`, `assignee`,
    /// `close_reason` and `dependencies[].created_by` whole, because one
    /// close reason named a person. `Bead::title` is required, so the file
    /// could not be parsed without one. Nothing else was touched — key order
    /// is the capture's and no value it carried was rewritten. Assert on a
    /// title and you are asserting on something we made up.
    const MOLECULE: &str = include_str!("../../tests/fixtures/bd_wisp_molecule.json");

    /// A row bd writes carries keys bdi does not read, and the ones bd will
    /// write next are not knowable. `await_type`, on the gate step of this
    /// run, is one nothing else in the tree has — which is the point of
    /// keeping a capture rather than a constant, because a constant carries
    /// only the keys its author already knew about.
    ///
    /// That `Bead` ignores such a key rather than rejecting the row is held
    /// by most of this module already. What is held here is the evidence:
    /// tidying the capture down to the fields bdi reads is what this refuses,
    /// because the untidy keys are the measurement.
    ///
    /// The counts go with it so a file shortened by accident is noticed. They
    /// are not evidence of anything — a run of any length can be typed.
    #[test]
    fn the_capture_keeps_a_field_bdi_does_not_read() {
        let run = parse_beads(MOLECULE).expect("the captured molecule parses");

        assert!(
            MOLECULE.contains(r#""await_type""#),
            "the capture still carries the key this is about"
        );
        assert_eq!(run.len(), 17, "the molecule and its steps");
        assert_eq!(
            run.iter()
                .map(|bead| bead.dependencies.len())
                .sum::<usize>(),
            37,
            "the edges bd wrote between them"
        );
        assert_eq!(
            run.iter()
                .filter(|bead| bead.issue_type == "molecule")
                .count(),
            1
        );
    }

    /// bd omits a field it has nothing for rather than writing it as null,
    /// which is what lets every field bdi reads carry `#[serde(default)]`.
    /// That is a claim about a program we do not own, so this holds the
    /// evidence for it: seventeen rows and their thirty-seven edges as bd
    /// wrote them, with no null anywhere — including on the molecule, which
    /// omits `parent` and `dependencies` rather than nulling them.
    ///
    /// It goes red if a later bd starts writing nulls, which is the day
    /// `none_is_empty` stops being enough.
    #[test]
    fn bd_omits_what_it_has_nothing_for_rather_than_writing_null() {
        let rows: serde_json::Value = serde_json::from_str(MOLECULE).expect("the capture is json");

        fn nulls(value: &serde_json::Value, at: &str, found: &mut Vec<String>) {
            match value {
                serde_json::Value::Null => found.push(at.to_string()),
                serde_json::Value::Object(fields) => {
                    for (key, field) in fields {
                        nulls(field, &format!("{at}.{key}"), found);
                    }
                }
                serde_json::Value::Array(items) => {
                    for (i, item) in items.iter().enumerate() {
                        nulls(item, &format!("{at}[{i}]"), found);
                    }
                }
                _ => {}
            }
        }

        let mut found = Vec::new();
        nulls(&rows, "", &mut found);

        assert_eq!(found, Vec::<String>::new(), "nulls bd wrote");
    }

    #[test]
    fn a_row_carries_every_bead_it_depends_on_and_the_kind_of_each() {
        let json = r#"[{"id":"p-1.4","title":"t","status":"open","dependencies":[
          {"issue_id":"p-1.4","depends_on_id":"p-1","type":"parent-child"},
          {"issue_id":"p-1.4","depends_on_id":"p-1.3","type":"blocks"}]}]"#;
        let bead = &parse_beads(json).expect("the row parses")[0];

        assert_eq!(
            bead.dependencies,
            vec![
                Dependency {
                    on: "p-1".to_string(),
                    edge: Edge::ParentChild,
                },
                Dependency {
                    on: "p-1.3".to_string(),
                    edge: Edge::Blocks,
                },
            ]
        );
    }

    #[test]
    fn ready_ids_returns_the_set_bd_considers_startable() {
        let out = r#"[{"id":"p-1.1","title":"a","status":"open"},
                      {"id":"p-1.3","title":"b","status":"open"}]"#;
        let runner = FakeRunner::default().with(&spelled("ready --limit 0 --json"), out);

        let got = opened(&runner).ready().unwrap();

        assert!(got.contains("p-1.1"));
        assert!(got.contains("p-1.3"));
        assert!(
            !got.contains("p-1.4"),
            "a bead bd did not list is not ready"
        );
    }

    /// The shape a real tracker produces: a bead blocked by two beads, whose
    /// dep-tree row names only one of them.
    #[test]
    fn blocked_by_carries_every_blocker_not_only_the_one_the_tree_shows() {
        let out = r#"[{"id":"p-1.9","title":"a","status":"blocked","blocked_by_count":2,
                       "blocked_by":["p-1.2","p-1.5"]},
                      {"id":"p-1.11","title":"b","status":"open","blocked_by_count":1,
                       "blocked_by":["p-1.10"]}]"#;
        let runner = FakeRunner::default().with(&spelled("blocked --json"), out);

        let got = opened(&runner).blocked().unwrap();

        assert_eq!(
            got.get("p-1.9").map(Vec::as_slice),
            Some(["p-1.2".to_string(), "p-1.5".to_string()].as_slice())
        );
        assert_eq!(got.len(), 2);
        assert_eq!(got.get("p-1.1"), None);
    }

    #[test]
    fn a_tracker_that_refuses_the_credential_reaches_the_caller_classified() {
        let runner = FakeRunner::default().failing(
            &spelled(TRACKER_CALL),
            RunFailure {
                kind: FailureKind::Auth,
                program: "bd".to_string(),
                detail: "bd was refused the tracker's credential".to_string(),
            },
        );

        let failure = opened(&runner).all().unwrap_err();

        assert_eq!(failure.kind, FailureKind::Auth);
    }

    #[test]
    fn output_bd_could_not_have_written_is_a_parse_failure_not_an_unreachable_tracker() {
        let runner = FakeRunner::default().with(&spelled("blocked --json"), "not json at all");

        let failure = opened(&runner).blocked().unwrap_err();

        assert_eq!(failure.kind, FailureKind::Parse);
    }

    /// The whole of the invocation, in one test because the two halves are
    /// one fact: bd is told which tracker to read, and told it may not write
    /// to it. Asserting the tracker alone would pass with the writes still
    /// allowed.
    #[test]
    fn every_call_names_the_tracker_outright_and_refuses_writes() {
        let runner = FakeRunner::default()
            .with(&spelled(TRACKER_CALL), FIXTURE)
            .with(&spelled(WISP_CALL), "[]");

        opened(&runner).all().unwrap();

        for call in runner.calls() {
            let after_the_program = call
                .argv
                .strip_prefix("bd ")
                .unwrap_or_else(|| panic!("{} is not a bd call", call.argv));
            assert!(
                after_the_program
                    .starts_with(&format!("-C {} --readonly ", project_dir().display())),
                "the tracker is left to the working directory in: {}",
                call.argv
            );
        }
    }

    /// A captured row carries the bead's own parent, which is the one the
    /// walk to a root needs: a dep-tree row's `parent_id` is the traversal's.
    #[test]
    fn a_captured_row_carries_the_bead_it_hangs_under() {
        assert_eq!(row("bdi-2bb.4").parent.as_deref(), Some("bdi-2bb"));
        assert_eq!(row("bdi-2bb").parent.as_deref(), Some("bdi-7ao"));
    }

    /// bd writes a root's absent parent as `null`; the dep tree writes the
    /// same absence as `""`, and an older bd omitted the field. All three
    /// mean the same thing.
    #[test]
    fn a_root_has_no_parent_however_bd_spells_the_absence() {
        for spelling in [r#","parent":null"#, r#","parent":"""#, ""] {
            let out = format!(r#"[{{"id":"p-1","title":"a","status":"open"{spelling}}}]"#);

            assert_eq!(
                parse_beads(&out).expect("the row parses")[0].parent,
                None,
                "on {spelling:?}"
            );
        }
    }
}
