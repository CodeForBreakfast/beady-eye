//! What a row is drawn in: `bd`'s own colour for a bead's status, and the
//! brightness scale that says how live the row it sits on is.

use ratatui::style::{Color, Style};

use crate::model::types::Status;
use crate::view::row::Row;

pub(super) const LIVE: Color = Color::Green;
pub(super) const LOOK_AT_THIS: Color = Color::Yellow;

/// `bd list`'s own colours for a status, read off `bd` 1.2.2's output. They
/// are literal rather than named because `bd`'s are: it sends 24-bit values
/// that do not move with the terminal's theme, so a named colour here would
/// track the theme away from the tool this is matching.
///
/// `open` is absent on purpose. `bd` sends no escape at all for it, and a
/// glyph that inherits is what lets a row's own brightness reach it.
const IN_PROGRESS: Color = Color::Rgb(255, 180, 84);
const BLOCKED: Color = Color::Rgb(242, 109, 120);
const CLOSED: Color = Color::Rgb(128, 144, 160);

/// `bd` draws a deferred bead's glyph and every cell of a finished row in
/// this one grey, so one name serves both.
pub(super) const DIM: Color = Color::Rgb(108, 118, 128);

/// The top of the brightness scale, and the one tier `bd list` could not
/// draw: a row a live agent is on. Named rather than literal because it is
/// `bdi`'s own and should follow the reader's terminal, not `bd`'s palette.
const STAFFED: Color = Color::White;

/// How live a row is, which is the one thing about a bead `bd list` has no
/// way to know — and so the one this scale is spent on.
///
/// | row | drawn |
/// |---|---|
/// | an agent is on it | brighter than the page |
/// | nobody on it, still going | the terminal's default |
/// | finished, nobody on it | the grey `bd` dims a closed row to |
///
/// Finished means what it means to `lines::split`: closed, no agent, no
/// anomaly. A closed bead whose pane is still alive is exactly the row worth
/// looking at, and dimming it is how it would be missed.
pub(super) fn tone(row: &Row) -> Style {
    if row.agent.is_some() {
        return Style::new().fg(STAFFED);
    }
    let finished = row.status.is_closed() && row.agent.is_none() && row.anomalies.is_none();

    fg(finished.then_some(DIM))
}

/// The colour a bead's status is drawn in.
///
/// Colour is the second channel and never the only one: the glyph already says
/// the status, so a terminal with no colour loses nothing.
pub(super) fn status_style(status: &Status) -> Style {
    fg(status_colour(status))
}

/// `bd`'s colour for a status, or none where `bd` sends no escape and the
/// glyph should take the brightness of the row it sits on.
pub(super) fn status_colour(status: &Status) -> Option<Color> {
    match status {
        Status::InProgress => Some(IN_PROGRESS),
        Status::Blocked => Some(BLOCKED),
        Status::Closed => Some(CLOSED),
        Status::Deferred => Some(DIM),
        Status::Open => None,
        // The one status `bd` has no colour for, because it has no such
        // status. It takes the colour of the note already beside it.
        Status::Other(_) => Some(LOOK_AT_THIS),
    }
}

/// A style that says a colour, or one that says nothing and lets the line's
/// own reach the span.
pub(super) fn fg(colour: Option<Color>) -> Style {
    colour.map_or_else(Style::new, |colour| Style::new().fg(colour))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    use crate::model::anomaly::Anomaly;
    use crate::view::draw::bead::{bead_line, elided_run};
    use crate::view::draw::project::project_line;
    use crate::view::draw::tests::*;
    use crate::view::row::{self, AGENT, WARNING};

    // ---- styling ---------------------------------------------------------

    /// One of each status, so a loop over them covers the set. The compiler
    /// holds `status_colour` total; this list is only what a test walks.
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
        let coloured: Vec<Color> = every_status().iter().filter_map(status_colour).collect();

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
        assert_eq!(status_colour(&Status::Open), None);
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
    /// terminal's default would leave a staffed row reading as two colours.
    #[test]
    fn an_open_glyph_takes_the_brightness_of_the_row_it_sits_on() {
        let mut staffed = node("nix-9670s.1", "a bead", Status::Open);
        staffed.agent = Some(a_pane());

        let painted = Painted::of(bead_line(&row(&staffed), BRANCH, 3), 90, 1).row(0);

        assert!(painted[1].said.starts_with('○'), "{painted:?}");
        assert_eq!(painted[1].style.fg, Some(Color::White), "{painted:?}");
    }

    /// The tier that earns the screen. `bd list` has no notion of a live
    /// agent, so it has no way to say which row is the one you came for.
    #[test]
    fn a_row_with_an_agent_on_it_is_drawn_brighter_than_one_without() {
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

        assert_eq!(bright[1].style.fg, Some(Color::White), "{bright:?}");
        assert_eq!(plain.len(), 1, "{plain:?}");
        assert_eq!(
            plain[0].style.fg,
            Some(Color::Reset),
            "nobody on it, so the whole line is the terminal's own"
        );
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

        assert_eq!(painted[2].style.fg, Some(Color::White), "{painted:?}");
    }

    /// Finished means what it means in `lines::split` — closed, no agent, no
    /// anomaly — so an anomaly alone is enough to keep a row out of the dim.
    #[test]
    fn a_closed_bead_with_an_anomaly_against_it_is_not_dimmed() {
        let mut odd = node("nix-9670s.1", "a bead", Status::Closed);
        odd.anomalies = vec![Anomaly::StalePane];

        let painted = Painted::of(bead_line(&row(&odd), BRANCH, 3), 110, 1).row(0);

        assert_eq!(painted[2].style.fg, Some(Color::Reset), "{painted:?}");
    }

    /// The box-drawing says how the tree is shaped, not how a bead is going,
    /// so it holds the terminal's default while the row around it moves.
    /// `bd list` leaves its own tree prefix undimmed on a closed row too.
    #[test]
    fn the_box_drawing_a_row_hangs_under_never_takes_the_rows_brightness() {
        let mut staffed = node("nix-9670s.1", "a bead", Status::Open);
        staffed.agent = Some(a_pane());
        let finished = node("nix-9670s.1", "a bead", Status::Closed);

        for bead in [staffed, finished] {
            let painted = Painted::of(bead_line(&row(&bead), BRANCH, 3), 90, 1).row(0);

            assert_eq!(painted[0].said, BRANCH, "{painted:?}");
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
                .any(|run| run.said.contains(AGENT) && run.style.fg == Some(LIVE)),
            "{painted:?}"
        );
        assert!(
            painted
                .iter()
                .any(|run| run.said.contains(WARNING) && run.style.fg == Some(LOOK_AT_THIS)),
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
        assert_eq!(painted[1].style.fg, Some(LOOK_AT_THIS), "{painted:?}");
    }
}
