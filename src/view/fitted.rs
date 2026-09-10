//! One screen row fitted to the width it is given, and the cutting that fits it.
//!
//! A general widget: three blocks, one row, cut rather than wrapped, with a
//! stated yield order. It knows nothing of what it is drawing.

use std::num::NonZeroU16;

use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::view::palette;

/// The mark left where a line ran out of width, so a cut line reads as cut
/// rather than as one that had nothing more to say.
pub(crate) const CUT: char = '…';

/// The blank columns that keep two blocks from reading as one.
pub(crate) const GAP: usize = 2;

/// The escape that opens an operating-system command naming a hyperlink, and
/// the one that ends any such command. A URL between them makes what follows
/// a link; nothing between them ends it.
const OSC_8: &str = "\x1b]8;;";
const ST: &str = "\x1b\\";

/// `said`, wrapped so the terminal makes it a link to `to`.
pub(crate) fn hyperlink(said: &str, to: &str) -> String {
    format!("{OSC_8}{to}{ST}{said}{OSC_8}{ST}")
}

/// A span of the title block that stands for somewhere the reader can go.
///
/// Named by its place rather than by what it draws: `Fitted` knows nothing of
/// what it is drawing, and a link is one more thing it does not have to know.
pub(crate) struct Link {
    /// Which span of the title block it is.
    pub(crate) at: usize,
    /// Where it points.
    pub(crate) to: String,
}

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
    briefly: Option<Vec<Span<'static>>>,
    links: Vec<Link>,
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
            briefly: None,
            links: Vec::new(),
            whole: Style::new(),
            title_or_nothing: false,
            state_or_nothing: false,
        }
    }

    /// Which spans of the title block are links, and where each one points.
    ///
    /// A link the row had to cut is dropped: an opening sequence with nothing
    /// to close it makes every cell after it on the terminal part of the link.
    #[must_use]
    pub(crate) fn linking(mut self, links: Vec<Link>) -> Self {
        self.links = links;
        self
    }

    /// A shorter form of the state, for a state holding something whose
    /// length is not this program's to choose.
    ///
    /// It changes which block gives way. Without one the state is fitted
    /// first and the title takes what is left, because a state is normally a
    /// handful of short cells this program wrote. With one, the row keeps
    /// room for the short form, gives the title the rest, and says the long
    /// form only where it costs the title nothing — so a caption that could
    /// be any length cannot eat the row it is drawn on.
    #[must_use]
    pub(crate) fn briefly(mut self, state: Vec<Span<'static>>) -> Self {
        self.briefly = Some(state);
        self
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
        self.whole = self.whole.patch(palette::SELECTED);
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

        let wanted = self.links;
        let identity = columns(&self.identity);
        let (spans, links) = if identity >= width {
            (cut_to(self.identity, width).0, Vec::new())
        } else {
            let room = width - identity;
            let ((title, whole), state) = match self.briefly {
                None => {
                    let state = fit(self.state, room.saturating_sub(GAP), self.state_or_nothing).0;
                    let left = room - columns(&state) - if state.is_empty() { 0 } else { GAP };
                    (
                        fit(self.title, left.saturating_sub(GAP), self.title_or_nothing),
                        state,
                    )
                }
                Some(briefly) => {
                    // Whichever form is smaller, rather than the one named
                    // `briefly`: a pane terse enough makes the long form the
                    // short one, and room kept for a form the row will not
                    // use is room taken off the title for nothing.
                    let kept = columns(&briefly).min(columns(&self.state)) + GAP;
                    let title = fit(
                        self.title,
                        room.saturating_sub(GAP + kept),
                        self.title_or_nothing,
                    );
                    let left = room - columns(&title.0) - if title.0.is_empty() { 0 } else { GAP };
                    let limit = left.saturating_sub(GAP);
                    let state = if columns(&self.state) <= limit {
                        self.state
                    } else {
                        fit(briefly, limit, self.state_or_nothing).0
                    };
                    (title, state)
                }
            };

            let links = surviving(&wanted, &title, whole, identity + GAP);

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
            (spans, links)
        };

        Line::from(spans).style(self.whole).render(area, buf);
        for link in links {
            link.written(area, buf);
        }
    }
}

/// A link that survived the cut, at the column it landed on.
struct Landed {
    at: usize,
    width: usize,
    said: String,
    to: String,
}

impl Landed {
    /// The link, told to the terminal in the cell it starts on.
    ///
    /// Everything the link says goes in that one cell, wrapped in the escape
    /// sequences, and the cell reports the width the words alone take. The
    /// diff then skips the columns behind it, so the opening sequence and its
    /// closer are one thing to send or to leave: a redraw carries both or
    /// neither, whatever else on the row changed.
    fn written(self, area: Rect, buf: &mut Buffer) {
        let (Ok(at), Some(width)) = (
            u16::try_from(self.at),
            u16::try_from(self.width).ok().and_then(NonZeroU16::new),
        ) else {
            return;
        };
        if let Some(cell) = buf.cell_mut((area.x + at, area.y)) {
            cell.set_symbol(&hyperlink(&self.said, &self.to))
                .set_diff_option(CellDiffOption::ForcedWidth(width));
        }
    }
}

/// The links whose span the title block kept whole, each at the column it
/// starts on. A link whose span was cut or dropped is not among them.
fn surviving(wanted: &[Link], title: &[Span<'static>], whole: usize, starts: usize) -> Vec<Landed> {
    wanted
        .iter()
        .filter(|link| link.at < whole)
        .map(|link| Landed {
            at: starts + columns(&title[..link.at]),
            width: title[link.at].width(),
            said: title[link.at].content.to_string(),
            to: link.to.clone(),
        })
        .collect()
}

/// What a run of spans takes up on screen, in columns rather than in bytes:
/// every glyph in this vocabulary is several bytes long, and a width counted
/// in bytes would put a cut inside one.
pub(crate) fn columns(spans: &[Span<'static>]) -> usize {
    spans.iter().map(Span::width).sum()
}

/// `spans` fitted into `limit` columns: cut with the cut marked, or, for a
/// block that says nothing in part, given up whole.
///
/// Alongside the spans, how many of them the block kept whole — the leading
/// run that says everything it was written to say. Anything after that run was
/// cut short or dropped, and what a caller holds against a span of it no
/// longer holds.
fn fit(spans: Vec<Span<'static>>, limit: usize, or_nothing: bool) -> (Vec<Span<'static>>, usize) {
    if or_nothing && columns(&spans) > limit {
        return (Vec::new(), 0);
    }
    cut_to(spans, limit)
}

/// `spans`, cut down to `limit` columns with the cut marked, and how many of
/// them survived whole.
fn cut_to(spans: Vec<Span<'static>>, limit: usize) -> (Vec<Span<'static>>, usize) {
    if columns(&spans) <= limit {
        let whole = spans.len();
        return (spans, whole);
    }
    if limit == 0 {
        return (Vec::new(), 0);
    }

    let room = limit - columns(&[Span::raw(CUT.to_string())]);
    let mut kept: Vec<Span<'static>> = Vec::new();
    let mut whole = 0;
    let mut used = 0;
    for span in spans {
        let width = span.width();
        if used + width <= room {
            used += width;
            whole += 1;
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
    (kept, whole)
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

    /// The URL a reader clicks, in the vocabulary the fixtures use.
    const SOMEWHERE: &str = "https://forge.invalid/orbital/atlas/pull/12";

    /// A row whose third title span is a link, so the link is neither the
    /// first thing on the line nor the last.
    fn a_linked_row() -> Fitted {
        Fitted::new(
            vec![Span::raw("orb-7")],
            vec![
                Span::raw("a title"),
                Span::raw(" "),
                Span::raw("⇢ #12"),
                Span::raw(" done"),
            ],
            Vec::new(),
        )
        .linking(vec![Link {
            at: 2,
            to: SOMEWHERE.to_string(),
        }])
    }

    /// Every symbol of a rendered row, run together.
    fn symbols(buf: &Buffer) -> String {
        (buf.area.left()..buf.area.right())
            .map(|x| buf[(x, 0)].symbol())
            .collect()
    }

    fn rendered(row: Fitted, width: u16) -> Buffer {
        let area = Rect::new(0, 0, width, 1);
        let mut buf = Buffer::empty(area);
        row.render(area, &mut buf);
        buf
    }

    /// The escape bytes cannot go in a span, because a span is measured by
    /// what it shows. They go in the cell the link starts on instead, around
    /// everything the link says.
    #[test]
    fn a_linked_span_is_wrapped_in_a_hyperlink_where_it_starts() {
        let buf = rendered(a_linked_row(), 40);

        assert!(
            symbols(&buf).contains(&hyperlink("⇢ #12", SOMEWHERE)),
            "the link is not on the row: {:?}",
            symbols(&buf)
        );
    }

    /// The cell says the width the link really takes, so the diff skips the
    /// columns behind it rather than counting the escape bytes as glyphs.
    #[test]
    fn a_linked_cell_reports_the_width_the_link_takes_on_screen() {
        let buf = rendered(a_linked_row(), 40);
        let at = opened_at(&buf);

        assert_eq!(
            buf[(at, 0)].diff_option,
            CellDiffOption::ForcedWidth(NonZeroU16::new(5).expect("⇢ #12 is five columns"))
        );
    }

    /// A link cut for width loses the link rather than its closing sequence.
    /// An opening sequence with nothing to close it makes every cell after it
    /// on the terminal part of the link.
    #[test]
    fn a_link_cut_for_width_is_not_opened_at_all() {
        let said = symbols(&rendered(a_linked_row(), 20));

        assert!(
            !said.contains(OSC_8),
            "a cut link opened a hyperlink: {said:?}"
        );
    }

    /// The opening and the closing sequence are one cell, so a diff carries
    /// both or neither. Here the link's first glyph changes and its last does
    /// not, which is the shape that would send an opening sequence alone.
    #[test]
    fn a_partial_redraw_cannot_send_an_opening_sequence_without_its_closer() {
        let moved = || {
            Fitted::new(
                vec![Span::raw("orb-7")],
                vec![Span::raw("a title"), Span::raw(" "), Span::raw("→ #12")],
                Vec::new(),
            )
            .linking(vec![Link {
                at: 2,
                to: SOMEWHERE.to_string(),
            }])
        };

        let before = rendered(a_linked_row(), 40);
        let after = rendered(moved(), 40);
        let sent: Vec<&str> = before
            .diff(&after)
            .into_iter()
            .map(|(_, _, cell)| cell.symbol())
            .collect();
        let opened: Vec<&&str> = sent.iter().filter(|said| said.contains(OSC_8)).collect();

        assert!(!opened.is_empty(), "the moved link was not sent: {sent:?}");
        for said in opened {
            assert!(
                said.ends_with(&format!("{OSC_8}{ST}")),
                "an opening sequence went without its closer: {said:?}"
            );
        }
    }

    /// Where the row's one hyperlink starts.
    fn opened_at(buf: &Buffer) -> u16 {
        (buf.area.left()..buf.area.right())
            .find(|&x| buf[(x, 0)].symbol().starts_with(OSC_8))
            .expect("a cell opening a hyperlink")
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
