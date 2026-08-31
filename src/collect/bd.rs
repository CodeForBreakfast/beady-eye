use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::Context;
use serde::Deserialize;

use crate::collect::run::{Env, RunFailure, Runner, CREDENTIAL_VAR};
use crate::config::Project;
use crate::model::types::Bead;

/// Parse a flat array of bd rows, however the answer that carried them was
/// asked for. `bd list`, `bd ready` and `bd query` all write the same row.
pub fn parse_beads(s: &str) -> anyhow::Result<Vec<Bead>> {
    serde_json::from_str(s).context("bd --json returned a shape we do not understand")
}

/// One row of `bd blocked --json`, which carries a blocker set no dep-tree
/// row has.
#[derive(Deserialize)]
struct BlockedRow {
    id: String,
    #[serde(default)]
    blocked_by: Vec<String>,
}

/// The credential the shell `bdi` was launched from holds, which a project
/// configuring none reaches its tracker on.
pub fn ambient_credential() -> Option<String> {
    std::env::var(CREDENTIAL_VAR).ok()
}

/// The environment one project's tracker is read with.
///
/// A shell that has entered a project's directory is already configured for
/// its tracker: direnv loads the flake, the bd version, and whatever holds
/// the password. So `bdi` reproduces entering the directory rather than
/// reconstructing what entering it would have produced, and a project entry
/// needs only a path — no assumption about what the secret is called, where
/// it lives, or what the DSN is.
///
/// Captured once per project rather than by wrapping every call, because
/// `direnv exec` reloads the directory each time it runs. Measured against
/// this repository on 2026-08-31: 1.3 to 2.4 seconds per invocation, where a
/// whole collection of both trackers costs 2.5 to 2.7. It also confines a
/// project whose `.envrc` writes to stdout to this one call, whose parser
/// tolerates it, rather than to every answer bd gives.
///
/// A `credential_command` is the escape hatch for a tracker outside direnv's
/// reach, and answers instead of entering the directory.
///
/// The ambient credential underneath both is what lets a single-tracker
/// setup configure nothing at all. It is safe here in a way it was not
/// before `-C`: a credential belonging to another tracker can now only fail
/// to authenticate against the right database, never open the wrong one.
pub fn tracker_env(
    runner: &dyn Runner,
    project: &Project,
    ambient: Option<&str>,
) -> Result<Env, RunFailure> {
    let mut env = ambient.map_or_else(Env::new, |password| {
        Env::from([(CREDENTIAL_VAR.to_string(), password.to_string())])
    });
    match &project.credential_command {
        Some(command) => {
            let password = runner.run("sh", &["-c", command], Some(&project.path), &Env::new())?;
            env.insert(
                CREDENTIAL_VAR.to_string(),
                password.trim_end_matches(['\r', '\n']).to_string(),
            );
        }
        None => env.extend(entering(&project.path, runner)?),
    }
    Ok(env)
}

/// The variables entering a directory produces.
///
/// direnv is given neither tracker nor credential of `bdi`'s own, so what
/// comes back is what entering that directory produces rather than what the
/// shell `bdi` was launched from was already carrying.
///
/// A directory direnv cannot enter fails this project rather than falling
/// back to the ambient environment. direnv itself fails open — it exits 0
/// and runs with the ambient environment where an `.envrc` is unallowed or a
/// flake will not evaluate — and a mechanism that silently does nothing is
/// indistinguishable from one that worked. What such a fallback cannot do,
/// because `-C` names the tracker, is read another project's database.
fn entering(path: &Path, runner: &dyn Runner) -> Result<Env, RunFailure> {
    let named = path.to_string_lossy();
    let out = runner.run(
        "direnv",
        &["exec", named.as_ref(), "env", "-0"],
        Some(path),
        &Env::new(),
    )?;
    Ok(variables(&out))
}

/// The variables in an `env -0` answer, tolerating whatever a project's
/// `.envrc` wrote to stdout before it.
///
/// direnv's own log lines reach stderr, measured, but nothing stops a
/// project's `.envrc` printing to stdout, and only this repository's has had
/// that fixed. Such text arrives ahead of the first variable and would
/// otherwise be read as part of its name, losing it. A variable's name holds
/// no newline, so whatever precedes the last one before the `=` is not part
/// of it.
fn variables(out: &str) -> Env {
    out.split('\0')
        .filter_map(|entry| {
            let (named, value) = entry.split_once('=')?;
            let name = named.rsplit('\n').next().unwrap_or(named);
            (!name.is_empty()).then(|| (name.to_string(), value.to_string()))
        })
        .collect()
}

/// One answer out of a project's tracker.
///
/// `-C` names the tracker outright, and it outranks `BEADS_DIR` in both
/// directions: a wrong variable still resolves the project, and a wrong
/// directory is refused rather than resolved to something plausible. That is
/// what makes entering the directory safe, because direnv can quietly do
/// nothing. Clearing the inherited variables stays as well; together they
/// mean a misconfiguration fails loudly.
///
/// `--readonly` has bd refuse the writes `bdi` never makes, so the rule is
/// enforced by bd rather than resting on `bdi` being well behaved.
fn asked(
    runner: &dyn Runner,
    tracker: &Path,
    env: &Env,
    subcommand: &[&str],
) -> Result<String, RunFailure> {
    let named = tracker.to_string_lossy();
    let mut argv = vec!["-C", named.as_ref(), "--readonly"];
    argv.extend_from_slice(subcommand);
    runner.run("bd", &argv, Some(tracker), env)
}

/// Every bead one tracker holds, each carrying the beads it depends on and
/// the kind of each dependency.
///
/// One call per project rather than one per root, because a tree is drawn
/// from dependency edges and `bd dep tree` cannot carry them: it walks
/// dependents and dedupes, so what comes back is a spanning tree — each bead
/// with the one edge the walk first reached it by, and every other edge into
/// it missing. Measured against this project's own tracker on 2026-08-30,
/// that walk carried 93 of the 176 edges among the beads it returned. It is
/// also the reason `blocked_by` is asked for separately and `parent_of`
/// exists at all.
///
/// `--all` is load-bearing: without it bd answers about open beads only, and
/// a smaller correct-looking answer about a different population is the kind
/// of wrong that reads as right.
pub fn all_beads(runner: &dyn Runner, cwd: &Path, env: &Env) -> Result<Vec<Bead>, RunFailure> {
    let out = asked(
        runner,
        cwd,
        env,
        &["list", "--all", "--limit", "0", "--json"],
    )?;
    let mut beads = rows(&out)?;
    beads.extend(rows(&wisps(runner, cwd, env, &["--all"])?)?);
    Ok(beads)
}

/// A tracker's wisps, in whichever population `also` asks for.
///
/// A second call, because bd keeps its ephemeral beads in a table `bd list`
/// does not read: measured against this project's own tracker on 2026-08-31,
/// `bd list --all` answered 120 rows both before and after two wisps were
/// written, and `bd list --wisp-type heartbeat` answered `[]` against a
/// heartbeat wisp that existed. `bd query` is the one call that reads them,
/// and it writes the same row `bd list` does.
fn wisps(runner: &dyn Runner, cwd: &Path, env: &Env, also: &[&str]) -> Result<String, RunFailure> {
    let mut subcommand = vec!["query", EPHEMERAL];
    subcommand.extend_from_slice(also);
    subcommand.extend_from_slice(&["--limit", "0", "--json"]);
    asked(runner, cwd, env, &subcommand)
}

/// The `bd query` expression that selects wisps and nothing else.
const EPHEMERAL: &str = "ephemeral=true";

/// The statuses bd stores for work that is not finished. `closed` is the only
/// one of its five this leaves out, and that is the whole of the rule.
const UNFINISHED: &str = "open,in_progress,blocked,deferred";

/// Every bead that marks unfinished work, and the bead each one hangs under.
///
/// Unfinished rather than claimed: an effort holds the work it has left after
/// its last seat stands down, and a rule that noticed only a claim lost the
/// whole tree at that moment.
///
/// The rows carry a bead's own `parent`, so discovery answers most of the walk
/// to a root by itself; `app::root_of` climbs only past what it did not see.
pub fn discover_roots(
    runner: &dyn Runner,
    cwd: &Path,
    env: &Env,
    metadata_keys: &[String],
) -> Result<BTreeMap<String, Option<String>>, RunFailure> {
    let mut found = BTreeMap::new();

    let out = asked(
        runner,
        cwd,
        env,
        &["list", "--status", UNFINISHED, "--limit", "0", "--json"],
    )?;
    note_parents(&out, &mut found)?;

    // Without this a wisp with no parent is collected and then hung nowhere.
    // Every step of a bd molecule hangs under it, so the one rootless row is
    // the whole run.
    note_parents(&wisps(runner, cwd, env, &[])?, &mut found)?;

    for key in metadata_keys {
        let out = asked(
            runner,
            cwd,
            env,
            &["list", "--has-metadata-key", key, "--limit", "0", "--json"],
        )?;
        note_parents(&out, &mut found)?;
    }

    Ok(found)
}

fn note_parents(out: &str, into: &mut BTreeMap<String, Option<String>>) -> Result<(), RunFailure> {
    for row in parent_rows(out)? {
        into.insert(row.id, row.parent.filter(|parent| !parent.is_empty()));
    }
    Ok(())
}

/// Ids beads considers ready to start. bd computes readiness itself and
/// treats it as a state of its own, so we ask for it rather than infer it
/// from status.
pub fn ready_ids(
    runner: &dyn Runner,
    cwd: &Path,
    env: &Env,
) -> Result<BTreeSet<String>, RunFailure> {
    let out = asked(runner, cwd, env, &["ready", "--limit", "0", "--json"])?;
    Ok(rows(&out)?.into_iter().map(|bead| bead.id).collect())
}

/// Every blocker of every blocked bead.
///
/// A dep-tree row carries its tree parent, not its blocker set: a bead
/// blocked by two others appears once, under one of them, with the second
/// nowhere in the output. `bd blocked` takes no limit of its own.
pub fn blocked_by(
    runner: &dyn Runner,
    cwd: &Path,
    env: &Env,
) -> Result<BTreeMap<String, Vec<String>>, RunFailure> {
    let out = asked(runner, cwd, env, &["blocked", "--json"])?;
    let blocked: Vec<BlockedRow> =
        serde_json::from_str(&out).map_err(|e| RunFailure::parse("bd", e))?;
    Ok(blocked
        .into_iter()
        .map(|row| (row.id, row.blocked_by))
        .collect())
}

/// One row of `bd show <id> --json`, which is the only call carrying a bead's
/// real parent.
/// One row of `bd show --json` or `bd list --json`. Both carry `parent`,
/// which is the bead's own — the field a dep-tree row does not have.
#[derive(Deserialize)]
struct ParentRow {
    id: String,
    #[serde(default)]
    parent: Option<String>,
}

fn parent_rows(out: &str) -> Result<Vec<ParentRow>, RunFailure> {
    serde_json::from_str(out).map_err(|e| RunFailure::parse("bd", e))
}

/// The bead a bead hangs under, or `None` at the top of a parent-child chain.
///
/// A dep-tree row cannot answer this. `bd dep tree --direction=up` walks
/// dependents, so whatever bead it is asked about comes back as its own root,
/// and the `parent_id` on every other row is that traversal's parent rather
/// than the bead's own.
pub fn parent_of(
    runner: &dyn Runner,
    cwd: &Path,
    env: &Env,
    id: &str,
) -> Result<Option<String>, RunFailure> {
    let out = asked(runner, cwd, env, &["show", id, "--json"])?;
    let row = parent_rows(&out)?
        .into_iter()
        .next()
        .ok_or_else(|| RunFailure::parse("bd", "bd show named no bead"))?;
    Ok(row.parent.filter(|parent| !parent.is_empty()))
}

/// `bd list`, `bd ready` and `bd query` all answer with the same rows.
fn rows(out: &str) -> Result<Vec<Bead>, RunFailure> {
    parse_beads(out).map_err(|e| RunFailure::parse("bd", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::run::RealRunner;
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
    fn truncation_survives_the_parse() {
        for bead in fixture() {
            assert!(!bead.truncated, "{} is not truncated", bead.id);
        }

        let json = r#"[{"id":"x","title":"t","status":"open","truncated":true}]"#;
        assert!(parse_beads(json).unwrap()[0].truncated);
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

    use crate::collect::run::testing::FakeRunner;
    use crate::collect::run::FailureKind;
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

    /// A project entry as the config now takes it: a path, and nothing else.
    fn ambient_project() -> Project {
        Project {
            name: "atlas".to_string(),
            path: project_dir(),
            credential_command: None,
            worktrees: Vec::new(),
        }
    }

    fn credentialled() -> Env {
        Env::from([(CREDENTIAL_VAR.to_string(), "hunter2".to_string())])
    }

    /// The one call a project's whole forest is drawn from, spelled as bd
    /// takes it. `--all` is what makes it the whole tracker rather than its
    /// open beads.
    const TRACKER_CALL: &str = "list --all --limit 0 --json";

    /// The second call the same forest needs, because `bd list` answers
    /// about the permanent table only.
    const WISP_CALL: &str = "query ephemeral=true --all --limit 0 --json";

    const WISPS: &str = include_str!("../../tests/fixtures/bd_wisps.json");

    #[test]
    fn the_tracker_is_read_in_the_projects_directory_with_its_credential() {
        let runner = FakeRunner::default()
            .with(&spelled(TRACKER_CALL), FIXTURE)
            .with(&spelled(WISP_CALL), "[]");

        let beads = all_beads(&runner, &project_dir(), &credentialled()).unwrap();

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

        let beads = all_beads(&runner, &project_dir(), &credentialled()).unwrap();

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

    const UNFINISHED_CALL: &str =
        "list --status open,in_progress,blocked,deferred --limit 0 --json";

    /// Discovery's wisp call. No `--all`, so it excludes closed wisps and
    /// nothing else — the same population `UNFINISHED_CALL` asks for.
    const UNFINISHED_WISP_CALL: &str = "query ephemeral=true --limit 0 --json";

    #[test]
    fn discovery_unions_the_unfinished_statuses_and_metadata_keys_without_duplicates() {
        let unfinished = r#"[{"id":"p-1.16","title":"a","status":"in_progress"},
                             {"id":"p-1.1","title":"b","status":"open"}]"#;
        // The metadata query returns a bead the status query already found.
        let carrying_the_key = r#"[{"id":"p-1.16","title":"a","status":"in_progress"}]"#;

        let runner = FakeRunner::default()
            .with(&spelled(UNFINISHED_CALL), unfinished)
            .with(&spelled(UNFINISHED_WISP_CALL), "[]")
            .with(
                &spelled("list --has-metadata-key working_topic --limit 0 --json"),
                carrying_the_key,
            );

        let got = discover_roots(
            &runner,
            &project_dir(),
            &credentialled(),
            &["working_topic".to_string()],
        )
        .unwrap();

        let ids: Vec<&str> = got.keys().map(String::as_str).collect();
        assert_eq!(ids, vec!["p-1.1", "p-1.16"]);

        let call = runner.call(&spelled(UNFINISHED_CALL));
        assert_eq!(call.cwd.as_deref(), Some(project_dir().as_path()));
        assert_eq!(call.env, credentialled());
    }

    /// The defect this rule replaces. Asking only for the statuses a seat
    /// leaves behind found nothing the moment every seat stood down, so there
    /// was no root, so the effort was not drawn at all.
    #[test]
    fn a_bead_nobody_has_started_is_discovered() {
        let runner = FakeRunner::default()
            .with(
                &spelled(UNFINISHED_CALL),
                r#"[{"id":"p-1.1","title":"the work that is left","status":"open"}]"#,
            )
            .with(&spelled(UNFINISHED_WISP_CALL), "[]");

        let got = discover_roots(&runner, &project_dir(), &credentialled(), &[]).unwrap();

        assert!(got.contains_key("p-1.1"));
    }

    /// A dep-tree row's `parent_id` is the traversal's parent. `bd list`
    /// carries the bead's own, and that is the one the walk to a root needs.
    #[test]
    fn discovery_keeps_each_beads_own_parent() {
        let runner = FakeRunner::default()
            .with(
                &spelled(UNFINISHED_CALL),
                r#"[{"id":"p-1.16","title":"a","status":"open","parent":"p-1"},
                {"id":"p-1","title":"b","status":"open","parent":""}]"#,
            )
            .with(&spelled(UNFINISHED_WISP_CALL), "[]");

        let got = discover_roots(&runner, &project_dir(), &credentialled(), &[]).unwrap();

        assert_eq!(got["p-1.16"], Some("p-1".to_string()));
        assert_eq!(
            got["p-1"], None,
            "bd writes the top of a chain as an empty parent"
        );
    }

    /// A free-standing wisp is the shape that passes a collection test and
    /// still draws nothing: it has no parent, so unless discovery names it a
    /// root of its own it is collected and then hung nowhere.
    #[test]
    fn a_free_standing_wisp_is_discovered_as_a_root_of_its_own() {
        let runner = FakeRunner::default()
            .with(&spelled(UNFINISHED_CALL), "[]")
            .with(&spelled(UNFINISHED_WISP_CALL), WISPS);

        let got = discover_roots(&runner, &project_dir(), &credentialled(), &[]).unwrap();

        assert_eq!(got["bdi-wisp-w3m"], None);
        assert_eq!(got["bdi-7ao.17.2"], Some("bdi-7ao.17".to_string()));

        let call = runner.call(&spelled(UNFINISHED_WISP_CALL));
        assert_eq!(call.cwd.as_deref(), Some(project_dir().as_path()));
        assert_eq!(call.env, credentialled());
    }

    /// The rule is *not finished*, so the query names every status bd stores
    /// bar `closed`. A status this missed would take its trees off the screen
    /// with it, which is the defect all over again.
    #[test]
    fn the_unfinished_statuses_are_every_status_bd_stores_but_closed() {
        let named: Vec<Status> = UNFINISHED
            .split(',')
            .map(|status| {
                let json = format!(r#"[{{"id":"x","title":"t","status":"{status}"}}]"#);
                parse_beads(&json).unwrap()[0].status.clone()
            })
            .collect();

        // A variant added to the enum fails this match, which is the point.
        let every = match Status::Open {
            Status::Open
            | Status::InProgress
            | Status::Blocked
            | Status::Closed
            | Status::Deferred
            | Status::Other(_) => [
                Status::Open,
                Status::InProgress,
                Status::Blocked,
                Status::Deferred,
                Status::Closed,
            ],
        };
        let want: Vec<Status> = every.into_iter().filter(|s| !s.is_closed()).collect();

        assert_eq!(named, want);
    }

    #[test]
    fn ready_ids_returns_the_set_bd_considers_startable() {
        let out = r#"[{"id":"p-1.1","title":"a","status":"open"},
                      {"id":"p-1.3","title":"b","status":"open"}]"#;
        let runner = FakeRunner::default().with(&spelled("ready --limit 0 --json"), out);

        let got = ready_ids(&runner, &project_dir(), &credentialled()).unwrap();

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

        let got = blocked_by(&runner, &project_dir(), &credentialled()).unwrap();

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

        let failure = all_beads(&runner, &project_dir(), &credentialled()).unwrap_err();

        assert_eq!(failure.kind, FailureKind::Auth);
    }

    #[test]
    fn output_bd_could_not_have_written_is_a_parse_failure_not_an_unreachable_tracker() {
        let runner = FakeRunner::default().with(&spelled("blocked --json"), "not json at all");

        let failure = blocked_by(&runner, &project_dir(), &credentialled()).unwrap_err();

        assert_eq!(failure.kind, FailureKind::Parse);
    }

    /// The direnv call that reproduces entering a project's directory,
    /// spelled as the runner makes it.
    fn entering_the_directory() -> String {
        format!("direnv exec {} env -0", project_dir().display())
    }

    /// An `env -0` answer: NUL between variables, and no separator after the
    /// last one that would make an empty final entry meaningful.
    fn exported(variables: &[(&str, &str)]) -> String {
        variables
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("\0")
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

        all_beads(&runner, &project_dir(), &credentialled()).unwrap();

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

    /// The invariant the whole design rests on: a shell that has entered the
    /// project's directory is configured for its tracker, so bdi reproduces
    /// entering it rather than restating what it would have produced.
    #[test]
    fn a_project_naming_only_a_path_is_read_by_entering_its_directory() {
        let runner = FakeRunner::default().with(
            &entering_the_directory(),
            &exported(&[
                ("BEADS_DIR", "/tmp/proj/.beads"),
                ("BEADS_DOLT_PASSWORD", "the-projects-own-password"),
            ]),
        );

        let env = tracker_env(&runner, &ambient_project(), None).unwrap();

        assert_eq!(
            env.get("BEADS_DOLT_PASSWORD").map(String::as_str),
            Some("the-projects-own-password"),
            "the credential entering the directory produces did not reach bd"
        );
        assert_eq!(
            env.get("BEADS_DIR").map(String::as_str),
            Some("/tmp/proj/.beads"),
            "the tracker entering the directory names did not reach bd"
        );
    }

    /// direnv is asked what entering the directory produces, not what bdi was
    /// already carrying. Handed bdi's own tracker and credential it would
    /// answer with them for every project alike, which is the defect `-C` and
    /// the cleared environment exist to stop.
    #[test]
    fn direnv_is_given_no_tracker_and_no_credential_of_bdis_own() {
        let runner = FakeRunner::default().with(&entering_the_directory(), &exported(&[]));

        tracker_env(
            &runner,
            &ambient_project(),
            Some("the-launching-shells-password"),
        )
        .unwrap();

        assert!(
            runner.call(&entering_the_directory()).env.is_empty(),
            "direnv was handed an environment to reproduce"
        );
    }

    /// A project's `.envrc` may print to stdout, and only this repository's
    /// has been fixed not to. The text lands ahead of the first variable, and
    /// reading it as part of that variable's name loses the variable.
    #[test]
    fn text_a_projects_envrc_wrote_first_does_not_lose_the_variable_behind_it() {
        let noise = "entering the atlas shell\n";
        let runner = FakeRunner::default().with(
            &entering_the_directory(),
            &format!(
                "{noise}{}",
                exported(&[("BEADS_DOLT_PASSWORD", "hunter2"), ("PATH", "/nix/bin")])
            ),
        );

        let env = tracker_env(&runner, &ambient_project(), None).unwrap();

        assert_eq!(env.get(CREDENTIAL_VAR).map(String::as_str), Some("hunter2"));
        assert_eq!(
            env.get("PATH").map(String::as_str),
            Some("/nix/bin"),
            "the first variable was read as part of the text in front of it"
        );
        assert!(
            !env.keys().any(|name| name.contains('\n')),
            "text written before the variables became a variable: {env:?}"
        );
    }

    /// direnv fails open: it exits 0 and runs with the ambient environment
    /// where an `.envrc` is unallowed or a flake will not evaluate. So a
    /// project whose directory cannot be entered at all is that project's
    /// failure, not a quiet fallback that reads as having worked.
    #[test]
    fn a_directory_that_cannot_be_entered_fails_the_project_rather_than_falling_back() {
        let runner = FakeRunner::default().failing(
            &entering_the_directory(),
            RunFailure::exec("direnv", "No such file or directory"),
        );

        let failure = tracker_env(&runner, &ambient_project(), Some("hunter2")).unwrap_err();

        assert_eq!(failure.kind, FailureKind::Exec);
        assert_eq!(failure.program, "direnv");
    }

    /// The escape hatch answers instead of entering the directory, for a
    /// tracker outside direnv's reach.
    #[test]
    fn a_credential_command_answers_instead_of_entering_the_directory() {
        let runner = FakeRunner::default().with("sh -c op read the/password", "hunter2\n");
        let project = Project {
            name: "atlas".to_string(),
            path: project_dir(),
            credential_command: Some("op read the/password".to_string()),
            worktrees: Vec::new(),
        };

        tracker_env(&runner, &project, None).unwrap();

        assert!(
            !runner
                .calls()
                .iter()
                .any(|call| call.argv.starts_with("direnv ")),
            "the directory was entered as well as the escape hatch being used"
        );
    }

    /// The parser reads back what `env` actually writes, rather than what we
    /// believe it writes: a real process, and every variable it exported.
    #[test]
    fn the_variables_read_back_are_the_ones_env_wrote() {
        let out = RealRunner
            .run(
                "env",
                &["-0"],
                None,
                &Env::from([("K".to_string(), "v".to_string())]),
            )
            .expect("env runs");

        let read = variables(&out);

        assert_eq!(read.get("K").map(String::as_str), Some("v"));
        assert_eq!(
            read.len(),
            out.split('\0').filter(|entry| !entry.is_empty()).count(),
            "a variable env wrote was not read back"
        );
    }

    #[test]
    fn a_projects_credential_command_supplies_its_password() {
        let runner = FakeRunner::default().with("sh -c op read the/password", "hunter2\n");
        let project = Project {
            name: "atlas".to_string(),
            path: project_dir(),
            credential_command: Some("op read the/password".to_string()),
            worktrees: Vec::new(),
        };

        let env = tracker_env(&runner, &project, Some("the-launching-shells-password")).unwrap();

        assert_eq!(
            env,
            credentialled(),
            "the trailing newline is not the password"
        );
        let call = runner.call("sh -c op read the/password");
        assert_eq!(call.cwd.as_deref(), Some(project_dir().as_path()));
        assert!(
            call.env.is_empty(),
            "the credential command gets no credential"
        );
    }

    /// A single-tracker setup configures no credential and reaches its tracker
    /// on the ambient one. It is handed that credential rather than left to
    /// inherit it, because nothing bdi launches inherits it any more.
    #[test]
    fn a_project_with_no_credential_command_is_handed_the_ambient_credential() {
        let runner = FakeRunner::default().with(
            &entering_the_directory(),
            &exported(&[("PATH", "/nix/bin")]),
        );

        let env = tracker_env(&runner, &ambient_project(), Some("hunter2")).unwrap();

        assert_eq!(env.get(CREDENTIAL_VAR).map(String::as_str), Some("hunter2"));
        assert_eq!(
            env.get("PATH").map(String::as_str),
            Some("/nix/bin"),
            "the directory was entered but what it produced did not reach bd"
        );
    }

    /// Nothing to hand on is not an empty password: a tracker that wants one
    /// should refuse the call rather than be told the password is "".
    #[test]
    fn a_project_with_no_credential_command_and_no_ambient_one_is_given_nothing() {
        let runner = FakeRunner::default().with(&entering_the_directory(), &exported(&[]));

        assert_eq!(
            tracker_env(&runner, &ambient_project(), None).unwrap(),
            Env::new()
        );
    }

    #[test]
    fn a_credential_command_that_fails_reaches_the_caller() {
        let runner = FakeRunner::default().failing(
            "sh -c op read the/password",
            RunFailure::exec("sh", "op: command not found"),
        );
        let project = Project {
            name: "atlas".to_string(),
            path: project_dir(),
            credential_command: Some("op read the/password".to_string()),
            worktrees: Vec::new(),
        };

        assert_eq!(
            tracker_env(&runner, &project, None).unwrap_err().kind,
            FailureKind::Exec
        );
    }

    #[test]
    fn the_parent_comes_from_bd_show_because_the_dep_tree_cannot_carry_it() {
        let out = r#"[{"id":"p-1.16","title":"a","status":"open","parent":"p-1.4"}]"#;
        let runner = FakeRunner::default().with(&spelled("show p-1.16 --json"), out);

        let parent = parent_of(&runner, &project_dir(), &credentialled(), "p-1.16").unwrap();

        assert_eq!(parent.as_deref(), Some("p-1.4"));
        let call = runner.call(&spelled("show p-1.16 --json"));
        assert_eq!(call.cwd.as_deref(), Some(project_dir().as_path()));
        assert_eq!(call.env, credentialled());
    }

    /// bd writes a root's absent parent as `null`; the dep tree writes the
    /// same absence as `""`, and an older bd omitted the field. All three
    /// mean the same thing.
    #[test]
    fn a_root_has_no_parent_however_bd_spells_the_absence() {
        for spelling in [r#","parent":null"#, r#","parent":"""#, ""] {
            let out = format!(r#"[{{"id":"p-1","title":"a","status":"open"{spelling}}}]"#);
            let runner = FakeRunner::default().with(&spelled("show p-1 --json"), &out);

            assert_eq!(
                parent_of(&runner, &project_dir(), &credentialled(), "p-1").unwrap(),
                None,
                "on {spelling:?}"
            );
        }
    }

    #[test]
    fn bd_show_naming_no_bead_is_a_parse_failure() {
        let runner = FakeRunner::default().with(&spelled("show p-9 --json"), "[]");

        let failure = parent_of(&runner, &project_dir(), &credentialled(), "p-9").unwrap_err();

        assert_eq!(failure.kind, FailureKind::Parse);
    }
}
