//! What a pane's rows look like, read off the SGR sequences herdr writes
//! into them.
//!
//! A fold and nothing more: every `ESC [ … m` moves the style the text after
//! it is drawn in, and the text between two of them is one span. It is not a
//! terminal emulator — a CSI that is not an SGR is dropped whole, because it
//! moves nothing on a screen `bdi` is not keeping, and an SGR parameter it
//! does not know is skipped rather than refused.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// A pane's rows as lines, styled the way the pane drew them.
///
/// Style carries from one row into the next, as it does on the screen the
/// rows were read off.
pub fn lines(rows: &[String]) -> Vec<Line<'static>> {
    let mut drawn_in = Sgr::default();
    rows.iter().map(|row| line(row, &mut drawn_in)).collect()
}

/// The escape that opens a control sequence.
const ESC: char = '\x1b';

/// The parameter that ends a control sequence, and the one that names a
/// select-graphic-rendition among them.
const FINAL_BYTES: std::ops::RangeInclusive<char> = '\x40'..='\x7e';
const SGR: char = 'm';

/// One row, cut into spans wherever the style moves.
fn line(row: &str, drawn_in: &mut Sgr) -> Line<'static> {
    let mut spans = Vec::new();
    let mut said = String::new();
    let mut rest = row;

    while let Some(at) = rest.find(ESC) {
        said.push_str(&rest[..at]);
        let Some(sequence) = rest[at + ESC.len_utf8()..].strip_prefix('[') else {
            rest = &rest[at + ESC.len_utf8()..];
            continue;
        };
        let Some(end) = sequence.find(|c| FINAL_BYTES.contains(&c)) else {
            rest = "";
            break;
        };
        let (parameters, after) = sequence.split_at(end);
        if after.starts_with(SGR) {
            spans.extend(span(&mut said, drawn_in));
            drawn_in.apply(parameters);
        }
        rest = &after[SGR.len_utf8()..];
    }
    said.push_str(rest);
    spans.extend(span(&mut said, drawn_in));

    Line::from(spans)
}

/// The text gathered so far as one span, where there is any, and the
/// gathering starts again.
fn span(said: &mut String, drawn_in: &Sgr) -> Option<Span<'static>> {
    if said.is_empty() {
        return None;
    }
    Some(Span::styled(std::mem::take(said), drawn_in.style()))
}

/// The rendition text is being drawn in: what SGR parameters set, as the
/// three things they set.
#[derive(Default)]
struct Sgr {
    fg: Option<Color>,
    bg: Option<Color>,
    attributes: Modifier,
}

impl Sgr {
    fn style(&self) -> Style {
        let mut style = Style::new().add_modifier(self.attributes);
        if let Some(fg) = self.fg {
            style = style.fg(fg);
        }
        if let Some(bg) = self.bg {
            style = style.bg(bg);
        }
        style
    }

    /// Fold one sequence's parameters in. A parameter left out is `0`, and
    /// one that is not a number this knows is skipped, with the ones after
    /// it still applying.
    fn apply(&mut self, parameters: &str) {
        let mut parameters = parameters.split(';').map(|parameter| {
            if parameter.is_empty() {
                Ok(0)
            } else {
                parameter.parse::<u8>()
            }
        });
        while let Some(parameter) = parameters.next() {
            let Ok(parameter) = parameter else {
                continue;
            };
            match parameter {
                0 => *self = Self::default(),
                1 => self.attributes |= Modifier::BOLD,
                2 => self.attributes |= Modifier::DIM,
                3 => self.attributes |= Modifier::ITALIC,
                4 => self.attributes |= Modifier::UNDERLINED,
                5 => self.attributes |= Modifier::SLOW_BLINK,
                6 => self.attributes |= Modifier::RAPID_BLINK,
                7 => self.attributes |= Modifier::REVERSED,
                8 => self.attributes |= Modifier::HIDDEN,
                9 => self.attributes |= Modifier::CROSSED_OUT,
                22 => self.attributes -= Modifier::BOLD | Modifier::DIM,
                23 => self.attributes -= Modifier::ITALIC,
                24 => self.attributes -= Modifier::UNDERLINED,
                25 => self.attributes -= Modifier::SLOW_BLINK | Modifier::RAPID_BLINK,
                27 => self.attributes -= Modifier::REVERSED,
                28 => self.attributes -= Modifier::HIDDEN,
                29 => self.attributes -= Modifier::CROSSED_OUT,
                30..=37 => self.fg = Some(named(parameter - 30)),
                38 => self.fg = extended(&mut parameters),
                39 => self.fg = None,
                40..=47 => self.bg = Some(named(parameter - 40)),
                48 => self.bg = extended(&mut parameters),
                49 => self.bg = None,
                90..=97 => self.fg = Some(bright(parameter - 90)),
                100..=107 => self.bg = Some(bright(parameter - 100)),
                _ => {}
            }
        }
    }
}

/// The colour a `38` or `48` goes on to name: `5;n` from the 256-colour
/// table, or `2;r;g;b`. Anything else names none, and the parameters it
/// took are spent.
fn extended(
    parameters: &mut impl Iterator<Item = Result<u8, std::num::ParseIntError>>,
) -> Option<Color> {
    let mut next = || parameters.next()?.ok();
    match next()? {
        5 => Some(Color::Indexed(next()?)),
        2 => Some(Color::Rgb(next()?, next()?, next()?)),
        _ => None,
    }
}

/// The eight named colours, in the order the codes count them.
fn named(offset: u8) -> Color {
    match offset {
        0 => Color::Black,
        1 => Color::Red,
        2 => Color::Green,
        3 => Color::Yellow,
        4 => Color::Blue,
        5 => Color::Magenta,
        6 => Color::Cyan,
        _ => Color::Gray,
    }
}

/// The same eight, bright.
fn bright(offset: u8) -> Color {
    match offset {
        0 => Color::DarkGray,
        1 => Color::LightRed,
        2 => Color::LightGreen,
        3 => Color::LightYellow,
        4 => Color::LightBlue,
        5 => Color::LightMagenta,
        6 => Color::LightCyan,
        _ => Color::White,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::text::Span;

    fn rows(rows: &[&str]) -> Vec<String> {
        rows.iter().map(|row| (*row).to_string()).collect()
    }

    fn line(spans: Vec<Span<'static>>) -> Vec<Line<'static>> {
        vec![Line::from(spans)]
    }

    #[test]
    fn text_between_a_colour_and_a_reset_is_one_span_in_that_colour() {
        assert_eq!(
            lines(&rows(&["\x1b[38;2;255;193;7mauto mode on\x1b[0m · plain"])),
            line(vec![
                Span::styled("auto mode on", Style::new().fg(Color::Rgb(255, 193, 7))),
                Span::raw(" · plain"),
            ])
        );
    }

    #[test]
    fn an_attribute_and_a_colour_set_in_two_sequences_both_hold() {
        assert_eq!(
            lines(&rows(&["\x1b[1m\x1b[38;5;3mbold amber"])),
            line(vec![Span::styled(
                "bold amber",
                Style::new()
                    .fg(Color::Indexed(3))
                    .add_modifier(Modifier::BOLD)
            )])
        );
    }

    #[test]
    fn several_parameters_in_one_sequence_all_apply() {
        assert_eq!(
            lines(&rows(&["\x1b[2;3;48;2;1;2;3mfaint slanted on a ground"])),
            line(vec![Span::styled(
                "faint slanted on a ground",
                Style::new()
                    .bg(Color::Rgb(1, 2, 3))
                    .add_modifier(Modifier::DIM | Modifier::ITALIC)
            )])
        );
    }

    #[test]
    fn a_reset_clears_everything_set_before_it() {
        assert_eq!(
            lines(&rows(&["\x1b[1;31;44mloud\x1b[0m quiet"])),
            line(vec![
                Span::styled(
                    "loud",
                    Style::new()
                        .fg(Color::Red)
                        .bg(Color::Blue)
                        .add_modifier(Modifier::BOLD)
                ),
                Span::raw(" quiet"),
            ])
        );
    }

    /// An attribute is turned off on its own, and the others stay.
    #[test]
    fn an_attribute_turned_off_leaves_the_rest_standing() {
        assert_eq!(
            lines(&rows(&[
                "\x1b[1;4;31mboth\x1b[22mstill underlined\x1b[39mno colour"
            ])),
            line(vec![
                Span::styled(
                    "both",
                    Style::new()
                        .fg(Color::Red)
                        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                ),
                Span::styled(
                    "still underlined",
                    Style::new()
                        .fg(Color::Red)
                        .add_modifier(Modifier::UNDERLINED)
                ),
                Span::styled("no colour", Style::new().add_modifier(Modifier::UNDERLINED)),
            ])
        );
    }

    /// The sixteen named colours, on the foreground and the ground, spelt
    /// the way ratatui spells them.
    #[test]
    fn the_named_colours_are_read_as_ratatui_names_them() {
        let named = [
            (30, Color::Black),
            (31, Color::Red),
            (32, Color::Green),
            (33, Color::Yellow),
            (34, Color::Blue),
            (35, Color::Magenta),
            (36, Color::Cyan),
            (37, Color::Gray),
            (90, Color::DarkGray),
            (91, Color::LightRed),
            (92, Color::LightGreen),
            (93, Color::LightYellow),
            (94, Color::LightBlue),
            (95, Color::LightMagenta),
            (96, Color::LightCyan),
            (97, Color::White),
        ];
        for (code, colour) in named {
            assert_eq!(
                lines(&rows(&[&format!("\x1b[{code}mfg\x1b[{}mbg", code + 10)])),
                line(vec![
                    Span::styled("fg", Style::new().fg(colour)),
                    Span::styled("bg", Style::new().fg(colour).bg(colour)),
                ]),
                "code {code}"
            );
        }
    }

    /// A parameter left out is `0`, so the bare `ESC[m` is the reset it is
    /// everywhere else, and an empty one among others is too.
    /// Every attribute, set by its own parameter and taken off again by the
    /// one that turns it off.
    #[test]
    fn each_attribute_is_set_by_its_parameter_and_taken_off_by_its_own() {
        let attributes = [
            (1, Modifier::BOLD, 22),
            (2, Modifier::DIM, 22),
            (3, Modifier::ITALIC, 23),
            (4, Modifier::UNDERLINED, 24),
            (5, Modifier::SLOW_BLINK, 25),
            (6, Modifier::RAPID_BLINK, 25),
            (7, Modifier::REVERSED, 27),
            (8, Modifier::HIDDEN, 28),
            (9, Modifier::CROSSED_OUT, 29),
        ];
        for (on, attribute, off) in attributes {
            assert_eq!(
                lines(&rows(&[&format!("\x1b[{on}mon\x1b[{off}moff")])),
                line(vec![
                    Span::styled("on", Style::new().add_modifier(attribute)),
                    Span::raw("off"),
                ]),
                "attribute {on}, turned off by {off}"
            );
        }
    }

    #[test]
    fn the_ground_is_given_back_on_its_own_leaving_the_foreground() {
        assert_eq!(
            lines(&rows(&["\x1b[31;44mon a ground\x1b[49mon none"])),
            line(vec![
                Span::styled("on a ground", Style::new().fg(Color::Red).bg(Color::Blue)),
                Span::styled("on none", Style::new().fg(Color::Red)),
            ])
        );
    }

    #[test]
    fn a_parameter_left_out_is_a_reset() {
        assert_eq!(
            lines(&rows(&["\x1b[1mbold\x1b[mplain\x1b[31;mred then plain"])),
            line(vec![
                Span::styled("bold", Style::new().add_modifier(Modifier::BOLD)),
                Span::raw("plain"),
                Span::raw("red then plain"),
            ])
        );
    }

    /// A parameter this fold does not know is skipped, and the ones beside
    /// it are not.
    #[test]
    fn an_unknown_parameter_is_skipped_and_the_rest_of_the_sequence_applies() {
        assert_eq!(
            lines(&rows(&["\x1b[53;1;38:2::9:9:9mframed bold"])),
            line(vec![Span::styled(
                "framed bold",
                Style::new().add_modifier(Modifier::BOLD)
            )])
        );
    }

    /// A CSI that is not an SGR moves nothing here and leaves no bytes in
    /// the text.
    #[test]
    fn a_control_sequence_that_is_not_an_sgr_is_dropped_whole() {
        assert_eq!(
            lines(&rows(&["one\x1b[2K\x1b[3;7Htwo\x1b[?25l"])),
            line(vec![Span::raw("onetwo")])
        );
    }

    /// A sequence the row ends inside is dropped with the rest of the row,
    /// and a bare escape is not text.
    #[test]
    fn an_unfinished_sequence_is_dropped_rather_than_drawn() {
        assert_eq!(
            lines(&rows(&["kept\x1b[38;2;1", "\x1bnext"])),
            vec![
                Line::from(vec![Span::raw("kept")]),
                Line::from(vec![Span::raw("next")]),
            ]
        );
    }

    #[test]
    fn a_style_left_on_at_the_end_of_a_row_carries_into_the_next() {
        assert_eq!(
            lines(&rows(&["\x1b[1mbold", "still bold", ""])),
            vec![
                Line::from(vec![Span::styled(
                    "bold",
                    Style::new().add_modifier(Modifier::BOLD)
                )]),
                Line::from(vec![Span::styled(
                    "still bold",
                    Style::new().add_modifier(Modifier::BOLD)
                )]),
                Line::from(Vec::<Span<'static>>::new()),
            ]
        );
    }

    /// Nothing in the row is an escape, so the row is one span as written.
    #[test]
    fn a_plain_row_is_one_plain_span() {
        assert_eq!(
            lines(&rows(&["rebuilt .#thinkpad, generation 541"])),
            line(vec![Span::raw("rebuilt .#thinkpad, generation 541")])
        );
    }
}
