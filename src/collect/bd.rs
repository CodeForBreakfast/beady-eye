use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::Context;
use serde::Deserialize;

use crate::collect::run::{Env, RunFailure, Runner, CREDENTIAL_VAR};
use crate::config::Project;
use crate::model::types::Bead;

/// Parse a flat array of bd rows, however the answer that carried them was
/// asked for. `bd list`, `bd ready` and `bd dep tree` all write the same
/// row.
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

/// The environment bd is given for one project's tracker.
///
/// A project naming a `credential_command` gets that command's stdout. A
/// project naming none is handed `ambient` instead, so a single-tracker setup
/// needs no credential configured at all — it is passed the one it would once
/// have inherited, which is what lets everything else `bdi` launches be
/// denied it.
pub fn credential_env(
    runner: &dyn Runner,
    project: &Project,
    ambient: Option<&str>,
) -> Result<Env, RunFailure> {
    let Some(command) = &project.credential_command else {
        return Ok(ambient.map_or_else(Env::new, |password| {
            Env::from([(CREDENTIAL_VAR.to_string(), password.to_string())])
        }));
    };
    let password = runner.run("sh", &["-c", command], Some(&project.path), &Env::new())?;
    Ok(Env::from([(
        CREDENTIAL_VAR.to_string(),
        password.trim_end_matches(['\r', '\n']).to_string(),
    )]))
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
    let out = runner.run(
        "bd",
        &["list", "--all", "--limit", "0", "--json"],
        Some(cwd),
        env,
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
    let mut args = vec!["query", EPHEMERAL];
    args.extend_from_slice(also);
    args.extend_from_slice(&["--limit", "0", "--json"]);
    runner.run("bd", &args, Some(cwd), env)
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

    let out = runner.run(
        "bd",
        &["list", "--status", UNFINISHED, "--limit", "0", "--json"],
        Some(cwd),
        env,
    )?;
    note_parents(&out, &mut found)?;

    // Without this a wisp with no parent is collected and then hung nowhere.
    // Every step of a bd molecule hangs under it, so the one rootless row is
    // the whole run.
    note_parents(&wisps(runner, cwd, env, &[])?, &mut found)?;

    for key in metadata_keys {
        let out = runner.run(
            "bd",
            &["list", "--has-metadata-key", key, "--limit", "0", "--json"],
            Some(cwd),
            env,
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
    let out = runner.run("bd", &["ready", "--limit", "0", "--json"], Some(cwd), env)?;
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
    let out = runner.run("bd", &["blocked", "--json"], Some(cwd), env)?;
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
    let out = runner.run("bd", &["show", id, "--json"], Some(cwd), env)?;
    let row = parent_rows(&out)?
        .into_iter()
        .next()
        .ok_or_else(|| RunFailure::parse("bd", "bd show named no bead"))?;
    Ok(row.parent.filter(|parent| !parent.is_empty()))
}

/// `bd list`, `bd ready` and `bd dep tree` all answer with the same rows.
fn rows(out: &str) -> Result<Vec<Bead>, RunFailure> {
    parse_beads(out).map_err(|e| RunFailure::parse("bd", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::types::{Dependency, Edge, Status};

    const FIXTURE: &str = include_str!("../../tests/fixtures/bd_dep_tree.json");

    fn fixture() -> Vec<Bead> {
        parse_beads(FIXTURE).expect("the captured tree parses")
    }

    fn row(id: &str) -> Bead {
        fixture()
            .into_iter()
            .find(|b| b.id == id)
            .unwrap_or_else(|| panic!("{id} is in the fixture"))
    }

    #[test]
    fn parses_every_row() {
        assert_eq!(fixture().len(), 6);
    }

    #[test]
    fn the_root_has_no_parent_and_children_carry_their_edge() {
        let root = row("bdi-3um");
        assert_eq!(root.parent_id, None);
        assert_eq!(root.edge_from_parent, None);

        let child = row("bdi-3um.10");
        assert_eq!(child.parent_id.as_deref(), Some("bdi-3um"));
        assert_eq!(child.edge_from_parent, Some(Edge::ParentChild));

        let blocker = row("bdi-3um.11");
        assert_eq!(blocker.parent_id.as_deref(), Some("bdi-3um.10"));
        assert_eq!(blocker.edge_from_parent, Some(Edge::Blocks));
    }

    #[test]
    fn statuses_map_onto_the_enum() {
        assert_eq!(row("bdi-3um").status, Status::InProgress);
        assert_eq!(row("bdi-3um.10").status, Status::Open);
        assert_eq!(row("bdi-3um.1").status, Status::Closed);
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
        let json = r#"[{"id":"x","title":"t","status":"open","edge_from_parent":"discovered-by"}]"#;
        let beads = parse_beads(json).expect("an unknown edge still parses");
        assert_eq!(
            beads[0].edge_from_parent,
            Some(Edge::Other("discovered-by".to_string()))
        );
    }

    #[test]
    fn metadata_is_carried_inline_and_absent_metadata_is_an_empty_map() {
        let carrying = row("bdi-3um.3");
        assert_eq!(
            carrying.metadata.get("working_topic").map(String::as_str),
            Some("beady-eye/core-json-bdi-3um.3")
        );

        assert!(row("bdi-3um.10").metadata.is_empty());
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
        let closed = row("bdi-3um.1");
        assert!(closed.started_at.is_some());
        assert!(closed.closed_at.is_some());

        let open = row("bdi-3um.10");
        assert_eq!(open.started_at, None);
        assert_eq!(open.closed_at, None);
        assert!(open.updated_at.is_some());
    }

    #[test]
    fn an_unclaimed_bead_has_no_assignee() {
        assert_eq!(row("bdi-3um.3").assignee.as_deref(), Some("Graeme Foster"));
        assert_eq!(row("bdi-3um.10").assignee, None);
    }

    #[test]
    fn issue_type_distinguishes_the_root_epic_from_its_tasks() {
        assert_eq!(row("bdi-3um").issue_type, "epic");
        assert_eq!(row("bdi-3um.10").issue_type, "task");
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

    fn credentialled() -> Env {
        Env::from([(CREDENTIAL_VAR.to_string(), "hunter2".to_string())])
    }

    /// The one call a project's whole forest is drawn from, spelled as bd
    /// takes it. `--all` is what makes it the whole tracker rather than its
    /// open beads.
    const TRACKER_CALL: &str = "bd list --all --limit 0 --json";

    /// The second call the same forest needs, because `bd list` answers
    /// about the permanent table only.
    const WISP_CALL: &str = "bd query ephemeral=true --all --limit 0 --json";

    const WISPS: &str = include_str!("../../tests/fixtures/bd_wisps.json");

    #[test]
    fn the_tracker_is_read_in_the_projects_directory_with_its_credential() {
        let runner = FakeRunner::default()
            .with(TRACKER_CALL, FIXTURE)
            .with(WISP_CALL, "[]");

        let beads = all_beads(&runner, &project_dir(), &credentialled()).unwrap();

        assert_eq!(beads.len(), 6);
        for argv in [TRACKER_CALL, WISP_CALL] {
            let call = runner.call(argv);
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
            .with(TRACKER_CALL, FIXTURE)
            .with(WISP_CALL, WISPS);

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
        assert_eq!(beads.len(), 8, "both answers, neither replacing the other");
    }

    /// A row naming no dependencies writes the field as null rather than
    /// omitting it, and `#[serde(default)]` does not cover an explicit null.
    /// A tracker is read whole, so such a row costs every bead in the
    /// project, not just its own edges.
    #[test]
    fn a_row_naming_its_dependencies_as_null_still_parses() {
        let json = r#"[{"id":"nix-wisp-gvi","title":"t","status":"open",
                        "issue_type":"molecule","parent":null,"dependencies":null}]"#;

        let bead = &parse_beads(json).expect("the row parses")[0];

        assert_eq!(bead.depends_on(), vec![]);
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
            by_id("bdi-7ao.17.2").depends_on(),
            vec![Dependency {
                on: "bdi-7ao.17".to_string(),
                edge: Edge::ParentChild,
            }]
        );
        assert_eq!(by_id("bdi-wisp-w3m").depends_on(), vec![]);
    }

    #[test]
    fn a_row_carries_every_bead_it_depends_on_and_the_kind_of_each() {
        let json = r#"[{"id":"p-1.4","title":"t","status":"open","dependencies":[
          {"issue_id":"p-1.4","depends_on_id":"p-1","type":"parent-child"},
          {"issue_id":"p-1.4","depends_on_id":"p-1.3","type":"blocks"}]}]"#;
        let bead = &parse_beads(json).expect("the row parses")[0];

        assert_eq!(
            bead.depends_on(),
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
    fn a_dep_tree_row_names_the_one_edge_the_walk_reached_it_by() {
        // `bd dep tree` carries a spanning tree rather than an edge set, and
        // the row shape says so: one parent, one kind, and no way to hold the
        // second bead this one waits on.
        let blocker = row("bdi-3um.11");

        assert_eq!(
            blocker.depends_on(),
            vec![Dependency {
                on: "bdi-3um.10".to_string(),
                edge: Edge::Blocks,
            }]
        );
    }

    const UNFINISHED_CALL: &str =
        "bd list --status open,in_progress,blocked,deferred --limit 0 --json";

    /// Discovery's wisp call. No `--all`, so it excludes closed wisps and
    /// nothing else — the same population `UNFINISHED_CALL` asks for.
    const UNFINISHED_WISP_CALL: &str = "bd query ephemeral=true --limit 0 --json";

    #[test]
    fn discovery_unions_the_unfinished_statuses_and_metadata_keys_without_duplicates() {
        let unfinished = r#"[{"id":"p-1.16","title":"a","status":"in_progress"},
                             {"id":"p-1.1","title":"b","status":"open"}]"#;
        // The metadata query returns a bead the status query already found.
        let carrying_the_key = r#"[{"id":"p-1.16","title":"a","status":"in_progress"}]"#;

        let runner = FakeRunner::default()
            .with(UNFINISHED_CALL, unfinished)
            .with(UNFINISHED_WISP_CALL, "[]")
            .with(
                "bd list --has-metadata-key working_topic --limit 0 --json",
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

        let call = runner.call(UNFINISHED_CALL);
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
                UNFINISHED_CALL,
                r#"[{"id":"p-1.1","title":"the work that is left","status":"open"}]"#,
            )
            .with(UNFINISHED_WISP_CALL, "[]");

        let got = discover_roots(&runner, &project_dir(), &credentialled(), &[]).unwrap();

        assert!(got.contains_key("p-1.1"));
    }

    /// A dep-tree row's `parent_id` is the traversal's parent. `bd list`
    /// carries the bead's own, and that is the one the walk to a root needs.
    #[test]
    fn discovery_keeps_each_beads_own_parent() {
        let runner = FakeRunner::default()
            .with(
                UNFINISHED_CALL,
                r#"[{"id":"p-1.16","title":"a","status":"open","parent":"p-1"},
                {"id":"p-1","title":"b","status":"open","parent":""}]"#,
            )
            .with(UNFINISHED_WISP_CALL, "[]");

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
            .with(UNFINISHED_CALL, "[]")
            .with(UNFINISHED_WISP_CALL, WISPS);

        let got = discover_roots(&runner, &project_dir(), &credentialled(), &[]).unwrap();

        assert_eq!(got["bdi-wisp-w3m"], None);
        assert_eq!(got["bdi-7ao.17.2"], Some("bdi-7ao.17".to_string()));

        let call = runner.call(UNFINISHED_WISP_CALL);
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
        let runner = FakeRunner::default().with("bd ready --limit 0 --json", out);

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
        let runner = FakeRunner::default().with("bd blocked --json", out);

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
            TRACKER_CALL,
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
        let runner = FakeRunner::default().with("bd blocked --json", "not json at all");

        let failure = blocked_by(&runner, &project_dir(), &credentialled()).unwrap_err();

        assert_eq!(failure.kind, FailureKind::Parse);
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

        let env = credential_env(&runner, &project, Some("the-launching-shells-password")).unwrap();

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
        let runner = FakeRunner::default();
        let project = Project {
            name: "beacon".to_string(),
            path: project_dir(),
            credential_command: None,
            worktrees: Vec::new(),
        };

        let env = credential_env(&runner, &project, Some("hunter2")).unwrap();

        assert_eq!(env, credentialled());
        assert!(
            runner.calls().is_empty(),
            "nothing is run to find no credential"
        );
    }

    /// Nothing to hand on is not an empty password: a tracker that wants one
    /// should refuse the call rather than be told the password is "".
    #[test]
    fn a_project_with_no_credential_command_and_no_ambient_one_is_given_nothing() {
        let runner = FakeRunner::default();
        let project = Project {
            name: "beacon".to_string(),
            path: project_dir(),
            credential_command: None,
            worktrees: Vec::new(),
        };

        assert_eq!(credential_env(&runner, &project, None).unwrap(), Env::new());
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
            credential_env(&runner, &project, None).unwrap_err().kind,
            FailureKind::Exec
        );
    }

    #[test]
    fn the_parent_comes_from_bd_show_because_the_dep_tree_cannot_carry_it() {
        let out = r#"[{"id":"p-1.16","title":"a","status":"open","parent":"p-1.4"}]"#;
        let runner = FakeRunner::default().with("bd show p-1.16 --json", out);

        let parent = parent_of(&runner, &project_dir(), &credentialled(), "p-1.16").unwrap();

        assert_eq!(parent.as_deref(), Some("p-1.4"));
        let call = runner.call("bd show p-1.16 --json");
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
            let runner = FakeRunner::default().with("bd show p-1 --json", &out);

            assert_eq!(
                parent_of(&runner, &project_dir(), &credentialled(), "p-1").unwrap(),
                None,
                "on {spelling:?}"
            );
        }
    }

    #[test]
    fn bd_show_naming_no_bead_is_a_parse_failure() {
        let runner = FakeRunner::default().with("bd show p-9 --json", "[]");

        let failure = parent_of(&runner, &project_dir(), &credentialled(), "p-9").unwrap_err();

        assert_eq!(failure.kind, FailureKind::Parse);
    }
}
