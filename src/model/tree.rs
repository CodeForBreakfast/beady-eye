//! What nests what, in one answer's worth of bd rows: the tree a root draws,
//! the beads no edge places, and where a tree that reaches one of those has
//! to start.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::bail;

use crate::model::types::{Bead, Edge};

/// One way down from a bead to a bead beneath it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// The bead beneath, by its place among the tree's beads.
    pub bead: usize,
    /// The kind of edge that hangs it there.
    pub edge: Edge,
    /// Whether this is the way the walk first reached the bead. A bead is
    /// drawn once for every way down to it, and the first of those lines is
    /// the one that stands for the work: it is the line whose every link
    /// down from the root is one of these.
    pub first: bool,
}

/// One row of the tree unrolled into render order: a bead, at the depth one
/// way down puts it, hung there by one kind of edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub bead: usize,
    pub depth: u16,
    /// The kind of edge this copy was reached by, and `None` on the root.
    pub edge: Option<Edge>,
}

/// One root's tree: each bead it reaches held once, and the ways down to it,
/// with every departure from a clean tree named rather than dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assembled {
    /// Every bead the root reaches, once each: the root first, then the rest
    /// in the order the walk first reaches them.
    pub beads: Vec<Bead>,
    /// The beads beneath each of `beads`, in render order. A bead reached
    /// several ways is linked from each of the beads that reach it.
    pub children: Vec<Vec<Link>>,
    /// Ids in `beads` naming a bead they depend on that the answer does not
    /// hold. A bead the root does not reach is in another tree and is not
    /// reported here, however incomplete its own dependencies are.
    pub dangling: Vec<String>,
    /// Ids whose own descendants lead back to them. Each is kept in `beads`,
    /// and drawn where the loop was cut.
    pub cycles: Vec<String>,
}

/// What one answer's edges do, read once for every tree drawn from it: the
/// same edges nest the same beads whichever root is being drawn, and an edge
/// naming a bead the answer does not hold is gone from every tree alike.
///
/// A bead's descendants are the things that must complete before it can, which
/// beads says with two edge kinds running opposite ways: a parent cannot
/// finish until its children do, so a child sits under its parent; a bead
/// cannot finish until its blockers do, so a blocker sits under the bead it
/// blocks. An edge kind beads may add later has no settled direction against
/// completion, so it nests nothing.
pub struct Nesting<'a> {
    by_id: BTreeMap<&'a str, &'a Bead>,
    /// The beads beneath each bead, in render order: siblings sort by state,
    /// then priority, then id.
    children: BTreeMap<&'a str, Vec<&'a str>>,
    /// Beads naming a dependency the answer does not hold, of any kind.
    waiting_on_the_absent: BTreeSet<&'a str>,
    /// Of those, the ones whose absent dependency would have placed them. An
    /// edge kind that nests nothing takes no place away by going missing.
    lost_their_place: BTreeSet<&'a str>,
}

#[cfg(test)]
thread_local! {
    static NESTINGS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many times this thread has read an answer's edges, so a test can say
/// what one read of a tracker costs.
#[cfg(test)]
pub(crate) fn nestings_on_this_thread() -> usize {
    NESTINGS.with(std::cell::Cell::get)
}

impl<'a> Nesting<'a> {
    /// Read every edge in the answer once, in the one place that says which
    /// way each kind runs — so a kind beads adds later is answered here and
    /// nowhere else.
    pub fn of(beads: &'a [Bead]) -> Self {
        #[cfg(test)]
        NESTINGS.with(|count| count.set(count.get() + 1));

        let by_id: BTreeMap<&str, &Bead> = beads.iter().map(|b| (b.id.as_str(), b)).collect();
        let mut children: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        let mut waiting_on_the_absent = BTreeSet::new();
        let mut lost_their_place = BTreeSet::new();

        for &bead in by_id.values() {
            for edge in &bead.dependencies {
                // Which bead this edge draws under which. An edge kind beads
                // may add later has no settled direction against completion,
                // so it nests nothing.
                let nests: Option<(&'a str, &'a str)> = match edge.edge {
                    Edge::ParentChild => Some((&edge.on, &bead.id)),
                    Edge::Blocks => Some((&bead.id, &edge.on)),
                    Edge::Other(_) => None,
                };

                if by_id.contains_key(edge.on.as_str()) {
                    if let Some((over, under)) = nests {
                        children.entry(over).or_default().insert(under);
                    }
                    continue;
                }

                waiting_on_the_absent.insert(bead.id.as_str());
                // Only the end that would have hung *under* the absent bead
                // lost anything by it going. A blocker the answer no longer
                // holds would have been drawn beneath the bead waiting on it,
                // and takes nothing away from where that bead itself is drawn.
                if let Some((_, under)) = nests.filter(|(over, _)| *over == edge.on) {
                    lost_their_place.insert(under);
                }
            }
        }

        let children = children
            .into_iter()
            .map(|(parent, kids)| {
                let mut kids: Vec<&str> = kids.into_iter().collect();
                kids.sort_by(|a, b| {
                    let (a, b) = (by_id[a], by_id[b]);
                    a.status
                        .rank()
                        .cmp(&b.status.rank())
                        .then(a.priority.cmp(&b.priority))
                        .then_with(|| a.id.cmp(&b.id))
                });
                (parent, kids)
            })
            .collect();

        Nesting {
            by_id,
            children,
            waiting_on_the_absent,
            lost_their_place,
        }
    }

    /// Order the answer into a tree under one root.
    ///
    /// A bead has as many places as there are ways down to it, and is drawn
    /// at each — but it is held once, and each way down points at it. Depth
    /// is a property of a way down rather than of a bead, and is counted by
    /// whoever walks the tree rather than taken from bd, which flattens it
    /// under `--max-depth`.
    pub fn assemble(&self, root: &str) -> anyhow::Result<Assembled> {
        let Some((&root, _)) = self.by_id.get_key_value(root) else {
            bail!("bd's answer holds no bead {root} to draw a tree from");
        };

        let Reached {
            order,
            children,
            looped,
        } = reach(root, &self.children, &self.by_id);

        // Only what this tree drew. A bead whose parent the tracker no longer
        // holds is top of its own graph, and reporting it against a root that
        // never reached it names it in every tree there is.
        let dangling: Vec<String> = order
            .iter()
            .filter(|id| self.waiting_on_the_absent.contains(*id))
            .map(|id| id.to_string())
            .collect();

        // A walk that found no way back up found no loop to cut, and a tree
        // with none is what most trees are — so saying where the loops are
        // cut costs only the trees that have any.
        let cycles = if looped {
            cuts(&children)
                .into_iter()
                .map(|bead| order[bead].to_string())
                .collect()
        } else {
            Vec::new()
        };

        let beads = order.iter().map(|id| self.by_id[id].clone()).collect();
        Ok(Assembled {
            beads,
            children,
            dangling,
            cycles,
        })
    }

    /// The beads that lost the edge that would have placed them.
    ///
    /// A tree reports the beads it drew, so a bead no tree draws is a bead no
    /// tree reports. Each of these is the evidence that the answer lost
    /// something, and `top_of` says where a tree that reaches it starts.
    pub fn adrift(&self) -> Vec<String> {
        self.lost_their_place
            .iter()
            .map(|id| id.to_string())
            .collect()
    }

    /// Where a tree that draws `id` has to start: climb every edge that still
    /// nests it, and answer with the roots that put everything the climb
    /// reached on the screen.
    ///
    /// This is how far the roots rule goes, and the line is that it goes
    /// exactly as far as the damage. It climbs only from a bead the answer
    /// left with no way down to it, so a component holding no such bead is
    /// drawn only where discovery named a root in it — rules 1 to 4 still say
    /// what unfinished work is. Stopping instead at "is anything nesting it"
    /// left whole components off the screen: a bead that lost one placing
    /// edge and kept another is nested, and the bead that kept it lost
    /// nothing and so was never discovered either.
    ///
    /// A bead can hang under more than one, so this is a set rather than one
    /// id.
    pub fn top_of(&self, id: &str) -> Vec<String> {
        let mut over: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (parent, kids) in &self.children {
            for kid in kids {
                over.entry(kid).or_default().push(parent);
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
        // reached, one of its own beads has to stand for it. Any of them
        // draws the whole loop, `assemble` cutting it where it comes back
        // round, so which one is arbitrary and has only to be the same every
        // time. Adding one and asking again covers a component with more
        // than one loop over it.
        let mut tops: BTreeSet<&str> = above
            .iter()
            .copied()
            .filter(|reached| !over.contains_key(reached))
            .collect();
        loop {
            let drawn = under(tops.iter().copied(), &self.children);
            let Some(&left) = above.iter().find(|reached| !drawn.contains(*reached)) else {
                break;
            };
            tops.insert(left);
        }

        // And the fewest of them that still does. A bead added to reach a
        // loop can turn out to sit under one added after it, and a tree that
        // another tree already draws puts every bead in it on the screen
        // twice.
        for top in tops.clone() {
            let without: BTreeSet<&str> =
                tops.iter().copied().filter(|kept| *kept != top).collect();
            let drawn = under(without.iter().copied(), &self.children);
            if above.iter().all(|reached| drawn.contains(reached)) {
                tops.remove(top);
            }
        }

        tops.into_iter().map(str::to_string).collect()
    }
}

/// Every bead a walk down from `from` reaches, a bead already reached ending
/// the branch it repeats on.
fn under<'a>(
    from: impl IntoIterator<Item = &'a str>,
    children: &BTreeMap<&'a str, Vec<&'a str>>,
) -> BTreeSet<&'a str> {
    let mut reached: BTreeSet<&str> = BTreeSet::new();
    let mut going: Vec<&str> = from.into_iter().collect();
    while let Some(bead) = going.pop() {
        if !reached.insert(bead) {
            continue;
        }
        going.extend(children.get(bead).into_iter().flatten().copied());
    }
    reached
}

/// What one walk down from the root found: every bead it reached, in the
/// order it first reached them, and the ways down between them.
struct Reached<'a> {
    order: Vec<&'a str>,
    children: Vec<Vec<Link>>,
    /// Whether any way down led back to a bead still above it. A loop is
    /// cut there, and where this is false nothing is.
    looped: bool,
}

/// Walk down from the root, placing each bead the first time it is reached
/// and linking every later way down to the place it already has.
///
/// The order the beads are first reached in is the order the first copy of
/// each is drawn in. A walk that drew a bead once for every way down to it
/// would reach nothing new on the second way, because everything under the
/// bead was reached under it the first time or was already above it.
fn reach<'a>(
    root: &'a str,
    ordered: &BTreeMap<&'a str, Vec<&'a str>>,
    by_id: &BTreeMap<&'a str, &'a Bead>,
) -> Reached<'a> {
    let mut found = Reached {
        order: vec![root],
        children: vec![Vec::new()],
        looped: false,
    };
    let mut placed: BTreeMap<&str, usize> = BTreeMap::from([(root, 0)]);
    descend(
        root,
        0,
        ordered,
        by_id,
        &mut placed,
        &mut Vec::new(),
        &mut found,
    );
    found
}

fn descend<'a>(
    id: &'a str,
    at: usize,
    ordered: &BTreeMap<&'a str, Vec<&'a str>>,
    by_id: &BTreeMap<&'a str, &'a Bead>,
    placed: &mut BTreeMap<&'a str, usize>,
    above: &mut Vec<&'a str>,
    found: &mut Reached<'a>,
) {
    above.push(id);
    for &child in ordered.get(id).into_iter().flatten() {
        let edge = if by_id[child]
            .dependencies
            .iter()
            .any(|d| d.on == id && d.edge == Edge::ParentChild)
        {
            Edge::ParentChild
        } else {
            Edge::Blocks
        };
        found.looped |= above.contains(&child);
        let (bead, first) = match placed.get(child) {
            Some(&bead) => (bead, false),
            None => {
                let bead = found.order.len();
                found.order.push(child);
                found.children.push(Vec::new());
                placed.insert(child, bead);
                (bead, true)
            }
        };
        found.children[at].push(Link { bead, edge, first });
        if first {
            descend(child, bead, ordered, by_id, placed, above, found);
        }
    }
    above.pop();
}
/// The beads at which a walk drawing every way down cuts a loop: each is
/// reached, then reached again from beneath itself, and the second time is
/// where the walk stops rather than going round again.
///
/// Which beads those are depends on the order the walk takes, and not only
/// on which beads sit on a loop: one the walk meets above the loop is cut
/// when the loop comes back to it, and one it meets only from inside the
/// loop never is. So this asks the same walk the same question — every way
/// down, each stopped where it meets a bead already above it — and remembers
/// each answer by what it can depend on. From one bead, the answer depends
/// on which of the beads above it are among the ones it can reach, and on
/// nothing else; a bead that reaches nothing above it gets one answer for
/// every way down to it, which is what keeps this from being the unrolling
/// it is asking about.
fn cuts(children: &[Vec<Link>]) -> BTreeSet<usize> {
    let reach: Vec<BTreeSet<usize>> = (0..children.len())
        .map(|from| {
            let mut reached = BTreeSet::new();
            let mut going = vec![from];
            while let Some(bead) = going.pop() {
                for link in &children[bead] {
                    if reached.insert(link.bead) {
                        going.push(link.bead);
                    }
                }
            }
            reached
        })
        .collect();
    cuts_from(0, &mut Vec::new(), children, &reach, &mut BTreeMap::new())
}

fn cuts_from(
    at: usize,
    above: &mut Vec<usize>,
    children: &[Vec<Link>],
    reach: &[BTreeSet<usize>],
    answered: &mut BTreeMap<(usize, Vec<usize>), BTreeSet<usize>>,
) -> BTreeSet<usize> {
    let seen: Vec<usize> = above
        .iter()
        .copied()
        .filter(|bead| reach[at].contains(bead))
        .collect();
    if let Some(cut) = answered.get(&(at, seen.clone())) {
        return cut.clone();
    }

    let mut cut = BTreeSet::new();
    above.push(at);
    for link in &children[at] {
        if above.contains(&link.bead) {
            cut.insert(link.bead);
        } else {
            cut.extend(cuts_from(link.bead, above, children, reach, answered));
        }
    }
    above.pop();
    answered.insert((at, seen), cut.clone());
    cut
}

/// The ways down from `at` that a walk takes, `above` being the beads it
/// came down through to reach `at`. A way back to one of those, or to `at`
/// itself, is where a loop is cut: the walk does not take it.
pub fn links_from<'a>(children: &'a [Vec<Link>], at: usize, above: &[usize]) -> Vec<&'a Link> {
    children[at]
        .iter()
        .filter(|link| link.bead != at && !above.contains(&link.bead))
        .collect()
}

/// Every bead a walk down from `at` reaches, `at` itself excluded, each once
/// and in no order worth relying on. `above` is the way down to `at`, and is
/// where the walk's loops are cut.
pub fn beneath(children: &[Vec<Link>], at: usize, above: &[usize]) -> Vec<usize> {
    let mut reached: Vec<bool> = vec![false; children.len()];
    reached[at] = true;
    for bead in above {
        reached[*bead] = true;
    }
    let mut found = Vec::new();
    let mut going = vec![at];
    while let Some(bead) = going.pop() {
        for link in &children[bead] {
            if !reached[link.bead] {
                reached[link.bead] = true;
                found.push(link.bead);
                going.push(link.bead);
            }
        }
    }
    found
}

/// The tree unrolled into render order: one row for every way down to a
/// bead, each loop cut where the way down comes back on itself.
///
/// This is the shape the tree is drawn in and the shape `--json` writes, and
/// it can be very much larger than the tree: a bead reached several ways is
/// a row for each, and so is everything beneath it, compounding down the
/// tree. So nothing asks for it per keystroke; it is walked once, when the
/// whole of it is what is wanted.
pub fn unroll(children: &[Vec<Link>]) -> Vec<Placed> {
    let mut rows = Vec::new();
    if !children.is_empty() {
        unroll_from(0, 0, None, children, &mut Vec::new(), &mut rows);
    }
    rows
}

fn unroll_from(
    at: usize,
    depth: u16,
    edge: Option<Edge>,
    children: &[Vec<Link>],
    above: &mut Vec<usize>,
    out: &mut Vec<Placed>,
) {
    out.push(Placed {
        bead: at,
        depth,
        edge,
    });
    let links = links_from(children, at, above);
    above.push(at);
    for link in links {
        unroll_from(
            link.bead,
            depth + 1,
            Some(link.edge.clone()),
            children,
            above,
            out,
        );
    }
    above.pop();
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
        let beads = parse_beads(json).expect("the rows parse");
        Nesting::of(&beads)
            .assemble(root)
            .expect("the rows assemble")
    }

    /// The tree as it is drawn: one row per way down to a bead.
    fn rows(a: &Assembled) -> Vec<(&str, u16, Option<Edge>)> {
        unroll(&a.children)
            .into_iter()
            .map(|p| (a.beads[p.bead].id.as_str(), p.depth, p.edge))
            .collect()
    }

    fn ids(a: &Assembled) -> Vec<&str> {
        rows(a).into_iter().map(|(id, _, _)| id).collect()
    }

    fn drawn(a: &Assembled, id: &str) -> usize {
        ids(a).into_iter().filter(|drawn| *drawn == id).count()
    }

    fn depth_of(a: &Assembled, id: &str) -> u16 {
        rows(a)
            .into_iter()
            .find(|(drawn, _, _)| *drawn == id)
            .unwrap_or_else(|| panic!("{id} is in the rows"))
            .1
    }

    fn index_of(a: &Assembled, id: &str) -> usize {
        a.beads
            .iter()
            .position(|b| b.id == id)
            .unwrap_or_else(|| panic!("{id} is among the beads"))
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

    /// The bead the first row of `id` hangs under.
    fn parent_of(a: &Assembled, id: &str) -> Option<String> {
        let rows = rows(a);
        let at = rows.iter().position(|(drawn, _, _)| *drawn == id)?;
        let depth = rows[at].1;
        rows[..at]
            .iter()
            .rev()
            .find(|(_, above, _)| *above < depth)
            .map(|(parent, _, _)| (*parent).to_string())
    }

    /// The beads each row of `id` hangs under, one per row.
    fn parents_of(a: &Assembled, id: &str) -> Vec<String> {
        let rows = rows(a);
        rows.iter()
            .enumerate()
            .filter(|(_, (drawn, _, _))| *drawn == id)
            .map(|(at, (_, depth, _))| {
                rows[..at]
                    .iter()
                    .rev()
                    .find(|(_, above, _)| above < depth)
                    .map(|(parent, _, _)| (*parent).to_string())
                    .expect("every row but the root hangs under one")
            })
            .collect()
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

        assert_eq!(
            drawn(&a, "done"),
            3,
            "once under the root, once under each waiter"
        );
        let under = parents_of(&a, "done");
        assert!(under.contains(&"a".to_string()));
        assert!(under.contains(&"b".to_string()));
    }

    #[test]
    fn a_bead_drawn_several_ways_is_held_once_and_each_way_down_points_at_it() {
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

        let held: Vec<&str> = a.beads.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(
            held,
            vec!["r", "a", "done", "b"],
            "once each, as first reached"
        );

        let done = index_of(&a, "done");
        let linked_from: Vec<&str> = a
            .children
            .iter()
            .enumerate()
            .filter(|(_, links)| links.iter().any(|link| link.bead == done))
            .map(|(from, _)| a.beads[from].id.as_str())
            .collect();
        assert_eq!(linked_from, vec!["r", "a", "b"]);
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

        assert!(
            a.children[index_of(&a, "done")].is_empty(),
            "a closed blocker has nothing beneath it"
        );
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
            drawn(&a, "r.2"),
            2,
            "once as the root's child, once as what r.1 waits on"
        );
        assert_eq!(a.beads.len(), 3, "and held once");
    }

    /// The first line drawn of a bead is the one that stands for the work,
    /// and it is the line whose every link down from the root is a first
    /// link. Here `r.2` is met under `r.1` before it is met under the root,
    /// so the deeper way down is the first one.
    #[test]
    fn the_first_link_to_a_bead_is_the_way_the_walk_first_reached_it() {
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
        let (r_1, r_2) = (index_of(&a, "r.1"), index_of(&a, "r.2"));

        let firsts: Vec<(usize, usize, bool)> = a
            .children
            .iter()
            .enumerate()
            .flat_map(|(from, links)| links.iter().map(move |l| (from, l.bead, l.first)))
            .collect();
        assert_eq!(
            firsts,
            vec![(0, r_1, true), (0, r_2, false), (r_1, r_2, true)]
        );
        assert_eq!(
            rows(&a)[..3]
                .iter()
                .map(|(id, depth, _)| (*id, *depth))
                .collect::<Vec<_>>(),
            vec![("r", 0), ("r.1", 1), ("r.2", 2)],
            "which is the way down its first row takes"
        );
    }

    #[test]
    fn a_link_carries_the_kind_of_edge_that_hangs_the_bead_there() {
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
        let r_1 = index_of(&a, "r.1");

        assert_eq!(a.children[0][0].edge, Edge::ParentChild);
        assert_eq!(a.children[r_1][0].edge, Edge::Blocks);
        assert_eq!(
            rows(&a),
            vec![
                ("r", 0, None),
                ("r.1", 1, Some(Edge::ParentChild)),
                ("r.2", 2, Some(Edge::Blocks)),
                ("r.2", 1, Some(Edge::ParentChild)),
            ]
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

    /// The link back up is held — it is a way down from `r.1`, and the
    /// tree says so — and every walk cuts it: the beads beneath `r.1` are
    /// none, because the only one is the one it came down from.
    #[test]
    fn a_loop_is_cut_where_the_way_down_comes_back_on_itself() {
        let json = tracker(&[
            bead("r", "open", &[]),
            bead(
                "r.1",
                "open",
                &[dep("r", "parent-child"), dep("r", "blocks")],
            ),
        ]);
        let a = assembled(&json, ROOT);
        let r_1 = index_of(&a, "r.1");

        assert_eq!(a.children[r_1].len(), 1, "the way back up is held");
        assert!(links_from(&a.children, r_1, &[0]).is_empty());
        assert_eq!(beneath(&a.children, r_1, &[0]), Vec::<usize>::new());
        assert_eq!(beneath(&a.children, 0, &[]), vec![r_1]);
    }

    /// A walk that draws every way down, stopping where it meets a bead
    /// already above it: the definition `cycles` is measured against.
    fn every_way_down(children: &[Vec<Link>]) -> BTreeSet<usize> {
        fn walk(
            at: usize,
            above: &mut Vec<usize>,
            children: &[Vec<Link>],
            cut: &mut BTreeSet<usize>,
        ) {
            above.push(at);
            for link in &children[at] {
                if above.contains(&link.bead) {
                    cut.insert(link.bead);
                } else {
                    walk(link.bead, above, children, cut);
                }
            }
            above.pop();
        }
        let mut cut = BTreeSet::new();
        walk(0, &mut Vec::new(), children, &mut cut);
        cut
    }

    /// Which beads a loop is cut at depends on the way the walk goes, and
    /// not only on which beads are on the loop: `c` and `x` are on one loop
    /// here and only `x`, met first, is where it is cut. Every shape below
    /// is answered the way a walk over every way down answers it — the
    /// last two are where remembering answers could go wrong, a loop
    /// reached under two beads that share it and a bead under itself.
    #[test]
    fn a_loop_is_reported_at_the_beads_a_walk_over_every_way_down_would_cut_it_at() {
        let met_from_above = tracker(&[
            bead("r", "open", &[]),
            bead("x", "open", &[dep("r", "parent-child"), dep("c", "blocks")]),
            bead("c", "open", &[dep("x", "blocks")]),
        ]);
        let both_ways_round = tracker(&[
            bead("r", "open", &[]),
            bead("a", "open", &[dep("r", "parent-child"), dep("b", "blocks")]),
            bead("b", "open", &[dep("r", "parent-child"), dep("a", "blocks")]),
        ]);
        let shared_above = tracker(&[
            bead("r", "open", &[]),
            bead("p", "open", &[dep("r", "parent-child"), dep("m", "blocks")]),
            bead("q", "open", &[dep("r", "parent-child"), dep("m", "blocks")]),
            bead("m", "open", &[dep("n", "blocks")]),
            bead("n", "open", &[dep("m", "blocks")]),
        ]);
        let under_itself = tracker(&[
            bead("r", "open", &[]),
            bead("s", "open", &[dep("r", "parent-child"), dep("s", "blocks")]),
        ]);

        for (json, expected) in [
            (met_from_above, vec!["x"]),
            (both_ways_round, vec!["a", "b"]),
            (shared_above, vec!["m"]),
            (under_itself, vec!["s"]),
        ] {
            let a = assembled(&json, ROOT);
            let reference: Vec<&str> = every_way_down(&a.children)
                .into_iter()
                .map(|bead| a.beads[bead].id.as_str())
                .collect();
            assert_eq!(a.cycles, expected, "{json}");
            assert_eq!(a.cycles, reference, "{json}");
        }
    }

    #[test]
    fn a_tree_with_no_loop_reports_none() {
        assert!(assembled(FIXTURE, FIXTURE_ROOT).cycles.is_empty());
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
        assert_eq!(drawn(&a, "r.2"), 1);
    }

    #[test]
    fn the_root_leads_the_rows_at_depth_zero() {
        let a = assembled(FIXTURE, FIXTURE_ROOT);
        assert_eq!(a.beads[0].id, FIXTURE_ROOT);
        assert_eq!(
            rows(&a)[0],
            (FIXTURE_ROOT, 0, None),
            "nothing reached the root"
        );
    }

    #[test]
    fn a_descendant_follows_its_own_forebear_rather_than_the_next_sibling() {
        // `bdi-2bb.5` waits on `bdi-2bb.3`, so the blocker sits directly under
        // it rather than after the sibling that comes next.
        let a = assembled(FIXTURE, FIXTURE_ROOT);
        let rows = rows(&a);
        let at = rows
            .iter()
            .position(|(id, _, _)| *id == "bdi-2bb.5")
            .expect("the waiting bead is drawn");

        assert_eq!(
            rows[at + 1],
            ("bdi-2bb.3", rows[at].1 + 1, Some(Edge::Blocks))
        );
    }

    #[test]
    fn the_tracker_draws_a_blocker_under_each_bead_that_waits_on_it() {
        // `bdi-2bb.3` blocks both `bdi-2bb.4` and `bdi-2bb.5`, and is a child
        // of the root besides.
        let a = assembled(FIXTURE, FIXTURE_ROOT);

        assert_eq!(drawn(&a, "bdi-2bb.3"), 3);
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

        assert_eq!(a.beads[0].id, "r");
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

        assert_eq!(ids(&a)[1], "bdi-r5l", "the bead in flight leads");
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
    /// every root there is. Measured against a real tracker on 2026-08-31: two
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
        let beads = parse_beads(json).unwrap();
        let err = Nesting::of(&beads)
            .assemble("three")
            .expect_err("no bead three to draw from")
            .to_string();

        assert!(err.contains("three"), "names the root asked for: {err}");
        assert!(
            Nesting::of(&[]).assemble("one").is_err(),
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

    #[test]
    fn nothing_is_unrolled_from_no_tree() {
        assert!(unroll(&[]).is_empty());
    }
}
