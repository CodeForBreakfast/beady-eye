//! The bands the screen is divided into, and which line of the forest a row
//! of one of them is showing.

use ratatui::layout::Rect;

use crate::view::tail;

/// How far `^D` and `^U` move the selection: half the forest's own band,
/// rather than half a screen the key bar and the tail also sit in.
pub fn half_screen(forest: Rect) -> usize {
    (forest.height / 2) as usize
}

/// The three bands of the screen, top to bottom.
///
/// Named rather than returned from `draw` because the tail is drawn by
/// whoever holds one, and `draw` is handed a forest and no tail. Both sides
/// ask here instead of agreeing a number twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Regions {
    pub forest: Rect,
    pub tail: Rect,
    pub keys: Rect,
}

/// Divide the screen between the forest, the tail and the key bar.
///
/// The tail gives up its rows before the forest gives up any, and the forest
/// is never left with none: a `bdi` with no tree on screen is not showing the
/// thing it exists to show.
pub fn regions(area: Rect) -> Regions {
    let mut rows = area.height;
    let keys = if rows >= 2 { 1 } else { 0 };
    rows -= keys;
    let tail = (tail::LINES + 1).min(rows.saturating_sub(1) / 2);
    let forest = rows - tail;

    Regions {
        forest: Rect {
            height: forest,
            ..area
        },
        tail: Rect {
            y: area.y + forest,
            height: tail,
            ..area
        },
        keys: Rect {
            y: area.y + forest + tail,
            height: keys,
            ..area
        },
    }
}

/// The first visible line, so that the selection is on screen.
///
/// A pure function of the selection, which is what lets the renderer hold no
/// scroll state of its own: the selection moves, the window follows it, and
/// there is no third thing to keep in step with the other two.
pub(super) fn scroll_offset(selected: usize, lines: usize, height: usize) -> usize {
    if lines <= height || height == 0 {
        return 0;
    }
    selected.saturating_sub(height / 2).min(lines - height)
}

/// Which line the forest draws on one row of the screen, where it draws one.
///
/// The inverse of the skip-and-take in `draw`, and here beside it rather than
/// beside the click that asks the question: the two are one agreement about
/// where a line goes, and the failure they can have is drifting apart.
///
/// The column is not asked for. Every band spans the width of the screen, so
/// a row is the whole of what a pointer names.
pub fn line_at(forest: Rect, selected: usize, lines: usize, row: u16) -> Option<usize> {
    let within = row.checked_sub(forest.y)? as usize;
    if within >= forest.height as usize {
        return None;
    }

    let at = scroll_offset(selected, lines, forest.height as usize) + within;
    (at < lines).then_some(at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    use crate::model::snapshot::ProviderState;
    use crate::view::draw::tests::*;
    use crate::view::{Action, Motion};

    // ---- the bands of the screen -----------------------------------------

    #[test]
    fn a_full_screen_gives_the_forest_most_of_it_the_tail_a_look_and_the_keys_a_row() {
        let bands = regions(Rect::new(0, 0, 80, 24));

        assert_eq!(bands.forest, Rect::new(0, 0, 80, 16));
        assert_eq!(
            bands.tail,
            Rect::new(0, 16, 80, 7),
            "six lines of pane, under the rule that names it"
        );
        assert_eq!(bands.keys, Rect::new(0, 23, 80, 1));
    }

    /// The tail yields first, because the forest is the thing this tool is
    /// for and a screen showing no tree is showing nothing.
    #[test]
    fn a_short_screen_takes_the_rows_from_the_tail_and_not_from_the_forest() {
        let bands = regions(Rect::new(0, 0, 80, 10));

        assert_eq!(bands.forest.height, 5);
        assert_eq!(bands.tail.height, 4);
        assert_eq!(bands.keys.height, 1);
    }

    #[test]
    fn the_forest_keeps_a_row_however_little_room_there_is() {
        for height in 1..=8 {
            let bands = regions(Rect::new(0, 0, 80, height));
            assert!(bands.forest.height >= 1, "{height} rows: {bands:?}");
        }
    }

    /// The two smallest screens that still show something, pinned so the rule
    /// that produces them cannot be simplified into one that does not.
    #[test]
    fn the_smallest_screens_spend_their_rows_on_the_forest_first() {
        assert_eq!(
            regions(Rect::new(0, 0, 80, 2)),
            Regions {
                forest: Rect::new(0, 0, 80, 1),
                tail: Rect::new(0, 1, 80, 0),
                keys: Rect::new(0, 1, 80, 1),
            }
        );
        assert_eq!(
            regions(Rect::new(0, 0, 80, 1)),
            Regions {
                forest: Rect::new(0, 0, 80, 1),
                tail: Rect::new(0, 1, 80, 0),
                keys: Rect::new(0, 1, 80, 0),
            }
        );
    }

    /// The three bands are the screen: a gap between them would draw whatever
    /// the last frame left there, and an overlap would draw two things at once.
    #[test]
    fn the_three_bands_tile_the_screen_exactly() {
        for height in 0..40 {
            let area = Rect::new(3, 7, 80, height);
            let bands = regions(area);

            assert_eq!(bands.forest.y, area.y, "{height}");
            assert_eq!(
                bands.tail.y,
                bands.forest.y + bands.forest.height,
                "{height}"
            );
            assert_eq!(bands.keys.y, bands.tail.y + bands.tail.height, "{height}");
            assert_eq!(
                bands.forest.height + bands.tail.height + bands.keys.height,
                area.height,
                "{height}"
            );
        }
    }

    // ---- the scroll offset -----------------------------------------------

    #[test]
    fn a_forest_that_fits_the_viewport_never_scrolls() {
        for selected in 0..5 {
            assert_eq!(scroll_offset(selected, 5, 10), 0, "{selected}");
        }
    }

    #[test]
    fn the_selection_is_always_inside_the_viewport() {
        let (lines, height) = (100, 10);
        for selected in 0..lines {
            let offset = scroll_offset(selected, lines, height);
            assert!(
                (offset..offset + height).contains(&selected),
                "row {selected} fell outside {offset}..{}",
                offset + height
            );
        }
    }

    /// Scrolling past the end would draw blank rows under the last one, which
    /// reads as a forest that has run out rather than one that has ended.
    #[test]
    fn the_last_row_is_reachable_without_scrolling_past_the_end() {
        assert_eq!(scroll_offset(99, 100, 10), 90);
        assert_eq!(scroll_offset(0, 100, 10), 0);
    }

    /// A viewport with no rows in it has nowhere to scroll to, and the
    /// arithmetic that finds the offset would run off the bottom of `usize`.
    #[test]
    fn a_viewport_with_no_room_asks_for_no_offset() {
        assert_eq!(scroll_offset(40, 100, 0), 0);
    }

    // ---- the line a screen row shows --------------------------------------

    /// The inverse held against the drawing rather than against itself. The
    /// fixture's rows are the header and then `bead number 1` upward, so what
    /// is on a row says which line was drawn there, and a forest taller than
    /// its band is scrolled far enough that an off-by-one in either direction
    /// shows.
    #[test]
    fn every_row_of_the_forest_names_the_line_drawn_on_it() {
        let (width, height) = (60, 24);
        let band = regions(Rect::new(0, 0, width, height)).forest;
        let mut forest = opened(&snapshot(
            vec![grove(40)],
            Vec::new(),
            ProviderState::Answering,
        ));
        forest.set_half_screen(half_screen(band));
        forest.apply(Action::Move(Motion::HalfScreenDown));

        let frame = frame_of(&forest, width, height).rows();
        let selected = forest.selected_line();
        let lines = forest.lines().len();

        for row in band.y..band.y + band.height {
            let at = line_at(band, selected, lines, row).expect("the band is full of lines");
            let shown = match at {
                0 => "summit-works".to_string(),
                1 => "lift the ground station".to_string(),
                at => format!("bead number {}", at - 1),
            };
            assert!(
                frame[row as usize].contains(&shown),
                "row {row} shows {:?}, not line {at}",
                frame[row as usize]
            );
        }
    }

    /// The rows under the last line of a short forest are blank, and a click
    /// on blank is a click on nothing.
    #[test]
    fn a_row_past_the_last_line_names_none() {
        let band = Rect::new(0, 0, 60, 16);

        assert_eq!(line_at(band, 0, 3, 2), Some(2));
        for row in 3..16 {
            assert_eq!(line_at(band, 0, 3, row), None, "row {row}");
        }
    }

    /// The tail and the key row are drawn by someone else and hold nothing
    /// the selection can sit on.
    #[test]
    fn a_row_outside_the_forest_band_names_none() {
        let bands = regions(Rect::new(0, 0, 60, 24));
        let (selected, lines) = (0, 100);

        for row in [bands.tail.y, bands.tail.y + 3, bands.keys.y] {
            assert_eq!(
                line_at(bands.forest, selected, lines, row),
                None,
                "row {row}"
            );
        }
    }

    /// A band that starts partway down the screen is the only kind the forest
    /// ever gets when something is drawn above it, and a row measured from
    /// the top of the screen rather than the top of the band would be wrong
    /// by exactly that offset.
    #[test]
    fn a_row_above_the_forest_band_names_none() {
        let band = Rect::new(0, 4, 60, 8);

        assert_eq!(line_at(band, 0, 100, 4), Some(0));
        for row in 0..4 {
            assert_eq!(line_at(band, 0, 100, row), None, "row {row}");
        }
    }

    /// `^D` and `^U` move by half the band the trees are in, not half a
    /// screen the keys and the tail also sit in.
    #[test]
    fn a_half_screen_is_half_the_forest_and_not_half_the_frame() {
        assert_eq!(half_screen(regions(Rect::new(0, 0, 80, 24)).forest), 8);
        assert_eq!(half_screen(Rect::new(0, 0, 80, 1)), 0);
    }
}
