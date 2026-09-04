//! The key bindings view: every binding drawn in a window over the forest.

use ratatui::layout::{Constraint, Rect};
use ratatui::text::Span;
use ratatui::widgets::{Block, Clear};
use ratatui::Frame;

use crate::view::fitted::{columns, indent, Fitted, CUT, GAP};
use crate::view::palette;

/// The first line of the key bindings view, and the way back out of it.
const CLOSE_BINDINGS: &str = "Key bindings · press any key to close";

/// Where the bindings window sits: the size its table wants, centred over the
/// forest, and clamped by the screen where the screen is the smaller.
///
/// `Rect::centered` is `Flex::Center` underneath, so a window wider or taller
/// than the terminal comes back the size of the terminal rather than
/// overflowing it.
///
/// The screen is the only ceiling, so on a terminal no taller than the table
/// the window fills it and the forest survives only in the columns beside. A
/// lower ceiling can only be paid for out of the table, and what it buys is
/// the forest's first lines: `Flex::Center` splits the leftover rows around
/// the window, and the selection is drawn mid-band, so the rows a cap frees
/// are the top of a tree rather than the row the reader opened `?` from. The
/// forest is one keypress away and the selection is where they left it; a
/// binding a cap hides is reachable from nowhere else, this being the view
/// that says which keys exist.
///
/// Which is also why the table growing by a row per key needs no ceiling of
/// its own. Past the height of the screen `key_bindings` counts the bindings
/// it left off, and that reads the same over fifty of them as over twenty.
pub fn bindings_window(area: Rect, bindings: &[(String, &str)]) -> Rect {
    area.centered(
        Constraint::Length(wanted_width(bindings)),
        Constraint::Length(bindings.len() as u16 + BORDERS),
    )
}

/// The width the whole table would like: its longest row, the count it would
/// draw if every binding were left off, and the title, whichever is widest.
fn wanted_width(bindings: &[(String, &str)]) -> u16 {
    let keys = key_column(bindings);
    let widest = bindings
        .iter()
        .map(|(_, does)| GAP + keys + GAP + does.chars().count())
        .chain([
            GAP + left_off(bindings.len()).chars().count(),
            CLOSE_BINDINGS.chars().count(),
        ])
        .max()
        .unwrap_or(0);

    u16::try_from(widest)
        .unwrap_or(u16::MAX)
        .saturating_add(BORDERS)
}

/// How wide the keys are set, so that what a binding does starts in the same
/// column on every row. The sizing and the drawing read it from here rather
/// than working it out twice.
fn key_column(bindings: &[(String, &str)]) -> usize {
    bindings
        .iter()
        .map(|(keys, _)| columns(&[Span::raw(keys.clone())]))
        .max()
        .unwrap_or(0)
}

/// Draw every binding in a window over the forest.
///
/// Each pair is the keys to press, already named, and what pressing them
/// does. `Clear` blanks the window first, which is what stops the trees
/// showing through between the rows.
///
/// The way out is the border's title, so a window too short for a single
/// binding still holds it: a reader who cannot see how to leave is stuck in a
/// view they may have opened by accident. Where the bindings do not all fit,
/// the last row counts the ones left off, because a list that simply stopped
/// would read as the whole of what the view answers to. A window is smaller
/// than the screen it sits on, so that is the ordinary case rather than the
/// short-terminal one.
pub fn key_bindings(frame: &mut Frame, area: Rect, bindings: &[(String, &str)]) {
    let window = bindings_window(area, bindings);
    if window.is_empty() {
        return;
    }

    let block = Block::bordered().title(Span::styled(CLOSE_BINDINGS, palette::TITLE));
    let inner = block.inner(window);
    frame.render_widget(Clear, window);
    frame.render_widget(block, window);

    let room = inner.height as usize;
    // A last row spent saying that one binding is missing would be better
    // spent on the binding, so the count is never drawn over fewer than two.
    let shown = if bindings.len() <= room {
        bindings.len()
    } else {
        room.saturating_sub(1)
    };
    let width = key_column(bindings);
    let row = |n: usize| Rect {
        y: inner.y + n as u16,
        height: 1,
        ..inner
    };

    for (n, (keys, does)) in bindings.iter().take(shown).enumerate() {
        frame.render_widget(
            Fitted::new(
                vec![Span::raw(format!("{}{keys:<width$}", indent()))],
                vec![Span::raw((*does).to_string())],
                Vec::new(),
            ),
            row(n),
        );
    }

    if shown < bindings.len() && room > 0 {
        frame.render_widget(
            Fitted::new(
                vec![Span::raw(format!(
                    "{}{}",
                    indent(),
                    left_off(bindings.len() - shown)
                ))],
                Vec::new(),
                Vec::new(),
            ),
            row(shown),
        );
    }
}

/// The bindings a screen this short had no room for, counted rather than
/// dropped.
fn left_off(count: usize) -> String {
    let binding = if count == 1 { "binding" } else { "bindings" };
    format!("{CUT} {count} more {binding} · no room on a screen this short")
}

/// The rows a bordered window spends on its own edges.
const BORDERS: u16 = 2;

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    // ---- the key bindings view -------------------------------------------

    /// Three bindings shaped like the real ones: several keys onto one action,
    /// a single key, and a control key alongside a plain one.
    fn a_few_bindings() -> Vec<(String, &'static str)> {
        vec![
            ("Down, j".to_string(), "move down one row"),
            (
                "Enter".to_string(),
                "focus the selected bead's pane in herdr",
            ),
            ("q, ^C".to_string(), "quit"),
        ]
    }

    fn bindings_frame(bindings: &[(String, &str)], width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test backend");
        terminal
            .draw(|frame| key_bindings(frame, frame.area(), bindings))
            .expect("a draw into memory");
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
            .collect()
    }

    /// The whole view, character for character: a bordered window sized to
    /// its own table and centred on the screen, with the way out in its
    /// title. The keys share a column so a reader's eye runs down one edge to
    /// find the one they want.
    #[test]
    fn the_key_bindings_view_names_the_keys_and_what_pressing_them_does() {
        assert_eq!(
            bindings_frame(&a_few_bindings(), 60, 5),
            vec![
                "   ┌Key bindings · press any key to close───────────────┐   ",
                "   │  Down, j  move down one row                        │   ",
                "   │  Enter    focus the selected bead's pane in herdr  │   ",
                "   │  q, ^C    quit                                     │   ",
                "   └────────────────────────────────────────────────────┘   ",
            ]
        );
    }

    /// A reader who cannot see how to leave is stuck in a view they may have
    /// opened by accident, so the way out is the line that survives every cut.
    #[test]
    fn the_way_out_is_drawn_before_any_binding_is() {
        let drawn = bindings_frame(&a_few_bindings(), 60, 1);

        assert!(drawn[0].contains("press any key to close"), "{drawn:?}");
    }

    /// Degrade, never disappear: a list that simply stopped would read as the
    /// whole of what the view answers to.
    ///
    /// A window costs two of the screen's rows on its own border, so this is
    /// what an ordinary screen does rather than what a short one does.
    #[test]
    fn a_screen_too_short_for_every_binding_counts_the_ones_it_left_off() {
        assert_eq!(
            bindings_frame(&a_few_bindings(), 60, 4),
            vec![
                "   ┌Key bindings · press any key to close───────────────┐   ",
                "   │  Down, j  move down one row                        │   ",
                "   │  … 2 more bindings · no room on a screen this short│   ",
                "   └────────────────────────────────────────────────────┘   ",
            ]
        );
    }

    #[test]
    fn one_binding_left_off_is_not_counted_in_the_plural() {
        assert!(left_off(1).contains("1 more binding ·"), "{}", left_off(1));
        assert!(left_off(2).contains("2 more bindings ·"), "{}", left_off(2));
    }

    /// Which is why the count is always in the plural on screen: the row it
    /// costs is a row a binding could have had.
    #[test]
    fn the_count_is_never_spent_to_hide_fewer_bindings_than_it_displaces() {
        let bindings = a_few_bindings();

        for height in 2..=(bindings.len() as u16 + 2) {
            let drawn = bindings_frame(&bindings, 60, height);
            let counted = drawn.iter().filter(|row| row.contains("more binding"));

            for row in counted {
                assert!(!row.contains("1 more binding"), "at {height} rows: {row}");
            }
        }
    }

    /// One row is room for the way out and nothing else: it is the window's
    /// top edge, and the title on it says what was opened and how to leave.
    /// That is the most a single row can do.
    #[test]
    fn a_screen_with_one_row_spends_it_on_the_way_out() {
        assert_eq!(
            bindings_frame(&a_few_bindings(), 60, 1),
            vec!["   ┌Key bindings · press any key to close───────────────┐   "]
        );
    }

    /// Two rows are both of the window's edges and no inside at all. A count
    /// drawn there would land on the bottom border, which is the one row that
    /// cannot be spent.
    #[test]
    fn a_window_with_no_room_inside_it_draws_nothing_inside_it() {
        assert_eq!(
            bindings_frame(&a_few_bindings(), 60, 2),
            vec![
                "   ┌Key bindings · press any key to close───────────────┐   ",
                "   └────────────────────────────────────────────────────┘   ",
            ]
        );
    }

    /// A table of any height, shaped like the real one. Each line is distinct
    /// so that a binding which reached the screen can be told from one that
    /// did not, and no line is a substring of another.
    fn a_table_of(count: usize) -> Vec<String> {
        (0..count).map(|n| format!("does the {n} thing")).collect()
    }

    fn bindings_over(lines: &[String]) -> Vec<(String, &str)> {
        lines
            .iter()
            .enumerate()
            .map(|(n, does)| (format!("k{n}"), does.as_str()))
            .collect()
    }

    /// The number the count row is carrying, or none where nothing is being
    /// counted.
    fn counted(drawn: &[String]) -> usize {
        drawn
            .iter()
            .find_map(|row| {
                row.split_once(CUT)?
                    .1
                    .split_whitespace()
                    .next()?
                    .parse()
                    .ok()
            })
            .unwrap_or(0)
    }

    /// Every binding is either on the screen or in the count of the ones that
    /// are not — at every screen height, over tables of every size.
    ///
    /// The table grows by a row per key and its only ceiling is the screen, so
    /// this relationship is what has to hold rather than any particular
    /// height. Swept from three rows, the shortest window with a row inside
    /// it: two rows are both of the borders and no inside at all, which is
    /// `a_window_with_no_room_inside_it_draws_nothing_inside_it` above.
    #[test]
    fn every_binding_is_drawn_or_counted_at_every_height() {
        for count in [1usize, 2, 3, 12, 30] {
            let lines = a_table_of(count);
            let bindings = bindings_over(&lines);

            for height in 3..=(count as u16 + 4) {
                let drawn = bindings_frame(&bindings, 80, height);
                let on_screen = lines
                    .iter()
                    .filter(|does| drawn.iter().any(|row| row.contains(*does)))
                    .count();

                assert_eq!(
                    on_screen + counted(&drawn),
                    count,
                    "at {height} rows over {count} bindings: {drawn:#?}"
                );
            }
        }
    }

    /// The screen is the window's only ceiling, and the table is what it
    /// wants: no table is too tall to ask for, and none is cut short of what
    /// the screen would hold.
    ///
    /// The relationship rather than a height, so that the table growing by a
    /// row per key can never make it false. A cap of any kind fails this, and
    /// is meant to — the ceiling is a decision, and `bindings_window` above
    /// says which one and why.
    #[test]
    fn the_screen_is_the_windows_only_ceiling() {
        for count in [1usize, 3, 12, 30] {
            let lines = a_table_of(count);
            let bindings = bindings_over(&lines);
            let wanted = count as u16 + BORDERS;

            for height in 1..=40u16 {
                let window = bindings_window(Rect::new(0, 0, 80, height), &bindings);

                assert_eq!(
                    window.height,
                    wanted.min(height),
                    "a window over {count} bindings on a screen {height} rows tall"
                );
            }
        }
    }

    /// The band can be nothing at all, and asking for a row inside it would
    /// draw outside the frame.
    #[test]
    fn a_band_with_no_rows_in_it_draws_nothing() {
        let mut terminal = Terminal::new(TestBackend::new(20, 1)).expect("a test backend");
        terminal
            .draw(|frame| {
                key_bindings(frame, Rect::new(0, 0, 20, 0), &a_few_bindings());
            })
            .expect("a draw into memory");

        assert_eq!(terminal.backend().buffer()[(0, 0)].symbol(), " ");
    }
}
