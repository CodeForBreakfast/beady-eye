//! What a widget drew, with the style it drew it in.
//!
//! The whole `Style` and not the one channel a caller had in mind: a helper
//! that reads a single channel is blind to whatever the view reaches for
//! next, and a blind helper is one a test can pick up by mistake.
//!
//! A row is cut into runs wherever the whole style changes, so a span naming
//! its own colour starts a run, and an attribute the whole row carries — the
//! selection's `REVERSED`, a tone — cannot split one.
//!
//! A test that wants only the words takes them off `rows`. The reverse is
//! impossible, which is why the words are never what is returned.

use ratatui::backend::TestBackend;
use ratatui::buffer::{Buffer, Cell, CellDiffOption};
use ratatui::style::Style;
use ratatui::widgets::Widget;
use ratatui::{Frame, Terminal};

/// A run of columns drawn in one style.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Run {
    pub(crate) said: String,
    pub(crate) style: Style,
}

impl Run {
    pub(crate) fn new(said: &str, style: Style) -> Self {
        Self {
            said: said.to_string(),
            style,
        }
    }
}

/// What a draw put on screen: every row, in runs of a single style.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Painted(Vec<Vec<Run>>);

impl Painted {
    /// What a widget puts on screen, given the whole area to itself.
    pub(crate) fn of<W: Widget>(widget: W, width: u16, height: u16) -> Self {
        Self::drawn_by(width, height, |frame| {
            frame.render_widget(widget, frame.area());
        })
    }

    /// What a draw puts on screen, for the draws that want the frame itself
    /// rather than a widget to fill it.
    pub(crate) fn drawn_by(width: u16, height: u16, draw: impl FnOnce(&mut Frame)) -> Self {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test backend");
        terminal.draw(draw).expect("a draw into memory");
        Self::read(terminal.backend().buffer())
    }

    /// What is already in a buffer, for a widget rendered into one by hand.
    pub(crate) fn read(buffer: &Buffer) -> Self {
        Self(
            (buffer.area.top()..buffer.area.bottom())
                .map(|y| {
                    let mut runs: Vec<Run> = Vec::new();
                    let mut x = buffer.area.left();
                    while x < buffer.area.right() {
                        let cell = &buffer[(x, y)];
                        x += covered(cell);
                        let said = words_of(cell.symbol());
                        match runs.last_mut() {
                            Some(run) if run.style == cell.style() => run.said.push_str(&said),
                            _ => runs.push(Run::new(&said, cell.style())),
                        }
                    }
                    runs
                })
                .collect(),
        )
    }

    /// The words, one string per row, for a test that is not about style.
    pub(crate) fn rows(&self) -> Vec<String> {
        self.0
            .iter()
            .map(|runs| runs.iter().map(|run| run.said.as_str()).collect())
            .collect()
    }

    /// One row, in runs of a single style.
    pub(crate) fn row(&self, y: usize) -> Vec<Run> {
        self.0[y].clone()
    }
}

/// How many columns a cell answers for. A cell that reports a width its
/// symbol does not have covers the ones behind it: the diff skips them and
/// the terminal never hears what they hold, so reading them would say the
/// cell's words a second time.
fn covered(cell: &Cell) -> u16 {
    match cell.diff_option {
        CellDiffOption::ForcedWidth(width) => width.get(),
        _ => 1,
    }
}

/// What a cell says, with any escape sequence taken out of it.
///
/// A hyperlink is written into the symbol because it cannot be written into a
/// span, but it is not among the words: a reader sees where the link goes
/// only by following it.
fn words_of(symbol: &str) -> String {
    let mut words = String::new();
    let mut rest = symbol;
    while let Some(open) = rest.find(ESCAPE) {
        words.push_str(&rest[..open]);
        rest = match rest[open..].find(ST) {
            Some(end) => &rest[open + end + ST.len()..],
            None => "",
        };
    }
    words.push_str(rest);
    words
}

/// The escape that opens an operating-system command, and the one that ends
/// it. `Fitted` writes a hyperlink between them.
const ESCAPE: char = '\x1b';
const ST: &str = "\x1b\\";

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    use std::num::NonZeroU16;

    use ratatui::layout::Rect;
    use ratatui::style::{Color, Modifier};
    use ratatui::text::{Line, Span};

    /// Two spans one colour cannot tell apart. Cut on the colour alone they
    /// would be a single run wearing the first one's style, and the second
    /// span's emphasis would be reported as absent — which is the blindness
    /// this helper exists to end.
    #[test]
    fn a_run_ends_wherever_any_part_of_the_style_changes() {
        let plain = Style::new().fg(Color::Red);
        let painted = Painted::of(
            Line::from(vec![
                Span::styled("a", plain),
                Span::styled("b", plain),
                Span::styled("cd", plain.add_modifier(Modifier::BOLD)),
            ]),
            4,
            1,
        );

        let runs = painted.row(0);

        assert_eq!(runs.len(), 2, "{runs:?}");
        assert_eq!(runs[0].said, "ab", "one style, one run: {runs:?}");
        assert_eq!(runs[1].said, "cd", "{runs:?}");
        assert!(
            !runs[0].style.add_modifier.contains(Modifier::BOLD),
            "{runs:?}"
        );
        assert!(
            runs[1].style.add_modifier.contains(Modifier::BOLD),
            "{runs:?}"
        );
    }

    /// A hyperlink is written into a cell's symbol because it cannot be
    /// written into a span. The reader sees the words and clicks them; the
    /// escape bytes are no more on screen than a colour is.
    #[test]
    fn a_hyperlink_is_not_among_the_words() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 5, 1));
        buffer
            .cell_mut((0, 0))
            .expect("a cell to write")
            .set_symbol("\x1b]8;;https://forge.invalid/orbital/atlas\x1b\\⇢ #12\x1b]8;;\x1b\\")
            .set_diff_option(CellDiffOption::ForcedWidth(
                NonZeroU16::new(5).expect("five columns"),
            ));

        assert_eq!(Painted::read(&buffer).rows(), vec!["⇢ #12"]);
    }

    /// The columns behind a forced width are the ones the diff skips, so the
    /// terminal never hears what they hold. Reading them as well would say
    /// the linked words twice.
    #[test]
    fn the_columns_a_forced_width_covers_are_not_read() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 4, 1));
        Line::from("beacon").render(Rect::new(0, 0, 4, 1), &mut buffer);
        buffer
            .cell_mut((0, 0))
            .expect("a cell to write")
            .set_diff_option(CellDiffOption::ForcedWidth(
                NonZeroU16::new(3).expect("three columns"),
            ));

        assert_eq!(Painted::read(&buffer).rows(), vec!["bc"]);
    }
}
