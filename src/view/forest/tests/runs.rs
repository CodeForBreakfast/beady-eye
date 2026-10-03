use super::*;
use pretty_assertions::assert_eq;

/// A run is drawn with one status glyph standing for every bead it hides,
/// which is only honest while a run is closed beads and nothing else.
/// `dep-1.1` is open beside the two closed siblings that make the run, so
/// widening the predicate sweeps it in and fails here — rather than
/// leaving the glyph to say `closed` over a bead that is not.
#[test]
fn a_run_holds_closed_beads_and_nothing_else_which_is_what_lets_one_glyph_stand_for_it() {
    let tree = tree_of("dunwich", DEPOT);

    let mut runs = 0;
    for at in 0..tree.beads.len() {
        let (_, run) = split(&tree, at, &above(&tree, at));
        runs += usize::from(!run.is_empty());
        for member in run {
            let bead = &tree.beads[member.bead];
            assert!(
                bead.status.is_closed(),
                "{} is in a run and is {:?}",
                bead.id,
                bead.status
            );
        }
    }

    assert!(runs > 0, "the fixture built no run to check");
}

/// The run rule is a property of the forest, not of a place in it, so it
/// holds inside an open run as it does everywhere else. Nothing
/// disappears; it is counted one level down. Two folds to reach it now:
/// the run, and then the finished branch that rests shut inside it.
#[test]
fn an_open_run_elides_again_inside_itself() {
    let mut forest = flatten(depot());
    select_run(&mut forest);
    forest.apply(Action::ToggleFold);

    select(&mut forest, &key("dunwich", "dep-1.2"));
    forest.apply(Action::ToggleFold);

    assert_eq!(
        sketch(&forest)[..7],
        [
            "▾ dunwich",
            "  └── ◐ dep-1 re-lay the sidings",
            "      ├── ○ .1 grade the bed",
            "      └── … 6 more",
            "          ├── ✓ .2 lift the old rail",
            "          │   └─▸ … 3 more",
            "          ├── ✓ .3 clear the ballast",
        ]
    );
}

/// A run is a fold like any other, so a collection that lands under an
/// open one leaves it open and leaves the cursor on it.
#[test]
fn an_open_elided_run_survives_a_refresh() {
    let mut forest = flatten(snapshot());
    select_run(&mut forest);
    forest.apply(Action::ToggleFold);

    let reordered = edited(DUNWICH, r#""priority":3"#, r#""priority":1"#);
    forest.refresh(gather(
        vec![tree_of("dunwich", &reordered)],
        Vec::new(),
        Filter::LiveAgents,
    ));

    let drawn = sketch(&forest);
    assert!(
        drawn.contains(&"  │       └── ✓ .5 set the guard rail".to_string()),
        "{drawn:#?}"
    );
    assert_eq!(drawn[forest.selected_line()], "  │   └── … 3 more");
}

/// A run has no bead of its own, so a line the cursor is holding must not
/// report one: the loop picks the tail's pane from that field.
#[test]
fn a_selected_elided_run_stands_for_no_bead_of_its_own() {
    let mut forest = flatten(snapshot());

    select_run(&mut forest);

    let line = &forest.lines()[forest.selected_line()];
    assert!(matches!(line.content, Content::Elided { .. }));
    assert_eq!(line.bead(), None);
}

/// A closed bead with a pane still on it is the stale-pane anomaly, and
/// eliding it would hide a live agent.
#[test]
fn a_closed_bead_with_a_live_agent_is_drawn_rather_than_elided() {
    let forest = flatten(snapshot());

    assert!(sketch(&forest)
        .iter()
        .any(|line| line.contains("clear the access road")));
}

#[test]
fn a_single_quiet_closed_sibling_is_drawn_rather_than_said_as_a_count() {
    let one_closed = edited(
        DUNWICH,
        r#"{"id":"dun-7.3","title":"pour the pad","status":"closed"#,
        r#"{"id":"dun-7.3","title":"pour the pad","status":"open"#,
    );
    let snapshot = gather(
        vec![tree_of("dunwich", &one_closed)],
        Vec::new(),
        Filter::LiveAgents,
    );

    let drawn = sketch(&flatten(snapshot));

    assert!(
        drawn.iter().any(|line| line.contains("survey the mast")),
        "{drawn:#?}"
    );
    assert!(
        !drawn.iter().any(|line| line.contains("more")),
        "{drawn:#?}"
    );
}

/// The invariant this bead exists to restore: nothing `bdi` folds of its
/// own accord closes over a live agent or over an anomaly. Asked of the
/// forest at rest, before any fold is set by hand, because that is the
/// only state `bdi` chooses for itself.
#[test]
fn nothing_the_forest_folds_by_itself_hides_a_live_agent_or_an_anomaly() {
    let mut worth_drawing = 0;
    for json in [DUNWICH, DEPOT, RELAY] {
        let snapshot = alone("dunwich", json, &two_panes());
        let forest = flatten(snapshot.clone());
        let drawn: Vec<&str> = forest
            .lines()
            .iter()
            .filter_map(|line| line.bead().map(|key| key.id.as_str()))
            .collect();

        for node in &snapshot.trees[0].beads {
            if node.agent.is_none() && node.anomalies.is_empty() {
                continue;
            }
            worth_drawing += 1;
            assert!(
                drawn.contains(&node.id.as_str()),
                "{} carries an agent or an anomaly and is not on screen: {:#?}",
                node.id,
                sketch(&forest)
            );
        }
    }

    assert!(worth_drawing > 0, "the fixtures staffed nothing to check");
}

/// The same invariant for the half `bdi-wt0` added, asked one bead at a
/// time so a screen that happened to be open cannot answer for a rule
/// that is not there. Every unfinished bead in every fixture takes its
/// turn as the only ready one, and each turn is a whole forest whose
/// default has to reach it.
///
/// Position is the point. A bead behind three closed forebears, or in the
/// run a branch collapses to, is where a fold that opens one level would
/// still lose it.
#[test]
fn nothing_the_forest_folds_by_itself_hides_work_bd_would_start() {
    let mut asked = 0;
    for json in [DUNWICH, DEPOT, RELAY, SIDING, TOWER, KADATH] {
        let unstaffed = alone("dunwich", json, &[]);
        let unfinished: Vec<String> = unstaffed.trees[0]
            .beads
            .iter()
            .filter(|node| !node.status.is_closed())
            .map(|node| node.id.clone())
            .collect();

        for id in unfinished {
            asked += 1;
            let forest = flatten(ready_alone("dunwich", json, &[], &[&id]));
            let drawn: Vec<&str> = forest
                .lines()
                .iter()
                .filter_map(|line| line.bead().map(|key| key.id.as_str()))
                .collect();

            assert!(
                drawn.contains(&id.as_str()),
                "{id} is the one bead bd would start and is not on screen: {:#?}",
                sketch(&forest)
            );
        }
    }

    assert!(asked > 0, "the fixtures held no unfinished bead to ready");
}

/// The shape `bdi-4av` was raised on: a closed parent, a closed child, and
/// a live agent under both of them. The run asked its question of the
/// child alone, swept it in, and printed a sentence saying nobody was on
/// the beads it had just hidden the agent among.
#[test]
fn a_run_never_closes_over_a_subtree_with_a_live_agent_in_it() {
    let forest = flatten(alone("dunwich", RELAY, &two_panes()));

    assert_eq!(
        sketch(&forest),
        vec![
            "▾ dunwich",
            "  └── ◐ rly-2 re-site the relay",
            "      ├── ○ .1 trench the run",
            "      ├── ✓ .2 strike the old mast",
            "      │   └── ✓ .1 drop the guys",
            "      │       └── ◐ .1 cut the stays",
            "      ├── ✓ .4 lift the feeder",
            "      │   └── ✓ .1 coil the heliax",
            "      └─▸ … 4 more",
        ]
    );
}

/// A run's phrase says nobody is on the beads it counts, so it has to be
/// true of every bead it counts — not merely of the siblings it names.
/// The count and the set it describes are checked together, because it was
/// their disagreement that let the sentence lie.
///
/// The set is of beads rather than of rows, so `SHARED_IN_A_RUN` is here:
/// it is the only fixture whose run reaches one bead two ways, and under
/// every other one the two answers are the same number.
#[test]
fn a_run_counts_exactly_the_beads_its_phrase_is_true_of() {
    let mut runs = 0;
    for json in [DUNWICH, DEPOT, RELAY, SHARED_IN_A_RUN] {
        let tree = alone("dunwich", json, &two_panes()).trees.remove(0);

        for at in 0..tree.beads.len() {
            let above = above(&tree, at);
            let (_, run) = split(&tree, at, &above);
            if run.is_empty() {
                continue;
            }
            runs += 1;

            let below = way_below(&above, at);
            let mut behind = BTreeSet::new();
            let mut walking: Vec<usize> = run.iter().map(|link| link.bead).collect();
            while let Some(node) = walking.pop() {
                let bead = &tree.beads[node];
                behind.insert(bead.id.clone());
                assert!(
                    bead.status.is_closed() && bead.agent.is_none() && bead.anomalies.is_empty(),
                    "{} is behind a run that says nobody is on it",
                    bead.id
                );
                walking.extend(
                    tree::links_from(&tree.children, node, &below)
                        .into_iter()
                        .map(|link| link.bead),
                );
            }

            assert_eq!(run_size(&tree, &run, &below), behind.len());
        }
    }

    assert!(runs > 0, "the fixtures built no run to check");
}

/// A run says how many beads it holds, and a blocker two of its branches
/// share is one bead however many ways down there are to it. The count
/// stands in for beads that are not on the screen, so counting the rows
/// it saved would say the run holds work that does not exist.
///
/// `SHARED_IN_A_RUN` draws five beads on six rows, and the expected
/// number is written out here rather than walked, because a count taken
/// from the tree the count is about cannot disagree with it.
#[test]
fn a_run_counts_a_blocker_two_of_its_branches_share_once() {
    let forest = flatten(alone("dunwich", SHARED_IN_A_RUN, &[]));

    assert_eq!(
        sketch(&forest),
        vec![
            "▾ dunwich",
            "  └── ◐ lck-1 refit the lock gates",
            "      ├── ◐ .5 hang the new gates",
            "      └─▸ … 5 more",
        ]
    );
}

/// A branch that is finished all the way down is one line saying so: the
/// glyph is its own closed status, the fraction says every bead beneath it
/// is closed too, and the shut marker says it still holds them.
#[test]
fn a_wholly_finished_subtree_rests_as_one_line_that_says_it_is_finished() {
    let forest = flatten(finished_branches());

    assert_eq!(
        sketch(&forest),
        vec![
            "▾ dunwich",
            "  └── ◐ dep-1 re-lay the sidings",
            "      ├── ○ .1 grade the bed",
            "      ├── ○ .3 clear the ballast",
            "      ├─▸ ✓ .2 lift the old rail",
            "      └── ✓ .4 burn the sleepers",
        ]
    );
    assert_eq!(
        row_of(&forest, "dep-1.2").progress,
        Some(Progress {
            finished: 4,
            total: 4
        })
    );
}

/// A blocker is drawn beneath the bead it blocks, so a bead's children
/// are the work closing it unblocked. A closed bead standing over open ones is
/// therefore the healthy shape of this tree, and where nobody is on them
/// and `bd` will start none of them the branch rests shut under a row
/// whose glyph says done. What it holds is out of sight either way, so
/// the line says how much.
#[test]
fn a_closed_branch_resting_over_unfinished_work_says_how_much_it_holds() {
    let forest = flatten(siding());

    assert_eq!(
        sketch(&forest),
        vec![
            "▾ dunwich",
            "  └── ◐ sdg-4 re-point the crossover",
            "      ├─▸ ◐ .3 re-signal the box",
            "      ├─▸ ✓ .1 slew the up line",
            "      └─▸ ✓ .2 clip the down line",
        ]
    );
    assert_eq!(
        row_of(&forest, "sdg-4.1").notes,
        vec![phrase::unfinished_beneath(3)]
    );
}

/// The common case, and the reason the sentence is not on every closed
/// row: a branch that is done all the way down has nothing further to
/// say, and a count on it would be noise wherever the eye landed.
#[test]
fn a_closed_branch_that_is_finished_all_the_way_down_says_nothing_extra() {
    let forest = flatten(siding());

    assert_eq!(row_of(&forest, "sdg-4.2").notes, Vec::<String>::new());
}

/// Asked of the branch, not of the bead: `sdg-4.3` is unfinished itself
/// and holds one unfinished bead, and a walk that counted the bead it was
/// asked about would say two. Only closed nodes reach the note from the
/// renderer, where a self that is closed adds nothing and the difference
/// cannot show — but the same walk answers the agent count, which every
/// shut line asks whatever its own status is.
#[test]
fn what_a_branch_holds_never_counts_the_bead_it_was_asked_about() {
    let tree = tree_of("dunwich", SIDING);
    let (at, above) = way_to(&tree, "sdg-4.3");

    assert_eq!(counts_beneath(&tree, at, &above).unfinished(), 1);
}

/// Only a line whose own glyph says done. An unfinished bead resting shut
/// over unfinished work is not hiding anything its status did not already
/// admit, and a sentence on every such row is the noise that would stop
/// the closed ones being read.
#[test]
fn an_unfinished_branch_resting_shut_over_its_own_work_says_nothing_extra() {
    let forest = flatten(siding());

    assert_eq!(row_of(&forest, "sdg-4.3").notes, Vec::<String>::new());
}

/// Counted at every depth. With the open bead directly under `sdg-4.1`
/// closed, everything unfinished is two levels down, and a count of the
/// immediate children would leave the row silent over both of them.
#[test]
fn unfinished_work_two_levels_under_a_closed_branch_is_still_counted() {
    let deep = edited(
        SIDING,
        r#""status":"open",
   "dependencies":[{"depends_on_id":"sdg-4.1","type":"parent-child"}]"#,
        r#""status":"closed","closed_at":"2026-08-26T09:00:00Z",
   "dependencies":[{"depends_on_id":"sdg-4.1","type":"parent-child"}]"#,
    );
    let forest = flatten(alone("dunwich", &deep, &panes_on(&["sdg-4.3"])));

    assert_eq!(
        row_of(&forest, "sdg-4.1").notes,
        vec![phrase::unfinished_beneath(2)]
    );
}

/// The sentence and the fraction are two readings of one walk, so they
/// can never disagree: a row saying `2/5` and `3 unfinished beads` is the
/// same fact twice, once as arithmetic and once in words.
#[test]
fn the_count_a_closed_branch_gives_is_the_remainder_of_its_own_fraction() {
    let forest = flatten(siding());
    let row = row_of(&forest, "sdg-4.1");
    let progress = row.progress.expect("a branch has a fraction");

    assert_eq!(
        row.notes,
        vec![phrase::unfinished_beneath(
            progress.total - progress.finished
        )]
    );
}

/// Opened, the beads are on screen and counting them again above would be
/// noise. The sentence is what the shut line is hiding, not a standing
/// property of the bead.
#[test]
fn a_closed_branch_opened_over_its_work_stops_counting_it() {
    let mut forest = flatten(siding());
    select(&mut forest, &key("dunwich", "sdg-4.1"));

    forest.apply(Action::ToggleFold);

    assert_eq!(row_of(&forest, "sdg-4.1").notes, Vec::<String>::new());
}
