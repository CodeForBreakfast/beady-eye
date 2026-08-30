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
        }
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
        let area = Rect { height: 1, ..area };
        if area.width == 0 || area.height == 0 {
            return;
        }
        let width = area.width as usize;

        let identity = columns(&self.identity);
        let spans = if identity >= width {
            cut_to(self.identity, width)
        } else {
            let mut room = width - identity;
            let state = cut_to(self.state, room.saturating_sub(GAP));
            room -= columns(&state) + if state.is_empty() { 0 } else { GAP };
            let title = cut_to(self.title, room.saturating_sub(GAP));

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
