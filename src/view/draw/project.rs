//! A project's own line — what it is, how fresh it is and how much work it
//! holds — and the roots beneath it that would not read.

use chrono::{DateTime, Utc};
use ratatui::style::{Color, Style};
use ratatui::text::Span;

use crate::model::snapshot::{Counts, TrackerState};
use crate::view::fitted::{Fitted, GAP};
use crate::view::lines::{ProjectLine, Recovery, Unread};
use crate::view::phrase;
use crate::view::row::WARNING;
use crate::view::Freshness;

use super::tone::{LIVE, LOOK_AT_THIS};
use super::{done, pane_marker, structure};

/// A project's own line: what it is, how fresh it is, how much work it holds,
/// and the live panes recovered for it where a root would not read.
///
/// It says nothing about any one root, because every root below it says that
/// for itself. What is left is what only a project can answer: which project,
/// when it was last read, how much of it there is, and — where a root refused
/// — which panes `bdi` found working here that no bead could be attributed
/// to.
///
/// How fresh it is sits directly beside the name, because it is a claim about
/// that name's rows and nothing else's. It is handed in rather than held on
/// the line: a collection starting and ending changes it without changing the
/// snapshot, and a line that carried it would have to be flattened again to
/// turn the mark one frame.
///
/// It goes in the title, so it is the first thing a narrowing line gives up —
/// whole rather than cut, because half a mark and half an age each say
/// nothing — and the counts a reader came for outlast it.
pub(super) fn project_line(
    project: &ProjectLine,
    prefix: &str,
    how_fresh: Option<Freshness>,
    now: DateTime<Utc>,
) -> Fitted {
    let identity = vec![
        Span::raw(prefix.to_string()),
        Span::raw(project.project.clone()),
    ];

    // Drawn plain and dim: it is what a reader glances at to place the rest,
    // not one of the things the rest is asking them to look at.
    let how_fresh = how_fresh
        .map(|how_fresh| {
            Span::styled(
                phrase::freshness(how_fresh, now),
                Style::new().fg(Color::DarkGray),
            )
        })
        .into_iter()
        .collect::<Vec<_>>();

    let mut state = summary(&project.counts);
    if let Some(found) = &project.recovery {
        if !state.is_empty() {
            state.push(Span::raw(" ".repeat(GAP)));
        }
        state.push(recovered(found));
    }

    Fitted::new(identity, how_fresh, state).title_or_nothing()
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

    use crate::app::Wanted;
    use crate::model::anomaly::Anomaly;
    use crate::model::snapshot::{HerdrState, LoosePane, Node, Snapshot, TrackerFailure, Tree};
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

    /// A project line with nothing to say about how fresh it is, which is
    /// every line whose subject is something else.
    fn line(project: &ProjectLine, prefix: &str) -> Fitted {
        project_line(project, prefix, None, drawn_at())
    }

    /// The same line, said at an instant, with one project's freshness on it.
    fn line_that_is(project: &ProjectLine, how_fresh: Freshness) -> Fitted {
        project_line(project, OPEN, Some(how_fresh), drawn_at())
    }

    /// Two projects read at the same instant, so a mark on one of them is a
    /// mark this collection put there rather than a difference in the
    /// fixture.
    fn two_projects() -> Snapshot {
        let harbour = Tree {
            counts: counts(0, 1, 0, 0),
            nodes: vec![Node {
                depth: 0,
                ..node("qua-1", "moor the barge", Status::InProgress)
            }],
            ..tree("harbour", "qua-1", "moor the barge", counts(0, 0, 0, 0))
        };

        let mut snapshot = snapshot(vec![grove(1), harbour], Vec::new(), HerdrState::Ok);
        snapshot.read_at.insert("harbour".to_string(), read_at());
        snapshot
    }

    /// The row of a frame a project's line is on.
    fn project_row<'a>(frame: &'a [String], project: &str) -> &'a str {
        frame
            .iter()
            .find(|row| row.contains(project))
            .unwrap_or_else(|| panic!("{project} is on the frame: {frame:?}"))
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
            drawn(line(&project("summit-works", counts), OPEN), 40, 1),
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
            drawn(line(&project("homelab", counts), SHUT), 30, 1),
            vec!["▸ homelab                  2/7"]
        );
    }

    #[test]
    fn one_agent_is_not_described_in_the_plural() {
        let counts = counts(2, 7, 1, 0);
        let drawn = drawn(line(&project("homelab", counts), SHUT), 40, 1);

        assert!(drawn[0].ends_with("2/7  1 agent"), "{drawn:?}");
    }

    /// `bdi-2bb.36`: the filter counts anomalies so a project whose only claim
    /// has no pane still draws, and this is the line that must not pay for it.
    /// `live_agents` is the number the header speaks for, so it is counted
    /// here rather than written down: a fix that widened it to save the
    /// project would put "1 agent" over a project nobody is in.
    #[test]
    fn a_project_whose_only_claim_has_no_pane_wears_the_warning_and_claims_no_agent() {
        let claimed = Node {
            status: Status::InProgress,
            anomalies: vec![Anomaly::OrphanClaim { refused: None }],
            ..node("orb-4.1", "seat the guy wires", Status::InProgress)
        };
        let counts = Counts::over(&[claimed]);

        let drawn = drawn(line(&project("orbital", counts), OPEN), 40, 1);

        says(&drawn[0], &format!("{WARNING} 1"));
        does_not_say(&drawn[0], "agent");
    }

    /// Narrower than the identity itself there is nothing left to protect, and
    /// the line is cut like any other.
    #[test]
    fn a_width_too_narrow_for_anything_else_keeps_as_much_of_the_project_as_it_can() {
        let counts = counts(8, 21, 3, 3);

        assert_eq!(
            drawn(line(&project("summit-works", counts), OPEN), 10, 1),
            vec!["▾ nixos-c…"]
        );
    }

    /// Width is columns on a screen, not bytes in a string. Every glyph in
    /// this vocabulary is several bytes long, and a cut counted in bytes would
    /// land inside one and put a broken character on the terminal.
    #[test]
    fn a_cut_is_counted_in_columns_and_never_lands_inside_a_glyph() {
        let name = "→→→→→→→→→→→→→→→→→→→→→→→→→→→→→→";
        let drawn = drawn(line(&project(name, counts(0, 1, 0, 0)), OPEN), 20, 1);

        assert_eq!(drawn[0].chars().count(), 20);
        assert!(!drawn[0].contains('\u{fffd}'), "{drawn:?}");
    }

    // ---- how fresh a project is ------------------------------------------

    /// Graeme, on where the indicator goes: *"positioned on the project line
    /// next to the project name, not the footer"*. It is a claim about this
    /// project's rows, so it stands beside the name those rows hang under.
    #[test]
    fn a_project_says_how_long_ago_it_was_read_beside_its_own_name() {
        let line = line_that_is(
            &project("summit-works", counts(8, 21, 3, 3)),
            Freshness::Collected(read_at()),
        );

        assert_eq!(
            drawn(line, 60, 1),
            vec!["▾ summit-works  30s ago                  8/21  3 agents  ⚠ 3"]
        );
    }

    /// A collection reading this project replaces its age with the mark that
    /// says so: the rows are seconds from being superseded, and how stale the
    /// ones about to go are is not what a reader watching them needs.
    #[test]
    fn a_project_being_read_wears_the_turning_mark_in_place_of_its_age() {
        let drawn = drawn(
            line_that_is(
                &project("summit-works", counts(8, 21, 0, 0)),
                Freshness::Collecting,
            ),
            60,
            1,
        );

        does_not_say(&drawn[0], "ago");
        says(
            &drawn[0],
            phrase::freshness(Freshness::Collecting, drawn_at()).as_str(),
        );
    }

    /// The bead: a refresh naming one project redrew every project's
    /// indicator, because there was one indicator and it spoke for the whole
    /// screen. Each line now answers for its own rows, so the project nothing
    /// is reading keeps the age it has.
    #[test]
    fn a_project_no_collection_names_keeps_its_age_while_another_is_read() {
        let forest = opened(&two_projects());

        let frame = frame_collecting(
            &forest,
            Some(&Wanted::Project("summit-works".to_string())),
            74,
            12,
        );

        let marked = phrase::freshness(Freshness::Collecting, drawn_at());
        says(&frame[0], &marked);
        says(project_row(&frame, "harbour"), "30s ago");
        does_not_say(project_row(&frame, "harbour"), &marked);
    }

    /// A collection over everything is reading every project, so every
    /// project's line says so.
    #[test]
    fn a_collection_over_everything_marks_every_project() {
        let forest = opened(&two_projects());

        let frame = frame_collecting(&forest, Some(&Wanted::Everything), 74, 12);

        let marked = phrase::freshness(Freshness::Collecting, drawn_at());
        says(&frame[0], &marked);
        says(project_row(&frame, "harbour"), &marked);
    }

    /// The counts are what a reader came to the line for and the age is what
    /// they check them against, so the age is the first thing a narrowing
    /// line gives up.
    ///
    /// `bdi-2bb.21` rejected the project line for this indicator on a
    /// measurement — at the narrowest supported width the design's worked
    /// example is full to the column — and put it in the foot instead. Forty
    /// columns is that width, and it is here because the position changed and
    /// the measurement did not: the line at forty is what it always was.
    #[test]
    fn a_narrow_project_line_gives_up_its_age_before_its_counts() {
        let with_age = |width| {
            drawn(
                line_that_is(
                    &project("summit-works", counts(8, 21, 3, 3)),
                    Freshness::Collected(read_at()),
                ),
                width,
                1,
            )
        };

        let roomy = with_age(60);
        says(&roomy[0], "30s ago");

        for narrow in [43, 40] {
            assert_eq!(
                with_age(narrow),
                drawn(
                    line(&project("summit-works", counts(8, 21, 3, 3)), OPEN),
                    narrow,
                    1
                ),
                "at {narrow} columns the age costs the line nothing"
            );
        }
    }

    /// An age is given up whole rather than cut. One column short of the
    /// seven it needs, a cut line would read `30s a…` — a duration that names
    /// no duration, spending the columns it kept to say the project has been
    /// read, which its own rows already said.
    #[test]
    fn an_age_with_no_room_for_it_is_dropped_rather_than_cut() {
        let drawn = drawn(
            line_that_is(
                &project("summit-works", counts(8, 21, 3, 3)),
                Freshness::Collected(read_at()),
            ),
            43,
            1,
        );

        does_not_say(&drawn[0], "…");
        says(&drawn[0], "8/21  3 agents  ⚠ 3");
    }

    /// Dim and plain: it is what a reader glances at to place the counts, not
    /// one of the things the line is asking them to look at.
    #[test]
    fn how_fresh_a_project_is_is_drawn_dim_so_the_counts_keep_the_eye() {
        let painted = painted(
            line_that_is(
                &project("summit-works", counts(8, 21, 0, 0)),
                Freshness::Collected(read_at()),
            ),
            60,
        );

        assert!(
            painted
                .iter()
                .any(|(said, colour)| said.contains("30s ago") && *colour == Color::DarkGray),
            "{painted:?}"
        );
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

        let painted = painted(fitted(root, 12, &at_rest()), 60);
        let drawn = drawn(fitted(root, 12, &at_rest()), 60, 1);

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
                line(&recovering("summit-works", &panes, true), NO_FOLD),
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
        let drawn = drawn(line(&recovering("summit-works", &[], true), OPEN), 120, 1);

        says(&drawn[0], "no live pane names this project");
    }

    /// A pane list that cannot be known to be whole says so. A silently short
    /// list is the one way this line can be read wrongly, because it looks
    /// exactly like a complete one.
    #[test]
    fn a_pane_list_that_may_be_short_says_so_rather_than_reading_as_complete() {
        let panes = [pane("wCM:p9", PaneStatus::Working)];

        let whole = drawn(
            line(&recovering("summit-works", &panes, true), OPEN),
            200,
            1,
        );
        let partial = drawn(
            line(&recovering("summit-works", &panes, false), OPEN),
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
