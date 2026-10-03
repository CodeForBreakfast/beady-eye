use super::*;
use pretty_assertions::assert_eq;

/// The bead this is for. A row shut over a branch is the only thing on
/// the screen standing for it, and until now it said its own fraction
/// and its own agent and nothing about the seats inside it.
///
/// A fold `bdi` set itself never closes over an agent, so the row that
/// needs this is one the reader shut by hand — which is exactly when
/// they have stopped looking at the branch and most need to be told
/// somebody is still in it.
#[test]
fn a_branch_shut_over_a_working_agent_says_how_many_are_inside_it() {
    let mut forest = flatten(alone("dunwich", SIDING, &panes_on(&["sdg-4.3.1"])));
    select(&mut forest, &key("dunwich", "sdg-4.3"));

    forest.apply(Action::ToggleFold);

    assert_eq!(fold_of(&forest, "sdg-4.3"), Some(false));
    assert_eq!(
        row_of(&forest, "sdg-4.3")
            .shut_over
            .as_ref()
            .map(|c| c.live_agents),
        Some(1)
    );
}

/// One rule at every depth, which is the principle the bead is about. A
/// root is a bead row like any other since `bdi-2bb.25`, and the aggregate
/// it lost went to the project line and widened over every root there. So
/// the root asks the same question a branch three levels down asks, and
/// gets the same answer about its own tree.
#[test]
fn a_root_shut_over_a_working_agent_says_it_exactly_as_a_branch_does() {
    let mut forest = flatten(alone("dunwich", SIDING, &panes_on(&["sdg-4.3.1"])));
    select(&mut forest, &key("dunwich", "sdg-4"));

    forest.apply(Action::ToggleFold);

    assert_eq!(fold_of(&forest, "sdg-4"), Some(false));
    assert_eq!(
        row_of(&forest, "sdg-4")
            .shut_over
            .as_ref()
            .map(|c| c.live_agents),
        Some(1)
    );
}

/// Opened, the seats are on their own rows and counting them again above
/// would be the same fact twice. What a line says here is what it is
/// hiding, not a standing property of the bead.
#[test]
fn a_branch_opened_over_its_agents_stops_counting_them() {
    let forest = flatten(alone("dunwich", SIDING, &panes_on(&["sdg-4.3.1"])));

    assert_eq!(fold_of(&forest, "sdg-4.3"), Some(true));
    assert_eq!(row_of(&forest, "sdg-4.3").shut_over, None);
}

/// Counted over what the fold hides and not over the bead asking. The
/// row already says its own agent by name, and a count taking that one in
/// would have a reader add the name to the number and come out with one
/// agent too many.
#[test]
fn the_agents_a_line_counts_are_the_ones_it_hides_and_never_its_own() {
    let mut forest = flatten(alone(
        "dunwich",
        SIDING,
        &panes_on(&["sdg-4.3", "sdg-4.3.1"]),
    ));
    select(&mut forest, &key("dunwich", "sdg-4.3"));

    forest.apply(Action::ToggleFold);

    let row = row_of(&forest, "sdg-4.3");
    assert!(row.agent.is_some(), "the row names its own agent");
    assert_eq!(row.shut_over.as_ref().map(|c| c.live_agents), Some(1));
}

/// The other half of what a fold hides, and the reason it is not agents
/// alone: `lines::live_beneath` — the whole of the fold default — is an
/// agent on a bead *or* an anomaly against it, so a row saying one and
/// not the other would leave a fresh exception where two were closed.
///
/// A pane still on a closed bead is the stale-pane anomaly, and it
/// carries an agent too, so the two counts are read off the one bead and
/// cannot be answering with each other.
#[test]
fn a_branch_shut_over_a_bead_wanting_looking_at_says_how_many_are_inside_it() {
    let stale = edited(
        SIDING,
        r#"{"id":"sdg-4.3.1","title":"prove the interlocking","status":"open"#,
        r#"{"id":"sdg-4.3.1","title":"prove the interlocking","closed_at":"2026-08-28T09:00:00Z","status":"closed"#,
    );
    let mut forest = flatten(alone("dunwich", &stale, &panes_on(&["sdg-4.3.1"])));
    select(&mut forest, &key("dunwich", "sdg-4.3"));

    forest.apply(Action::ToggleFold);

    let shut_over = row_of(&forest, "sdg-4.3")
        .shut_over
        .clone()
        .expect("a shut branch says what it hides");
    assert_eq!(shut_over.anomalies, 1);
    assert_eq!(shut_over.live_agents, 1);
}

/// Work, not rows — the rule every other count on this screen follows. A
/// blocker two of a branch's descendants share is drawn beneath each of
/// them and is one seat, and a line adding its rows would send a reader
/// hunting for a second agent that is not there.
#[test]
fn one_agent_reached_two_ways_down_is_counted_once() {
    let mut forest = under_every_copy(alone("dunwich", SHARED_IN_A_RUN, &panes_on(&["lck-2"])));
    assert_eq!(
        lines_of(&forest, "lck-2").len(),
        2,
        "{:#?}",
        sketch(&forest)
    );
    select(&mut forest, &key("dunwich", "lck-1"));

    forest.apply(Action::ToggleFold);

    assert_eq!(
        row_of(&forest, "lck-1")
            .shut_over
            .as_ref()
            .map(|c| c.live_agents),
        Some(1)
    );
}

/// The second depth exception the bead names, in the same place as the
/// first: a root shut over unfinished work said nothing, while a closed
/// branch one line down in the same state said how much. One rule, two
/// answers, and nothing about a root that earns the difference.
#[test]
fn a_closed_root_shut_over_unfinished_work_says_how_much_like_any_other_row() {
    // `sdg-4.3` opens too: `in_progress` with no pane is an orphan claim,
    // and no fold `bdi` sets itself closes over one.
    let done = edited(
        &edited(
            SIDING,
            r#"{"id":"sdg-4.3","title":"re-signal the box","status":"in_progress"#,
            r#"{"id":"sdg-4.3","title":"re-signal the box","status":"open"#,
        ),
        r#"{"id":"sdg-4","title":"re-point the crossover","status":"in_progress"#,
        r#"{"id":"sdg-4","title":"re-point the crossover","closed_at":"2026-08-29T09:00:00Z","status":"closed"#,
    );
    let forest = flatten(alone("dunwich", &done, &[]));

    assert_eq!(fold_of(&forest, "sdg-4"), Some(false));
    assert_eq!(
        row_of(&forest, "sdg-4").notes,
        vec![phrase::unfinished_beneath(5)]
    );
}

/// The case Graeme chose this default for. `sdg-4.1` is closed and its
/// glyph says so, but `bd` will start `sdg-4.1.2` today, and a reader
/// looking for what to pick up should not have to press a key to find it.
///
/// Down the spine and no wider: `sdg-4.1` opens because the ready bead is
/// under it, `sdg-4.1.1` stays shut because none is under that, and the
/// count moves down to the line that is now the one doing the hiding.
#[test]
fn a_closed_branch_over_ready_work_rests_open_down_the_spine_to_it() {
    let forest = flatten(ready_alone(
        "dunwich",
        SIDING,
        &panes_on(&["sdg-4.3"]),
        &["sdg-4.1.2"],
    ));

    assert_eq!(
        sketch(&forest),
        vec![
            "▾ dunwich",
            "  └── ◐ sdg-4 re-point the crossover",
            "      ├─▸ ◐ .3 re-signal the box",
            "      ├── ✓ .1 slew the up line",
            "      │   ├── ○ .2 weld the closure rail",
            "      │   ├─▸ ✓ .1 key the switch",
            "      │   └── ✓ .3 lift the old chairs",
            "      └─▸ ✓ .2 clip the down line",
        ]
    );
    assert_eq!(row_of(&forest, "sdg-4.1").notes, Vec::<String>::new());
    assert_eq!(
        row_of(&forest, "sdg-4.1.1").notes,
        vec![phrase::unfinished_beneath(2)]
    );
}

/// The other half of the widened rule, and the reason it is `bd ready`
/// and not a status test: work that is unfinished but blocked or deferred
/// is not what a reader needs next, so it earns no fold. The statuses
/// here are the ones that most look like work in hand, and the branch
/// rests shut over all three exactly as it does over open ones.
#[test]
fn a_closed_branch_over_work_bd_will_not_start_rests_shut_and_says_how_much() {
    let waiting = edited(
        &edited(
            SIDING,
            r#""status":"open",
   "dependencies":[{"depends_on_id":"sdg-4.1.1","type":"parent-child"}]"#,
            r#""status":"blocked",
   "dependencies":[{"depends_on_id":"sdg-4.1.1","type":"parent-child"}]"#,
        ),
        r#""status":"open",
   "dependencies":[{"depends_on_id":"sdg-4.1","type":"parent-child"}]"#,
        r#""status":"deferred",
   "dependencies":[{"depends_on_id":"sdg-4.1","type":"parent-child"}]"#,
    );
    let forest = flatten(alone("dunwich", &waiting, &panes_on(&["sdg-4.3"])));

    assert_eq!(fold_of(&forest, "sdg-4.1"), Some(false));
    assert_eq!(
        row_of(&forest, "sdg-4.1").notes,
        vec![phrase::unfinished_beneath(3)]
    );
}

/// Collapsed, not dropped: it is the existing fold, and opening it draws
/// what it held under the same rules as anywhere else.
#[test]
fn opening_a_finished_subtree_draws_what_it_holds() {
    let mut forest = flatten(finished_branches());
    select(&mut forest, &key("dunwich", "dep-1.2"));

    forest.apply(Action::ToggleFold);

    assert_eq!(
        sketch(&forest),
        vec![
            "▾ dunwich",
            "  └── ◐ dep-1 re-lay the sidings",
            "      ├── ○ .1 grade the bed",
            "      ├── ○ .3 clear the ballast",
            "      ├── ✓ .2 lift the old rail",
            "      │   └─▸ … 3 more",
            "      └── ✓ .4 burn the sleepers",
        ]
    );
}
