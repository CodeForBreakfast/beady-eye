use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;

use serde::Serialize;

use crate::config::{Badge, Join, Project};
use crate::model::tree::Placed;
use crate::model::types::{Bead, Pane, PaneStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JoinSource {
    /// The bead named the pane. Exact.
    AgentPane,
    /// The pane named the bead. Inferred from free text.
    DisplayAgent,
}

/// A bead, across every tracker `bdi` reads. Prefixes are per-tracker and
/// uncoordinated, so an id on its own does not name a bead.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct BeadKey {
    pub project: String,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentRef {
    pub pane: String,
    pub pane_status: PaneStatus,
    pub title: Option<String>,
    pub source: JoinSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Badged {
    pub key: String,
    pub text: String,
}

/// A disagreement between the two directions of the join. Each is reported
/// rather than resolved: picking a winner would hide exactly the drift `bdi`
/// exists to surface.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "conflict", rename_all = "kebab-case")]
pub enum Conflict {
    /// The bead's own key and a pane's `display_agent` name different panes.
    BeadAndPaneDisagree {
        bead: BeadKey,
        named_by_bead: String,
        named_by_pane: String,
    },
    /// Several panes name one bead. None of them wins it.
    SeveralPanesNameOneBead { bead: BeadKey, panes: Vec<String> },
    /// Several beads name one pane. None of them gets it.
    ///
    /// `caption` is the pane's own account of what it is working on, carried
    /// here because it is the one thing that tells a live claim from a stale
    /// one and it is nowhere else on the screen: a caption is drawn off the
    /// agent a pane was awarded, and a contested pane is awarded to nobody.
    SeveralBeadsNameOnePane {
        pane: String,
        caption: Option<String>,
        beads: Vec<BeadKey>,
    },
    /// A pane and the bead naming it, or named by it, sit in different
    /// projects. No join. `pane_project` is absent when the pane's `cwd` is
    /// under no configured project at all.
    PaneInAnotherProject {
        bead: BeadKey,
        pane: String,
        pane_project: Option<String>,
    },
}

/// One project's assembled rows, as the join reads them.
pub struct ProjectRows<'a> {
    pub project: &'a str,
    pub rows: &'a [Placed],
}

/// The agents the join could account for, and every disagreement it could not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Joined {
    pub agents: BTreeMap<BeadKey, AgentRef>,
    /// Beads whose own key named a live pane the join would not award them,
    /// and the disagreement that refused it. Only the exact direction is here:
    /// a bead that named nothing has no claim to refuse, and a pane naming an
    /// id another tracker happens to reuse says nothing about that tracker's
    /// bead.
    pub refused: BTreeMap<BeadKey, Conflict>,
    pub conflicts: Vec<Conflict>,
}

/// The project a path sits in: the one whose deepest working tree contains
/// it. `None` when no project's does.
pub fn project_of<'a>(path: &Path, projects: &'a [Project]) -> Option<&'a Project> {
    projects
        .iter()
        .filter_map(|p| Some((p.holds(path)?, p)))
        .max_by_key(|(depth, _)| *depth)
        .map(|(_, project)| project)
}

/// Join live panes onto beads, scoped to each pane's own project.
///
/// Bidirectional, because each direction alone has a hole: a bead's configured
/// key names its pane exactly, and a pane's `display_agent` names its bead by
/// inference. A bead takes its agent from the exact direction when that
/// direction resolved uncontested, otherwise from the inferred direction when
/// exactly one pane names it, otherwise none — and wherever the two directions
/// name different live panes, the disagreement is reported however it went.
pub fn resolve(
    trees: &[ProjectRows<'_>],
    panes: &[Pane],
    projects: &[Project],
    join: &Join,
) -> Joined {
    let live: BTreeMap<&str, &Pane> = panes.iter().map(|p| (p.pane_id.as_str(), p)).collect();

    // Every pane is placed in a project before any bead is looked at, so a
    // colliding id in another tracker never reaches the join at all.
    let pane_project: BTreeMap<&str, Option<&str>> = panes
        .iter()
        .map(|p| {
            (
                p.pane_id.as_str(),
                project_of(&p.cwd, projects).map(|q| q.name.as_str()),
            )
        })
        .collect();

    // Which projects hold a bead of each id, so a pane naming one outside its
    // own project is told apart from a pane naming nothing at all.
    let mut projects_holding: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for tree in trees {
        for row in tree.rows {
            projects_holding
                .entry(row.bead.id.as_str())
                .or_default()
                .insert(tree.project);
        }
    }

    let mut conflicts: Vec<Conflict> = Vec::new();
    let mut refused: BTreeMap<BeadKey, Conflict> = BTreeMap::new();

    // The exact direction: the bead names its pane.
    let mut claims: BTreeMap<&str, BTreeSet<BeadKey>> = BTreeMap::new();
    let mut claimed_pane: BTreeMap<BeadKey, String> = BTreeMap::new();
    for tree in trees {
        for row in tree.rows {
            let Some(named) = row.bead.metadata.get(&join.pane_key) else {
                continue;
            };
            // A named pane that is not live resolves to nothing; that absence
            // is `orphan-claim`'s to report, not a disagreement.
            let Some(pane) = live.get(named.as_str()) else {
                continue;
            };
            let bead = BeadKey {
                project: tree.project.to_string(),
                id: row.bead.id.clone(),
            };
            let holds = pane_project[pane.pane_id.as_str()];
            if holds != Some(tree.project) {
                let elsewhere = Conflict::PaneInAnotherProject {
                    bead: bead.clone(),
                    pane: pane.pane_id.clone(),
                    pane_project: holds.map(str::to_string),
                };
                conflicts.push(elsewhere.clone());
                refused.insert(bead, elsewhere);
                continue;
            }
            claimed_pane.insert(bead.clone(), pane.pane_id.clone());
            claims
                .entry(pane.pane_id.as_str())
                .or_default()
                .insert(bead);
        }
    }

    let mut agents: BTreeMap<BeadKey, AgentRef> = BTreeMap::new();
    for (pane_id, beads) in claims {
        let pane = live[pane_id];
        let mut named: Vec<BeadKey> = beads.into_iter().collect();
        if named.len() == 1 {
            agents.insert(named.remove(0), agent_ref(pane, JoinSource::AgentPane));
        } else {
            let shared = Conflict::SeveralBeadsNameOnePane {
                pane: pane.pane_id.clone(),
                caption: pane.caption().map(str::to_string),
                beads: named.clone(),
            };
            conflicts.push(shared.clone());
            for bead in named {
                refused.insert(bead, shared.clone());
            }
        }
    }

    // The inferred direction: the pane names its bead.
    let mut named_by: BTreeMap<BeadKey, BTreeSet<&str>> = BTreeMap::new();
    for pane in panes {
        let Some(id) = &pane.display_agent else {
            continue;
        };
        let Some(project) = pane_project[pane.pane_id.as_str()] else {
            continue;
        };
        // Most `display_agent` values are free text rather than a bead id.
        let Some(holders) = projects_holding.get(id.as_str()) else {
            continue;
        };
        if holders.contains(project) {
            named_by
                .entry(BeadKey {
                    project: project.to_string(),
                    id: id.clone(),
                })
                .or_default()
                .insert(pane.pane_id.as_str());
        } else {
            for holder in holders {
                conflicts.push(Conflict::PaneInAnotherProject {
                    bead: BeadKey {
                        project: holder.to_string(),
                        id: id.clone(),
                    },
                    pane: pane.pane_id.clone(),
                    pane_project: Some(project.to_string()),
                });
            }
        }
    }

    for (bead, from_panes) in named_by {
        let inferred: Vec<&str> = from_panes.into_iter().collect();

        if inferred.len() > 1 {
            conflicts.push(Conflict::SeveralPanesNameOneBead {
                bead,
                panes: inferred.into_iter().map(str::to_string).collect(),
            });
            continue;
        }

        let only = inferred[0];
        // The disagreement stands on what the bead's key named, whether or not
        // that claim survived to be awarded.
        if let Some(claimed) = claimed_pane.get(&bead).filter(|p| p.as_str() != only) {
            conflicts.push(Conflict::BeadAndPaneDisagree {
                bead: bead.clone(),
                named_by_bead: claimed.clone(),
                named_by_pane: only.to_string(),
            });
        }
        // The exact direction wins where it resolved uncontested; the inferred
        // one only fills the gap it left.
        agents
            .entry(bead)
            .or_insert_with(|| agent_ref(live[only], JoinSource::DisplayAgent));
    }

    conflicts.sort();
    conflicts.dedup();

    Joined {
        agents,
        refused,
        conflicts,
    }
}

fn agent_ref(pane: &Pane, source: JoinSource) -> AgentRef {
    AgentRef {
        pane: pane.pane_id.clone(),
        pane_status: pane.agent_status.clone(),
        title: pane.caption().map(str::to_string),
        source,
    }
}

/// Render the configured badges that apply to this bead.
pub fn badges_for(bead: &Bead, badges: &[Badge]) -> Vec<Badged> {
    badges
        .iter()
        .filter_map(|b| {
            let value = bead.metadata.get(&b.key)?;
            let text = b.apply(value)?;
            Some(Badged {
                key: b.key.clone(),
                text,
            })
        })
        .collect()
}

/// Live panes that resolved to no bead in any tree.
pub fn unattributed<'a>(panes: &'a [Pane], joined: &Joined) -> Vec<&'a Pane> {
    let taken: HashSet<&str> = joined.agents.values().map(|a| a.pane.as_str()).collect();
    panes
        .iter()
        .filter(|p| !taken.contains(p.pane_id.as_str()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::bd::parse_dep_tree;
    use crate::collect::herdr::parse_agent_list;
    use crate::model::tree::assemble;
    use pretty_assertions::assert_eq;

    const BEADS: &str = include_str!("../../tests/fixtures/bd_dep_tree.json");
    const PANES: &str = include_str!("../../tests/fixtures/herdr_agent_list.json");

    /// A tracker and a herdr session captured from the same live moment, so
    /// the pane a bead names is a pane the list reports. The pair the other
    /// fixtures cannot make: `bd_dep_tree.json` was captured before any seat
    /// wrote `agent_pane`, so nothing in it exercises the exact direction.
    const JOINED_BEADS: &str = include_str!("../../tests/fixtures/joined_bd_dep_tree.json");
    const JOINED_PANES: &str = include_str!("../../tests/fixtures/joined_herdr_agent_list.json");

    const FIXTURE_PROJECT_PATH: &str = "/tmp/bdi-ground/beady-eye";

    fn project(name: &str, path: &str) -> Project {
        Project {
            name: name.to_string(),
            path: path.into(),
            credential_command: None,
            worktrees: Vec::new(),
        }
    }

    fn project_working_in(name: &str, path: &str, worktrees: &[&str]) -> Project {
        Project {
            worktrees: worktrees.iter().map(Into::into).collect(),
            ..project(name, path)
        }
    }

    fn rows(json: &str) -> Vec<Placed> {
        assemble(parse_dep_tree(json).expect("the rows parse"))
            .expect("the rows assemble")
            .rows
    }

    /// The bodies of `herdr agent list`'s `agents` array, wrapped in its
    /// envelope so the tests exercise the real parser.
    fn panes(agents: &str) -> Vec<Pane> {
        parse_agent_list(&format!(
            r#"{{"id":"cli:agent:list","result":{{"agents":[{agents}]}}}}"#
        ))
        .expect("the panes parse")
    }

    fn key(project: &str, id: &str) -> BeadKey {
        BeadKey {
            project: project.to_string(),
            id: id.to_string(),
        }
    }

    fn pane_of<'a>(joined: &'a Joined, project: &str, id: &str) -> &'a AgentRef {
        joined
            .agents
            .get(&key(project, id))
            .unwrap_or_else(|| panic!("{project}/{id} resolved to a pane"))
    }

    fn loose(panes: &[Pane], joined: &Joined) -> Vec<String> {
        let mut ids: Vec<String> = unattributed(panes, joined)
            .into_iter()
            .map(|p| p.pane_id.clone())
            .collect();
        ids.sort();
        ids
    }

    // ---- the two directions -------------------------------------------

    /// The whole point of the tool, against the shapes a real tracker and a
    /// real herdr session actually emit rather than the ones a test writes.
    #[test]
    fn a_captured_bead_takes_the_captured_pane_it_names() {
        let beads = rows(JOINED_BEADS);
        let live = parse_agent_list(JOINED_PANES).expect("the fixture parses");
        let cfg = vec![project("beady-eye", FIXTURE_PROJECT_PATH)];

        let joined = resolve(
            &[ProjectRows {
                project: "beady-eye",
                rows: &beads,
            }],
            &live,
            &cfg,
            &Join::default(),
        );

        let a = pane_of(&joined, "beady-eye", "bdi-2bb.18");
        assert_eq!(a.pane, "wCW:p1Q");
        assert_eq!(a.source, JoinSource::AgentPane);
        assert_eq!(joined.conflicts, vec![]);
        assert_eq!(joined.refused, BTreeMap::new());
    }

    #[test]
    fn the_bead_naming_its_pane_resolves_exactly() {
        let beads = rows(
            r#"[
              {"id":"p-1","title":"root","status":"open","parent_id":""},
              {"id":"p-1.1","title":"work","status":"in_progress","parent_id":"p-1",
               "metadata":{"agent_pane":"w:p1"}}
            ]"#,
        );
        let live = panes(
            r#"{"pane_id":"w:p1","cwd":"/home/user/proj","agent_status":"working",
                "title":"doing the work"}"#,
        );
        let cfg = vec![project("proj", "/home/user/proj")];

        let joined = resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            &live,
            &cfg,
            &Join::default(),
        );

        let a = pane_of(&joined, "proj", "p-1.1");
        assert_eq!(a.pane, "w:p1");
        assert_eq!(a.source, JoinSource::AgentPane);
        assert_eq!(a.pane_status, PaneStatus::Working);
        assert_eq!(a.title.as_deref(), Some("doing the work"));
        assert_eq!(joined.conflicts, vec![]);
    }

    /// The line an agent gets is the pane's caption — its label for the state
    /// it is actually in — rather than its title.
    #[test]
    fn an_agent_carries_the_panes_caption() {
        let beads = rows(
            r#"[{"id":"p-1","title":"root","status":"in_progress","parent_id":"",
                 "metadata":{"agent_pane":"w:p1"}}]"#,
        );
        let live = panes(
            r#"{"pane_id":"w:p1","cwd":"/home/user/proj","agent_status":"working",
                "title":"the title",
                "state_labels":{"idle":"the idle line","working":"the working line"}}"#,
        );
        let cfg = vec![project("proj", "/home/user/proj")];

        let joined = resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            &live,
            &cfg,
            &Join::default(),
        );

        assert_eq!(
            pane_of(&joined, "proj", "p-1").title.as_deref(),
            Some("the working line")
        );
    }

    /// The key is configurable, so a hardcoded `agent_pane` cannot pass.
    #[test]
    fn the_exact_direction_reads_the_configured_key() {
        let beads = rows(
            r#"[
              {"id":"p-1","title":"root","status":"open","parent_id":"",
               "metadata":{"herdr_pane":"w:p1","agent_pane":"w:p2"}}
            ]"#,
        );
        let live = panes(
            r#"{"pane_id":"w:p1","cwd":"/home/user/proj","agent_status":"idle"},
               {"pane_id":"w:p2","cwd":"/home/user/proj","agent_status":"idle"}"#,
        );
        let cfg = vec![project("proj", "/home/user/proj")];
        let join = Join {
            pane_key: "herdr_pane".to_string(),
        };

        let joined = resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            &live,
            &cfg,
            &join,
        );

        assert_eq!(pane_of(&joined, "proj", "p-1").pane, "w:p1");
    }

    #[test]
    fn a_pane_naming_its_bead_resolves_as_inferred() {
        let beads = rows(BEADS);
        let live = parse_agent_list(PANES).expect("the fixture parses");
        let cfg = vec![project("beady-eye", FIXTURE_PROJECT_PATH)];

        let joined = resolve(
            &[ProjectRows {
                project: "beady-eye",
                rows: &beads,
            }],
            &live,
            &cfg,
            &Join::default(),
        );

        // wCW:p5 carries display_agent=bdi-3um.3; no bead in the tracker
        // names a pane, so the inferred direction is the only one that fires.
        let a = pane_of(&joined, "beady-eye", "bdi-3um.3");
        assert_eq!(a.pane, "wCW:p5");
        assert_eq!(a.source, JoinSource::DisplayAgent);
        assert_eq!(joined.agents.len(), 1);
        assert_eq!(joined.conflicts, vec![]);
    }

    /// The pane exists and its bead does not — that absence is `orphan-claim`'s
    /// to report, not a conflict.
    #[test]
    fn a_bead_naming_a_dead_pane_resolves_to_nothing() {
        let beads = rows(
            r#"[{"id":"p-1","title":"root","status":"in_progress","parent_id":"",
                 "metadata":{"agent_pane":"w:pGONE"}}]"#,
        );
        let live = panes(r#"{"pane_id":"w:p1","cwd":"/home/user/proj","agent_status":"idle"}"#);
        let cfg = vec![project("proj", "/home/user/proj")];

        let joined = resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            &live,
            &cfg,
            &Join::default(),
        );

        assert_eq!(joined.agents, BTreeMap::new());
        assert_eq!(joined.conflicts, vec![]);
    }

    /// `display_agent` is free text. Most of it is not a bead id.
    #[test]
    fn a_pane_whose_display_agent_names_no_bead_joins_nothing() {
        let beads = rows(r#"[{"id":"p-1","title":"root","status":"open","parent_id":""}]"#);
        let live = panes(
            r#"{"pane_id":"w:p1","cwd":"/home/user/proj","agent_status":"working",
                "display_agent":"orch: some-effort"}"#,
        );
        let cfg = vec![project("proj", "/home/user/proj")];

        let joined = resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            &live,
            &cfg,
            &Join::default(),
        );

        assert_eq!(joined.agents, BTreeMap::new());
        assert_eq!(joined.conflicts, vec![]);
        assert_eq!(loose(&live, &joined), vec!["w:p1"]);
    }

    // ---- project scope -------------------------------------------------

    #[test]
    fn the_longest_configured_path_containing_a_pane_wins() {
        let cfg = vec![
            project("outer", "/home/user/dev"),
            project("inner", "/home/user/dev/inner"),
        ];

        assert_eq!(
            project_of(Path::new("/home/user/dev/inner/src"), &cfg).map(|p| p.name.as_str()),
            Some("inner")
        );
        assert_eq!(
            project_of(Path::new("/home/user/dev/other"), &cfg).map(|p| p.name.as_str()),
            Some("outer")
        );
        assert_eq!(project_of(Path::new("/home/user"), &cfg), None);
    }

    /// One worktree per seat puts the panes in sibling worktrees, under
    /// neither each other nor the checkout bdi was run from.
    #[test]
    fn a_pane_in_another_worktree_of_the_repository_is_in_the_project() {
        let cfg = vec![project_working_in(
            "proj",
            "/home/user/proj",
            &["/home/user/proj", "/tmp/seat-a/wt"],
        )];

        assert_eq!(
            project_of(Path::new("/tmp/seat-a/wt/src"), &cfg).map(|p| p.name.as_str()),
            Some("proj")
        );
        assert_eq!(project_of(Path::new("/tmp/seat-b/wt"), &cfg), None);
    }

    /// A worktree is territory like any other, so the deepest directory
    /// containing the pane still wins the paths two projects share.
    #[test]
    fn a_worktree_deeper_than_another_projects_path_wins_the_pane() {
        let cfg = vec![
            project("outer", "/home/user/dev"),
            project_working_in("inner", "/srv/inner", &["/home/user/dev/wt"]),
        ];

        assert_eq!(
            project_of(Path::new("/home/user/dev/wt/src"), &cfg).map(|p| p.name.as_str()),
            Some("inner")
        );
    }

    /// A sibling directory sharing a textual prefix is a different project.
    #[test]
    fn a_path_is_matched_by_whole_directories_rather_than_by_text() {
        let cfg = vec![project("bead", "/home/user/dev/bead")];

        assert_eq!(project_of(Path::new("/home/user/dev/beady"), &cfg), None);
    }

    #[test]
    fn a_pane_in_no_configured_project_joins_nothing_and_is_unattributed() {
        let beads = rows(BEADS);
        let live = parse_agent_list(PANES).expect("the fixture parses");
        let cfg = vec![project("beady-eye", FIXTURE_PROJECT_PATH)];

        let joined = resolve(
            &[ProjectRows {
                project: "beady-eye",
                rows: &beads,
            }],
            &live,
            &cfg,
            &Join::default(),
        );

        // wCM:pN names bead nix-9670s.5 from an unconfigured summit-works, and
        // is as unattributed as the panes that named nothing at all.
        assert_eq!(
            loose(&live, &joined),
            vec![
                "wCF:p2", "wCF:pC", "wCM:p2", "wCM:pN", "wCW:p1", "wCW:p2", "wCW:p4", "wCW:p6",
                "wCY:p1",
            ]
        );
        assert_eq!(joined.conflicts, vec![]);
    }

    /// The reason the key is `(project, id)`: prefixes are per-tracker and
    /// nothing coordinates them.
    #[test]
    fn colliding_prefixes_do_not_cross_attach_an_inferred_agent() {
        let one = rows(
            r#"[{"id":"x-1","title":"in project one","status":"in_progress","parent_id":""}]"#,
        );
        let two = rows(
            r#"[{"id":"x-1","title":"in project two","status":"in_progress","parent_id":""}]"#,
        );
        let live = panes(
            r#"{"pane_id":"w:p1","cwd":"/home/user/one/src","agent_status":"working",
                "display_agent":"x-1"}"#,
        );
        let cfg = vec![
            project("one", "/home/user/one"),
            project("two", "/home/user/two"),
        ];

        let joined = resolve(
            &[
                ProjectRows {
                    project: "one",
                    rows: &one,
                },
                ProjectRows {
                    project: "two",
                    rows: &two,
                },
            ],
            &live,
            &cfg,
            &Join::default(),
        );

        assert_eq!(pane_of(&joined, "one", "x-1").pane, "w:p1");
        assert_eq!(
            joined.agents.len(),
            1,
            "project two's x-1 is a different bead"
        );
    }

    #[test]
    fn colliding_prefixes_do_not_cross_attach_an_exact_agent() {
        let one = rows(r#"[{"id":"x-1","title":"in project one","status":"open","parent_id":""}]"#);
        let two = rows(
            r#"[{"id":"x-1","title":"in project two","status":"in_progress","parent_id":"",
                 "metadata":{"agent_pane":"w:p1"}}]"#,
        );
        let live =
            panes(r#"{"pane_id":"w:p1","cwd":"/home/user/one/src","agent_status":"working"}"#);
        let cfg = vec![
            project("one", "/home/user/one"),
            project("two", "/home/user/two"),
        ];

        let joined = resolve(
            &[
                ProjectRows {
                    project: "one",
                    rows: &one,
                },
                ProjectRows {
                    project: "two",
                    rows: &two,
                },
            ],
            &live,
            &cfg,
            &Join::default(),
        );

        assert_eq!(
            joined.agents,
            BTreeMap::new(),
            "the pane is not project two's to claim"
        );
        assert_eq!(
            joined.conflicts,
            vec![Conflict::PaneInAnotherProject {
                bead: key("two", "x-1"),
                pane: "w:p1".to_string(),
                pane_project: Some("one".to_string()),
            }]
        );
        assert_eq!(
            joined.refused,
            BTreeMap::from([(
                key("two", "x-1"),
                Conflict::PaneInAnotherProject {
                    bead: key("two", "x-1"),
                    pane: "w:p1".to_string(),
                    pane_project: Some("one".to_string()),
                }
            )])
        );
    }

    /// A pane naming a bead that exists only in another project is the same
    /// finding from the other direction.
    #[test]
    fn a_pane_naming_another_projects_bead_does_not_join_and_is_reported() {
        let one = rows(r#"[{"id":"a-1","title":"only in one","status":"open","parent_id":""}]"#);
        let two = rows(r#"[{"id":"b-1","title":"only in two","status":"open","parent_id":""}]"#);
        let live = panes(
            r#"{"pane_id":"w:p1","cwd":"/home/user/two","agent_status":"working",
                "display_agent":"a-1"}"#,
        );
        let cfg = vec![
            project("one", "/home/user/one"),
            project("two", "/home/user/two"),
        ];

        let joined = resolve(
            &[
                ProjectRows {
                    project: "one",
                    rows: &one,
                },
                ProjectRows {
                    project: "two",
                    rows: &two,
                },
            ],
            &live,
            &cfg,
            &Join::default(),
        );

        assert_eq!(joined.agents, BTreeMap::new());
        assert_eq!(
            joined.conflicts,
            vec![Conflict::PaneInAnotherProject {
                bead: key("one", "a-1"),
                pane: "w:p1".to_string(),
                pane_project: Some("two".to_string()),
            }]
        );
        assert_eq!(
            joined.refused,
            BTreeMap::new(),
            "project one's bead named no pane, so it is owed no reason it has none: \
             what a pane in another project calls itself is not its claim"
        );
    }

    #[test]
    fn a_bead_naming_a_pane_in_no_configured_project_does_not_join_and_is_reported() {
        let beads = rows(
            r#"[{"id":"p-1","title":"root","status":"in_progress","parent_id":"",
                 "metadata":{"agent_pane":"w:p1"}}]"#,
        );
        let live = panes(r#"{"pane_id":"w:p1","cwd":"/tmp","agent_status":"working"}"#);
        let cfg = vec![project("proj", "/home/user/proj")];

        let joined = resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            &live,
            &cfg,
            &Join::default(),
        );

        assert_eq!(joined.agents, BTreeMap::new());
        let outside = Conflict::PaneInAnotherProject {
            bead: key("proj", "p-1"),
            pane: "w:p1".to_string(),
            pane_project: None,
        };
        assert_eq!(joined.conflicts, vec![outside.clone()]);
        assert_eq!(
            joined.refused,
            BTreeMap::from([(key("proj", "p-1"), outside)])
        );
    }

    // ---- the disagreements ---------------------------------------------

    #[test]
    fn the_two_directions_naming_different_panes_is_reported_and_the_beads_key_wins() {
        let beads = rows(
            r#"[{"id":"p-1","title":"root","status":"in_progress","parent_id":"",
                 "metadata":{"agent_pane":"w:p1"}}]"#,
        );
        let live = panes(
            r#"{"pane_id":"w:p1","cwd":"/home/user/proj","agent_status":"working"},
               {"pane_id":"w:p2","cwd":"/home/user/proj","agent_status":"idle",
                "display_agent":"p-1"}"#,
        );
        let cfg = vec![project("proj", "/home/user/proj")];

        let joined = resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            &live,
            &cfg,
            &Join::default(),
        );

        let a = pane_of(&joined, "proj", "p-1");
        assert_eq!(a.pane, "w:p1", "the bead's own key is exact and wins");
        assert_eq!(a.source, JoinSource::AgentPane);
        assert_eq!(
            joined.conflicts,
            vec![Conflict::BeadAndPaneDisagree {
                bead: key("proj", "p-1"),
                named_by_bead: "w:p1".to_string(),
                named_by_pane: "w:p2".to_string(),
            }],
            "winning is not the same as agreeing"
        );
    }

    /// Discrimination for the test above: both directions pointing at one pane
    /// is agreement, and agreement is not a finding.
    #[test]
    fn the_two_directions_naming_one_pane_is_not_a_conflict() {
        let beads = rows(
            r#"[{"id":"p-1","title":"root","status":"in_progress","parent_id":"",
                 "metadata":{"agent_pane":"w:p1"}}]"#,
        );
        let live = panes(
            r#"{"pane_id":"w:p1","cwd":"/home/user/proj","agent_status":"working",
                "display_agent":"p-1"}"#,
        );
        let cfg = vec![project("proj", "/home/user/proj")];

        let joined = resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            &live,
            &cfg,
            &Join::default(),
        );

        assert_eq!(
            pane_of(&joined, "proj", "p-1").source,
            JoinSource::AgentPane
        );
        assert_eq!(joined.conflicts, vec![]);
    }

    #[test]
    fn several_panes_naming_one_bead_leaves_it_unclaimed_and_reported() {
        let beads = rows(r#"[{"id":"p-1","title":"root","status":"in_progress","parent_id":""}]"#);
        let live = panes(
            r#"{"pane_id":"w:p2","cwd":"/home/user/proj","agent_status":"working",
                "display_agent":"p-1"},
               {"pane_id":"w:p1","cwd":"/home/user/proj","agent_status":"idle",
                "display_agent":"p-1"}"#,
        );
        let cfg = vec![project("proj", "/home/user/proj")];

        let joined = resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            &live,
            &cfg,
            &Join::default(),
        );

        assert_eq!(joined.agents, BTreeMap::new(), "neither pane wins");
        assert_eq!(
            joined.conflicts,
            vec![Conflict::SeveralPanesNameOneBead {
                bead: key("proj", "p-1"),
                panes: vec!["w:p1".to_string(), "w:p2".to_string()],
            }]
        );
        assert_eq!(loose(&live, &joined), vec!["w:p1", "w:p2"]);
    }

    #[test]
    fn several_beads_naming_one_pane_leaves_them_all_unclaimed_and_reported() {
        let beads = rows(
            r#"[
              {"id":"p-1","title":"root","status":"open","parent_id":""},
              {"id":"p-1.1","title":"one","status":"in_progress","parent_id":"p-1",
               "metadata":{"agent_pane":"w:p1"}},
              {"id":"p-1.2","title":"two","status":"in_progress","parent_id":"p-1",
               "metadata":{"agent_pane":"w:p1"}}
            ]"#,
        );
        let live = panes(r#"{"pane_id":"w:p1","cwd":"/home/user/proj","agent_status":"working"}"#);
        let cfg = vec![project("proj", "/home/user/proj")];

        let joined = resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            &live,
            &cfg,
            &Join::default(),
        );

        assert_eq!(joined.agents, BTreeMap::new(), "neither bead gets the pane");
        let contested = Conflict::SeveralBeadsNameOnePane {
            pane: "w:p1".to_string(),
            caption: None,
            beads: vec![key("proj", "p-1.1"), key("proj", "p-1.2")],
        };
        assert_eq!(joined.conflicts, vec![contested.clone()]);
        assert_eq!(
            joined.refused,
            BTreeMap::from([
                (key("proj", "p-1.1"), contested.clone()),
                (key("proj", "p-1.2"), contested),
            ]),
            "each of them claimed the pane, so each is owed the reason it has none"
        );
    }

    /// The reading this was measured from: one seat claimed three beads in
    /// turn and cleared its key on none of them, so three claims stood on the
    /// pane it was still sitting in. `bdi` awards it to none of them, and the
    /// only thing on the screen that tells the live claim from the two stale
    /// ones is the pane's own account of what it is working on — which the
    /// disagreement therefore carries. The session here is the one captured
    /// while it was happening.
    #[test]
    fn a_contested_pane_carries_its_own_account_of_what_it_is_working_on() {
        let beads = rows(
            r#"[
              {"id":"bdi-7ao","title":"bdi v1","status":"open","parent_id":""},
              {"id":"bdi-2bb.16","title":"a claim its seat moved on from",
               "status":"in_progress","parent_id":"bdi-7ao",
               "metadata":{"agent_pane":"wCW:p1P"}},
              {"id":"bdi-xey","title":"open the spine to every live agent",
               "status":"in_progress","parent_id":"bdi-7ao",
               "metadata":{"agent_pane":"wCW:p1P"}},
              {"id":"bdi-2bb.19","title":"the other claim it moved on from",
               "status":"in_progress","parent_id":"bdi-7ao",
               "metadata":{"agent_pane":"wCW:p1P"}}
            ]"#,
        );
        let live = parse_agent_list(JOINED_PANES).expect("the fixture parses");
        let cfg = vec![project("beady-eye", FIXTURE_PROJECT_PATH)];

        let joined = resolve(
            &[ProjectRows {
                project: "beady-eye",
                rows: &beads,
            }],
            &live,
            &cfg,
            &Join::default(),
        );

        assert_eq!(joined.agents, BTreeMap::new(), "none of the three gets it");
        assert_eq!(
            joined.conflicts,
            vec![Conflict::SeveralBeadsNameOnePane {
                pane: "wCW:p1P".to_string(),
                caption: Some("bdi-xey: open the spine to every live agent".to_string()),
                beads: vec![
                    key("beady-eye", "bdi-2bb.16"),
                    key("beady-eye", "bdi-2bb.19"),
                    key("beady-eye", "bdi-xey"),
                ],
            }]
        );
    }

    /// A pane with nothing to say about itself still contests, and the
    /// disagreement says what it can rather than inventing the rest.
    #[test]
    fn a_contested_pane_that_says_nothing_about_itself_carries_nothing() {
        let beads = rows(
            r#"[
              {"id":"p-1","title":"root","status":"open","parent_id":""},
              {"id":"p-1.1","title":"one","status":"in_progress","parent_id":"p-1",
               "metadata":{"agent_pane":"w:p1"}},
              {"id":"p-1.2","title":"two","status":"in_progress","parent_id":"p-1",
               "metadata":{"agent_pane":"w:p1"}}
            ]"#,
        );
        let live = panes(r#"{"pane_id":"w:p1","cwd":"/home/user/proj","agent_status":"working"}"#);
        let cfg = vec![project("proj", "/home/user/proj")];

        let joined = resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            &live,
            &cfg,
            &Join::default(),
        );

        assert_eq!(
            joined.conflicts,
            vec![Conflict::SeveralBeadsNameOnePane {
                pane: "w:p1".to_string(),
                caption: None,
                beads: vec![key("proj", "p-1.1"), key("proj", "p-1.2")],
            }]
        );
    }

    /// A pane contested by two beads is still a pane one of them might be
    /// working. The inferred direction fills the gap the void left; both
    /// findings stand.
    #[test]
    fn a_bead_whose_exact_claim_was_voided_still_takes_an_uncontested_inferred_pane() {
        let beads = rows(
            r#"[
              {"id":"p-1","title":"root","status":"open","parent_id":""},
              {"id":"p-1.1","title":"one","status":"in_progress","parent_id":"p-1",
               "metadata":{"agent_pane":"w:p1"}},
              {"id":"p-1.2","title":"two","status":"in_progress","parent_id":"p-1",
               "metadata":{"agent_pane":"w:p1"}}
            ]"#,
        );
        let live = panes(
            r#"{"pane_id":"w:p1","cwd":"/home/user/proj","agent_status":"working"},
               {"pane_id":"w:p9","cwd":"/home/user/proj","agent_status":"working",
                "display_agent":"p-1.1"}"#,
        );
        let cfg = vec![project("proj", "/home/user/proj")];

        let joined = resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            &live,
            &cfg,
            &Join::default(),
        );

        let a = pane_of(&joined, "proj", "p-1.1");
        assert_eq!(a.pane, "w:p9");
        assert_eq!(a.source, JoinSource::DisplayAgent);
        assert_eq!(
            joined.conflicts,
            vec![
                Conflict::BeadAndPaneDisagree {
                    bead: key("proj", "p-1.1"),
                    named_by_bead: "w:p1".to_string(),
                    named_by_pane: "w:p9".to_string(),
                },
                Conflict::SeveralBeadsNameOnePane {
                    pane: "w:p1".to_string(),
                    caption: None,
                    beads: vec![key("proj", "p-1.1"), key("proj", "p-1.2")],
                },
            ]
        );
    }

    // ---- badges ---------------------------------------------------------

    fn bead_with(metadata: &str) -> Bead {
        let json = format!(
            r#"[{{"id":"p-1","title":"root","status":"open","parent_id":"","metadata":{metadata}}}]"#
        );
        rows(&json).remove(0).bead
    }

    #[test]
    fn badges_render_only_where_the_key_and_match_agree() {
        let bead = bead_with(r#"{"blocked_on":"human","delivery_pr":"owner/repo#7"}"#);
        let cfg = vec![
            Badge {
                key: "blocked_on".into(),
                match_value: Some("human".into()),
                render: "waiting".into(),
            },
            Badge {
                key: "blocked_on".into(),
                match_value: Some("dependency".into()),
                render: "dep".into(),
            },
            Badge {
                key: "absent_key".into(),
                match_value: None,
                render: "never".into(),
            },
        ];

        let got = badges_for(&bead, &cfg);

        assert_eq!(
            got,
            vec![Badged {
                key: "blocked_on".to_string(),
                text: "waiting".to_string(),
            }]
        );
    }

    /// Nothing in the model learns what a metadata key means: a key it has
    /// never heard of renders exactly as well as a familiar one.
    #[test]
    fn badges_render_a_configured_key_without_interpreting_it() {
        let bead = bead_with(r#"{"xyzzy":"plugh"}"#);
        let cfg = vec![Badge {
            key: "xyzzy".into(),
            match_value: None,
            render: "→ {}".into(),
        }];

        let got = badges_for(&bead, &cfg);

        assert_eq!(
            got,
            vec![Badged {
                key: "xyzzy".to_string(),
                text: "→ plugh".to_string(),
            }]
        );
    }

    #[test]
    fn a_bead_with_no_configured_badges_renders_none() {
        let bead = bead_with(r#"{"blocked_on":"human"}"#);

        assert_eq!(badges_for(&bead, &[]), vec![]);
    }

    // ---- the contract ----------------------------------------------------

    #[test]
    fn the_join_source_serialises_the_way_the_contract_spells_it() {
        assert_eq!(
            serde_json::to_string(&JoinSource::AgentPane).unwrap(),
            r#""agent_pane""#
        );
        assert_eq!(
            serde_json::to_string(&JoinSource::DisplayAgent).unwrap(),
            r#""display_agent""#
        );
    }

    #[test]
    fn a_conflict_serialises_under_its_own_name() {
        let out = serde_json::to_string(&Conflict::SeveralPanesNameOneBead {
            bead: key("proj", "p-1"),
            panes: vec!["w:p1".to_string()],
        })
        .unwrap();

        assert_eq!(
            out,
            r#"{"conflict":"several-panes-name-one-bead","bead":{"project":"proj","id":"p-1"},"panes":["w:p1"]}"#
        );
    }
}
