//! What nests what, in one answer's worth of bd rows: the tree a root draws,
//! the beads no edge places, and where a tree that reaches one of those has
//! to start.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::bail;

use crate::model::types::{Bead, Edge};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub bead: Bead,
    pub depth: u16,
    /// The kind of edge this copy was reached by, and `None` on the root.
    pub edge: Option<Edge>,
}

/// A flat set of bd rows in render order, with every departure from a clean
/// tree named rather than dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assembled {
    pub rows: Vec<Placed>,
    /// Ids in `rows` naming a bead they depend on that the answer does not
    /// hold. A bead the root does not reach is in another tree and is not
    /// reported here, however incomplete its own dependencies are.
    pub dangling: Vec<String>,
    /// Ids whose own descendants lead back to them. Each is kept in `rows`,
    /// drawn where the loop was cut.
    pub cycles: Vec<String>,
}

/// Order a flat set of bd rows into render order under one root.
///
/// A bead's descendants are the things that must complete before it can, which
/// beads says with two edge kinds running opposite ways: a parent cannot
/// finish until its children do, so a child sits under its parent; a bead
/// cannot finish until its blockers do, so a blocker sits under the bead it
/// blocks. An edge kind beads may add later has no settled direction against
/// completion, so it nests nothing.
///
/// So a bead is drawn once for every way down to it, and depth is counted from
/// the walk rather than taken from bd, which flattens it under `--max-depth`.
///
/// Siblings sort by state, then priority, then id.
pub fn assemble(beads: Vec<Bead>, root: &str) -> anyhow::Result<Assembled> {
    let by_id: BTreeMap<String, Bead> = beads.into_iter().map(|b| (b.id.clone(), b)).collect();
    if !by_id.contains_key(root) {
        bail!("bd's answer holds no bead {root} to draw a tree from");
    }

    let Nesting {
        children,
        waiting_on_the_absent,
        ..
    } = nesting(&by_id);

    let ordered: BTreeMap<String, Vec<String>> = children
        .into_iter()
        .map(|(parent, kids)| {
            let mut kids: Vec<String> = kids.into_iter().collect();
            kids.sort_by(|a, b| {
                let (a, b) = (&by_id[a], &by_id[b]);
                a.status
                    .rank()
                    .cmp(&b.status.rank())
                    .then(a.priority.cmp(&b.priority))
                    .then_with(|| a.id.cmp(&b.id))
            });
            (parent, kids)
        })
        .collect();

    let mut rows = Vec::new();
    let mut cycles = BTreeSet::new();
    walk(
        root,
        0,
        None,
        &ordered,
        &by_id,
        &mut Vec::new(),
        &mut cycles,
        &mut rows,
    );

    // Only what this tree drew. A bead whose parent the tracker no longer
    // holds is top of its own graph, and reporting it against a root that
    // never reached it names it in every tree there is.
    let dangling: BTreeSet<String> = rows
        .iter()
        .map(|placed| placed.bead.id.clone())
        .filter(|id| waiting_on_the_absent.contains(id))
        .collect();

    Ok(Assembled {
        rows,
        dangling: dangling.into_iter().collect(),
        cycles: cycles.into_iter().collect(),
    })
}

/// What the answer's edges do, asked once of the whole answer: the same edges
/// nest the same beads whichever root is being drawn, and an edge naming a
/// bead the answer does not hold is gone from every tree alike.
struct Nesting {
    /// Which beads sit under each bead.
    children: BTreeMap<String, BTreeSet<String>>,
    /// Beads naming a dependency the answer does not hold, of any kind.
    waiting_on_the_absent: BTreeSet<String>,
    /// Of those, the ones whose absent dependency would have placed them. An
    /// edge kind that nests nothing takes no place away by going missing.
    lost_their_place: BTreeSet<String>,
}

/// Read every edge in the answer once, in the one place that says which way
/// each kind runs — so a kind beads adds later is answered here and nowhere
/// else.
fn nesting(by_id: &BTreeMap<String, Bead>) -> Nesting {
    let mut found = Nesting {
        children: BTreeMap::new(),
        waiting_on_the_absent: BTreeSet::new(),
        lost_their_place: BTreeSet::new(),
    };

    for bead in by_id.values() {
        for edge in &bead.dependencies {
            // Which bead this edge draws under which. An edge kind beads may
            // add later has no settled direction against completion, so it
            // nests nothing.
            let nests: Option<(&String, &String)> = match edge.edge {
                Edge::ParentChild => Some((&edge.on, &bead.id)),
                Edge::Blocks => Some((&bead.id, &edge.on)),
                Edge::Other(_) => None,
            };

            if by_id.contains_key(&edge.on) {
                if let Some((over, under)) = nests {
                    found
                        .children
                        .entry(over.clone())
                        .or_default()
                        .insert(under.clone());
                }
                continue;
            }

            found.waiting_on_the_absent.insert(bead.id.clone());
            // Only the end that would have hung *under* the absent bead lost
            // anything by it going. A blocker the answer no longer holds
            // would have been drawn beneath the bead waiting on it, and takes
            // nothing away from where that bead itself is drawn.
            if let Some((_, under)) = nests.filter(|(over, _)| *over == &edge.on) {
                found.lost_their_place.insert(under.clone());
            }
        }
    }

    found
}

/// The beads that lost the edge that would have placed them.
///
/// A tree reports the beads it drew, so a bead no tree draws is a bead no tree
/// reports. Each of these is the evidence that the answer lost something, and
/// `top_of` says where a tree that reaches it starts.
pub fn adrift(beads: &[Bead]) -> Vec<String> {
    let by_id: BTreeMap<String, Bead> = beads.iter().map(|b| (b.id.clone(), b.clone())).collect();
    nesting(&by_id).lost_their_place.into_iter().collect()
}

/// Where a tree that draws `id` has to start: climb every edge that still
/// nests it, and answer with the roots that put everything the climb reached
/// on the screen.
///
/// This is how far the roots rule goes, and the line is that it goes exactly
/// as far as the damage. It climbs only from a bead the answer left with no
/// way down to it, so a component holding no such bead is drawn only where
/// discovery named a root in it — rules 1 to 4 still say what unfinished work
/// is. Stopping instead at "is anything nesting it" left whole components off
/// the screen: a bead that lost one placing edge and kept another is nested,
/// and the bead that kept it lost nothing and so was never discovered either.
///
/// A bead can hang under more than one, so this is a set rather than one id.
pub fn top_of(beads: &[Bead], id: &str) -> Vec<String> {
    let by_id: BTreeMap<String, Bead> = beads.iter().map(|b| (b.id.clone(), b.clone())).collect();
    let children = nesting(&by_id).children;
    let mut over: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (parent, kids) in &children {
        for kid in kids {
            over.entry(kid.as_str()).or_default().push(parent.as_str());
        }
    }

    let mut above: BTreeSet<&str> = BTreeSet::new();
    let mut climbing: Vec<&str> = vec![id];
    while let Some(reached) = climbing.pop() {
        if !above.insert(reached) {
            continue;
        }
        climbing.extend(over.get(reached).into_iter().flatten());
    }

    // A bead nothing nests is where a tree starts, and a loop has no such
    // bead — so where a loop is all that stands over something the climb
    // reached, one of its own beads has to stand for it. Any of them draws the
    // whole loop, `assemble` cutting it where it comes back round, so which
    // one is arbitrary and has only to be the same every time. Adding one and
    // asking again covers a component with more than one loop over it.
    let mut tops: BTreeSet<&str> = above
        .iter()
        .copied()
        .filter(|reached| !over.contains_key(reached))
        .collect();
    loop {
        let drawn = under(tops.iter().copied(), &children);
        let Some(&left) = above.iter().find(|reached| !drawn.contains(*reached)) else {
            break;
        };
        tops.insert(left);
    }

    // And the fewest of them that still does. A bead added to reach a loop
    // can turn out to sit under one added after it, and a tree that another
    // tree already draws puts every bead in it on the screen twice.
    for top in tops.clone() {
        let without: BTreeSet<&str> = tops.iter().copied().filter(|kept| *kept != top).collect();
        let drawn = under(without.iter().copied(), &children);
        if above.iter().all(|reached| drawn.contains(reached)) {
            tops.remove(top);
        }
    }

    tops.into_iter().map(str::to_string).collect()
}

/// Every bead a walk down from `from` reaches, a bead already reached ending
/// the branch it repeats on.
fn under<'a>(
    from: impl IntoIterator<Item = &'a str>,
    children: &'a BTreeMap<String, BTreeSet<String>>,
) -> BTreeSet<&'a str> {
    let mut reached: BTreeSet<&str> = BTreeSet::new();
    let mut going: Vec<&str> = from.into_iter().collect();
    while let Some(bead) = going.pop() {
        if !reached.insert(bead) {
            continue;
        }
        going.extend(children.get(bead).into_iter().flatten().map(String::as_str));
    }
    reached
}

/// Emit a bead and everything that must finish before it, depth-first.
///
/// `path` is the way down to this bead, and a bead already on it would loop:
/// that copy is cut and reported, which is the only thing standing between a
/// parent blocked by its own child and a walk that never ends.
#[allow(clippy::too_many_arguments)]
fn walk(
    id: &str,
    depth: u16,
    edge: Option<Edge>,
    children: &BTreeMap<String, Vec<String>>,
    by_id: &BTreeMap<String, Bead>,
    path: &mut Vec<String>,
    cycles: &mut BTreeSet<String>,
    out: &mut Vec<Placed>,
) {
    if path.iter().any(|seen| seen == id) {
        cycles.insert(id.to_string());
        return;
    }
    out.push(Placed {
        bead: by_id[id].clone(),
        depth,
        edge,
    });

    path.push(id.to_string());
    for child in children.get(id).into_iter().flatten() {
        let edge = if by_id[child]
            .dependencies
            .iter()
            .any(|d| d.on == id && d.edge == Edge::ParentChild)
        {
            Edge::ParentChild
        } else {
            Edge::Blocks
        };
        walk(
            child,
            depth + 1,
            Some(edge),
            children,
            by_id,
            path,
            cycles,
            out,
        );
    }
    path.pop();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The root of every hand-written tree below.
    const ROOT: &str = "r";
    use crate::collect::bd::parse_beads;
    use crate::model::types::Edge;

    /// A slice of this project's own tracker as `bd list --all --json`
    /// writes it: one open root, one bead in flight, and closed beads whose
    /// blocks edges cross between them.
    const FIXTURE: &str = include_str!("../../tests/fixtures/bd_list.json");
    const FIXTURE_ROOT: &str = "bdi-2bb";

    fn assembled(json: &str, root: &str) -> Assembled {
        assemble(parse_beads(json).expect("the rows parse"), root).expect("the rows assemble")
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

    /// One bead depending on another, in the shape `bd list --json` writes.
    fn dep(on: &str, kind: &str) -> String {
        format!(r#"{{"depends_on_id":"{on}","type":"{kind}"}}"#)
    }

    fn bead(id: &str, status: &str, deps: &[String]) -> String {
        format!(
            r#"{{"id":"{id}","title":"{id}","status":"{status}","dependencies":[{}]}}"#,
            deps.join(",")
        )
    }

    fn tracker(beads: &[String]) -> String {
        format!("[{}]", beads.join(","))
    }

    fn parent_of(a: &Assembled, id: &str) -> Option<String> {
        let at = a.rows.iter().position(|p| p.bead.id == id)?;
        let depth = a.rows[at].depth;
        a.rows[..at]
            .iter()
            .rev()
            .find(|p| p.depth < depth)
            .map(|p| p.bead.id.clone())
    }

    #[test]
    fn a_blocker_is_drawn_beneath_the_bead_it_blocks() {
        // `late` cannot finish until `early` does, so `early` is what stands
        // between `late` and done — which is what a nesting now means.
        let json = tracker(&[
            bead("r", "open", &[]),
            bead(
                "late",
                "open",
                &[dep("r", "parent-child"), dep("early", "blocks")],
            ),
            bead("early", "closed", &[dep("r", "parent-child")]),
        ]);
        let a = assembled(&json, ROOT);

        assert_eq!(parent_of(&a, "early").as_deref(), Some("late"));
    }

    #[test]
    fn a_child_stays_beneath_its_own_parent() {
        // The other half of the rule: a parent cannot finish until its child
        // does, so completion runs against the edge and the nesting does not
        // move.
        let json = tracker(&[
            bead("r", "open", &[]),
            bead("r.1", "open", &[dep("r", "parent-child")]),
        ]);
        let a = assembled(&json, ROOT);

        assert_eq!(parent_of(&a, "r.1").as_deref(), Some("r"));
    }

    #[test]
    fn a_blocker_of_several_beads_is_drawn_under_each_of_them() {
        let json = tracker(&[
            bead("r", "open", &[]),
            bead(
                "a",
                "open",
                &[dep("r", "parent-child"), dep("done", "blocks")],
            ),
            bead(
                "b",
                "open",
                &[dep("r", "parent-child"), dep("done", "blocks")],
            ),
            bead("done", "closed", &[dep("r", "parent-child")]),
        ]);
        let a = assembled(&json, ROOT);

        let drawn: Vec<&str> = a
            .rows
            .iter()
            .filter(|p| p.bead.id == "done")
            .map(|_| "done")
            .collect();
        assert_eq!(
            drawn.len(),
            3,
            "once under the root, once under each waiter"
        );

        let under: Vec<Option<String>> = a
            .rows
            .iter()
            .enumerate()
            .filter(|(_, p)| p.bead.id == "done")
            .map(|(at, p)| {
                a.rows[..at]
                    .iter()
                    .rev()
                    .find(|q| q.depth < p.depth)
                    .map(|q| q.bead.id.clone())
            })
            .collect();
        assert!(under.contains(&Some("a".to_string())));
        assert!(under.contains(&Some("b".to_string())));
    }

    #[test]
    fn a_closed_blocker_is_a_leaf_rather_than_a_branch_over_what_waited_on_it() {
        // The reported defect: `done` is finished, and the beads queued behind
        // it were being drawn as its subtree and counted into its fraction.
        let json = tracker(&[
            bead("r", "open", &[]),
            bead(
                "waited",
                "open",
                &[dep("r", "parent-child"), dep("done", "blocks")],
            ),
            bead("done", "closed", &[dep("r", "parent-child")]),
        ]);
        let a = assembled(&json, ROOT);

        for (at, placed) in a.rows.iter().enumerate() {
            if placed.bead.id != "done" {
                continue;
            }
            assert!(
                a.rows[at + 1..]
                    .iter()
                    .take_while(|p| p.depth > placed.depth)
                    .next()
                    .is_none(),
                "a closed blocker has nothing beneath it"
            );
        }
    }

    #[test]
    fn a_bead_reached_two_ways_is_drawn_once_for_each() {
        let json = tracker(&[
            bead("r", "open", &[]),
            bead(
                "r.1",
                "open",
                &[dep("r", "parent-child"), dep("r.2", "blocks")],
            ),
            bead("r.2", "open", &[dep("r", "parent-child")]),
        ]);
        let a = assembled(&json, ROOT);

        assert_eq!(
            a.rows.iter().filter(|p| p.bead.id == "r.2").count(),
            2,
            "once as the root's child, once as what r.1 waits on"
        );
    }

    #[test]
    fn a_bead_blocked_by_one_of_its_own_forebears_is_reported_and_its_beads_kept() {
        // The shape the rule makes reachable, and one beads permits: `r.1`
        // depends on `r` twice, once by hanging under it and once by being
        // blocked by it. The first draws `r.1` under `r`, the second draws
        // `r` back under `r.1`.
        let json = tracker(&[
            bead("r", "open", &[]),
            bead(
                "r.1",
                "open",
                &[dep("r", "parent-child"), dep("r", "blocks")],
            ),
        ]);
        let a = assembled(&json, ROOT);

        assert_eq!(a.cycles, vec!["r".to_string()]);
        assert_eq!(ids(&a), vec!["r", "r.1"], "both are kept");
    }

    #[test]
    fn an_edge_kind_bdi_does_not_know_nests_nothing() {
        // Only the two kinds beads defines have a settled direction against
        // completion. A third says nothing about what must finish first, and
        // guessing one would draw a nesting that means something else.
        let json = tracker(&[
            bead("r", "open", &[]),
            bead(
                "r.1",
                "open",
                &[dep("r", "parent-child"), dep("r.2", "discovered-by")],
            ),
            bead("r.2", "open", &[dep("r", "parent-child")]),
        ]);
        let a = assembled(&json, ROOT);

        assert_eq!(parent_of(&a, "r.2").as_deref(), Some("r"));
        assert_eq!(a.rows.iter().filter(|p| p.bead.id == "r.2").count(), 1);
    }

    #[test]
    fn the_root_leads_the_rows_at_depth_zero() {
        let a = assembled(FIXTURE, FIXTURE_ROOT);
        assert_eq!(a.rows[0].bead.id, FIXTURE_ROOT);
        assert_eq!(a.rows[0].depth, 0);
        assert_eq!(a.rows[0].edge, None, "nothing reached the root");
    }

    #[test]
    fn a_descendant_follows_its_own_forebear_rather_than_the_next_sibling() {
        // `bdi-2bb.5` waits on `bdi-2bb.3`, so the blocker sits directly under
        // it rather than after the sibling that comes next.
        let a = assembled(FIXTURE, FIXTURE_ROOT);
        let at = a
            .rows
            .iter()
            .position(|p| p.bead.id == "bdi-2bb.5")
            .expect("the waiting bead is drawn");

        assert_eq!(a.rows[at + 1].bead.id, "bdi-2bb.3");
        assert_eq!(a.rows[at + 1].depth, a.rows[at].depth + 1);
        assert_eq!(a.rows[at + 1].edge, Some(Edge::Blocks));
    }

    #[test]
    fn the_tracker_draws_a_blocker_under_each_bead_that_waits_on_it() {
        // `bdi-2bb.3` blocks both `bdi-2bb.4` and `bdi-2bb.5`, and is a child
        // of the root besides.
        let a = assembled(FIXTURE, FIXTURE_ROOT);

        assert_eq!(
            a.rows.iter().filter(|p| p.bead.id == "bdi-2bb.3").count(),
            3
        );
    }

    #[test]
    fn depth_counts_the_parent_chain_rather_than_what_bd_reported() {
        // bd flattens `depth` under --max-depth, so these values are wrong on
        // purpose: an implementation that trusts them cannot pass.
        let json = r#"[
          {"id":"r","title":"root","status":"open","depth":7},
          {"id":"r.1","title":"child","status":"open",
           "dependencies":[{"depends_on_id":"r","type":"parent-child"}],"depth":7},
          {"id":"r.1.1","title":"grandchild","status":"open",
           "dependencies":[{"depends_on_id":"r.1","type":"parent-child"}],"depth":0}
        ]"#;
        let a = assembled(json, ROOT);

        assert_eq!(depth_of(&a, "r"), 0);
        assert_eq!(depth_of(&a, "r.1"), 1);
        assert_eq!(depth_of(&a, "r.1.1"), 2);
    }

    #[test]
    fn a_root_depending_on_nothing_is_not_reported_as_a_dangling_parent() {
        let json = r#"[
          {"id":"r","title":"root","status":"open"},
          {"id":"r.1","title":"child","status":"open",
           "dependencies":[{"depends_on_id":"r","type":"parent-child"}]}
        ]"#;
        let a = assembled(json, ROOT);

        assert_eq!(a.rows[0].bead.id, "r");
        assert!(a.dangling.is_empty(), "the root is not a dangling parent");
    }

    #[test]
    fn siblings_order_by_state_then_priority_then_id() {
        // Every tie-break is decisive here: `d` is in flight at the worst
        // priority, `c` and `a` share a state and differ only by priority,
        // `a` and `b` share both and differ only by id.
        let json = r#"[
          {"id":"t","title":"root","status":"open"},
          {"id":"t.a","title":"a","status":"open","priority":1,
           "dependencies":[{"depends_on_id":"t","type":"parent-child"}]},
          {"id":"t.b","title":"b","status":"open","priority":1,
           "dependencies":[{"depends_on_id":"t","type":"parent-child"}]},
          {"id":"t.c","title":"c","status":"open","priority":0,
           "dependencies":[{"depends_on_id":"t","type":"parent-child"}]},
          {"id":"t.d","title":"d","status":"in_progress","priority":9,
           "dependencies":[{"depends_on_id":"t","type":"parent-child"}]},
          {"id":"t.e","title":"e","status":"closed","priority":0,
           "dependencies":[{"depends_on_id":"t","type":"parent-child"}]},
          {"id":"t.f","title":"f","status":"blocked","priority":5,
           "dependencies":[{"depends_on_id":"t","type":"parent-child"}]}
        ]"#;
        let a = assembled(json, "t");

        assert_eq!(ids(&a), vec!["t", "t.d", "t.f", "t.c", "t.a", "t.b", "t.e"]);
    }

    #[test]
    fn real_siblings_from_the_tracker_come_back_in_flight_first() {
        // bd's own order is arbitrary; the tree owes them one.
        let a = assembled(FIXTURE, FIXTURE_ROOT);

        assert_eq!(a.rows[1].bead.id, "bdi-r5l", "the bead in flight leads");
    }

    #[test]
    fn a_bead_whose_parent_is_absent_is_reported_and_kept() {
        let json = r#"[
          {"id":"r","title":"root","status":"open"},
          {"id":"r.9","title":"orphan","status":"open",
           "dependencies":[{"depends_on_id":"r.404","type":"parent-child"}]}
        ]"#;

        // Nothing above it survives, so the tree it belongs to is its own.
        let its_own = assembled(json, "r.9");
        assert_eq!(
            ids(&its_own),
            vec!["r.9"],
            "the orphan is kept, not dropped"
        );
        assert_eq!(its_own.dangling, vec!["r.9".to_string()]);
        assert_eq!(depth_of(&its_own, "r.9"), 0);
        assert!(its_own.cycles.is_empty());

        // And `r` never named it, so `r` neither draws it nor reports it.
        let elsewhere = assembled(json, ROOT);
        assert_eq!(ids(&elsewhere), vec!["r"]);
        assert!(elsewhere.dangling.is_empty());
    }

    /// The answer is the whole tracker, so "hang it off the root" hangs it off
    /// every root there is. Measured against summit-works on 2026-08-31: two
    /// closed beads with a deleted parent reached all 562 trees, and each drew
    /// a warning saying two beads were waiting on work outside that tree.
    #[test]
    fn an_orphan_does_not_join_a_tree_that_never_named_it() {
        let json = r#"[
          {"id":"one","title":"one","status":"open"},
          {"id":"two","title":"two","status":"open"},
          {"id":"lost","title":"its parent was deleted","status":"closed",
           "dependencies":[{"depends_on_id":"gone","type":"parent-child"}]}
        ]"#;

        assert_eq!(ids(&assembled(json, "one")), vec!["one"]);
        assert_eq!(ids(&assembled(json, "two")), vec!["two"]);
    }

    #[test]
    fn a_root_the_answer_does_not_hold_is_a_loud_failure() {
        let json = r#"[
          {"id":"one","title":"one","status":"open"},
          {"id":"two","title":"two","status":"open"}
        ]"#;
        let err = assemble(parse_beads(json).unwrap(), "three")
            .expect_err("no bead three to draw from")
            .to_string();

        assert!(err.contains("three"), "names the root asked for: {err}");
        assert!(
            assemble(Vec::new(), "one").is_err(),
            "an empty answer holds no root"
        );
    }

    #[test]
    fn a_bead_the_root_does_not_reach_belongs_to_another_tree_and_is_not_drawn() {
        // Two unrelated roots in one answer, which is the ordinary shape of a
        // whole tracker. Drawing `two` under `one` would put every other
        // effort's work inside this one.
        let json = r#"[
          {"id":"one","title":"one","status":"open"},
          {"id":"one.1","title":"child","status":"open",
           "dependencies":[{"depends_on_id":"one","type":"parent-child"}]},
          {"id":"two","title":"two","status":"open"},
          {"id":"two.1","title":"child","status":"open",
           "dependencies":[{"depends_on_id":"two","type":"parent-child"}]}
        ]"#;
        let a = assembled(json, "one");

        assert_eq!(ids(&a), vec!["one", "one.1"]);
    }
}
