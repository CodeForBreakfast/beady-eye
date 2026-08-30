//! The key bindings view: every binding drawn in a window over the forest.

use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Clear};
use ratatui::Frame;

use crate::view::fitted::{columns, indent, Fitted, CUT, GAP};

/// The first line of the key bindings view, and the way back out of it.
const CLOSE_BINDINGS: &str = "Key bindings · press any key to close";

/// Where the bindings window sits: the size its table wants, centred over the
/// forest, and clamped by the screen where the screen is the smaller.
///
/// `Rect::centered` is `Flex::Center` underneath, so a window wider or taller
/// than the terminal comes back the size of the terminal rather than
/// overflowing it.
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

    let block = Block::bordered().title(Span::styled(
        CLOSE_BINDINGS,
        Style::new().add_modifier(Modifier::BOLD),
    ));
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
