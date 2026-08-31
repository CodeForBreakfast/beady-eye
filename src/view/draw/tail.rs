//! The rule the forest stops at, and as much of what the pane beneath it
//! last said as there is room for.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::Frame;

use crate::view::fitted::{columns, indent};
use crate::view::tail::Tail;

use super::sentence;

/// What the rule above the tail is drawn from.
const RULE: char = '─';

/// Draw the tail into the band `regions` reserved for it: a rule naming the
/// pane, and as much of what that pane last wrote as fits beneath it.
///
/// The newest lines are the ones kept. A pane's last line is what it is
/// doing now, and a tail that dropped it to keep older ones would be
/// answering yesterday's question.
pub fn draw_tail(frame: &mut Frame, area: Rect, tail: &Tail) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let row = |n: usize| Rect {
        y: area.y + n as u16,
        height: 1,
        ..area
    };
    let room = area.height as usize - 1;

    match tail {
        Tail::Pane { pane, lines } => {
            frame.render_widget(rule(Some(pane), area.width as usize), row(0));
            for (n, said) in lines
                .iter()
                .skip(lines.len().saturating_sub(room))
                .enumerate()
            {
                frame.render_widget(sentence(&indent(), said.clone(), Color::Reset), row(n + 1));
            }
        }
        Tail::Silent(why) => {
            frame.render_widget(rule(None, area.width as usize), row(0));
            if room > 0 {
                frame.render_widget(
                    sentence(&indent(), (*why).to_string(), Color::DarkGray),
                    row(1),
                );
            }
        }
    }
}

/// The rule between the forest and the tail, with the pane the tail is
/// showing named in the middle of it.
///
/// A tail with no pane draws the rule alone. The band is reserved either
/// way, and the rule is what says where the forest stopped.
fn rule(pane: Option<&str>, width: usize) -> Line<'static> {
    let drawn =
        |n: usize| Span::styled(RULE.to_string().repeat(n), Style::new().fg(Color::DarkGray));

    let named = match pane {
        Some(pane) => format!(" {pane} "),
        None => return Line::from(drawn(width)),
    };
    let taken = columns(&[Span::raw(named.clone())]);
    if taken >= width {
        return Line::from(drawn(width));
    }

    let left = (width - taken) / 2;
    Line::from(vec![
        drawn(left),
        Span::raw(named),
        drawn(width - taken - left),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use crate::view::phrase;

    /// The tail drawn into a band that starts partway down the screen, which
    /// is the only place it ever is.
    fn tail_frame(tail: &Tail, width: u16, height: u16, at: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test backend");
        terminal
            .draw(|frame| draw_tail(frame, Rect::new(0, at, width, height - at), tail))
            .expect("a draw into memory");
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
            .collect()
    }

    fn tailing(pane: &str, lines: &[&str]) -> Tail {
        Tail::Pane {
            pane: pane.into(),
            lines: lines.iter().map(|line| (*line).to_string()).collect(),
        }
    }

    #[test]
    fn the_tail_fills_the_band_it_is_given_and_no_row_above_it() {
        let tail = tailing("wCM:p9", &["rebuilt .#thinkpad, generation 541"]);

        assert_eq!(
            tail_frame(&tail, 44, 5, 2),
            vec![
                "                                            ",
                "                                            ",
                "────────────────── wCM:p9 ──────────────────",
                "  rebuilt .#thinkpad, generation 541        ",
                "                                            ",
            ]
        );
    }

    /// A pane writes more than the band holds. What it wrote last is what it
    /// is doing now.
    #[test]
    fn a_pane_with_more_to_say_than_there_is_room_shows_the_newest_of_it() {
        let tail = tailing("w:p1", &["one", "two", "three", "four"]);

        assert_eq!(
            tail_frame(&tail, 20, 3, 0),
            vec![
                "─────── w:p1 ───────",
                "  three             ",
                "  four              "
            ]
        );
    }

    #[test]
    fn a_tail_with_no_pane_says_why_rather_than_leaving_the_band_blank() {
        assert_eq!(
            tail_frame(&Tail::Silent(phrase::no_agent_to_tail()), 40, 3, 0),
            vec![
                "────────────────────────────────────────",
                "  no pane · nobody is working this bead ",
                "                                        ",
            ]
        );
    }

    #[test]
    fn a_pane_line_too_wide_for_the_screen_is_cut_rather_than_wrapped() {
        let tail = tailing("w:p1", &["a line with a great deal more to say than this"]);

        assert_eq!(
            tail_frame(&tail, 20, 2, 0),
            vec!["─────── w:p1 ───────", "  a line with a gre…"]
        );
    }

    /// A pane named so widely there is no rule left to draw around it, and a
    /// band with no room under its rule. Neither draws outside itself.
    #[test]
    fn a_tail_with_barely_any_room_draws_what_it_can() {
        let tail = tailing("a-very-long-pane-identifier", &["never seen"]);

        assert_eq!(tail_frame(&tail, 10, 1, 0), vec!["──────────"]);
        assert_eq!(
            tail_frame(&Tail::Silent(phrase::no_bead_to_tail()), 10, 1, 0),
            vec!["──────────"]
        );
    }

    #[test]
    fn a_band_with_no_rows_draws_nothing() {
        let mut terminal = Terminal::new(TestBackend::new(20, 1)).expect("a test backend");
        terminal
            .draw(|frame| draw_tail(frame, Rect::new(0, 0, 20, 0), &tailing("w:p1", &["x"])))
            .expect("a draw into memory");

        let buffer = terminal.backend().buffer();
        assert_eq!(
            (0..20).map(|x| buffer[(x, 0)].symbol()).collect::<String>(),
            " ".repeat(20)
        );
    }
}
