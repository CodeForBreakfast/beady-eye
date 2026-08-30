use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::Context;
use serde::Deserialize;

use crate::collect::run::{Env, RunFailure, Runner};
use crate::config::Project;
use crate::model::types::Bead;

/// The variable bd authenticates its Dolt server with.
const CREDENTIAL_VAR: &str = "BEADS_DOLT_PASSWORD";

/// Parse the output of `bd dep tree <root> --direction=up --json`.
///
/// bd returns a flat array already in its own render order, each row carrying
/// `parent_id` and `edge_from_parent`. We keep the rows and re-order them
/// ourselves; see `model::tree`.
pub fn parse_dep_tree(s: &str) -> anyhow::Result<Vec<Bead>> {
    serde_json::from_str(s).context("bd dep tree --json returned a shape we do not understand")
}

/// One row of `bd blocked --json`, which carries a blocker set no dep-tree
/// row has.
#[derive(Deserialize)]
struct BlockedRow {
    id: String,
    #[serde(default)]
    blocked_by: Vec<String>,
}

/// The environment bd is given for one project's tracker.
///
/// A project naming a `credential_command` gets that command's stdout, which
/// replaces whatever the shell bdi was started from holds. A project naming
/// none adds nothing and reaches its tracker on the ambient credential, so a
/// single-tracker setup needs no credential configured at all.
pub fn credential_env(runner: &dyn Runner, project: &Project) -> Result<Env, RunFailure> {
    let Some(command) = &project.credential_command else {
        return Ok(Env::new());
    };
    let password = runner.run("sh", &["-c", command], Some(&project.path), &Env::new())?;
    Ok(Env::from([(
        CREDENTIAL_VAR.to_string(),
        password.trim_end_matches(['\r', '\n']).to_string(),
    )]))
}

/// `bd dep tree <root> --direction=up --json`, in the project's directory and
/// with its credential.
///
/// Never `--max-depth`: bd reparents rows past the limit rather than marking
/// them, so a depth-limited call returns a different tree, not an incomplete
/// one.
pub fn dep_tree(
    runner: &dyn Runner,
    cwd: &Path,
    env: &Env,
    root: &str,
) -> Result<Vec<Bead>, RunFailure> {
    let out = runner.run(
        "bd",
        &["dep", "tree", root, "--direction=up", "--json"],
        Some(cwd),
        env,
    )?;
    rows(&out)
}

/// Beads that mark live work: bd's own in-flight statuses, plus any bead
/// carrying one of the configured metadata keys.
pub fn discover_roots(
    runner: &dyn Runner,
    cwd: &Path,
    env: &Env,
    metadata_keys: &[String],
) -> Result<Vec<Bead>, RunFailure> {
    let mut found: Vec<Bead> = Vec::new();

    for status in ["in_progress", "blocked"] {
        let out = runner.run(
            "bd",
            &["list", "--status", status, "--limit", "0", "--json"],
            Some(cwd),
            env,
        )?;
        found.extend(rows(&out)?);
    }

    for key in metadata_keys {
        let out = runner.run(
            "bd",
            &["list", "--has-metadata-key", key, "--limit", "0", "--json"],
            Some(cwd),
            env,
        )?;
        found.extend(rows(&out)?);
    }

    found.sort_by(|a, b| a.id.cmp(&b.id));
    found.dedup_by(|a, b| a.id == b.id);
    Ok(found)
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

/// `bd dep tree`, `bd list` and `bd ready` all answer with the same rows.
fn rows(out: &str) -> Result<Vec<Bead>, RunFailure> {
    parse_dep_tree(out).map_err(|e| RunFailure::parse("bd", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::types::{Edge, Status};

    const FIXTURE: &str = include_str!("../../tests/fixtures/bd_dep_tree.json");

    fn fixture() -> Vec<Bead> {
        parse_dep_tree(FIXTURE).expect("the captured tree parses")
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
            assert_eq!(parse_dep_tree(&json).unwrap()[0].status, want);
        }
    }

    #[test]
    fn a_status_a_later_bd_invents_is_kept_rather_than_rejected() {
        let json = r#"[{"id":"x","title":"t","status":"marinating"}]"#;
        let beads = parse_dep_tree(json).expect("an unknown status still parses");
        assert_eq!(beads[0].status, Status::Other("marinating".to_string()));
    }

    #[test]
    fn an_edge_a_later_bd_invents_is_kept_rather_than_rejected() {
        let json = r#"[{"id":"x","title":"t","status":"open","edge_from_parent":"discovered-by"}]"#;
        let beads = parse_dep_tree(json).expect("an unknown edge still parses");
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
        assert!(parse_dep_tree(json).unwrap()[0].truncated);
    }

    #[test]
    fn a_wrongly_typed_field_is_an_error_not_a_default() {
        let bad = r#"[{"id":"x","title":"t","status":"open","priority":"high"}]"#;
        assert!(parse_dep_tree(bad).is_err());
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

    #[test]
    fn dep_tree_asks_bd_in_the_projects_directory_with_its_credential() {
        let runner = FakeRunner::default().with("bd dep tree nix-1 --direction=up --json", FIXTURE);

        let beads = dep_tree(&runner, &project_dir(), &credentialled(), "nix-1").unwrap();

        assert_eq!(beads.len(), 6);
        let call = runner.call("bd dep tree nix-1 --direction=up --json");
        assert_eq!(call.cwd.as_deref(), Some(project_dir().as_path()));
        assert_eq!(call.env, credentialled());
    }

    #[test]
    fn discovery_unions_statuses_and_metadata_keys_without_duplicates() {
        let in_flight = r#"[{"id":"nix-1.16","title":"a","status":"in_progress"}]"#;
        let stuck = r#"[{"id":"nix-1.1","title":"b","status":"blocked"}]"#;
        // The metadata query returns a bead the status query already found.
        let carrying_the_key = r#"[{"id":"nix-1.16","title":"a","status":"in_progress"}]"#;

        let runner = FakeRunner::default()
            .with("bd list --status in_progress --limit 0 --json", in_flight)
            .with("bd list --status blocked --limit 0 --json", stuck)
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

        let ids: Vec<&str> = got.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, vec!["nix-1.1", "nix-1.16"]);

        let call = runner.call("bd list --status blocked --limit 0 --json");
        assert_eq!(call.cwd.as_deref(), Some(project_dir().as_path()));
        assert_eq!(call.env, credentialled());
    }

    #[test]
    fn ready_ids_returns_the_set_bd_considers_startable() {
        let out = r#"[{"id":"nix-1.1","title":"a","status":"open"},
                      {"id":"nix-1.3","title":"b","status":"open"}]"#;
        let runner = FakeRunner::default().with("bd ready --limit 0 --json", out);

        let got = ready_ids(&runner, &project_dir(), &credentialled()).unwrap();

        assert!(got.contains("nix-1.1"));
        assert!(got.contains("nix-1.3"));
        assert!(
            !got.contains("nix-1.4"),
            "a bead bd did not list is not ready"
        );
    }

    /// The shape measured on this repo's own tracker: bdi-3um.9 is blocked by
    /// two beads and its dep-tree row names only one of them.
    #[test]
    fn blocked_by_carries_every_blocker_not_only_the_one_the_tree_shows() {
        let out = r#"[{"id":"bdi-3um.9","title":"a","status":"blocked","blocked_by_count":2,
                       "blocked_by":["bdi-3um.2","bdi-3um.5"]},
                      {"id":"bdi-3um.11","title":"b","status":"open","blocked_by_count":1,
                       "blocked_by":["bdi-3um.10"]}]"#;
        let runner = FakeRunner::default().with("bd blocked --json", out);

        let got = blocked_by(&runner, &project_dir(), &credentialled()).unwrap();

        assert_eq!(
            got.get("bdi-3um.9").map(Vec::as_slice),
            Some(["bdi-3um.2".to_string(), "bdi-3um.5".to_string()].as_slice())
        );
        assert_eq!(got.len(), 2);
        assert_eq!(got.get("nix-1.1"), None);
    }

    #[test]
    fn a_tracker_that_refuses_the_credential_reaches_the_caller_classified() {
        let runner = FakeRunner::default().failing(
            "bd dep tree nix-1 --direction=up --json",
            RunFailure {
                kind: FailureKind::Auth,
                program: "bd".to_string(),
                detail: "bd was refused the tracker's credential".to_string(),
            },
        );

        let failure = dep_tree(&runner, &project_dir(), &credentialled(), "nix-1").unwrap_err();

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
        };

        let env = credential_env(&runner, &project).unwrap();

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

    /// A single-tracker setup configures no credential and reaches its
    /// tracker on the ambient one, so nothing is added and nothing removed.
    #[test]
    fn a_project_with_no_credential_command_adds_nothing_to_the_environment() {
        let runner = FakeRunner::default();
        let project = Project {
            name: "beacon".to_string(),
            path: project_dir(),
            credential_command: None,
        };

        assert_eq!(credential_env(&runner, &project).unwrap(), Env::new());
        assert!(
            runner.calls().is_empty(),
            "nothing is run to find no credential"
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
        };

        assert_eq!(
            credential_env(&runner, &project).unwrap_err().kind,
            FailureKind::Exec
        );
    }
}
