//! The rule the forest stops at, and as much of what the pane beneath it
//! last said as there is room for.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::Frame;

use crate::config::Background;
use crate::view::fitted::{columns, indent, Fitted};
use crate::view::palette;
use crate::view::phrase;
use crate::view::sgr;
use crate::view::tail::Tail;

use super::sentence;

/// What the rule above the tail is drawn from.
const RULE: char = '─';

/// The band, and the background it is drawn on.
///
/// The one part of the frame the reader's background decides, which is why
/// the background is carried here and nowhere else: what tells `bdi`'s own
/// rows from the pane's is a treatment the terminal resolves against the
/// background, and every other tone on the screen is either the reader's
/// own or `bd`'s.
pub struct Band<'a> {
    pub tail: &'a Tail,
    pub background: Background,
}

/// Draw the tail into the band `regions` reserved for it: a rule naming the
/// pane, and as much of what that pane last wrote as fits beneath it.
///
/// The newest lines are the ones kept. A pane's last line is what it is
/// doing now, and a tail that dropped it to keep older ones would be
/// answering yesterday's question.
pub fn draw_tail(frame: &mut Frame, area: Rect, band: Band<'_>) {
    let Band { tail, background } = band;
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
            frame.render_widget(rule(Some(&pane.id), area.width as usize), row(0));
            let styled = sgr::lines(lines);
            for (n, said) in styled
                .into_iter()
                .skip(lines.len().saturating_sub(room))
                .enumerate()
            {
                frame.render_widget(as_the_pane_drew_it(said), row(n + 1));
            }
        }
        Tail::Reading { pane } => {
            frame.render_widget(rule(Some(&pane.id), area.width as usize), row(0));
            if room > 0 {
                frame.render_widget(
                    sentence(
                        &indent(),
                        phrase::pane_being_read().to_string(),
                        palette::voice(background),
                    ),
                    row(1),
                );
            }
        }
        Tail::Silent(why) => {
            frame.render_widget(rule(None, area.width as usize), row(0));
            if room > 0 {
                frame.render_widget(
                    sentence(&indent(), (*why).to_string(), palette::voice(background)),
                    row(1),
                );
            }
        }
    }
}

/// One of the pane's own rows, indented like the phrases and cut to the
/// band's width. The row keeps every colour and attribute the pane gave it,
/// which is what tells it from a row `bdi` wrote.
fn as_the_pane_drew_it(said: Line<'static>) -> Fitted {
    let mut spans = vec![Span::raw(indent())];
    spans.extend(said.spans);
    Fitted::new(spans, Vec::new(), Vec::new())
}

/// The rule between the forest and the tail, with the pane the tail is
/// showing named in the middle of it.
///
/// A tail with no pane draws the rule alone. The band is reserved either
/// way, and the rule is what says where the forest stopped.
fn rule(pane: Option<&str>, width: usize) -> Line<'static> {
    let drawn = |n: usize| Span::styled(RULE.to_string().repeat(n), palette::QUIET);

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
    use crate::model::types::testing::key;
    use pretty_assertions::assert_eq;
    use ratatui::style::Color;
    use ratatui::style::Modifier;
    use ratatui::style::Style;

    use crate::view::painted::{Painted, Run};
    use crate::view::phrase;

    /// The tail drawn into a band that starts partway down the screen, which
    /// is the only place it ever is.
    fn tail_frame(tail: &Tail, width: u16, height: u16, at: u16) -> Painted {
        tail_frame_on(Background::Dark, tail, width, height, at)
    }

    /// The same, for a reader who has said which background they are on.
    fn tail_frame_on(
        background: Background,
        tail: &Tail,
        width: u16,
        height: u16,
        at: u16,
    ) -> Painted {
        Painted::drawn_by(width, height, |frame| {
            draw_tail(
                frame,
                Rect::new(0, at, width, height - at),
                Band { tail, background },
            );
        })
    }

    fn tailing(pane: &str, lines: &[&str]) -> Tail {
        Tail::Pane {
            pane: key(pane),
            lines: lines.iter().map(|line| (*line).to_string()).collect(),
        }
    }

    #[test]
    fn the_tail_fills_the_band_it_is_given_and_no_row_above_it() {
        let tail = tailing("wCM:p9", &["rebuilt .#thinkpad, generation 541"]);

        assert_eq!(
            tail_frame(&tail, 44, 5, 2).rows(),
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
            tail_frame(&tail, 20, 3, 0).rows(),
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
            tail_frame(&Tail::Silent(phrase::no_agent_to_tail()), 40, 3, 0).rows(),
            vec![
                "────────────────────────────────────────",
                "  no pane · nobody is working this bead ",
                "                                        ",
            ]
        );
    }

    /// The band between the selection landing on a pane and herdr saying
    /// what is on it. The rule names the pane already, so the row beneath it
    /// says only that `bdi` is waiting — a blank one would read as a pane
    /// sitting quiet.
    #[test]
    fn a_pane_not_yet_read_says_it_is_being_read() {
        assert_eq!(
            tail_frame(
                &Tail::Reading {
                    pane: key("wCM:p9")
                },
                40,
                3,
                0
            )
            .rows(),
            vec![
                "──────────────── wCM:p9 ────────────────",
                "  reading that pane                     ",
                "                                        ",
            ]
        );
    }

    /// Wide enough for the longest phrase the band says, which a narrower one
    /// cuts an ellipsis into and so splits the run holding it.
    const BAND: u16 = 60;

    /// What is left of a style when the reader has `NO_COLOR` set: crossterm
    /// writes nothing at all for `SetColors` and has no such guard on
    /// `SetAttribute`, so the modifiers arrive and no colour does.
    fn as_a_reader_with_no_colour_sees_it(style: Style) -> Style {
        Style::new()
            .add_modifier(style.add_modifier)
            .remove_modifier(style.sub_modifier)
    }

    /// The style the words are drawn in, from the row they are on.
    fn drawn_style(row: &[Run], words: &str) -> Style {
        row.iter()
            .find(|run| run.said.contains(words))
            .unwrap_or_else(|| panic!("no run saying {words:?} in {row:?}"))
            .style
    }

    /// A pane line the pane chose nothing for.
    fn a_plain_pane_line(background: Background, said: &str) -> Style {
        drawn_style(
            &tail_frame_on(background, &tailing("w:p1", &[said]), BAND, 2, 0).row(1),
            said,
        )
    }

    /// The two rows the band writes in `bdi`'s own voice, with the words
    /// each says.
    fn every_row_bdi_says_itself() -> [(Tail, &'static str); 2] {
        [
            (
                Tail::Reading { pane: key("w:p1") },
                phrase::pane_being_read(),
            ),
            (
                Tail::Silent(phrase::no_bead_to_tail()),
                phrase::no_bead_to_tail(),
            ),
        ]
    }

    /// On a dark background, what `bdi` says in the band is dimmer than
    /// what the pane says, which is what stops a reader taking `bdi`'s own
    /// words for the pane's. Nothing in the symbols says which of the two a
    /// row is.
    ///
    /// Dark rather than either background, because dim is the treatment a
    /// light one inverts. This is the tone an undeclared reader gets.
    #[test]
    fn what_bdi_says_on_a_dark_background_is_drawn_dimmer_than_the_pane() {
        let waiting = tail_frame(&Tail::Reading { pane: key("w:p1") }, 40, 2, 0).row(1);
        assert!(
            waiting.iter().any(|run| {
                run.said.contains(phrase::pane_being_read())
                    && run.style.fg == Some(Color::Reset)
                    && run.style.add_modifier.contains(Modifier::DIM)
            }),
            "the row saying the pane is being read: {waiting:?}"
        );

        let said = tail_frame(&tailing("w:p1", &["rebuilt .#thinkpad"]), 40, 2, 0).row(1);
        assert!(
            said.iter().any(|run| {
                run.said.contains("rebuilt .#thinkpad") && run.style.fg == Some(Color::Reset)
            }),
            "the pane's own line: {said:?}"
        );
    }

    /// `design.md:1570` makes the band the one place on the screen where
    /// nothing but the tone says whose words a row is — none of the glyphs
    /// and symbols carrying that distinction everywhere else are in it. So
    /// the tone has to be a channel a reader with `NO_COLOR` set still has,
    /// and both sides of the comparison are read with the colour taken off.
    ///
    /// The pane is compared plain because that is the only line it can draw
    /// that `bdi` has to stand apart from: one in the pane's own colours is
    /// already told from `bdi` by those, and one the pane drew dim is the
    /// pane choosing `bdi`'s tone, which no band can hold against it.
    /// On the dark background, and on that one only. Dim over the default
    /// foreground is the composition a light background inverts, so a light
    /// reader is answered at colour 8 and has no attribute to be left with
    /// — their band falls back to the rule and to which state it is in,
    /// which is what every reader had before the attribute was chosen. The
    /// dark background is what an undeclared reader gets, so this is the
    /// case that covers most of them.
    #[test]
    fn what_tells_bdi_from_the_pane_on_a_dark_background_is_not_colour() {
        let pane = a_plain_pane_line(Background::Dark, "rebuilt .#thinkpad");

        for (voice, said) in every_row_bdi_says_itself() {
            let spoken = drawn_style(&tail_frame(&voice, BAND, 2, 0).row(1), said);

            assert_ne!(
                as_a_reader_with_no_colour_sees_it(spoken),
                as_a_reader_with_no_colour_sees_it(pane),
                "with no colour, {said:?} is drawn as the pane's own line is"
            );
        }
    }

    /// On both grounds, `bdi`'s own words are told from the pane's by
    /// something. Which channel carries it is the background's to decide;
    /// that it is carried is not, because `design.md`'s account of the band
    /// makes the tone the whole of what says whose words a row is.
    #[test]
    fn what_bdi_says_is_told_from_the_pane_on_either_background() {
        for background in [Background::Dark, Background::Light] {
            let pane = a_plain_pane_line(background, "rebuilt .#thinkpad");

            for (voice, said) in every_row_bdi_says_itself() {
                let spoken =
                    drawn_style(&tail_frame_on(background, &voice, BAND, 2, 0).row(1), said);

                assert_ne!(spoken, pane, "on a {background:?} background, {said:?}");
            }
        }
    }

    /// And the reader's declaration reaches the band, which is the whole of
    /// what the `[theme]` key buys. A key nothing downstream read would
    /// leave a light reader who found it and set it exactly where they
    /// started, with one more reason to think `bdi` had heard them.
    #[test]
    fn the_declared_background_reaches_the_band() {
        for (voice, said) in every_row_bdi_says_itself() {
            let undeclared = drawn_style(&tail_frame(&voice, BAND, 2, 0).row(1), said);
            let light = drawn_style(
                &tail_frame_on(Background::Light, &voice, BAND, 2, 0).row(1),
                said,
            );

            assert_ne!(light, undeclared, "{said:?} is drawn on neither background");
        }
    }

    /// What herdr wrote for a real pane on this machine, read the way the
    /// tail reads it.
    fn a_captured_pane() -> Tail {
        use crate::collect::agents::Agents;
        use crate::collect::herdr::Herdr;
        use crate::collect::run::testing::FakeRunner;
        use crate::view::tail;

        const ARGV: &str =
            "herdr --session default agent read wDV:p1 --source visible --lines 6 --format ansi";
        let runner = FakeRunner::default().with(
            ARGV,
            include_str!("../../../tests/fixtures/herdr_agent_read_ansi.txt"),
        );
        tail::read(key("wDV:p1"), Herdr::new(&runner).read(&key("wDV:p1"), 6))
    }

    /// The bead: the band draws the pane's own colour and attributes, read
    /// off the styling herdr sent, and none of that styling reaches the
    /// screen as text.
    #[test]
    fn the_tail_draws_the_panes_own_colour_and_attributes() {
        let painted = tail_frame(&a_captured_pane(), 120, 7, 0);

        let host = painted.row(4);
        assert!(
            host.iter().any(|run| {
                run.said == "thinkpad"
                    && run.style.fg == Some(Color::Rgb(255, 121, 198))
                    && run.style.add_modifier.contains(Modifier::BOLD)
            }),
            "the host, bold and pink as the pane drew it: {host:?}"
        );
        assert!(
            host.iter()
                .any(|run| run.said == "main" && run.style.fg == Some(Color::Indexed(6))),
            "the branch, in the pane's 256-colour cyan: {host:?}"
        );
        assert!(
            painted.rows().iter().all(|row| !row.contains('\x1b')),
            "no escape reaches the screen as text: {:?}",
            painted.rows()
        );
    }

    #[test]
    fn a_pane_line_too_wide_for_the_screen_is_cut_rather_than_wrapped() {
        let tail = tailing("w:p1", &["a line with a great deal more to say than this"]);

        assert_eq!(
            tail_frame(&tail, 20, 2, 0).rows(),
            vec!["─────── w:p1 ───────", "  a line with a gre…"]
        );
    }

    /// A pane named so widely there is no rule left to draw around it, and a
    /// band with no room under its rule. Neither draws outside itself.
    #[test]
    fn a_tail_with_barely_any_room_draws_what_it_can() {
        let tail = tailing("a-very-long-pane-identifier", &["never seen"]);

        assert_eq!(tail_frame(&tail, 10, 1, 0).rows(), vec!["──────────"]);
        assert_eq!(
            tail_frame(&Tail::Silent(phrase::no_bead_to_tail()), 10, 1, 0).rows(),
            vec!["──────────"]
        );
    }

    /// A band one row high has room for its rule and nothing else, and the
    /// row beneath it is not the band's to write in. At four rows the screen
    /// gives the tail exactly one, and puts the key hints on the row under
    /// it: a phrase written there would be drawn over them.
    #[test]
    fn a_band_one_row_high_writes_nothing_under_its_rule() {
        for tail in [
            Tail::Reading { pane: key("w:p1") },
            Tail::Silent(phrase::no_bead_to_tail()),
        ] {
            let screen = Painted::drawn_by(20, 2, |frame| {
                draw_tail(
                    frame,
                    Rect::new(0, 0, 20, 1),
                    Band {
                        tail: &tail,
                        background: Background::Dark,
                    },
                );
            });

            assert_eq!(
                screen.rows()[1],
                " ".repeat(20),
                "the row under the band, for {tail:?}"
            );
        }
    }

    #[test]
    fn a_band_with_no_rows_draws_nothing() {
        let screen = Painted::drawn_by(20, 1, |frame| {
            draw_tail(
                frame,
                Rect::new(0, 0, 20, 0),
                Band {
                    tail: &tailing("w:p1", &["x"]),
                    background: Background::Dark,
                },
            );
        });

        assert_eq!(screen.rows()[0], " ".repeat(20));
    }
}
