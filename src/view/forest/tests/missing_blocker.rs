//! A blocker no tracker holds, drawn where it would hang.

use super::*;
use pretty_assertions::assert_eq;

/// The lines from `id`'s first line down to the next line at its depth
/// or above, sketched.
fn sketch_under(forest: &Forest, id: &str) -> Vec<String> {
    let at = lines_of(forest, id)[0];
    let depth = forest.lines()[at].depth;
    let below = forest
        .lines()
        .iter()
        .skip(at + 1)
        .take_while(|line| line.depth > depth)
        .count();
    sketch(forest)[at..=at + below].to_vec()
}

/// Where the blocker would hang, a line says why it does not, after the
/// blockers that are drawn.
#[test]
fn a_blocker_no_tracker_holds_is_drawn_where_it_would_hang() {
    let mut forest = flatten(harbour_and_dunwich(
        r#"[{"id":"hbr-1","title":"clear the berth","status":"blocked",
             "dependencies":[{"depends_on_id":"dun-404","type":"blocks"},
                             {"depends_on_id":"dun-7","type":"blocks"}]}]"#,
        r#"[{"id":"dun-7","title":"lift the ground station","status":"open"}]"#,
        &[("harbour", "hbr-1")],
        &[],
    ));
    toggle_fold_of(&mut forest, "hbr-1");

    assert_eq!(
        sketch_under(&forest, "hbr-1"),
        vec![
            "  └── ● hbr-1 clear the berth".to_string(),
            "      ├── ! OrphanedDependencies(1)".to_string(),
            "      ├┄┄ ○ dun-7 lift the ground station".to_string(),
            "      └┄┄ ⚠ dun-404 NotHeld { projects: [\"dunwich\"] }".to_string(),
        ],
        "{:#?}",
        sketch(&forest)
    );
}

/// A bead the forest opens on its own, for an agent beneath it, draws the
/// line after the blocker it opened for, as the last thing beneath it.
#[test]
fn a_bead_opened_for_an_agent_draws_its_missing_blocker_last_beneath_it() {
    let forest = flatten(harbour_and_dunwich(
        r#"[{"id":"hbr-1","title":"clear the berth","status":"blocked"},
            {"id":"hbr-1.1","title":"sound the channel","status":"open",
             "dependencies":[{"depends_on_id":"hbr-1","type":"parent-child"},
                             {"depends_on_id":"dun-404","type":"blocks"},
                             {"depends_on_id":"dun-7","type":"blocks"}]},
            {"id":"hbr-1.2","title":"moor the tender","status":"open",
             "dependencies":[{"depends_on_id":"hbr-1","type":"parent-child"}]}]"#,
        r#"[{"id":"dun-7","title":"lift the ground station","status":"open"}]"#,
        &[("harbour", "hbr-1")],
        &["dun-7"],
    ));

    assert_eq!(
        sketch_under(&forest, "hbr-1"),
        vec![
            "  └── ● hbr-1 clear the berth".to_string(),
            "      ├── ! OrphanedDependencies(1)".to_string(),
            "      ├── ○ .1 sound the channel".to_string(),
            "      │   ├┄┄ ○ dun-7 lift the ground station".to_string(),
            "      │   └┄┄ ⚠ dun-404 NotHeld { projects: [\"dunwich\"] }".to_string(),
            "      └── ○ .2 moor the tender".to_string(),
        ],
        "{:#?}",
        sketch(&forest)
    );
}

/// A bead whose one blocker is missing, opened with the rest of the
/// forest rather than by a fold of its own, is counted with the line
/// beneath it. What is drawn on reaching in is exactly what was counted.
#[test]
fn a_bead_opened_with_the_forest_counts_the_line_saying_its_blocker_is_missing() {
    let mut forest = flatten(harbour_and_dunwich(
        r#"[{"id":"hbr-1","title":"clear the berth","status":"blocked"},
            {"id":"hbr-1.1","title":"sound the channel","status":"open",
             "dependencies":[{"depends_on_id":"hbr-1","type":"parent-child"},
                             {"depends_on_id":"dun-404","type":"blocks"}]}]"#,
        r#"[{"id":"dun-7","title":"lift the ground station","status":"open"}]"#,
        &[("harbour", "hbr-1")],
        &[],
    ));
    forest.apply(Action::ExpandForest);
    let drawn = forest.lines();
    let mut lines = 0;
    for top in drawn.top() {
        drawn.visit(top, &mut |_| {
            lines += 1;
            true
        });
    }

    assert_eq!(lines, drawn.len(), "{:#?}", sketch(&forest));
    assert_eq!(
        sketch_under(&forest, "hbr-1.1")[1..],
        ["          └┄┄ ⚠ dun-404 NotHeld { projects: [\"dunwich\"] }".to_string()],
        "{:#?}",
        sketch(&forest)
    );
}

/// A run of finished children stays last, after the line saying which
/// blocker is missing, whether the bead was opened by its fold or with
/// the rest of the forest.
#[test]
fn a_missing_blocker_is_drawn_above_the_run_of_finished_children() {
    let snapshot = || {
        harbour_and_dunwich(
            r#"[{"id":"hbr-1","title":"clear the berth","status":"blocked"},
                {"id":"hbr-1.1","title":"sound the channel","status":"blocked",
                 "dependencies":[{"depends_on_id":"hbr-1","type":"parent-child"},
                                 {"depends_on_id":"dun-404","type":"blocks"}]},
                {"id":"hbr-1.1.1","title":"take the soundings","status":"closed",
                 "dependencies":[{"depends_on_id":"hbr-1.1","type":"parent-child"}]},
                {"id":"hbr-1.1.2","title":"chart the shoal","status":"closed",
                 "dependencies":[{"depends_on_id":"hbr-1.1","type":"parent-child"}]},
                {"id":"hbr-1.1.3","title":"buoy the channel","status":"closed",
                 "dependencies":[{"depends_on_id":"hbr-1.1","type":"parent-child"}]}]"#,
            r#"[{"id":"dun-7","title":"lift the ground station","status":"open"}]"#,
            &[("harbour", "hbr-1")],
            &[],
        )
    };
    let mut folded = flatten(snapshot());
    toggle_fold_of(&mut folded, "hbr-1");
    toggle_fold_of(&mut folded, "hbr-1.1");
    let mut expanded = flatten(snapshot());
    expanded.apply(Action::ExpandForest);

    for forest in [&folded, &expanded] {
        let under = sketch_under(forest, "hbr-1.1");
        assert_eq!(
            under[1],
            "          ├┄┄ ⚠ dun-404 NotHeld { projects: [\"dunwich\"] }",
            "{:#?}",
            sketch(forest)
        );
        assert!(
            under[2].starts_with("          └") && under[2].ends_with("… 3 more"),
            "{:#?}",
            sketch(forest)
        );
    }
}

/// Drawn on reaching in, the lines saying which blockers are missing
/// follow the blockers that are drawn, and only the last of them closes
/// the arm.
#[test]
fn a_bead_opened_with_the_forest_draws_its_missing_blockers_after_the_rest() {
    let mut forest = flatten(harbour_and_dunwich(
        r#"[{"id":"hbr-1","title":"clear the berth","status":"blocked"},
            {"id":"hbr-1.1","title":"sound the channel","status":"open",
             "dependencies":[{"depends_on_id":"hbr-1","type":"parent-child"},
                             {"depends_on_id":"dun-7","type":"blocks"},
                             {"depends_on_id":"dun-404","type":"blocks"},
                             {"depends_on_id":"dun-405","type":"blocks"}]}]"#,
        r#"[{"id":"dun-7","title":"lift the ground station","status":"open"}]"#,
        &[("harbour", "hbr-1")],
        &[],
    ));
    forest.apply(Action::ExpandForest);

    assert_eq!(
        sketch_under(&forest, "hbr-1.1")[1..],
        [
            "          ├┄┄ ○ dun-7 lift the ground station",
            "          ├┄┄ ⚠ dun-404 NotHeld { projects: [\"dunwich\"] }",
            "          └┄┄ ⚠ dun-405 NotHeld { projects: [\"dunwich\"] }",
        ]
        .map(String::from),
        "{:#?}",
        sketch(&forest)
    );
}

/// A bead whose one blocker is missing has something beneath it, so it
/// folds, and the line is drawn when it opens.
#[test]
fn a_bead_waiting_only_on_a_blocker_no_tracker_holds_folds_over_the_line_saying_so() {
    let mut forest = flatten(harbour_and_dunwich(
        r#"[{"id":"hbr-1","title":"clear the berth","status":"blocked"},
            {"id":"hbr-1.1","title":"sound the channel","status":"open",
             "dependencies":[{"depends_on_id":"hbr-1","type":"parent-child"},
                             {"depends_on_id":"dun-404","type":"blocks"}]}]"#,
        r#"[{"id":"dun-7","title":"lift the ground station","status":"open"}]"#,
        &[("harbour", "hbr-1")],
        &[],
    ));
    toggle_fold_of(&mut forest, "hbr-1");
    let waiting = lines_of(&forest, "hbr-1.1")[0];
    let shut = forest.lines()[waiting].folded;

    toggle_fold_of(&mut forest, "hbr-1.1");

    assert_eq!(shut, Some(false), "{:#?}", sketch(&forest));
    assert_eq!(
        sketch_under(&forest, "hbr-1.1")[1..],
        ["          └┄┄ ⚠ dun-404 NotHeld { projects: [\"dunwich\"] }".to_string()],
        "{:#?}",
        sketch(&forest)
    );
}
