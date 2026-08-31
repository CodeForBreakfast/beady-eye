//! A project's own line — what it is and how much work it holds — and the
//! roots beneath it that would not read.

use ratatui::style::Style;
use ratatui::text::Span;

use crate::model::snapshot::{Counts, TrackerState};
use crate::view::fitted::{Fitted, GAP};
use crate::view::lines::{ProjectLine, Recovery, Unread};
use crate::view::phrase;
use crate::view::row::WARNING;

use super::tone::{LIVE, LOOK_AT_THIS};
use super::{done, pane_marker, structure};

/// A project's own line: what it is, how much work it holds, and the live
/// panes recovered for it where a root would not read.
///
/// It says nothing about any one root, because every root below it says that
/// for itself. What is left is what only a project can answer: which project,
/// how much of it there is, and — where a root refused — which panes `bdi`
/// found working here that no bead could be attributed to.
pub(super) fn project_line(project: &ProjectLine, prefix: &str) -> Fitted {
    let identity = vec![
        Span::raw(prefix.to_string()),
        Span::raw(project.project.clone()),
    ];

    let mut state = summary(&project.counts);
    if let Some(found) = &project.recovery {
        if !state.is_empty() {
            state.push(Span::raw(" ".repeat(GAP)));
        }
        state.push(recovered(found));
    }

    Fitted::new(identity, Vec::new(), state)
}

/// A root that drew no row, said where its row would have been.
///
/// It holds the same columns a bead row does — the mark, then the id — so a
/// reader scanning a project's roots meets it in the column the others are in
/// rather than having to find it.
pub(super) fn unread_line(unread: &Unread, prefix: &str, id_width: usize) -> Fitted {
    let identity = vec![
        structure(prefix),
        Span::styled(WARNING.to_string(), Style::new().fg(LOOK_AT_THIS)),
        Span::raw(format!(" {:id_width$}", unread.root)),
    ];
    let why = match unread.tracker {
        TrackerState::Unreachable(failure) => phrase::tracker_failure(failure),
        TrackerState::Ok => phrase::root_unread(),
    };

    Fitted::new(
        identity,
        vec![Span::styled(why.to_string(), Style::new().fg(LOOK_AT_THIS))],
        Vec::new(),
    )
}

/// How much of a tree is done, who is on it, and how much of it wants looking
/// at. A count that is zero is left out rather than drawn as a zero: a row of
/// noughts reads as something to check.
fn summary(counts: &Counts) -> Vec<Span<'static>> {
    // Nothing counted means no root here read at all, and `0/0` would say the
    // opposite of what is true — that they were read and hold nothing.
    let mut said = match counts.total {
        0 => Vec::new(),
        total => vec![Span::raw(done(counts.closed, total))],
    };
    if counts.live_agents > 0 {
        let agent = if counts.live_agents == 1 {
            "agent"
        } else {
            "agents"
        };
        said.push(Span::raw(" ".repeat(GAP)));
        said.push(Span::styled(
            format!("{} {agent}", counts.live_agents),
            Style::new().fg(LIVE),
        ));
    }
    if counts.anomalies > 0 {
        said.push(Span::raw(" ".repeat(GAP)));
        said.push(Span::styled(
            format!("{WARNING} {}", counts.anomalies),
            Style::new().fg(LOOK_AT_THIS),
        ));
    }
    said
}

/// The live panes found working in a project no bead could be read to
/// attribute them to. Where they cannot be known to be all of them it says so
/// — a list that is quietly short is the one way this can be read wrongly,
/// because it looks exactly like a complete one.
fn recovered(found: &Recovery) -> Span<'static> {
    let panes = &found.panes;
    let mut said = Vec::new();
    said.push(if panes.is_empty() {
        phrase::no_live_panes().to_string()
    } else {
        panes
            .iter()
            .map(|pane| pane_marker(&pane.pane, &pane.pane_status))
            .collect::<Vec<_>>()
            .join(" · ")
    });
    if !found.complete {
        said.push(phrase::panes_may_be_incomplete().to_string());
    }

    Span::styled(said.join(" · "), Style::new().fg(LOOK_AT_THIS))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    use crate::model::snapshot::{HerdrState, LoosePane, TrackerFailure};
    use crate::model::types::{PaneStatus, Status};
    use crate::view::draw::tone::status_colour;
    use crate::view::draw::{fitted, tests::*};
    use crate::view::forest::flatten;
    use crate::view::row;

    // ---- a project's line ------------------------------------------------

    /// The same, for a project where a root refused and whose panes had to be
    /// recovered from herdr instead.
    fn recovering(name: &str, panes: &[LoosePane], complete: bool) -> ProjectLine {
        ProjectLine {
            recovery: Some(Recovery {
                panes: panes.to_vec(),
                complete,
            }),
            ..project(name, counts(0, 0, 0, 0))
        }
    }

    fn unread(root: &str, tracker: TrackerState) -> Unread {
        Unread {
            root: root.into(),
            tracker,
        }
    }

    /// The design's own example, at the width it was written for. What is the
    /// project's is here; what is a root's is on the root's own row below.
    #[test]
    fn a_project_line_says_which_project_it_is_and_how_much_of_it_is_done() {
        let counts = counts(8, 21, 3, 3);

        assert_eq!(
            drawn(project_line(&project("summit-works", counts), OPEN), 40, 1),
            vec!["▾ summit-works       8/21  3 agents  ⚠ 3"]
        );
    }

    /// A count of nothing is left out rather than drawn as a nought: a line
    /// reading `0 agents  ⚠ 0` sends a reader looking for rows that are not
    /// there.
    #[test]
    fn a_project_with_no_live_agent_and_nothing_wrong_says_only_how_much_is_done() {
        let counts = counts(2, 7, 0, 0);

        assert_eq!(
            drawn(project_line(&project("homelab", counts), SHUT), 30, 1),
            vec!["▸ homelab                  2/7"]
        );
    }

    #[test]
    fn one_agent_is_not_described_in_the_plural() {
        let counts = counts(2, 7, 1, 0);
        let drawn = drawn(project_line(&project("homelab", counts), SHUT), 40, 1);

        assert!(drawn[0].ends_with("2/7  1 agent"), "{drawn:?}");
    }

    /// Narrower than the identity itself there is nothing left to protect, and
    /// the line is cut like any other.
    #[test]
    fn a_width_too_narrow_for_anything_else_keeps_as_much_of_the_project_as_it_can() {
        let counts = counts(8, 21, 3, 3);

        assert_eq!(
            drawn(project_line(&project("summit-works", counts), OPEN), 10, 1),
            vec!["▾ nixos-c…"]
        );
    }

    /// Width is columns on a screen, not bytes in a string. Every glyph in
    /// this vocabulary is several bytes long, and a cut counted in bytes would
    /// land inside one and put a broken character on the terminal.
    #[test]
    fn a_cut_is_counted_in_columns_and_never_lands_inside_a_glyph() {
        let name = "→→→→→→→→→→→→→→→→→→→→→→→→→→→→→→";
        let drawn = drawn(
            project_line(&project(name, counts(0, 1, 0, 0)), OPEN),
            20,
            1,
        );

        assert_eq!(drawn[0].chars().count(), 20);
        assert!(!drawn[0].contains('\u{fffd}'), "{drawn:?}");
    }

    // ---- a root's own row ------------------------------------------------

    /// `bdi-2bb.25`: a root is a bead like any other, so its status reaches
    /// the screen through the two channels every other bead's does — the
    /// glyph, and the colour that glyph is painted. Before this it was drawn
    /// on a header that spoke a project's language and answered none of it.
    ///
    /// Asked through `status_glyph` and `status_colour` rather than written
    /// out, so the mappings stay in the one place each owns.
    #[test]
    fn a_root_is_drawn_with_its_own_status_glyph_like_any_other_bead() {
        let forest = flatten(&snapshot(vec![grove(2)], Vec::new(), HerdrState::Ok));
        let root = &forest.lines()[1];

        let painted = painted(fitted(root, 12), 60);
        let drawn = drawn(fitted(root, 12), 60, 1);

        assert!(
            drawn[0].contains(&format!(
                "{} nix-9670s",
                row::status_glyph(&Status::InProgress)
            )),
            "{drawn:?}"
        );
        assert!(
            painted.iter().any(|(said, colour)| said
                .contains(row::status_glyph(&Status::InProgress))
                && Some(*colour) == status_colour(&Status::InProgress)),
            "{painted:?}"
        );
    }

    // ---- a root that would not read --------------------------------------

    /// A root `bdi` was told about and could not read has no row to draw, and
    /// leaving it out would lose it as surely as dropping it. It is named
    /// where its row would have been, with the reason beside it.
    #[test]
    fn an_unread_root_is_named_where_its_row_would_have_been_with_the_reason() {
        let unread = unread(
            "nix-9670s",
            TrackerState::Unreachable(TrackerFailure::Unavailable),
        );
        let drawn = drawn(unread_line(&unread, LAST, 9), 60, 1);

        says(&drawn[0], "nix-9670s");
        says(&drawn[0], "the tracker did not answer");
    }

    /// A tracker that could not be read has no counts, and `0/0` would say the
    /// opposite of what is true — that it was read and holds nothing.
    #[test]
    fn an_unread_root_never_shows_a_count_it_could_not_read() {
        let unread = unread("nix-9670s", TrackerState::Unreachable(TrackerFailure::Auth));
        let drawn = drawn(unread_line(&unread, LAST, 9), 120, 1);

        does_not_say(&drawn[0], "0/0");
    }

    /// Nothing should reach this: a root that read is a bead row, and one that
    /// did not carries the failure that stopped it. A root that got here
    /// anyway is still a root on the screen, which is the whole point.
    #[test]
    fn a_root_with_no_row_and_no_reason_still_says_it_is_there() {
        let drawn = drawn(
            unread_line(&unread("nix-9670s", TrackerState::Ok), LAST, 9),
            90,
            1,
        );

        says(&drawn[0], "nix-9670s");
        says(&drawn[0], "this root drew no rows, and nothing said why");
    }

    /// The design has a project whose roots would not read render its panes.
    /// They are named the way a bead's agent is named, so one reads as the
    /// other.
    #[test]
    fn a_project_with_a_root_it_could_not_read_shows_the_panes_working_in_it() {
        let panes = [
            pane("wCM:p9", PaneStatus::Working),
            pane("wCM:p6", PaneStatus::Idle),
        ];

        assert_eq!(
            drawn(
                project_line(&recovering("summit-works", &panes, true), NO_FOLD),
                80,
                1
            ),
            vec![
                "  summit-works                                  ◍ wCM:p9 working · ◍ wCM:p6 idle"
                    .to_string()
            ]
        );
    }

    #[test]
    fn a_project_with_no_pane_to_show_says_that_rather_than_nothing() {
        let drawn = drawn(
            project_line(&recovering("summit-works", &[], true), OPEN),
            120,
            1,
        );

        says(&drawn[0], "no live pane names this project");
    }

    /// A pane list that cannot be known to be whole says so. A silently short
    /// list is the one way this line can be read wrongly, because it looks
    /// exactly like a complete one.
    #[test]
    fn a_pane_list_that_may_be_short_says_so_rather_than_reading_as_complete() {
        let panes = [pane("wCM:p9", PaneStatus::Working)];

        let whole = drawn(
            project_line(&recovering("summit-works", &panes, true), OPEN),
            200,
            1,
        );
        let partial = drawn(
            project_line(&recovering("summit-works", &panes, false), OPEN),
            200,
            1,
        );

        does_not_say(&whole[0], "and possibly more");
        says(
            &partial[0],
            "and possibly more · a live pane under no configured project could belong here",
        );
    }

    /// The identity of a root outlasts everything else on its line: a reader
    /// who cannot tell which root failed learns nothing from knowing one did.
    #[test]
    fn a_narrow_unread_root_keeps_the_root_over_the_reason() {
        let unread = unread("nix-9670s", TrackerState::Unreachable(TrackerFailure::Auth));
        let drawn = drawn(unread_line(&unread, LAST, 9), 24, 1);

        says(&drawn[0], "nix-9670s");
        assert_eq!(drawn[0].chars().count(), 24);
    }
}
