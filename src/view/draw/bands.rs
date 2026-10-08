//! The bands the screen is divided into, and which line of the forest a row
//! of one of them is showing.

use ratatui::layout::Rect;

use crate::view::tail;

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

/// Whether the tail's band is on the screen, which the reader turns with `t`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TailBand {
    Shown,
    Hidden,
}

/// Divide the screen between the forest, the tail and the key bar.
///
/// `lines` is how many rows the forest has to show. The tail takes the rows
/// the forest leaves free, up to half the screen, so a short tree leaves no
/// blank between itself and the pane. Where the forest needs the rows, the
/// tail keeps `LINES` and its rule and gives up those before the forest gives
/// up any, and the forest is never left with none: a `bdi` with no tree on
/// screen is not showing the thing it exists to show.
///
/// A hidden tail takes no rows, and the forest has them all.
pub fn regions(area: Rect, lines: usize, band: TailBand) -> Regions {
    let keys = key_rows(area);
    let rows = area.height - keys;
    let tail = match band {
        TailBand::Shown => {
            let looking = (tail::LINES + 1).min(rows.saturating_sub(1) / 2);
            let free = rows.saturating_sub(u16::try_from(lines).unwrap_or(u16::MAX));
            looking
                .max(free.min(area.height / 2))
                .min(rows.saturating_sub(1))
        }
        TailBand::Hidden => 0,
    };
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

/// How many rows the key bar takes at the foot of the screen: one, unless
/// the screen is a single row and the forest needs it.
pub fn key_rows(area: Rect) -> u16 {
    if area.height >= 2 {
        1
    } else {
        0
    }
}

/// Which line the forest draws on one row of the screen, where it draws one.
///
/// The inverse of the skip-and-take in `draw`, and here beside it rather than
/// beside the click that asks the question: the two are one agreement about
/// where a line goes, and the failure they can have is drifting apart. Both
/// are handed `from` rather than working one out, so there is one viewport
/// and not two answers about it.
///
/// The column is not asked for. Every band spans the width of the screen, so
/// a row is the whole of what a pointer names.
pub fn line_at(forest: Rect, from: usize, lines: usize, row: u16) -> Option<usize> {
    let within = row.checked_sub(forest.y)? as usize;
    if within >= forest.height as usize {
        return None;
    }

    let at = from + within;
    (at < lines).then_some(at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    use crate::model::snapshot::ProviderState;
    use crate::view::draw::tests::*;
    use crate::view::{Action, Motion, Notch};

    /// How far a notch is told to go here, which the config settles for a
    /// reader and neither this file nor the forest decides.
    const A_NOTCH: usize = 3;

    /// A forest with more lines than any screen here has rows, so the tail
    /// is left only what it keeps when the forest needs the rest.
    const A_TALL_TREE: usize = 100;

    // ---- the bands of the screen -----------------------------------------

    #[test]
    fn a_full_screen_gives_the_forest_most_of_it_the_tail_a_look_and_the_keys_a_row() {
        let bands = regions(Rect::new(0, 0, 80, 24), A_TALL_TREE, TailBand::Shown);

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
        let bands = regions(Rect::new(0, 0, 80, 10), A_TALL_TREE, TailBand::Shown);

        assert_eq!(bands.forest.height, 5);
        assert_eq!(bands.tail.height, 4);
        assert_eq!(bands.keys.height, 1);
    }

    /// A short tree leaves rows free under it, and the tail takes them up
    /// to half the screen, so no blank sits between the tree and the pane.
    #[test]
    fn a_short_tree_gives_the_tail_its_free_rows_up_to_half_the_screen() {
        let bands = regions(Rect::new(0, 0, 80, 40), 5, TailBand::Shown);

        assert_eq!(bands.tail.height, 20, "half of forty rows");
        assert_eq!(bands.forest.height, 19);
        assert_eq!(bands.keys, Rect::new(0, 39, 80, 1));
    }

    #[test]
    fn a_tree_leaving_less_than_half_the_screen_free_gives_the_tail_exactly_that() {
        let bands = regions(Rect::new(0, 0, 80, 40), 25, TailBand::Shown);

        assert_eq!(bands.forest.height, 25, "every line of the tree is drawn");
        assert_eq!(bands.tail.height, 14);
    }

    /// The tail never takes less than it keeps for a tree that needs the
    /// screen, however short the tree.
    #[test]
    fn a_tree_leaving_fewer_rows_free_than_the_tail_keeps_is_scrolled_instead() {
        let bands = regions(Rect::new(0, 0, 80, 24), 20, TailBand::Shown);

        assert_eq!(bands.tail.height, tail::LINES + 1);
        assert_eq!(bands.forest.height, 16);
    }

    #[test]
    fn the_forest_keeps_a_row_however_little_room_there_is() {
        for lines in [0, 1, A_TALL_TREE] {
            for height in 1..=8 {
                let bands = regions(Rect::new(0, 0, 80, height), lines, TailBand::Shown);
                assert!(
                    bands.forest.height >= 1,
                    "{height} rows, {lines} lines: {bands:?}"
                );
            }
        }
    }

    /// The two smallest screens that still show something, pinned so the rule
    /// that produces them cannot be simplified into one that does not.
    #[test]
    fn the_smallest_screens_spend_their_rows_on_the_forest_first() {
        assert_eq!(
            regions(Rect::new(0, 0, 80, 2), 0, TailBand::Shown),
            Regions {
                forest: Rect::new(0, 0, 80, 1),
                tail: Rect::new(0, 1, 80, 0),
                keys: Rect::new(0, 1, 80, 1),
            }
        );
        assert_eq!(
            regions(Rect::new(0, 0, 80, 1), 0, TailBand::Shown),
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
        for band in [TailBand::Shown, TailBand::Hidden] {
            for (height, lines) in
                (0..40).flat_map(|height| [(height, 0), (height, 9), (height, A_TALL_TREE)])
            {
                let area = Rect::new(3, 7, 80, height);
                let bands = regions(area, lines, band);
                let at = format!("{band:?} on {height} rows");

                assert_eq!(bands.forest.y, area.y, "{at}");
                assert_eq!(bands.tail.y, bands.forest.y + bands.forest.height, "{at}");
                assert_eq!(bands.keys.y, bands.tail.y + bands.tail.height, "{at}");
                assert_eq!(
                    bands.forest.height + bands.tail.height + bands.keys.height,
                    area.height,
                    "{at}"
                );
            }
        }
    }

    /// A hidden tail gives the forest every row the keys leave, whether the
    /// tree needs them or not.
    #[test]
    fn a_hidden_tail_gives_the_forest_every_row_above_the_keys() {
        for lines in [5, A_TALL_TREE] {
            let bands = regions(Rect::new(0, 0, 80, 24), lines, TailBand::Hidden);

            assert_eq!(bands.forest, Rect::new(0, 0, 80, 23), "{lines} lines");
            assert_eq!(bands.tail.height, 0, "{lines} lines");
            assert_eq!(bands.keys, Rect::new(0, 23, 80, 1), "{lines} lines");
        }
    }

    // ---- the line a screen row shows --------------------------------------

    /// The inverse held against the drawing rather than against itself. The
    /// fixture's rows are the header and then `bead number 1` upward, so what
    /// is on a row says which line was drawn there, and a forest taller than
    /// its band is scrolled far enough that an off-by-one in either direction
    /// shows.
    ///
    /// Both ways of scrolling it, because they are what the inverse and the
    /// drawing could disagree about: a motion moves the selection and the
    /// view after it, and a notch moves the view alone. An inverse still
    /// deriving the offset from the selection agrees with the drawing on the
    /// first and is wrong by the whole scroll on the second.
    #[test]
    fn every_row_of_the_forest_names_the_line_drawn_on_it() {
        for wheeled in 0..3 {
            let (width, height) = (60, 24);
            let band = regions(Rect::new(0, 0, width, height), A_TALL_TREE, TailBand::Shown).forest;
            let mut forest = opened(&snapshot(
                vec![grove(40)],
                Vec::new(),
                ProviderState::Answering,
            ));
            forest.fit(band.height as usize);
            forest.apply(Action::Move(Motion::HalfScreenDown));
            for _ in 0..wheeled {
                forest.scrolled(Notch::Down, A_NOTCH);
            }

            let frame = frame_of(&forest, width, height).rows();
            let lines = forest.lines().len();

            for row in band.y..band.y + band.height {
                let at =
                    line_at(band, forest.from(), lines, row).expect("the band is full of lines");
                let shown = match at {
                    0 => "summit-works".to_string(),
                    1 => "lift the ground station".to_string(),
                    at => format!("bead number {}", at - 1),
                };
                assert!(
                    frame[row as usize].contains(&shown),
                    "after {wheeled} notches, row {row} shows {:?}, not line {at}",
                    frame[row as usize]
                );
            }
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
        let bands = regions(Rect::new(0, 0, 60, 24), A_TALL_TREE, TailBand::Shown);
        let (from, lines) = (0, 100);

        for row in [bands.tail.y, bands.tail.y + 3, bands.keys.y] {
            assert_eq!(line_at(bands.forest, from, lines, row), None, "row {row}");
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

    /// A band scrolled away from the top names the lines it is showing, and
    /// not the ones the forest starts with.
    #[test]
    fn a_scrolled_band_names_the_lines_it_is_showing() {
        let band = Rect::new(0, 0, 60, 8);

        assert_eq!(line_at(band, 12, 100, 0), Some(12));
        assert_eq!(line_at(band, 12, 100, 7), Some(19));
        assert_eq!(line_at(band, 12, 100, 8), None);
    }
}
