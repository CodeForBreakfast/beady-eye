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
use ratatui::buffer::Buffer;
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
                    for x in buffer.area.left()..buffer.area.right() {
                        let cell = &buffer[(x, y)];
                        match runs.last_mut() {
                            Some(run) if run.style == cell.style() => {
                                run.said.push_str(cell.symbol())
                            }
                            _ => runs.push(Run::new(cell.symbol(), cell.style())),
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

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

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
}
