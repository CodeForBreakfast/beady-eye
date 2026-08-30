use std::collections::{BTreeMap, HashSet};

use anyhow::bail;

use crate::model::types::Bead;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub bead: Bead,
    pub depth: u16,
}

/// A flat set of bd rows in render order, with every departure from a clean
/// tree named rather than dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assembled {
    pub rows: Vec<Placed>,
    /// Ids whose declared parent is not among the rows. Each is kept in `rows`,
    /// re-parented onto the root.
    pub dangling: Vec<String>,
    /// Ids no walk down from the root reaches, because their parent chain
    /// loops. Each is kept in `rows`, its component hanging off the root.
    pub unreachable: Vec<String>,
}

/// Order a flat set of bd rows into render order.
///
/// bd has already resolved and deduplicated the tree, so this re-parents by
/// `parent_id` and sorts siblings; it does not walk dependency edges. Depth is
/// counted from the parent chain rather than taken from bd, which flattens it
/// under `--max-depth`.
///
/// Siblings sort by state, then priority, then id.
pub fn assemble(beads: Vec<Bead>) -> anyhow::Result<Assembled> {
    let roots: Vec<&str> = beads
        .iter()
        .filter(|b| b.parent_id.is_none())
        .map(|b| b.id.as_str())
        .collect();

    let root_id = match roots.as_slice() {
        [only] => only.to_string(),
        [] => bail!("bd returned no root row; a dependency tree has exactly one"),
        several => bail!(
            "bd returned {} root rows ({}); a dependency tree has exactly one",
            several.len(),
            several.join(", ")
        ),
    };

    let present: HashSet<&str> = beads.iter().map(|b| b.id.as_str()).collect();
    let mut dangling: Vec<String> = beads
        .iter()
        .filter(|b| {
            b.parent_id
                .as_deref()
                .is_some_and(|parent| !present.contains(parent))
        })
        .map(|b| b.id.clone())
        .collect();
    dangling.sort();

    let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for bead in &beads {
        if bead.id == root_id {
            continue;
        }
        let parent = match bead.parent_id.as_deref() {
            Some(parent) if present.contains(parent) => parent,
            _ => &root_id,
        };
        children
            .entry(parent.to_string())
            .or_default()
            .push(bead.id.clone());
    }

    let by_id: BTreeMap<String, Bead> = beads.into_iter().map(|b| (b.id.clone(), b)).collect();
    for siblings in children.values_mut() {
        siblings.sort_by(|a, b| {
            let (a, b) = (&by_id[a], &by_id[b]);
            a.status
                .rank()
                .cmp(&b.status.rank())
                .then(a.priority.cmp(&b.priority))
                .then_with(|| a.id.cmp(&b.id))
        });
    }

    let mut rows = Vec::new();
    let mut walked = HashSet::new();
    walk(&root_id, 0, &children, &by_id, &mut walked, &mut rows);

    let unreachable: Vec<String> = by_id
        .keys()
        .filter(|id| !walked.contains(id.as_str()))
        .cloned()
        .collect();
    for id in &unreachable {
        walk(id, 1, &children, &by_id, &mut walked, &mut rows);
    }

    Ok(Assembled {
        rows,
        dangling,
        unreachable,
    })
}

/// Emit a bead and everything below it, depth-first so a child sits under its
/// own parent rather than after the next sibling. The visited set is what stops
/// a parent cycle, which no walk from the root can reach but the unreachable
/// pass does.
fn walk(
    id: &str,
    depth: u16,
    children: &BTreeMap<String, Vec<String>>,
    by_id: &BTreeMap<String, Bead>,
    walked: &mut HashSet<String>,
    out: &mut Vec<Placed>,
) {
    if !walked.insert(id.to_string()) {
        return;
    }
    out.push(Placed {
        bead: by_id[id].clone(),
        depth,
    });

    for child in children.get(id).into_iter().flatten() {
        walk(child, depth + 1, children, by_id, walked, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::bd::parse_dep_tree;

    const FIXTURE: &str = include_str!("../../tests/fixtures/bd_dep_tree.json");

    fn assembled(json: &str) -> Assembled {
        assemble(parse_dep_tree(json).expect("the rows parse")).expect("the rows assemble")
    }

    fn ids(a: &Assembled) -> Vec<&str> {
        a.rows.iter().map(|p| p.bead.id.as_str()).collect()
    }

    fn depth_of(a: &Assembled, id: &str) -> u16 {
        a.rows
            .iter()
            .find(|p| p.bead.id == id)
            .unwrap_or_else(|| panic!("{id} is in the rows"))
            .depth
    }

    #[test]
    fn every_bead_appears_exactly_once() {
        let a = assembled(FIXTURE);
        assert_eq!(a.rows.len(), 6);

        let mut seen = ids(&a);
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 6, "a bead was emitted more than once");
    }

    #[test]
    fn the_root_leads_the_rows_at_depth_zero() {
        let a = assembled(FIXTURE);
        assert_eq!(a.rows[0].bead.id, "bdi-3um");
        assert_eq!(a.rows[0].depth, 0);
    }

    #[test]
    fn a_child_follows_its_own_parent_rather_than_the_next_sibling() {
        let a = assembled(FIXTURE);
        let order = ids(&a);
        let pos = |id: &str| order.iter().position(|x| *x == id).unwrap();

        assert_eq!(pos("bdi-3um.4"), pos("bdi-3um.3") + 1);
        assert_eq!(pos("bdi-3um.11"), pos("bdi-3um.10") + 1);
    }

    #[test]
    fn depth_counts_the_parent_chain_rather_than_what_bd_reported() {
        // bd flattens `depth` under --max-depth, so these values are wrong on
        // purpose: an implementation that trusts them cannot pass.
        let json = r#"[
          {"id":"r","title":"root","status":"open","parent_id":"","depth":7},
          {"id":"r.1","title":"child","status":"open","parent_id":"r","depth":7},
          {"id":"r.1.1","title":"grandchild","status":"open","parent_id":"r.1","depth":0}
        ]"#;
        let a = assembled(json);

        assert_eq!(depth_of(&a, "r"), 0);
        assert_eq!(depth_of(&a, "r.1"), 1);
        assert_eq!(depth_of(&a, "r.1.1"), 2);
    }

    #[test]
    fn an_empty_string_parent_marks_the_root_rather_than_a_dangling_one() {
        let json = r#"[
          {"id":"r","title":"root","status":"open","parent_id":""},
          {"id":"r.1","title":"child","status":"open","parent_id":"r"}
        ]"#;
        let a = assembled(json);

        assert_eq!(a.rows[0].bead.id, "r");
        assert!(a.dangling.is_empty(), "the root is not a dangling parent");
    }

    #[test]
    fn siblings_order_by_state_then_priority_then_id() {
        // Every tie-break is decisive here: `d` is in flight at the worst
        // priority, `c` and `a` share a state and differ only by priority,
        // `a` and `b` share both and differ only by id.
        let json = r#"[
          {"id":"t","title":"root","status":"open","parent_id":""},
          {"id":"t.a","title":"a","status":"open","priority":1,"parent_id":"t"},
          {"id":"t.b","title":"b","status":"open","priority":1,"parent_id":"t"},
          {"id":"t.c","title":"c","status":"open","priority":0,"parent_id":"t"},
          {"id":"t.d","title":"d","status":"in_progress","priority":9,"parent_id":"t"},
          {"id":"t.e","title":"e","status":"closed","priority":0,"parent_id":"t"},
          {"id":"t.f","title":"f","status":"blocked","priority":5,"parent_id":"t"}
        ]"#;
        let a = assembled(json);

        assert_eq!(ids(&a), vec!["t", "t.d", "t.f", "t.c", "t.a", "t.b", "t.e"]);
    }

    #[test]
    fn real_siblings_from_the_tracker_come_back_in_flight_first() {
        // bd handed these back as .10, .3, .1; the tree owes them an order.
        let a = assembled(FIXTURE);
        let order = ids(&a);
        let pos = |id: &str| order.iter().position(|x| *x == id).unwrap();

        assert!(
            pos("bdi-3um.3") < pos("bdi-3um.10"),
            "in flight before open"
        );
        assert!(pos("bdi-3um.10") < pos("bdi-3um.1"), "open before closed");
    }

    #[test]
    fn a_bead_whose_parent_is_absent_is_reported_and_kept() {
        let json = r#"[
          {"id":"r","title":"root","status":"open","parent_id":""},
          {"id":"r.9","title":"orphan","status":"open","parent_id":"r.404"}
        ]"#;
        let a = assembled(json);

        assert_eq!(a.dangling, vec!["r.9".to_string()]);
        assert_eq!(ids(&a), vec!["r", "r.9"], "the orphan is kept, not dropped");
        assert_eq!(depth_of(&a, "r.9"), 1, "the orphan hangs off the root");
        assert!(a.unreachable.is_empty(), "re-parenting made it reachable");
    }

    #[test]
    fn a_parent_cycle_terminates_and_everything_inside_it_is_kept() {
        // b and c point at each other, so no walk down from the root reaches
        // them; d hangs off the cycle. All three are kept and reported.
        let json = r#"[
          {"id":"a","title":"a","status":"open","parent_id":""},
          {"id":"b","title":"b","status":"open","parent_id":"c"},
          {"id":"c","title":"c","status":"open","parent_id":"b"},
          {"id":"d","title":"d","status":"open","parent_id":"c"}
        ]"#;
        let a = assembled(json);

        assert_eq!(ids(&a), vec!["a", "b", "c", "d"]);
        assert_eq!(
            a.unreachable,
            vec!["b".to_string(), "c".to_string(), "d".to_string()]
        );
        assert!(a.dangling.is_empty(), "every parent named is present");
    }

    #[test]
    fn a_second_root_is_a_loud_failure_rather_than_a_discarded_component() {
        let json = r#"[
          {"id":"one","title":"one","status":"open","parent_id":""},
          {"id":"two","title":"two","status":"open","parent_id":""},
          {"id":"two.1","title":"child","status":"open","parent_id":"two"}
        ]"#;
        let err = assemble(parse_dep_tree(json).unwrap())
            .expect_err("two roots is not a tree")
            .to_string();

        assert!(
            err.contains("one") && err.contains("two"),
            "names both: {err}"
        );
    }

    #[test]
    fn no_root_at_all_is_a_loud_failure() {
        assert!(assemble(Vec::new()).is_err(), "an empty tree has no root");

        let json = r#"[{"id":"x","title":"x","status":"open","parent_id":"gone"}]"#;
        assert!(assemble(parse_dep_tree(json).unwrap()).is_err());
    }
}
