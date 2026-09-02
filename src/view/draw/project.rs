//! A project's own line — what it is, how fresh it is and how much work it
//! holds — and the roots beneath it that would not read.

use chrono::{DateTime, Utc};
use ratatui::style::{Color, Style};
use ratatui::text::Span;

use crate::model::snapshot::{Counts, TrackerState};
use crate::view::fitted::Fitted;
use crate::view::lines::{ProjectLine, Recovery, Unread};
use crate::view::phrase;
use crate::view::row::WARNING;
use crate::view::{Freshness, Mark};

use super::tone::{LIVE, LOOK_AT_THIS};
use super::{beside, done, pane_marker, structure};

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

    let mut state = summary(&project.counts);
    if let Some(found) = &project.recovery {
        beside(&mut state, recovered(found));
    }

    Fitted::new(identity, freshness(how_fresh, now), state).title_or_nothing()
}

/// The mark and the age beside a project's name, in that order and both of
/// them in every state the collection can be in.
///
/// The age is drawn plain and dim throughout: it is what a reader glances at
/// to place the rest, not one of the things the rest is asking them to look
/// at. The mark is dim for the same reason wherever the collection is going
/// well, and wears the warning's colour where it is not — a project folded
/// shut draws none of the `unread_line`s naming the root that refused, so
/// this is then the only thing on the screen saying the rows are short of
/// one.
///
/// A collection that has stopped answering wears it for the same reason: no
/// row anywhere else on the screen says the tracker has gone quiet, because
/// the rows are the last collection's and look exactly as they did.
fn freshness(how_fresh: Option<Freshness>, now: DateTime<Utc>) -> Vec<Span<'static>> {
    let Some(how_fresh) = how_fresh else {
        return Vec::new();
    };
    let mark = match how_fresh.mark {
        Mark::Refused | Mark::Unanswered => LOOK_AT_THIS,
        Mark::Collecting | Mark::Read => Color::DarkGray,
    };

    let mut said = vec![Span::styled(
        phrase::mark(how_fresh, now),
        Style::new().fg(mark),
    )];
    said.extend(
        phrase::last_read(how_fresh, now)
            .map(|age| {
                // One space rather than a `GAP`: the mark and the age are two
                // halves of one claim about this project's rows, and a gap
                // between them would read as two cells.
                [" ".to_string(), age]
                    .map(|said| Span::styled(said, Style::new().fg(Color::DarkGray)))
            })
            .into_iter()
            .flatten(),
    );
    said
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
        TrackerState::RootNotFound => phrase::root_not_found(),
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
    let mut said = Vec::new();
    // Nothing counted means no root here read at all, and `0/0` would say the
    // opposite of what is true — that they were read and hold nothing.
    if counts.total > 0 {
        beside(&mut said, Span::raw(done(counts.closed, counts.total)));
    }
    if counts.live_agents > 0 {
        let agent = if counts.live_agents == 1 {
            "agent"
        } else {
            "agents"
        };
        beside(
            &mut said,
            Span::styled(
                format!("{} {agent}", counts.live_agents),
                Style::new().fg(LIVE),
            ),
        );
    }
    if counts.anomalies > 0 {
        beside(
            &mut said,
            Span::styled(
                format!("{WARNING} {}", counts.anomalies),
                Style::new().fg(LOOK_AT_THIS),
            ),
        );
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

    use crate::app::{Awaited, Wanted};
    use crate::model::anomaly::Anomaly;
    use std::sync::Arc;

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

    /// A project read half a minute ago, in whichever state its collection is
    /// in now.
    fn half_a_minute_old(mark: Mark) -> Freshness {
        Freshness {
            mark,
            read_at: Some(read_at()),
        }
    }

    /// Two projects read at the same instant, so a mark on one of them is a
    /// mark this collection put there rather than a difference in the
    /// fixture.
    fn two_projects() -> Snapshot {
        let harbour = Tree {
            counts: counts(0, 1, 0, 0),
            beads: vec![node("qua-1", "moor the barge", Status::InProgress)],
            children: vec![Vec::new()],
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
            Painted::of(line(&project("summit-works", counts), OPEN), 40, 1).rows(),
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
            Painted::of(line(&project("homelab", counts), SHUT), 30, 1).rows(),
            vec!["▸ homelab                  2/7"]
        );
    }

    #[test]
    fn one_agent_is_not_described_in_the_plural() {
        let counts = counts(2, 7, 1, 0);
        let drawn = Painted::of(line(&project("homelab", counts), SHUT), 40, 1).rows();

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

        let drawn = Painted::of(line(&project("orbital", counts), OPEN), 40, 1).rows();

        says(&drawn[0], &format!("{WARNING} 1"));
        does_not_say(&drawn[0], "agent");
    }

    /// The gap goes *between* the counts and the panes recovered after them.
    /// A project can have one root that read and another that refused, so
    /// both cells are on the row at once — and run together they read as one
    /// cell naming neither, `2/7◍ wCM:p9`. In front of the counts the same
    /// two columns say nothing, because the block is set against the row's
    /// right edge and the padding swallows them.
    ///
    /// The block is written out here rather than asked of the code that drew
    /// it, and read off the row's end, so nothing satisfies it but those
    /// words in that order with those two columns between them.
    #[test]
    fn a_projects_recovered_panes_are_held_apart_from_its_counts() {
        let mut recovering = project("harbour", counts(2, 7, 0, 0));
        recovering.recovery = Some(Recovery {
            panes: vec![pane("wCM:p9", PaneStatus::Working)],
            complete: true,
        });

        let drawn = Painted::of(line(&recovering, OPEN), 60, 1).rows();

        assert!(drawn[0].ends_with("2/7  ◍ wCM:p9 working"), "{drawn:?}");
    }

    /// Narrower than the identity itself there is nothing left to protect, and
    /// the line is cut like any other.
    #[test]
    fn a_width_too_narrow_for_anything_else_keeps_as_much_of_the_project_as_it_can() {
        let counts = counts(8, 21, 3, 3);

        assert_eq!(
            Painted::of(line(&project("summit-works", counts), OPEN), 10, 1).rows(),
            vec!["▾ nixos-c…"]
        );
    }

    /// Width is columns on a screen, not bytes in a string. Every glyph in
    /// this vocabulary is several bytes long, and a cut counted in bytes would
    /// land inside one and put a broken character on the terminal.
    #[test]
    fn a_cut_is_counted_in_columns_and_never_lands_inside_a_glyph() {
        let name = "→→→→→→→→→→→→→→→→→→→→→→→→→→→→→→";
        let drawn = Painted::of(line(&project(name, counts(0, 1, 0, 0)), OPEN), 20, 1).rows();

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
            half_a_minute_old(Mark::Read),
        );

        assert_eq!(
            Painted::of(line, 60, 1).rows(),
            vec!["▾ summit-works  ✓ 30s ago                8/21  3 agents  ⚠ 3"]
        );
    }

    /// The bead, in one assertion. Graeme: *"while refreshing, the displayed
    /// data continues to have an age. it should continue to be shown while
    /// the spinner is going"*, and *"when not collecting, the spinner can be
    /// replaced with something to indicate success/failure so that it doesn't
    /// jump around"*.
    ///
    /// Three states, one shape: a mark, a space, the same age. Every column
    /// after the mark holds the same thing in all three, so a collection
    /// starting or ending moves nothing on the line. The old cell drew a mark
    /// and no age while collecting and an age and no mark at rest, and every
    /// column after the name shifted each way.
    #[test]
    fn the_cell_says_a_mark_and_an_age_in_every_state_and_nothing_after_it_moves() {
        let said = |mark| {
            Painted::of(
                line_that_is(
                    &project("summit-works", counts(8, 21, 0, 0)),
                    half_a_minute_old(mark),
                ),
                60,
                1,
            )
            .rows()
            .remove(0)
        };

        assert_eq!(
            [
                said(Mark::Collecting),
                said(Mark::Read),
                said(Mark::Refused)
            ],
            [
                "▾ summit-works  ⠴ 30s ago                               8/21",
                "▾ summit-works  ✓ 30s ago                               8/21",
                "▾ summit-works  ⚠ 30s ago                               8/21",
            ]
        );
    }

    /// The startup frame, and the one state with no age in it: nothing has
    /// come back, so there is no read to date the rows to and no rows either.
    #[test]
    fn a_project_nothing_has_read_yet_shows_the_mark_over_no_age() {
        let starting = Freshness {
            mark: Mark::Collecting,
            read_at: None,
        };

        assert_eq!(
            Painted::of(
                line_that_is(&project("summit-works", counts(0, 0, 0, 0)), starting),
                60,
                1
            )
            .rows(),
            vec!["▾ summit-works  ⠴                                           "]
        );
    }

    /// The bead: a refresh naming one project redrew every project's
    /// indicator, because there was one indicator and it spoke for the whole
    /// screen. Each line now answers for its own rows, so the project nothing
    /// is reading keeps the age it has — and says, with its own mark, that
    /// nothing is reading it.
    #[test]
    fn a_project_no_collection_names_keeps_its_age_while_another_is_read() {
        let forest = opened(&two_projects());

        let frame = frame_collecting(
            &forest,
            &[reading(Wanted::Project("summit-works".to_string()))],
            74,
            12,
        )
        .rows();

        says(&frame[0], "⠴ 30s ago");
        says(project_row(&frame, "harbour"), "✓ 30s ago");
    }

    /// A collection over everything is reading every project, so every
    /// project's line says so.
    #[test]
    fn a_collection_over_everything_marks_every_project() {
        let forest = opened(&two_projects());

        let frame = frame_collecting(&forest, &[reading(Wanted::Everything)], 74, 12).rows();

        says(&frame[0], "⠴ 30s ago");
        says(project_row(&frame, "harbour"), "⠴ 30s ago");
    }

    /// The whole way through from the model: a project one of whose roots
    /// would not read wears the refused mark, resolved over the roots by
    /// `Snapshot::every_root_read`. Nothing between the tracker and the cell
    /// is stubbed, which is what makes this different from the tests above.
    #[test]
    fn a_project_with_a_root_that_would_not_read_wears_the_refused_mark() {
        let mut snapshot = two_projects();
        snapshot.trees.push(Arc::new(Tree::tracker_unreachable(
            "harbour",
            "qua-9",
            TrackerFailure::Auth,
        )));
        snapshot.collected.clone_from(&snapshot.trees);

        let frame = frame_of(&opened(&snapshot), 74, 12).rows();

        says(project_row(&frame, "harbour"), "⚠ 30s ago");
        says(&frame[0], "✓ 30s ago");
    }

    /// The counts are what a reader came to the line for and the cell is what
    /// they check them against, so the cell is the first thing a narrowing
    /// line gives up.
    ///
    /// `bdi-2bb.21` rejected the project line for this indicator on a
    /// measurement — at the narrowest supported width the design's worked
    /// example is full to the column — and put it in the foot instead. Forty
    /// columns is that width, and it is here because the position changed and
    /// the measurement did not: the line at forty is what it always was.
    #[test]
    fn a_narrow_project_line_gives_up_the_whole_cell_before_its_counts() {
        let with_a_cell = |width| {
            Painted::of(
                line_that_is(
                    &project("summit-works", counts(8, 21, 3, 3)),
                    half_a_minute_old(Mark::Read),
                ),
                width,
                1,
            )
            .rows()
        };

        assert_eq!(
            with_a_cell(46),
            vec!["▾ summit-works  ✓ 30s ago  8/21  3 agents  ⚠ 3"]
        );

        for narrow in [45, 40] {
            assert_eq!(
                with_a_cell(narrow),
                Painted::of(
                    line(&project("summit-works", counts(8, 21, 3, 3)), OPEN),
                    narrow,
                    1
                )
                .rows(),
                "at {narrow} columns the cell costs the line nothing"
            );
        }
    }

    /// The cell is given up whole rather than cut, and the mark goes with the
    /// age rather than standing on alone.
    ///
    /// One column short of the nine it needs, a cut line would read `✓ 30s
    /// a…` — a duration that names no duration. The mark alone would fit, but
    /// a mark with no age beside it is the cell the bead exists to remove:
    /// the reader would be back to a glyph over rows of unknown age, at the
    /// one width where they can least afford to guess.
    #[test]
    fn a_cell_with_no_room_for_it_is_dropped_whole_rather_than_cut_or_halved() {
        assert_eq!(
            Painted::of(
                line_that_is(
                    &project("summit-works", counts(8, 21, 3, 3)),
                    half_a_minute_old(Mark::Read),
                ),
                45,
                1
            )
            .rows(),
            vec!["▾ summit-works            8/21  3 agents  ⚠ 3"]
        );
    }

    /// Dim and plain: it is what a reader glances at to place the counts, not
    /// one of the things the line is asking them to look at. That holds while
    /// a collection runs as much as at rest — a mark turning in colour would
    /// pull the eye off the counts every eighty milliseconds.
    #[test]
    fn how_fresh_a_project_is_is_drawn_dim_so_the_counts_keep_the_eye() {
        let dim = |mark, said: &str| {
            let painted = Painted::of(
                line_that_is(
                    &project("summit-works", counts(8, 21, 0, 0)),
                    half_a_minute_old(mark),
                ),
                60,
                1,
            )
            .row(0);
            assert!(
                painted
                    .iter()
                    .any(|run| run.said.contains(said) && run.style.fg == Some(Color::DarkGray)),
                "{said:?} is not dim: {painted:?}"
            );
        };

        dim(Mark::Read, "✓ 30s ago");
        dim(Mark::Collecting, "⠴ 30s ago");
    }

    /// The one part of the cell that is not dim. A project folded shut draws
    /// none of the `unread_line`s naming the root that refused, so this mark
    /// is then the only thing on the screen saying the rows are short of one
    /// — and a dim glyph beside a dim age is not something a reader scanning
    /// a screen of projects will stop at.
    ///
    /// The age beside it stays dim: how stale the rows are is the same kind
    /// of fact whether the collection came back whole or not.
    #[test]
    fn a_mark_saying_a_root_refused_wears_the_colour_that_asks_to_be_looked_at() {
        let painted = Painted::of(
            line_that_is(
                &project("summit-works", counts(8, 21, 0, 0)),
                half_a_minute_old(Mark::Refused),
            ),
            60,
            1,
        )
        .row(0);

        assert!(
            painted
                .iter()
                .any(|run| run.said.contains(WARNING) && run.style.fg == Some(LOOK_AT_THIS)),
            "{painted:?}"
        );
        assert!(
            painted
                .iter()
                .any(|run| run.said.contains("30s ago") && run.style.fg == Some(Color::DarkGray)),
            "{painted:?}"
        );
    }

    /// `bdi-7ao.51`, the whole way through from the collection: a tracker
    /// that has stopped answering is drawn as having stopped, rather than as
    /// a collection that has just started. Nothing between the loop's stamp
    /// and the cell is stubbed.
    ///
    /// The age is what makes this readable and it is why the mark alone is
    /// not enough: `⠿ 30s ago` says the rows are half a minute old *and* that
    /// nothing is going to replace them, which are the two facts a reader
    /// needs and neither of which implies the other.
    #[test]
    fn a_tracker_that_has_stopped_answering_is_drawn_as_stopped_rather_than_as_starting() {
        let forest = opened(&two_projects());
        let asked_at = drawn_at() - PATIENCE;

        let frame = frame_collecting(
            &forest,
            &[Awaited {
                wanted: Wanted::Everything,
                asked_at,
                patience: PATIENCE,
            }],
            74,
            12,
        )
        .rows();

        says(&frame[0], "⠿ 30s ago");
        says(project_row(&frame, "harbour"), "⠿ 30s ago");
    }

    /// `bdi-7ao.81`, which is `.51`'s defect one level up: a project queued
    /// behind a tracker that has stopped answering has to be told apart from
    /// one nothing has needed to read.
    ///
    /// The two are asserted from one frame, because the defect was never that
    /// either line drew wrongly — it was that they drew the same. Both wear
    /// `✓ 30s ago` under the old rule, and everything on both is true: their
    /// rows really were read half a minute ago. What neither says is the
    /// thing that matters, that one of them will go on ageing however long
    /// the reader waits.
    #[test]
    fn a_project_queued_behind_a_stopped_tracker_is_drawn_apart_from_a_quiet_one() {
        let forest = opened(&two_projects());
        let stopped = drawn_at() - PATIENCE;

        let frame = frame_collecting(
            &forest,
            &[
                reading_since(Wanted::Project("summit-works".to_string()), stopped),
                reading_since(Wanted::Project("harbour".to_string()), stopped),
            ],
            74,
            12,
        )
        .rows();

        says(project_row(&frame, "harbour"), "⠿ 30s ago");
        says(&frame[0], "⠿ 30s ago");
    }

    /// And a project no read is outstanding for keeps its resting mark while
    /// the one beside it says its reads have stopped — the other half of the
    /// same claim, and the half that makes the mark worth reading.
    #[test]
    fn a_project_nothing_has_needed_to_read_rests_beside_one_that_has_stopped() {
        let forest = opened(&two_projects());

        let frame = frame_collecting(
            &forest,
            &[reading_since(
                Wanted::Project("summit-works".to_string()),
                drawn_at() - PATIENCE,
            )],
            74,
            12,
        )
        .rows();

        says(&frame[0], "⠿ 30s ago");
        says(project_row(&frame, "harbour"), "✓ 30s ago");
    }

    /// Two outstanding reads naming one project is the ordinary case, not an
    /// edge: a whole collection names every project, so one queued behind a
    /// single project names that project a second time. What the line has to
    /// answer is how long *its* rows have been on their way, which the older
    /// of the two says — the newer one would have the project that has waited
    /// longest report the shortest wait on the screen.
    #[test]
    fn a_project_two_outstanding_reads_name_is_drawn_against_the_older_of_them() {
        let forest = opened(&two_projects());

        let frame = frame_collecting(
            &forest,
            &[
                reading_since(
                    Wanted::Project("harbour".to_string()),
                    drawn_at() - PATIENCE,
                ),
                reading_since(Wanted::Everything, drawn_at()),
            ],
            74,
            12,
        )
        .rows();

        says(project_row(&frame, "harbour"), "⠿ 30s ago");
        says(&frame[0], "⠴ 30s ago");
    }

    /// A read of `wanted` outstanding since `asked_at`, whether it is the one
    /// the collector has or one waiting behind it: the screen is told the
    /// same thing about both, because the wait is the same wait.
    fn reading_since(wanted: Wanted, asked_at: chrono::DateTime<Utc>) -> Awaited {
        Awaited {
            wanted,
            asked_at,
            patience: PATIENCE,
        }
    }

    /// The colour is the other half of that claim, and the half `drawn()`
    /// cannot see. No row anywhere else on the screen says the tracker has
    /// gone quiet — the rows are the last collection's and look exactly as
    /// they did — so a dim glyph would leave the one thing saying so as the
    /// one thing nobody stops at.
    #[test]
    fn a_mark_saying_a_tracker_stopped_answering_wears_the_colour_that_asks_to_be_looked_at() {
        let painted = Painted::of(
            line_that_is(
                &project("summit-works", counts(8, 21, 0, 0)),
                half_a_minute_old(Mark::Unanswered),
            ),
            60,
            1,
        )
        .row(0);

        assert!(
            painted
                .iter()
                .any(|run| run.said.contains('⠿') && run.style.fg == Some(LOOK_AT_THIS)),
            "{painted:?}"
        );
        assert!(
            painted
                .iter()
                .any(|run| run.said.contains("30s ago") && run.style.fg == Some(Color::DarkGray)),
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
        let forest = flatten(snapshot(vec![grove(2)], Vec::new(), HerdrState::Ok));
        let root = &forest.lines()[1];

        let painted = Painted::of(fitted(root, 12, &at_rest()), 60, 1).row(0);
        let drawn = Painted::of(fitted(root, 12, &at_rest()), 60, 1).rows();

        assert!(
            drawn[0].contains(&format!(
                "{} nix-9670s",
                row::status_glyph(&Status::InProgress)
            )),
            "{drawn:?}"
        );
        assert!(
            painted.iter().any(
                |run| run.said.contains(row::status_glyph(&Status::InProgress))
                    && run.style.fg == status_colour(&Status::InProgress)
            ),
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
        let drawn = Painted::of(unread_line(&unread, LAST, 9), 60, 1).rows();

        says(&drawn[0], "nix-9670s");
        says(&drawn[0], "the tracker did not answer");
    }

    /// A root the tracker holds no bead for is named the same way, and the
    /// reason beside it is about what named the root, not about the tracker.
    #[test]
    fn a_root_the_tracker_does_not_hold_says_so_beside_its_id() {
        let unread = unread("nix-9670s", TrackerState::RootNotFound);
        let drawn = Painted::of(unread_line(&unread, LAST, 9), 90, 1).rows();

        says(&drawn[0], "nix-9670s");
        says(&drawn[0], "no such bead in this tracker");
        does_not_say(&drawn[0], "bdi cannot read");
    }

    /// A tracker that could not be read has no counts, and `0/0` would say the
    /// opposite of what is true — that it was read and holds nothing.
    #[test]
    fn an_unread_root_never_shows_a_count_it_could_not_read() {
        let unread = unread("nix-9670s", TrackerState::Unreachable(TrackerFailure::Auth));
        let drawn = Painted::of(unread_line(&unread, LAST, 9), 120, 1).rows();

        does_not_say(&drawn[0], "0/0");
    }

    /// Nothing should reach this: a root that read is a bead row, and one that
    /// did not carries the failure that stopped it. A root that got here
    /// anyway is still a root on the screen, which is the whole point.
    #[test]
    fn a_root_with_no_row_and_no_reason_still_says_it_is_there() {
        let drawn = Painted::of(
            unread_line(&unread("nix-9670s", TrackerState::Ok), LAST, 9),
            90,
            1,
        )
        .rows();

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
            Painted::of(
                line(&recovering("summit-works", &panes, true), NO_FOLD),
                80,
                1
            )
            .rows(),
            vec![
                "  summit-works                                  ◍ wCM:p9 working · ◍ wCM:p6 idle"
                    .to_string()
            ]
        );
    }

    #[test]
    fn a_project_with_no_pane_to_show_says_that_rather_than_nothing() {
        let drawn = Painted::of(line(&recovering("summit-works", &[], true), OPEN), 120, 1).rows();

        says(&drawn[0], "no live pane names this project");
    }

    /// A pane list that cannot be known to be whole says so. A silently short
    /// list is the one way this line can be read wrongly, because it looks
    /// exactly like a complete one.
    #[test]
    fn a_pane_list_that_may_be_short_says_so_rather_than_reading_as_complete() {
        let panes = [pane("wCM:p9", PaneStatus::Working)];

        let whole = Painted::of(
            line(&recovering("summit-works", &panes, true), OPEN),
            200,
            1,
        )
        .rows();
        let partial = Painted::of(
            line(&recovering("summit-works", &panes, false), OPEN),
            200,
            1,
        )
        .rows();

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
        let drawn = Painted::of(unread_line(&unread, LAST, 9), 24, 1).rows();

        says(&drawn[0], "nix-9670s");
        assert_eq!(drawn[0].chars().count(), 24);
    }
}
