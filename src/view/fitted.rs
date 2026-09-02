//! One screen row fitted to the width it is given, and the cutting that fits it.
//!
//! A general widget: three blocks, one row, cut rather than wrapped, with a
//! stated yield order. It knows nothing of what it is drawing.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

/// The mark left where a line ran out of width, so a cut line reads as cut
/// rather than as one that had nothing more to say.
pub(crate) const CUT: char = '…';

/// The blank columns that keep two blocks from reading as one.
pub(crate) const GAP: usize = 2;

pub(crate) fn indent() -> String {
    " ".repeat(GAP)
}

/// One screen row, fitted to the width it is given by cutting rather than
/// wrapping. One line is one row: the selection is an index into the forest's
/// lines, so a line that wrapped would put that index and the screen row out
/// of step and the selection would leave the viewport with nothing failing.
///
/// The three blocks yield in a stated order. The `title` goes first, because a
/// reader who loses it still knows which row this is. The `state` goes next,
/// cut from its own end so its opening words survive. The `identity` yields
/// only when there is nothing else left to give.
pub struct Fitted {
    identity: Vec<Span<'static>>,
    title: Vec<Span<'static>>,
    state: Vec<Span<'static>>,
    whole: Style,
    title_or_nothing: bool,
    state_or_nothing: bool,
}

impl Fitted {
    pub(crate) fn new(
        identity: Vec<Span<'static>>,
        title: Vec<Span<'static>>,
        state: Vec<Span<'static>>,
    ) -> Self {
        Self {
            identity,
            title,
            state,
            whole: Style::new(),
            title_or_nothing: false,
            state_or_nothing: false,
        }
    }

    /// Give the title up whole rather than cut it.
    ///
    /// For a title that says nothing in part. An age cut to `30s a…` names no
    /// duration, so the columns it kept are spent saying that the project has
    /// been read — which its rows already said.
    #[must_use]
    pub(crate) fn title_or_nothing(mut self) -> Self {
        self.title_or_nothing = true;
        self
    }

    /// Give the state up whole rather than cut it.
    ///
    /// For a state that says nothing in part. A key row cut to `Ent…` names
    /// no key, so the columns it kept are spent saying that a key row exists
    /// — which the reader could already see.
    #[must_use]
    pub(crate) fn state_or_nothing(mut self) -> Self {
        self.state_or_nothing = true;
        self
    }

    /// The row under the cursor, drawn so the eye finds it without reading it.
    #[must_use]
    pub fn selected(mut self) -> Self {
        self.whole = self.whole.add_modifier(Modifier::REVERSED);
        self
    }

    /// How live this line is, drawn under every span that did not ask for a
    /// colour of its own. A span that named one keeps it: the status glyph,
    /// the agent and the anomalies say what they say at any brightness.
    #[must_use]
    pub(crate) fn toned(mut self, tone: Style) -> Self {
        self.whole = tone.patch(self.whole);
        self
    }
}

impl Widget for Fitted {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let area = Rect { height: 1, ..area };
        let width = area.width as usize;

        let identity = columns(&self.identity);
        let spans = if identity >= width {
            cut_to(self.identity, width)
        } else {
            let mut room = width - identity;
            let state = fit(self.state, room.saturating_sub(GAP), self.state_or_nothing);
            room -= columns(&state) + if state.is_empty() { 0 } else { GAP };
            let title = fit(self.title, room.saturating_sub(GAP), self.title_or_nothing);

            let mut spans = self.identity;
            if !title.is_empty() {
                spans.push(Span::raw(" ".repeat(GAP)));
                spans.extend(title);
            }
            if !state.is_empty() {
                let pad = width.saturating_sub(columns(&spans) + columns(&state));
                spans.push(Span::raw(" ".repeat(pad)));
                spans.extend(state);
            }
            spans
        };

        Line::from(spans).style(self.whole).render(area, buf);
    }
}

/// What a run of spans takes up on screen, in columns rather than in bytes:
/// every glyph in this vocabulary is several bytes long, and a width counted
/// in bytes would put a cut inside one.
pub(crate) fn columns(spans: &[Span<'static>]) -> usize {
    spans.iter().map(Span::width).sum()
}

/// `spans` fitted into `limit` columns: cut with the cut marked, or, for a
/// block that says nothing in part, given up whole.
fn fit(spans: Vec<Span<'static>>, limit: usize, or_nothing: bool) -> Vec<Span<'static>> {
    if or_nothing && columns(&spans) > limit {
        return Vec::new();
    }
    cut_to(spans, limit)
}

/// `spans`, cut down to `limit` columns with the cut marked.
fn cut_to(spans: Vec<Span<'static>>, limit: usize) -> Vec<Span<'static>> {
    if columns(&spans) <= limit {
        return spans;
    }
    if limit == 0 {
        return Vec::new();
    }

    let room = limit - columns(&[Span::raw(CUT.to_string())]);
    let mut kept: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    for span in spans {
        let width = span.width();
        if used + width <= room {
            used += width;
            kept.push(span);
            continue;
        }
        let head = head_of(&span.content, room - used);
        if !head.is_empty() {
            kept.push(Span::styled(head, span.style));
        }
        break;
    }
    kept.push(Span::raw(CUT.to_string()));
    kept
}

/// As much of `text` as fits in `limit` columns, never splitting a glyph.
fn head_of(text: &str, limit: usize) -> String {
    let mut head = String::new();
    let mut used = 0;
    for glyph in text.chars() {
        let width = Span::raw(String::from(glyph)).width();
        if used + width > limit {
            break;
        }
        used += width;
        head.push(glyph);
    }
    head
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    use crate::view::painted::Painted;

    /// A row with something in all three blocks, so any painting at all shows.
    fn a_row() -> Fitted {
        Fitted::new(
            vec![Span::raw("orb-7")],
            vec![Span::raw("a title")],
            vec![Span::raw("open")],
        )
    }

    fn blank(width: usize, height: usize) -> Vec<String> {
        vec![" ".repeat(width); height]
    }

    /// One row of what a widget puts on screen.
    fn drawn(row: Fitted, width: u16) -> String {
        Painted::of(row, width, 1).rows().remove(0)
    }

    /// A title cut in half can still be worth its columns — half a bead's
    /// title names the bead. A title that cannot is drawn whole or not at
    /// all, and the columns go to the blocks that can use them.
    #[test]
    fn a_title_that_says_nothing_in_part_is_given_up_whole() {
        let row = || {
            Fitted::new(
                vec![Span::raw("orb-7")],
                vec![Span::raw("collected 10:22:14")],
                vec![Span::raw("open")],
            )
            .title_or_nothing()
        };

        assert_eq!(drawn(row(), 31), "orb-7  collected 10:22:14  open");
        assert_eq!(drawn(row(), 30), "orb-7                     open");
    }

    /// Only where it is asked for. Every other row keeps the cut it had.
    #[test]
    fn a_title_is_cut_like_any_other_block_unless_it_asks_not_to_be() {
        let row = Fitted::new(
            vec![Span::raw("orb-7")],
            vec![Span::raw("collected 10:22:14")],
            vec![Span::raw("open")],
        );

        assert_eq!(drawn(row, 30), "orb-7  collected 10:22:…  open");
    }

    /// The foot's key row is the same case on the state block: half a key
    /// name presses nothing, so the row is drawn whole or not at all, and the
    /// columns it would have taken go to the title instead.
    #[test]
    fn a_state_that_says_nothing_in_part_is_given_up_whole() {
        let row = || {
            Fitted::new(
                vec![Span::raw("orb-7")],
                vec![Span::raw("a title")],
                vec![Span::raw("Enter focus   q quit")],
            )
            .state_or_nothing()
        };

        assert_eq!(drawn(row(), 27), "orb-7  Enter focus   q quit");
        assert_eq!(drawn(row(), 26), "orb-7  a title            ");
    }

    /// Only where it is asked for, as with the title.
    #[test]
    fn a_state_is_cut_like_any_other_block_unless_it_asks_not_to_be() {
        let row = Fitted::new(
            vec![Span::raw("orb-7")],
            vec![Span::raw("a title")],
            vec![Span::raw("Enter focus   q quit")],
        );

        assert_eq!(drawn(row, 26), "orb-7  Enter focus   q qu…");
    }

    /// A band of no rows is a band that was not asked for. Nothing on screen
    /// shows this going wrong: the row a zero-height band lands on is one the
    /// buffer is happy to be written to, so the drawing simply covers whatever
    /// was beneath it.
    #[test]
    fn a_band_of_no_rows_is_left_alone() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 3));

        a_row().render(
            Rect {
                x: 0,
                y: 1,
                width: 20,
                height: 0,
            },
            &mut buf,
        );

        assert_eq!(Painted::read(&buf).rows(), blank(20, 3));
    }

    /// A band of no columns likewise.
    #[test]
    fn a_band_of_no_columns_is_left_alone() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 3));

        a_row().render(
            Rect {
                x: 0,
                y: 1,
                width: 0,
                height: 1,
            },
            &mut buf,
        );

        assert_eq!(Painted::read(&buf).rows(), blank(20, 3));
    }

    /// One line is one row. The selection is an index into the forest's lines,
    /// so a band that spilled past its first row would put that index and the
    /// screen out of step.
    #[test]
    fn a_band_of_several_rows_is_given_one() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 3));

        a_row().render(Rect::new(0, 0, 20, 3), &mut buf);

        assert_eq!(Painted::read(&buf).rows()[1..], blank(20, 2));
    }
}
