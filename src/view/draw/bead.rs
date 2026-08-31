//! A bead's own line, and the run of closed siblings drawn in place of the
//! several beads it stands for.

use ratatui::style::{Color, Style};
use ratatui::text::Span;

use crate::model::types::Status;
use crate::view::fitted::{Fitted, GAP};
use crate::view::phrase;
use crate::view::row::{self, Row};

use super::tone::{fg, status_style, tone, DIM, LIVE, LOOK_AT_THIS};
use super::{done, structure};

/// A run of closed siblings said as a count, carrying the glyph each of them
/// would carry on a line of its own.
///
/// `lines::split` builds a run out of closed beads and nothing else, so this
/// is not a summary over mixed states — it is the one state every member
/// holds. It goes through `status_glyph` and `status_style` exactly as a
/// bead's does, so a run cannot drift away from the beads it stands for.
pub(super) fn elided_run(prefix: &str, count: usize) -> Fitted {
    let status = Status::Closed;
    let glyph = row::status_glyph(&status);
    Fitted::new(
        vec![
            structure(prefix),
            Span::styled(glyph.to_string(), status_style(&status)),
            Span::raw(format!(" {}", phrase::elided(count))),
        ],
        Vec::new(),
        Vec::new(),
    )
    .toned(Style::new().fg(DIM))
}

/// One bead's line, under the box-drawing run its ancestors leave.
///
/// `id_width` is the widest abbreviated id in the tree, so a column of ids
/// lines up under one another and the titles start together.
pub(super) fn bead_line(row: &Row, prefix: &str, id_width: usize) -> Fitted {
    let identity = vec![
        structure(prefix),
        Span::styled(row.glyph.to_string(), status_style(&row.status)),
        Span::raw(format!(" {:id_width$}", row.id)),
    ];

    let mut title = vec![Span::raw(row.title.clone())];
    for badge in &row.badges {
        title.push(Span::raw(" ".repeat(GAP)));
        title.push(Span::raw(badge.clone()));
    }

    let mut state: Vec<Span<'static>> = Vec::new();
    let mut say = |text: &str, colour: Option<Color>| {
        if !state.is_empty() {
            state.push(Span::raw(" ".repeat(GAP)));
        }
        state.push(Span::styled(text.to_string(), fg(colour)));
    };
    if let Some(progress) = row.progress {
        say(&done(progress.closed, progress.total), None);
    }
    if let Some(agent) = &row.agent {
        say(agent, Some(LIVE));
    }
    if let Some(anomalies) = &row.anomalies {
        say(anomalies, Some(LOOK_AT_THIS));
    }
    for note in &row.notes {
        say(note, Some(LOOK_AT_THIS));
    }

    Fitted::new(identity, title, state).toned(tone(row))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    use crate::model::anomaly::Anomaly;
    use crate::model::badges::Badged;
    use crate::model::join::AgentRef;
    use crate::model::snapshot::Node;
    use crate::view::draw::tone::status_colour;
    use crate::view::draw::{fitted, tests::*};

    #[test]
    fn a_bead_line_says_its_glyph_its_id_and_its_title_in_that_order() {
        let node = node("nix-9670s.20", "wallpaper timer calls dms", Status::Blocked);

        assert_eq!(
            drawn(bead_line(&row(&node), BRANCH, 4), 46, 1),
            vec!["  ├── ● .20   wallpaper timer calls dms       "]
        );
    }

    /// An epic reads like the root above it: how far along, then who is on
    /// it, then what wants looking at. The count leads the state column
    /// because that is the order a header already puts them in.
    #[test]
    fn a_bead_standing_for_a_subtree_says_how_much_of_it_is_done_before_who_is_on_it() {
        let mut epic = row(&node(
            "nix-9670s.2",
            "the noctalia widget",
            Status::InProgress,
        ));
        epic.progress = Some(row::Progress {
            closed: 3,
            total: 8,
        });
        epic.agent = Some(row::agent_marker(&a_pane()));

        let drawn = drawn(bead_line(&epic, BRANCH, 3), 60, 1);

        let count = drawn[0].find("3/8").expect("the count is drawn");
        let agent = drawn[0].find("wCM:p9").expect("the agent is drawn");
        assert!(count < agent, "{drawn:?}");
    }

    /// A leaf stands for itself alone. A fraction over one bead would say
    /// nothing its glyph has not already said, and would spend width a title
    /// needs.
    #[test]
    fn a_bead_standing_only_for_itself_draws_no_count() {
        let leaf = row(&node(
            "nix-9670s.20",
            "wallpaper timer calls dms",
            Status::Open,
        ));

        let drawn = drawn(bead_line(&leaf, BRANCH, 4), 60, 1);

        assert!(!drawn[0].contains('/'), "{drawn:?}");
    }

    /// Ids are padded to the widest in the tree so the titles start together;
    /// a column that did not line up would be read as a tree shape it is not.
    #[test]
    fn ids_are_padded_so_the_titles_below_one_another_start_together() {
        let short = node("nix-9670s.1", "wire the niri theme include", Status::Open);
        let long = node("nix-9670s.20", "wallpaper timer calls dms", Status::Open);

        let short = drawn(bead_line(&row(&short), BRANCH, 4), 60, 1);
        let long = drawn(bead_line(&row(&long), BRANCH, 4), 60, 1);

        assert_eq!(
            short[0].find("wire the"),
            long[0].find("wallpaper timer"),
            "{short:?} {long:?}"
        );
    }

    #[test]
    fn a_bead_line_carries_its_agent_and_its_anomalies() {
        let mut staffed = node(
            "nix-9670s.20",
            "wallpaper timer calls dms",
            Status::InProgress,
        );
        staffed.agent = Some(a_pane());
        staffed.anomalies = vec![Anomaly::StaleClaim { days: 58 }];
        let drawn = drawn(bead_line(&row(&staffed), LAST, 4), 100, 1);

        assert!(drawn[0].contains("◍ wCM:p9 · working"), "{drawn:?}");
        assert!(drawn[0].contains("58"), "{drawn:?}");
    }

    fn captioned(caption: &str) -> Node {
        let mut staffed = node(
            "nix-9670s.20",
            "wallpaper timer calls dms",
            Status::InProgress,
        );
        staffed.agent = Some(AgentRef {
            title: Some(caption.into()),
            ..a_pane()
        });
        staffed
    }

    /// A caption is the first unbounded string to reach this cell — a pane id
    /// was short and fixed — so the cut it takes is the one every other cell
    /// takes, and the row is still exactly as wide as it was given.
    #[test]
    fn a_caption_too_long_for_the_row_is_cut_like_every_other_cell() {
        let staffed = captioned("teach the elided run to fold back open on a keypress");

        let drawn = drawn(bead_line(&row(&staffed), LAST, 4), 50, 1);

        assert_eq!(drawn[0].chars().count(), 50, "{drawn:?}");
        assert!(drawn[0].ends_with('…'), "{drawn:?}");
        assert!(!drawn[0].contains("keypress"), "{drawn:?}");
    }

    /// The cell is fitted before the title is, so a caption long enough takes
    /// the room the title would have had. The bead is still named by its id,
    /// which is fitted before either of them and cannot be crowded out.
    #[test]
    fn a_caption_long_enough_takes_the_room_the_title_would_have_had() {
        let staffed = captioned("teach the elided run to fold back open on a keypress");

        let drawn = drawn(bead_line(&row(&staffed), LAST, 4), 80, 1);

        assert!(!drawn[0].contains("wallpaper"), "{drawn:?}");
        assert!(drawn[0].contains(".20"), "{drawn:?}");
    }

    /// Narrow enough and the cell has no room at all. It goes whole rather
    /// than leaving a marker standing for a caption that is not there.
    #[test]
    fn a_caption_with_no_room_left_takes_the_whole_agent_cell_with_it() {
        let staffed = captioned("teach the elided run to fold back open on a keypress");

        let drawn = drawn(bead_line(&row(&staffed), LAST, 4), 14, 1);

        assert!(!drawn[0].contains(row::AGENT), "{drawn:?}");
        assert!(drawn[0].contains(".20"), "{drawn:?}");
    }

    #[test]
    fn a_bead_lines_badges_are_drawn_in_the_order_they_were_configured() {
        let mut badged = node("nix-9670s.20", "a bead", Status::Blocked);
        badged.badges = vec![
            Badged {
                key: "delivery_pr".into(),
                text: "⇢ #12".into(),
            },
            Badged {
                key: "blocked_on".into(),
                text: "⏸ waiting".into(),
            },
        ];
        let drawn = drawn(bead_line(&row(&badged), BRANCH, 4), 100, 1);
        let first = drawn[0].find("⇢ #12").expect("the first badge");
        let second = drawn[0].find("⏸ waiting").expect("the second badge");

        assert!(first < second, "{drawn:?}");
    }

    /// A row bd stopped at means what hangs beneath it is not in the tree at
    /// all, which is the silent partial answer this tool exists to avoid — so
    /// it survives all the way to the screen.
    #[test]
    fn a_bead_the_tracker_stopped_at_says_so_on_screen() {
        let mut stopped = node("nix-9670s.20", "a bead", Status::Open);
        stopped.truncated = true;
        let drawn = drawn(bead_line(&row(&stopped), BRANCH, 4), 120, 1);

        says(
            &drawn[0],
            "more beneath this · the tracker stopped at its depth limit",
        );
    }

    #[test]
    fn a_bead_line_too_long_for_the_width_is_cut_rather_than_wrapped() {
        let long = node("nix-9670s.20", &"wallpaper ".repeat(20), Status::Open);
        let drawn = drawn(bead_line(&row(&long), BRANCH, 4), 40, 3);

        assert_eq!(drawn[0], "  ├── ○ .20   wallpaper wallpaper wallp…");
        assert_eq!(drawn[1].trim(), "");
        assert_eq!(drawn[2].trim(), "");
    }

    /// A run stands for closed beads and nothing else — `split` selects on
    /// exactly that — so its glyph is not a summary over mixed states but the
    /// one state every member holds. Resolved through `status_glyph` and
    /// `status_style`, the same two the beads themselves go through, so a run
    /// and the beads it stands for cannot drift apart.
    #[test]
    fn an_elided_run_carries_the_closed_glyph_each_bead_it_stands_for_would() {
        let painted = painted(fitted(&under(BRANCH, elided(15)), 0, &at_rest()), 72);

        assert_eq!(
            painted[1],
            (
                row::status_glyph(&Status::Closed).to_string(),
                status_colour(&Status::Closed).expect("closed is one bd colours")
            )
        );
    }

    /// A reader follows the vertical rules down a tree. A sentence that took
    /// its box-drawing into its own colour would break that run wherever it
    /// fell, so the drawing stays in the terminal's own foreground and only
    /// the words beside it are coloured.
    #[test]
    fn an_elided_run_leaves_its_box_drawing_in_the_terminals_own_colour() {
        let painted = painted(fitted(&under(BRANCH, elided(3)), 0, &at_rest()), 72);

        assert_eq!(painted[0], (BRANCH.to_string(), Color::Reset));
        assert_eq!(painted[2].1, DIM);
    }
}
