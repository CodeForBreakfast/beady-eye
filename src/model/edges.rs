//! Every bead an answer ties a bead to, as `bd show` lists them: its parent,
//! what it depends on, and what it blocks.
//!
//! Read once for the whole answer rather than per tree, because what a bead
//! blocks is the reverse of an edge on some other bead, and that bead can sit
//! in another tree — or in no tree at all.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::model::tree::held_by_one_other;
use crate::model::types::{Bead, Edge, Status};

/// A bead an edge ties this one to: its id, and what the answer said of it
/// where the answer held its row. Both absent where it did not, which is a
/// dependency on work the tracker no longer holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Related {
    pub id: String,
    /// The project whose bead this is, where that is not the project of the
    /// bead naming it.
    pub project: Option<String>,
    pub edge: Edge,
    pub status: Option<Status>,
    pub title: Option<String>,
}

/// What ties one bead to the rest of the answer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Relations {
    /// The bead this one hangs under, by its parent-child edge.
    pub parent: Option<Related>,
    /// What this bead waits on: every dependency but the parent edge, in the
    /// order the row lists them.
    pub depends_on: Vec<Related>,
    /// What waits on this bead: every bead whose `blocks` edge names it, in
    /// id order.
    pub blocks: Vec<Related>,
}

/// What every bead in the answer is tied to.
pub fn relations(beads: &[Arc<Bead>]) -> BTreeMap<String, Relations> {
    let by_id: BTreeMap<&str, &Bead> = beads.iter().map(|b| (b.id.as_str(), &**b)).collect();
    let related = |id: &str, edge: Edge| {
        let row = by_id.get(id);
        Related {
            id: id.to_string(),
            project: None,
            edge,
            status: row.map(|bead| bead.status.clone()),
            title: row.map(|bead| bead.title.clone()),
        }
    };

    let mut relations: BTreeMap<String, Relations> = beads
        .iter()
        .map(|bead| {
            let mut ties = Relations::default();
            for dependency in &bead.dependencies {
                let tied = related(&dependency.on, dependency.edge.clone());
                match dependency.edge {
                    Edge::ParentChild => ties.parent = Some(tied),
                    Edge::Blocks | Edge::Other(_) => ties.depends_on.push(tied),
                }
            }
            (bead.id.clone(), ties)
        })
        .collect();

    for bead in by_id.values() {
        for dependency in &bead.dependencies {
            if dependency.edge != Edge::Blocks {
                continue;
            }
            if let Some(blocker) = relations.get_mut(&dependency.on) {
                blocker.blocks.push(related(&bead.id, Edge::Blocks));
            }
        }
    }

    relations
}

/// The beads waiting on a bead in another project, by that project and then
/// the bead's id.
///
/// bd reads no edge to another project's bead, so a project's answer never
/// lists a waiter in another project under what a bead blocks. The waiter's
/// own answer names the blocker, and only an id its answer lacks, which
/// exactly one other answer holds, is that other project's bead — the rule
/// the tree uses to hang the blocker beneath it.
pub fn waiting_across(
    answers: &[(&str, &[Arc<Bead>])],
) -> BTreeMap<String, BTreeMap<String, Vec<Related>>> {
    let mut holding: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (at, (_, beads)) in answers.iter().enumerate() {
        for bead in *beads {
            holding.entry(bead.id.as_str()).or_default().push(at);
        }
    }

    let mut waiting: BTreeMap<String, BTreeMap<String, Vec<Related>>> = BTreeMap::new();
    for (at, (project, beads)) in answers.iter().enumerate() {
        for bead in *beads {
            for dependency in &bead.dependencies {
                if dependency.edge != Edge::Blocks {
                    continue;
                }
                let Some(held_by) = held_by_one_other(&holding, at, &dependency.on) else {
                    continue;
                };
                waiting
                    .entry(answers[held_by].0.to_string())
                    .or_default()
                    .entry(dependency.on.clone())
                    .or_default()
                    .push(Related {
                        id: bead.id.clone(),
                        project: Some((*project).to_string()),
                        edge: Edge::Blocks,
                        status: Some(bead.status.clone()),
                        title: Some(bead.title.clone()),
                    });
            }
        }
    }
    for waiters in waiting.values_mut().flat_map(BTreeMap::values_mut) {
        waiters.sort_by(|a, b| (&a.project, &a.id).cmp(&(&b.project, &b.id)));
    }
    waiting
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::bd::parse_shared_beads;
    use pretty_assertions::assert_eq;

    /// `dun-3.1` hangs under `dun-3` and waits on `dun-3.2`, which is closed;
    /// `dun-3.2` also waits on `dun-9`, which the answer does not hold.
    const BEADS: &str = r#"[
      {"id":"dun-3","title":"the root","status":"in_progress"},
      {"id":"dun-3.1","title":"the waiting one","status":"open",
       "dependencies":[{"depends_on_id":"dun-3","type":"parent-child"},
                       {"depends_on_id":"dun-3.2","type":"blocks"}]},
      {"id":"dun-3.2","title":"the one waited on","status":"closed",
       "dependencies":[{"depends_on_id":"dun-3","type":"parent-child"},
                       {"depends_on_id":"dun-9","type":"blocks"}]}
    ]"#;

    fn tied() -> BTreeMap<String, Relations> {
        relations(&parse_shared_beads(BEADS).expect("the rows parse"))
    }

    fn held(id: &str, edge: Edge, status: Status, title: &str) -> Related {
        Related {
            id: id.to_string(),
            project: None,
            edge,
            status: Some(status),
            title: Some(title.to_string()),
        }
    }

    #[test]
    fn the_parent_is_read_off_the_parent_child_edge_with_its_status_and_title() {
        assert_eq!(
            tied()["dun-3.1"].parent,
            Some(held(
                "dun-3",
                Edge::ParentChild,
                Status::InProgress,
                "the root"
            ))
        );
        assert_eq!(tied()["dun-3"].parent, None, "the root hangs under nothing");
    }

    #[test]
    fn what_a_bead_depends_on_is_every_edge_but_the_parent() {
        assert_eq!(
            tied()["dun-3.1"].depends_on,
            vec![held(
                "dun-3.2",
                Edge::Blocks,
                Status::Closed,
                "the one waited on"
            )]
        );
        assert_eq!(tied()["dun-3"].depends_on, vec![]);
    }

    /// Degrade, never disappear: a dependency on a bead the tracker no longer
    /// holds is still a dependency, and the id is all the answer has of it.
    #[test]
    fn a_dependency_the_answer_does_not_hold_keeps_its_id_and_nothing_else() {
        assert_eq!(
            tied()["dun-3.2"].depends_on,
            vec![Related {
                id: "dun-9".to_string(),
                project: None,
                edge: Edge::Blocks,
                status: None,
                title: None,
            }]
        );
    }

    /// The reverse edge, which no row carries: what waits on this bead is
    /// found on the beads that wait.
    #[test]
    fn what_a_bead_blocks_is_read_off_the_beads_that_wait_on_it() {
        assert_eq!(
            tied()["dun-3.2"].blocks,
            vec![held(
                "dun-3.1",
                Edge::Blocks,
                Status::Open,
                "the waiting one"
            )]
        );
        assert_eq!(
            tied()["dun-3"].blocks,
            vec![],
            "a parent is waited on by nothing; its children hang under it"
        );
    }

    #[test]
    fn what_a_bead_blocks_is_in_id_order_however_the_answer_was_ordered() {
        let rows = r#"[
          {"id":"dun-4","title":"waited on","status":"open"},
          {"id":"dun-4.9","title":"last","status":"open",
           "dependencies":[{"depends_on_id":"dun-4","type":"blocks"}]},
          {"id":"dun-4.1","title":"first","status":"open",
           "dependencies":[{"depends_on_id":"dun-4","type":"blocks"}]}
        ]"#;
        let tied = relations(&parse_shared_beads(rows).expect("the rows parse"));

        assert_eq!(
            tied["dun-4"]
                .blocks
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            ["dun-4.1", "dun-4.9"]
        );
    }

    fn waiting_among(answers: &[(&str, &str)]) -> BTreeMap<String, BTreeMap<String, Vec<Related>>> {
        let parsed: Vec<(&str, Vec<Arc<Bead>>)> = answers
            .iter()
            .map(|(project, rows)| (*project, parse_shared_beads(rows).expect("the rows parse")))
            .collect();
        let borrowed: Vec<(&str, &[Arc<Bead>])> = parsed
            .iter()
            .map(|(project, beads)| (*project, beads.as_slice()))
            .collect();
        waiting_across(&borrowed)
    }

    const FERRY_WAITING: &str = r#"[
      {"id":"fer-2","title":"moor the barge","status":"open",
       "dependencies":[{"depends_on_id":"dun-7","type":"blocks"}]}
    ]"#;

    const DUNWICH_WAITED_ON: &str = r#"[
      {"id":"dun-7","title":"the one waited on","status":"open"}
    ]"#;

    /// What ferry's answer names and only dunwich's holds is a bead of
    /// dunwich's, and the bead naming it is listed under it as ferry's.
    #[test]
    fn a_bead_naming_only_another_answers_bead_is_listed_under_that_bead() {
        let waiting = waiting_among(&[("ferry", FERRY_WAITING), ("dunwich", DUNWICH_WAITED_ON)]);

        assert_eq!(
            waiting["dunwich"]["dun-7"],
            vec![Related {
                id: "fer-2".to_string(),
                project: Some("ferry".to_string()),
                edge: Edge::Blocks,
                status: Some(Status::Open),
                title: Some("moor the barge".to_string()),
            }]
        );
        assert_eq!(waiting.len(), 1, "ferry's bead is waited on by nothing");
    }

    /// Prefixes are uncoordinated: the edge is to the answer's own bead
    /// where it holds one, and to nothing where several others do.
    #[test]
    fn an_id_the_waiters_answer_holds_or_several_others_hold_names_no_other_projects_bead() {
        let ferry_holding_a_dun_7 = r#"[
          {"id":"dun-7","title":"ferry's own","status":"open"},
          {"id":"fer-2","title":"moor the barge","status":"open",
           "dependencies":[{"depends_on_id":"dun-7","type":"blocks"}]}
        ]"#;
        assert_eq!(
            waiting_among(&[
                ("ferry", ferry_holding_a_dun_7),
                ("dunwich", DUNWICH_WAITED_ON)
            ]),
            BTreeMap::new(),
            "ferry holds a dun-7 of its own"
        );
        assert_eq!(
            waiting_among(&[
                ("ferry", FERRY_WAITING),
                ("dunwich", DUNWICH_WAITED_ON),
                ("harbour", DUNWICH_WAITED_ON),
            ]),
            BTreeMap::new(),
            "two answers hold a dun-7, and nothing says which is meant"
        );
    }

    /// Only a blocks edge says the other bead is waited on.
    #[test]
    fn an_edge_that_is_not_a_blocks_edge_lists_no_waiter_in_another_project() {
        let ferry = r#"[
          {"id":"fer-2","title":"moor the barge","status":"open",
           "dependencies":[{"depends_on_id":"dun-7","type":"relates-to"}]}
        ]"#;

        assert_eq!(
            waiting_among(&[("ferry", ferry), ("dunwich", DUNWICH_WAITED_ON)]),
            BTreeMap::new()
        );
    }

    /// An edge kind beads may add later is still something the bead waits
    /// on, and the kind is kept so the screen can say which.
    #[test]
    fn an_edge_of_a_kind_bdi_does_not_know_is_kept_with_its_kind() {
        let rows = r#"[
          {"id":"dun-5","title":"one","status":"open"},
          {"id":"dun-5.1","title":"two","status":"open",
           "dependencies":[{"depends_on_id":"dun-5","type":"relates-to"}]}
        ]"#;
        let tied = relations(&parse_shared_beads(rows).expect("the rows parse"));

        assert_eq!(
            tied["dun-5.1"].depends_on[0].edge,
            Edge::Other("relates-to".to_string())
        );
        assert_eq!(
            tied["dun-5"].blocks,
            vec![],
            "only a blocks edge says the other bead is waited on"
        );
    }
}
