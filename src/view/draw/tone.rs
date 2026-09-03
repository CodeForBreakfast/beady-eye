//! What a row is drawn in: which of the palette's slots a bead's status and
//! the liveness of the row it sits on choose. The values are the palette's;
//! what is here is the rule that picks between them.

use ratatui::style::Style;

use crate::model::types::Status;
use crate::view::palette;
use crate::view::row::Row;

/// How live a row is, which is the one thing about a bead `bd list` has no
/// way to know — and so the one this scale is spent on.
///
/// | row | drawn |
/// |---|---|
/// | an agent is on it | the terminal's default |
/// | nobody on it, still going | the theme's colour 8 |
/// | finished, nobody on it | the grey `bd` dims a closed row to |
///
/// Finished means what it means to `lines::split`: closed, no agent, no
/// anomaly. A closed bead whose pane is still alive is exactly the row worth
/// looking at, and dimming it is how it would be missed.
pub(super) fn tone(row: &Row) -> Style {
    let finished = row.status.is_closed() && row.agent.is_none() && row.anomalies.is_none();
    if row.agent.is_some() {
        palette::TIER_STAFFED
    } else if finished {
        palette::TIER_FINISHED
    } else {
        palette::TIER_OPEN
    }
}

/// What a bead's status is drawn in: `bd`'s own colour for it, or nothing
/// where `bd` sends no escape and the glyph should take the brightness of the
/// row it sits on.
///
/// Colour is the second channel and never the only one: the glyph already says
/// the status, so a terminal with no colour loses nothing.
pub(crate) fn status_style(status: &Status) -> Style {
    match status {
        Status::InProgress => palette::STATUS_IN_PROGRESS,
        Status::Blocked => palette::STATUS_BLOCKED,
        Status::Closed => palette::STATUS_CLOSED,
        Status::Deferred => palette::STATUS_DEFERRED,
        Status::Open => palette::STATUS_OPEN,
        // The one status `bd` has no colour for, because it has no such
        // status. It takes the colour of the note already beside it.
        Status::Other(_) => palette::ATTENTION,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use ratatui::style::Color;

    use crate::model::anomaly::Anomaly;
    use crate::view::draw::bead::{bead_line, elided_run};
    use crate::view::draw::project::project_line;
    use crate::view::draw::tests::*;
    use crate::view::painted::Run;
    use crate::view::row::{self, Row, AGENT, WARNING};

    // ---- styling ---------------------------------------------------------

    /// One of each status, so a loop over them covers the set. The compiler
    /// holds `status_style` total; this list is only what a test walks.
    fn every_status() -> [Status; 6] {
        [
            Status::InProgress,
            Status::Blocked,
            Status::Open,
            Status::Deferred,
            Status::Closed,
            Status::Other(String::new()),
        ]
    }

    /// Colour is the second channel and never the only one: the glyph already
    /// says the status, so a terminal that drops colour must lose nothing.
    #[test]
    fn no_status_is_told_apart_by_colour_alone() {
        for status in every_status() {
            let node = node("nix-9670s.1", "a bead", status.clone());
            let drawn = Painted::of(bead_line(&row(&node), BRANCH, 3), 40, 1).rows();

            assert!(
                drawn[0].contains(row::status_glyph(&status)),
                "{status:?} lost its glyph: {drawn:?}"
            );
        }
    }

    /// Two statuses sharing a colour would tell one story between them. Only
    /// `open` may arrive without one at all: `bd` sends no escape for it, and
    /// the glyph already says which status it is.
    #[test]
    fn no_colour_is_given_to_two_statuses_and_only_open_goes_without_one() {
        let coloured: Vec<Color> = every_status()
            .iter()
            .filter_map(|status| status_style(status).fg)
            .collect();

        for (nth, colour) in coloured.iter().enumerate() {
            assert!(
                !coloured[nth + 1..].contains(colour),
                "{colour:?} is drawn for two statuses"
            );
        }
        assert_eq!(
            coloured.len(),
            every_status().len() - 1,
            "one status goes without a colour and it is open"
        );
        assert_eq!(status_style(&Status::Open).fg, None);
    }

    // ---- bd's palette, and the brightness only bdi can draw ---------------

    /// Read off `bd` 1.2.2's own output. A reader coming from `bd list` has
    /// already learned these, and a status drawn in a colour `bd` gives to a
    /// different one would be worse than no colour at all.
    #[test]
    fn a_status_glyph_is_painted_the_colour_bd_paints_it() {
        let bds = [
            (Status::InProgress, Color::Rgb(255, 180, 84)),
            (Status::Blocked, Color::Rgb(242, 109, 120)),
            (Status::Closed, Color::Rgb(128, 144, 160)),
            (Status::Deferred, Color::Rgb(108, 118, 128)),
        ];

        for (status, colour) in bds {
            let bead = node("nix-9670s.1", "a bead", status.clone());
            let painted = Painted::of(bead_line(&row(&bead), BRANCH, 3), 60, 1).row(0);

            assert_eq!(
                painted[1].said,
                row::status_glyph(&status).to_string(),
                "{status:?}: {painted:?}"
            );
            assert_eq!(painted[1].style.fg, Some(colour), "{status:?}: {painted:?}");
        }
    }

    /// `bd` sends no escape at all for an open bead's glyph, and inheriting is
    /// what lets the row's own brightness reach it. A glyph pinned to the
    /// terminal's default would leave an unworked row reading as two colours.
    #[test]
    fn an_open_glyph_takes_the_brightness_of_the_row_it_sits_on() {
        let unworked = node("nix-9670s.1", "a bead", Status::Open);

        let painted = Painted::of(bead_line(&row(&unworked), BRANCH, 3), 90, 1).row(0);

        assert!(painted[1].said.starts_with('○'), "{painted:?}");
        assert_eq!(painted[1].style.fg, Some(Color::DarkGray), "{painted:?}");
    }

    /// The tier that earns the screen. `bd list` has no notion of a live
    /// agent, so it has no way to say which row is the one you came for.
    ///
    /// The staffed row is the terminal's own foreground and the unworked one
    /// sits below it, rather than the other way up: a theme's default is
    /// already the brightest thing on its page, so there is nothing above it
    /// for a staffed row to be painted — on the theme this was measured on,
    /// `color15` and the default resolve to the same hex, and the two tiers
    /// were one. The scale is shifted down instead of extended up.
    #[test]
    fn a_row_with_an_agent_on_it_is_the_terminals_own_and_one_without_falls_below_it() {
        let mut staffed = node("nix-9670s.1", "a bead", Status::Open);
        staffed.agent = Some(a_pane());

        let bright = Painted::of(bead_line(&row(&staffed), BRANCH, 3), 90, 1).row(0);
        let plain = Painted::of(
            bead_line(
                &row(&node("nix-9670s.1", "a bead", Status::Open)),
                BRANCH,
                3,
            ),
            90,
            1,
        )
        .row(0);

        assert_eq!(
            the_words(&bright).style.fg,
            Some(Color::Reset),
            "an agent on it, so the row is the terminal's own: {bright:?}"
        );
        assert_eq!(
            the_words(&plain).style.fg,
            Some(Color::DarkGray),
            "nobody on it, so the row drops to the theme's colour 8: {plain:?}"
        );
    }

    /// The run a row's own words are drawn in. Found by what it says rather
    /// than where it falls, because a staffed row's box-drawing shares its
    /// style and merges into it while an unworked row's stands apart.
    fn the_words(painted: &[Run]) -> &Run {
        painted
            .iter()
            .find(|run| run.said.contains("a bead"))
            .expect("the title is drawn")
    }

    /// What `bd` already does to a closed row, arrived at from the other
    /// side: a finished branch nobody is on falls back into the page.
    #[test]
    fn a_finished_row_nobody_is_on_is_dimmed_to_the_grey_bd_dims_one_to() {
        let painted = Painted::of(
            bead_line(
                &row(&node("nix-9670s.1", "a bead", Status::Closed)),
                BRANCH,
                3,
            ),
            60,
            1,
        )
        .row(0);

        assert_eq!(
            painted[1].style.fg,
            Some(Color::Rgb(128, 144, 160)),
            "the glyph keeps its own status colour: {painted:?}"
        );
        assert_eq!(
            painted[2].style.fg,
            Some(Color::Rgb(108, 118, 128)),
            "{painted:?}"
        );
    }

    /// Exactly the row worth looking at, and dimming it is how it would be
    /// missed. `lines::split` leaves it out of a run for the same reason.
    #[test]
    fn a_closed_bead_whose_pane_is_still_alive_is_not_dimmed() {
        let mut alive = node("nix-9670s.1", "a bead", Status::Closed);
        alive.agent = Some(a_pane());
        alive.anomalies = vec![Anomaly::StalePane];

        let painted = Painted::of(bead_line(&row(&alive), BRANCH, 3), 110, 1).row(0);

        assert_eq!(painted[2].style.fg, Some(Color::Reset), "{painted:?}");
    }

    /// Finished means what it means in `lines::split` — closed, no agent, no
    /// anomaly — so an anomaly alone is enough to keep a row out of the dim.
    /// Nobody is on it, so it takes the middle tier, not the top.
    #[test]
    fn a_closed_bead_with_an_anomaly_against_it_is_not_dimmed() {
        let mut odd = node("nix-9670s.1", "a bead", Status::Closed);
        odd.anomalies = vec![Anomaly::StalePane];

        let painted = Painted::of(bead_line(&row(&odd), BRANCH, 3), 110, 1).row(0);

        assert_eq!(painted[2].style.fg, Some(Color::DarkGray), "{painted:?}");
    }

    // ---- what the scale may not be the only carrier of --------------------

    /// Every row `tone` can tell one from another, holding still everything it
    /// does not read. It reads three things — whether an agent is on the row,
    /// whether the bead is closed, whether anything is wrong with it — so the
    /// corpus is those three over each status, under one id and one title.
    fn every_liveness_row() -> Vec<Row> {
        let mut rows = Vec::new();
        for status in every_status() {
            for staffed in [false, true] {
                for odd in [false, true] {
                    let mut bead = node("nix-9670s.1", "a bead", status.clone());
                    bead.agent = staffed.then(a_pane);
                    if odd {
                        bead.anomalies = vec![Anomaly::StaleClaim { days: 58 }];
                    }
                    rows.push(row(&bead));
                }
            }
        }
        rows
    }

    /// A floor on the harm and not a detector of the fault. It says a reader
    /// can still tell the liveness states apart once the tones have run
    /// together; it does not say they have not run together, and nothing this
    /// project runs does. Neither `bdi-sw4` nor `bdi-kbd2` would have gone red
    /// here — `◍` and `✓` were on those rows the whole time while the tones
    /// collapsed — so a green board here is no evidence the scale is
    /// separated, and a reader who wants that has to go on looking for it.
    ///
    /// The claim is that a row's tier is a function of its words: two rows
    /// that read the same are drawn the same, so no rung carries a meaning by
    /// itself. Nothing here names a colour, which is what lets the scale move
    /// underneath it.
    #[test]
    fn no_liveness_state_is_told_apart_by_its_tone_alone() {
        let mut read: Vec<(String, Style)> = Vec::new();

        for row in every_liveness_row() {
            let tone = tone(&row);
            let words = Painted::of(bead_line(&row, BRANCH, 3), 200, 1)
                .rows()
                .swap_remove(0);

            if let Some((_, already)) = read.iter().find(|(said, _)| *said == words) {
                assert_eq!(
                    *already, tone,
                    "two liveness states read the same: {words:?}"
                );
            }
            read.push((words, tone));
        }

        // The one way this goes quietly vacuous is a corpus that stopped
        // reaching the states it means to. Said of the corpus rather than of
        // the tones it drew: a count of the distinct tones would be this test
        // asserting the rungs are apart, which is the claim `visual-language`
        // establishes cannot be made.
        assert_eq!(
            read.len(),
            every_status().len() * 4,
            "every status, against both of the other two things `tone` reads"
        );
    }

    /// The box-drawing says how the tree is shaped, not how a bead is going,
    /// so it holds the terminal's default while the row around it moves.
    /// `bd list` leaves its own tree prefix undimmed on a closed row too.
    #[test]
    fn the_box_drawing_a_row_hangs_under_never_takes_the_rows_brightness() {
        let mut staffed = node("nix-9670s.1", "a bead", Status::Open);
        staffed.agent = Some(a_pane());
        let unworked = node("nix-9670s.1", "a bead", Status::Open);
        let finished = node("nix-9670s.1", "a bead", Status::Closed);

        for bead in [staffed, unworked, finished] {
            let painted = Painted::of(bead_line(&row(&bead), BRANCH, 3), 90, 1).row(0);

            assert!(painted[0].said.starts_with(BRANCH), "{painted:?}");
            assert_eq!(painted[0].style.fg, Some(Color::Reset), "{painted:?}");
        }
    }

    /// A run stands for finished rows and is drawn as one of them, so the two
    /// cannot fall out of step and the palette holds one grey, not two.
    #[test]
    fn an_elided_run_is_dimmed_the_same_grey_a_finished_row_is() {
        let run = Painted::of(elided_run(BRANCH, 4), 60, 1).row(0);
        let finished = Painted::of(
            bead_line(
                &row(&node("nix-9670s.1", "a bead", Status::Closed)),
                BRANCH,
                3,
            ),
            60,
            1,
        )
        .row(0);

        assert_eq!(run[0].said, BRANCH, "{run:?}");
        assert_eq!(run[0].style.fg, Some(Color::Reset), "{run:?}");
        assert_eq!(run[1].style.fg, finished[1].style.fg, "the glyph: {run:?}");
        assert_eq!(
            run[2].style.fg, finished[2].style.fg,
            "what follows it: {run:?}"
        );
    }

    /// A project line's counts are its whole project's and not any one bead's,
    /// so the rule that decides a row's tier cannot be asked of it without
    /// quietly changing what it means. It stays off the scale. A root does
    /// not: it is a bead row, and the rule is asked of it like any other.
    #[test]
    fn a_project_line_is_left_off_the_scale_a_bead_row_is_on() {
        let quiet = project("homelab", counts(7, 7, 0, 0));

        let painted = Painted::of(project_line(&quiet, OPEN, None, drawn_at()), 60, 1).row(0);

        assert!(
            painted.iter().all(|run| run.style.fg == Some(Color::Reset)),
            "{painted:?}"
        );
    }

    /// `bd`'s hues belong to `bd`'s concepts. The live agent and the anomaly
    /// are the two things it cannot say, so they are a different axis and
    /// keep a different colour system whatever the row around them does.
    #[test]
    fn the_cells_bd_cannot_draw_keep_their_own_colours_however_bright_the_row() {
        let mut staffed = node("nix-9670s.1", "a bead", Status::InProgress);
        staffed.agent = Some(a_pane());
        staffed.anomalies = vec![Anomaly::StaleClaim { days: 58 }];

        let painted = Painted::of(bead_line(&row(&staffed), BRANCH, 3), 120, 1).row(0);

        assert!(
            painted
                .iter()
                .any(|run| run.said.contains(AGENT) && run.style.fg == palette::AGENT.fg),
            "{painted:?}"
        );
        assert!(
            painted
                .iter()
                .any(|run| run.said.contains(WARNING) && run.style.fg == palette::ATTENTION.fg),
            "{painted:?}"
        );
    }

    /// The one status `bd` has no colour for, because it has no such status.
    /// It takes the colour of the note already beside it on the row.
    #[test]
    fn a_status_bd_never_had_is_painted_the_colour_of_the_note_beside_it() {
        let odd = node("nix-9670s.1", "a bead", Status::Other("triage".into()));

        let painted = Painted::of(bead_line(&row(&odd), BRANCH, 3), 120, 1).row(0);

        assert_eq!(painted[1].said, "?", "{painted:?}");
        assert_eq!(painted[1].style.fg, palette::ATTENTION.fg, "{painted:?}");
    }
}
