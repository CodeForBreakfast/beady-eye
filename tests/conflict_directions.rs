//! The two questions one `Conflict` is asked, crossed.
//!
//! A bead's row asks *which disagreement took my claim*, and carries the
//! answer in `Anomaly::OrphanClaim`'s `refused` so the row can say why it has
//! no pane. The tail asks the opposite question of that same disagreement —
//! *which pane do you name* — so `⏎` and the band under the forest have
//! somewhere to go. The two are built in different directions by different
//! code, and nothing between them holds them to one story.
//!
//! So each reading here goes the whole way `bdi` goes, from tracker rows and a
//! herdr session down to the lines on screen, and the tests cross the answers:
//! wherever a disagreement refused a bead's claim, the pane the tail points at
//! on that disagreement's row is the pane that bead's own key named.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use beady_eye::collect::bd::parse_beads;
use beady_eye::collect::herdr::parse_agent_list;
use beady_eye::config::Config;
use beady_eye::model::anomaly::Anomaly;
use beady_eye::model::join::{resolve, BeadKey, Conflict, ProjectRows};
use beady_eye::model::snapshot::{self, Collected, Filter, HerdrState, Readiness, Snapshot};
use beady_eye::model::tree::Nesting;
use beady_eye::view::forest;
use beady_eye::view::lines::{Content, Item};
use beady_eye::view::tail;
use beady_eye::view::{Action, Motion};
use chrono::{DateTime, Utc};
use pretty_assertions::assert_eq;

const CONFIG: &str = r#"
[[projects]]
name = "orbital"
path = "/srv/orbital"
credential_command = "echo orbital"

[[projects]]
name = "ferry"
path = "/srv/ferry"
credential_command = "echo ferry"
"#;

/// Every working tree of each project's repository, as git would have listed
/// them. `ferry` keeps one inside `orbital`'s own checkout, so the ranking
/// that gives a shared path to the deepest tree has something to rank.
const WORKING_TREES: [(&str, &[&str]); 2] = [
    ("orbital", &["/srv/orbital", "/srv/wt/orbital-lift"]),
    ("ferry", &["/srv/ferry", "/srv/orbital/vendor/ferry"]),
];

/// The other tracker exists so a pane can sit in a project that is not the one
/// claiming it. Nothing in it claims anything.
const FERRY_ROWS: &str = r#"[
  {"id":"fry-1","title":"run the ferry","status":"open"}
]"#;

fn config() -> Config {
    let mut cfg = Config::from_toml(CONFIG).expect("the config parses");
    // Working trees are discovered from git rather than written by hand, so
    // they are filled in here the way discovery fills them.
    for project in &mut cfg.projects {
        let (_, trees) = WORKING_TREES
            .iter()
            .find(|(name, _)| *name == project.name)
            .expect("every configured project has its working trees listed");
        project.worktrees = trees.iter().map(PathBuf::from).collect();
    }
    cfg
}

fn now() -> DateTime<Utc> {
    "2026-08-30T12:00:00Z".parse().expect("the instant parses")
}

/// One reading of both trackers and the herdr session, taken to the snapshot
/// the screen is drawn from.
struct Reading {
    snapshot: Snapshot,
    /// The pane each bead's own key named, read back off the tracker rows
    /// rather than off anything the join produced. This is the independent
    /// side of the crossing: the join gets no say in what a claim was.
    claimed: BTreeMap<BeadKey, String>,
}

fn read(orbital_rows: &str, agents: &str) -> Reading {
    let cfg = config();
    let assembled: Vec<(&str, _)> = [("orbital", orbital_rows), ("ferry", FERRY_ROWS)]
        .into_iter()
        .map(|(project, rows)| {
            let beads = parse_beads(rows).expect("the rows parse");
            let root = beads
                .iter()
                .find(|b| b.dependencies.is_empty())
                .expect("a root row")
                .id
                .clone();
            (
                project,
                Nesting::of(&beads)
                    .assemble(&root)
                    .expect("the rows assemble"),
            )
        })
        .collect();

    let panes = parse_agent_list(&format!(
        r#"{{"id":"cli:agent:list","result":{{"agents":[{agents}]}}}}"#
    ))
    .expect("the panes parse");

    let trees: Vec<ProjectRows> = assembled
        .iter()
        .map(|(project, a)| ProjectRows {
            project,
            rows: &a.beads,
        })
        .collect();
    let joined = resolve(&trees, &panes, &cfg);

    let claimed = assembled
        .iter()
        .flat_map(|(project, a)| {
            a.beads.iter().filter_map(move |bead| {
                Some((
                    BeadKey {
                        project: (*project).to_string(),
                        id: bead.id.clone(),
                    },
                    bead.metadata.get("agent_pane")?.clone(),
                ))
            })
        })
        .collect();

    let collected = Collected {
        trees: assembled
            .iter()
            .map(|(project, a)| {
                snapshot::build_tree(
                    project,
                    a,
                    &joined,
                    &Readiness::default(),
                    &BTreeMap::new(),
                    &cfg,
                    now(),
                )
            })
            .collect(),
        failed_projects: Vec::new(),
        ..Default::default()
    };

    Reading {
        snapshot: snapshot::build(
            collected,
            &panes,
            &joined,
            &cfg,
            HerdrState::Ok,
            Filter::All,
            now(),
        ),
        claimed,
    }
}

/// Every bead whose own row says a disagreement took its claim, with that
/// disagreement. The bead → conflict direction, read where a reader reads it.
fn refusals(snapshot: &Snapshot) -> Vec<(BeadKey, Conflict)> {
    snapshot
        .trees
        .iter()
        .flat_map(|tree| {
            tree.beads.iter().flat_map(move |node| {
                node.anomalies
                    .iter()
                    .filter_map(move |anomaly| match anomaly {
                        Anomaly::OrphanClaim {
                            refused: Some(conflict),
                        } => Some((
                            BeadKey {
                                project: tree.project.clone(),
                                id: node.id.clone(),
                            },
                            conflict.clone(),
                        )),
                        _ => None,
                    })
            })
        })
        .collect()
}

/// The pane the tail points at with the selection on this disagreement's row
/// in the conflicts group. The conflict → pane direction, read the same way.
fn pane_the_tail_points_at(snapshot: &Snapshot, conflict: &Conflict) -> Option<String> {
    let mut forest = forest::flatten(snapshot.clone());
    let at = forest
        .lines()
        .iter()
        .position(|line| matches!(&line.content, Content::Item(Item::Conflict(c)) if c == conflict))
        .unwrap_or_else(|| panic!("{conflict:?} has a row of its own in the conflicts group"));

    forest.apply(Action::Move(Motion::FirstRow));
    for _ in 0..at {
        forest.apply(Action::Move(Motion::NextRow));
    }
    assert_eq!(
        forest.selected_line(),
        at,
        "the selection reached the disagreement's row"
    );

    tail::target(&forest).pane().map(str::to_string)
}

fn arm(conflict: &Conflict) -> &'static str {
    match conflict {
        Conflict::BeadAndPaneDisagree { .. } => "bead-and-pane-disagree",
        Conflict::SeveralPanesNameOneBead { .. } => "several-panes-name-one-bead",
        Conflict::SeveralBeadsNameOnePane { .. } => "several-beads-name-one-pane",
        Conflict::PaneInAnotherProject { .. } => "pane-in-another-project",
    }
}

/// Four claims on panes outside the project claiming them, and one on a pane
/// in another working tree of the project's own repository, which is not
/// outside it at all.
fn a_claim_reaching_out_of_its_project() -> Reading {
    read(
        r#"[
          {"id":"orb-1","title":"lift the ground station","status":"open"},
          {"id":"orb-1.1","title":"its pane sits in the ferry's checkout",
           "status":"in_progress",
           "dependencies":[{"depends_on_id":"orb-1","type":"parent-child"}],
           "metadata":{"agent_pane":"w:p1"}},
          {"id":"orb-1.2","title":"its pane sits under no configured project",
           "status":"in_progress",
           "dependencies":[{"depends_on_id":"orb-1","type":"parent-child"}],
           "metadata":{"agent_pane":"w:p2"}},
          {"id":"orb-1.3","title":"its pane sits in another tree of its own repository",
           "status":"in_progress",
           "dependencies":[{"depends_on_id":"orb-1","type":"parent-child"}],
           "metadata":{"agent_pane":"w:p3"}},
          {"id":"orb-1.4","title":"its pane sits in the ferry's tree inside orbital's",
           "status":"in_progress",
           "dependencies":[{"depends_on_id":"orb-1","type":"parent-child"}],
           "metadata":{"agent_pane":"w:p4"}}
        ]"#,
        r#"{"pane_id":"w:p1","cwd":"/srv/ferry/src","agent_status":"working"},
           {"pane_id":"w:p2","cwd":"/tmp/nowhere","agent_status":"idle"},
           {"pane_id":"w:p3","cwd":"/srv/wt/orbital-lift/src","agent_status":"working"},
           {"pane_id":"w:p4","cwd":"/srv/orbital/vendor/ferry/src","agent_status":"idle"}"#,
    )
}

/// Two beads of one tracker naming one pane. Neither gets it.
fn two_claims_on_one_pane() -> Reading {
    read(
        r#"[
          {"id":"orb-2","title":"lay the feeder cable","status":"open"},
          {"id":"orb-2.1","title":"one of two claims on the same pane",
           "status":"in_progress",
           "dependencies":[{"depends_on_id":"orb-2","type":"parent-child"}],
           "metadata":{"agent_pane":"w:p1"}},
          {"id":"orb-2.2","title":"the other","status":"in_progress",
           "dependencies":[{"depends_on_id":"orb-2","type":"parent-child"}],
           "metadata":{"agent_pane":"w:p1"}}
        ]"#,
        r#"{"pane_id":"w:p1","cwd":"/srv/orbital/src","agent_status":"working"}"#,
    )
}

/// The two disagreements that refuse nobody's claim: a bead and a pane naming
/// each other's opposite number, and two panes naming one bead.
fn disagreements_that_refuse_no_claim() -> Reading {
    read(
        r#"[
          {"id":"orb-3","title":"survey the mast","status":"open"},
          {"id":"orb-3.1","title":"names one pane while another names it",
           "status":"in_progress",
           "dependencies":[{"depends_on_id":"orb-3","type":"parent-child"}],
           "metadata":{"agent_pane":"w:p1"}},
          {"id":"orb-3.2","title":"two panes name it and it names none",
           "status":"in_progress","dependencies":[{"depends_on_id":"orb-3","type":"parent-child"}]}
        ]"#,
        r#"{"pane_id":"w:p1","cwd":"/srv/orbital/src","agent_status":"working"},
           {"pane_id":"w:p2","cwd":"/srv/orbital/src","agent_status":"idle",
            "display_agent":"orb-3.1"},
           {"pane_id":"w:p3","cwd":"/srv/orbital/src","agent_status":"idle",
            "display_agent":"orb-3.2"},
           {"pane_id":"w:p4","cwd":"/srv/orbital/src","agent_status":"idle",
            "display_agent":"orb-3.2"}"#,
    )
}

fn readings() -> Vec<Reading> {
    vec![
        a_claim_reaching_out_of_its_project(),
        two_claims_on_one_pane(),
        disagreements_that_refuse_no_claim(),
    ]
}

/// The crossing itself. A row that says a disagreement took this bead's claim
/// sends the reader to that disagreement; the tail must then put them in front
/// of the pane the claim was for, and not some other pane the disagreement
/// happens to mention.
#[test]
fn a_disagreement_that_took_a_claim_names_the_pane_that_claim_named() {
    let mut crossed = 0;
    for reading in readings() {
        for (bead, refused) in refusals(&reading.snapshot) {
            // A refusal is a signpost, so both halves of it have to hold: it
            // must point somewhere, and where it points must be the pane the
            // refused claim was for.
            let points_at = pane_the_tail_points_at(&reading.snapshot, &refused);
            assert_eq!(
                points_at,
                reading.claimed.get(&bead).cloned(),
                "{}/{} says {} took its claim, so the tail on that disagreement's \
                 row must reach the pane the bead's own key named",
                bead.project,
                bead.id,
                arm(&refused)
            );
            assert!(
                points_at.is_some(),
                "{}/{} says {} took its claim, so its row sends a reader to that \
                 disagreement — and the tail there reaches no pane at all",
                bead.project,
                bead.id,
                arm(&refused)
            );
            crossed += 1;
        }
    }
    assert_eq!(
        crossed, 5,
        "the readings refuse five claims between them, so a crossing that checked \
         fewer than five checked nothing"
    );
}

/// The other half of the biconditional. These two disagreements name no single
/// pane, so the tail has nowhere to send a reader on their rows — which is
/// only safe for as long as neither of them takes a bead's claim.
#[test]
fn a_disagreement_the_tail_cannot_point_at_takes_no_bead_s_claim() {
    let reading = disagreements_that_refuse_no_claim();

    let unpointable: Vec<&Conflict> = reading
        .snapshot
        .conflicts
        .iter()
        .filter(|c| pane_the_tail_points_at(&reading.snapshot, c).is_none())
        .collect();

    assert_eq!(
        unpointable.iter().map(|c| arm(c)).collect::<Vec<_>>(),
        ["bead-and-pane-disagree", "several-panes-name-one-bead"],
        "these are the arms that name no one pane"
    );
    assert_eq!(
        refusals(&reading.snapshot),
        vec![],
        "and none of them may be recorded as the reason a bead lost its pane"
    );
}

/// A guard on the two tests above rather than a property of its own: neither
/// says anything about an arm no reading produced.
#[test]
fn every_arm_of_a_disagreement_is_asked_both_questions() {
    let seen: BTreeSet<&str> = readings()
        .iter()
        .flat_map(|reading| reading.snapshot.conflicts.iter().map(arm))
        .collect();

    assert_eq!(
        seen,
        BTreeSet::from([
            "bead-and-pane-disagree",
            "pane-in-another-project",
            "several-beads-name-one-pane",
            "several-panes-name-one-bead",
        ])
    );
}
