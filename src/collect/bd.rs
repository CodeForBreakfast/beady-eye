use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::Context;
use serde::Deserialize;

use crate::collect::run::{Env, RunFailure, Runner, CREDENTIAL_VAR};
use crate::config::Project;
use crate::model::types::Bead;

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
        let runner = FakeRunner::default().with("bd dep tree p-1 --direction=up --json", FIXTURE);

        let beads = dep_tree(&runner, &project_dir(), &credentialled(), "p-1").unwrap();

        assert_eq!(beads.len(), 6);
        let call = runner.call("bd dep tree p-1 --direction=up --json");
        assert_eq!(call.cwd.as_deref(), Some(project_dir().as_path()));
        assert_eq!(call.env, credentialled());
    }

    const UNFINISHED_CALL: &str =
        "bd list --status open,in_progress,blocked,deferred --limit 0 --json";

    #[test]
    fn discovery_unions_the_unfinished_statuses_and_metadata_keys_without_duplicates() {
        let unfinished = r#"[{"id":"p-1.16","title":"a","status":"in_progress"},
                             {"id":"p-1.1","title":"b","status":"open"}]"#;
        // The metadata query returns a bead the status query already found.
        let carrying_the_key = r#"[{"id":"p-1.16","title":"a","status":"in_progress"}]"#;

        let runner = FakeRunner::default()
            .with(UNFINISHED_CALL, unfinished)
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
        let runner = FakeRunner::default().with(
            UNFINISHED_CALL,
            r#"[{"id":"p-1.1","title":"the work that is left","status":"open"}]"#,
        );

        let got = discover_roots(&runner, &project_dir(), &credentialled(), &[]).unwrap();

        assert!(got.contains_key("p-1.1"));
    }

    /// A dep-tree row's `parent_id` is the traversal's parent. `bd list`
    /// carries the bead's own, and that is the one the walk to a root needs.
    #[test]
    fn discovery_keeps_each_beads_own_parent() {
        let runner = FakeRunner::default().with(
            UNFINISHED_CALL,
            r#"[{"id":"p-1.16","title":"a","status":"open","parent":"p-1"},
                {"id":"p-1","title":"b","status":"open","parent":""}]"#,
        );

        let got = discover_roots(&runner, &project_dir(), &credentialled(), &[]).unwrap();

        assert_eq!(got["p-1.16"], Some("p-1".to_string()));
        assert_eq!(
            got["p-1"], None,
            "bd writes the top of a chain as an empty parent"
        );
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
                parse_dep_tree(&json).unwrap()[0].status.clone()
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
            "bd dep tree p-1 --direction=up --json",
            RunFailure {
                kind: FailureKind::Auth,
                program: "bd".to_string(),
                detail: "bd was refused the tracker's credential".to_string(),
            },
        );

        let failure = dep_tree(&runner, &project_dir(), &credentialled(), "p-1").unwrap_err();

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
