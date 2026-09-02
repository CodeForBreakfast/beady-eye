//! Every bead an answer ties a bead to, as `bd show` lists them: its parent,
//! what it depends on, and what it blocks.
//!
//! Read once for the whole answer rather than per tree, because what a bead
//! blocks is the reverse of an edge on some other bead, and that bead can sit
//! in another tree — or in no tree at all.

use std::collections::BTreeMap;

use crate::model::types::{Bead, Edge, Status};

/// A bead an edge ties this one to: its id, and what the answer said of it
/// where the answer held its row. Both absent where it did not, which is a
/// dependency on work the tracker no longer holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Related {
    pub id: String,
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
pub fn relations(beads: &[Bead]) -> BTreeMap<String, Relations> {
    let by_id: BTreeMap<&str, &Bead> = beads.iter().map(|b| (b.id.as_str(), b)).collect();
    let related = |id: &str, edge: Edge| {
        let row = by_id.get(id);
        Related {
            id: id.to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::bd::parse_beads;
    use pretty_assertions::assert_eq;

    /// `orb-3.1` hangs under `orb-3` and waits on `orb-3.2`, which is closed;
    /// `orb-3.2` also waits on `orb-9`, which the answer does not hold.
    const BEADS: &str = r#"[
      {"id":"orb-3","title":"the root","status":"in_progress"},
      {"id":"orb-3.1","title":"the waiting one","status":"open",
       "dependencies":[{"depends_on_id":"orb-3","type":"parent-child"},
                       {"depends_on_id":"orb-3.2","type":"blocks"}]},
      {"id":"orb-3.2","title":"the one waited on","status":"closed",
       "dependencies":[{"depends_on_id":"orb-3","type":"parent-child"},
                       {"depends_on_id":"orb-9","type":"blocks"}]}
    ]"#;

    fn tied() -> BTreeMap<String, Relations> {
        relations(&parse_beads(BEADS).expect("the rows parse"))
    }

    fn held(id: &str, edge: Edge, status: Status, title: &str) -> Related {
        Related {
            id: id.to_string(),
            edge,
            status: Some(status),
            title: Some(title.to_string()),
        }
    }

    #[test]
    fn the_parent_is_read_off_the_parent_child_edge_with_its_status_and_title() {
        assert_eq!(
            tied()["orb-3.1"].parent,
            Some(held(
                "orb-3",
                Edge::ParentChild,
                Status::InProgress,
                "the root"
            ))
        );
        assert_eq!(tied()["orb-3"].parent, None, "the root hangs under nothing");
    }

    #[test]
    fn what_a_bead_depends_on_is_every_edge_but_the_parent() {
        assert_eq!(
            tied()["orb-3.1"].depends_on,
            vec![held(
                "orb-3.2",
                Edge::Blocks,
                Status::Closed,
                "the one waited on"
            )]
        );
        assert_eq!(tied()["orb-3"].depends_on, vec![]);
    }

    /// Degrade, never disappear: a dependency on a bead the tracker no longer
    /// holds is still a dependency, and the id is all the answer has of it.
    #[test]
    fn a_dependency_the_answer_does_not_hold_keeps_its_id_and_nothing_else() {
        assert_eq!(
            tied()["orb-3.2"].depends_on,
            vec![Related {
                id: "orb-9".to_string(),
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
            tied()["orb-3.2"].blocks,
            vec![held(
                "orb-3.1",
                Edge::Blocks,
                Status::Open,
                "the waiting one"
            )]
        );
        assert_eq!(
            tied()["orb-3"].blocks,
            vec![],
            "a parent is waited on by nothing; its children hang under it"
        );
    }

    #[test]
    fn what_a_bead_blocks_is_in_id_order_however_the_answer_was_ordered() {
        let rows = r#"[
          {"id":"orb-4","title":"waited on","status":"open"},
          {"id":"orb-4.9","title":"last","status":"open",
           "dependencies":[{"depends_on_id":"orb-4","type":"blocks"}]},
          {"id":"orb-4.1","title":"first","status":"open",
           "dependencies":[{"depends_on_id":"orb-4","type":"blocks"}]}
        ]"#;
        let tied = relations(&parse_beads(rows).expect("the rows parse"));

        assert_eq!(
            tied["orb-4"]
                .blocks
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            ["orb-4.1", "orb-4.9"]
        );
    }

    /// An edge kind beads may add later is still something the bead waits
    /// on, and the kind is kept so the screen can say which.
    #[test]
    fn an_edge_of_a_kind_bdi_does_not_know_is_kept_with_its_kind() {
        let rows = r#"[
          {"id":"orb-5","title":"one","status":"open"},
          {"id":"orb-5.1","title":"two","status":"open",
           "dependencies":[{"depends_on_id":"orb-5","type":"relates-to"}]}
        ]"#;
        let tied = relations(&parse_beads(rows).expect("the rows parse"));

        assert_eq!(
            tied["orb-5.1"].depends_on[0].edge,
            Edge::Other("relates-to".to_string())
        );
        assert_eq!(
            tied["orb-5"].blocks,
            vec![],
            "only a blocks edge says the other bead is waited on"
        );
    }
}
