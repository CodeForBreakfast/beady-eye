//! One screen row fitted to the width it is given, and the cutting that fits it.
//!
//! A general widget: three blocks, one row, cut rather than wrapped, with a
//! stated yield order. It knows nothing of what it is drawing.

use std::cmp::Reverse;
use std::num::NonZeroU16;

use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Widget};
use ratatui::Frame;

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

/// Whether a terminal can be told that `said` points at `to`.
///
/// Both come from a tracker rather than from this program, and what ends this
/// sequence is itself a control character. One inside the sequence ends it
/// early, and the rest of that value reaches the terminal as commands of its
/// own — a row of a bead list saying whatever it likes to the terminal the
/// reader is sitting at.
///
/// Asked wherever a link is spoken about and not only where one is written:
/// a badge styled as a link the emitter then refuses is a row that says it
/// can be followed and cannot.
pub(crate) fn openable(said: &str, to: &str) -> bool {
    let holds_control = |text: &str| text.chars().any(char::is_control);
    !holds_control(said) && !holds_control(to)
}

/// `said`, wrapped so the terminal makes it a link to `to`. Nothing for a
/// pair `openable` refuses, which is this program's last word before the
/// bytes go out.
pub(crate) fn hyperlink(said: &str, to: &str) -> Option<String> {
    openable(said, to).then(|| format!("{OSC_8}{to}{ST}{said}{OSC_8}{ST}"))
}

/// What a cell says, with any escape sequence taken out of it.
///
/// A hyperlink is written into the symbol because it cannot be written into a
/// span, but it is not among the words: a reader sees where a link goes only
/// by following it.
pub(crate) fn words_of(symbol: &str) -> String {
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

const ESCAPE: char = '\x1b';

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

/// A span of the title block that can also say itself more shortly.
///
/// Named by its place, as `Link` is, and carrying words rather than a span:
/// the row dresses the short form in the long one's own style, so a form the
/// row falls back to is not a different-looking thing.
pub(crate) struct Shorter {
    /// Which span of the title block it is.
    pub(crate) at: usize,
    /// What it says where the row cannot afford what it usually says.
    pub(crate) said: String,
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
    shorter: Vec<Shorter>,
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
            shorter: Vec::new(),
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

    /// Which spans of the title block can say themselves more shortly, and
    /// what each says then.
    ///
    /// `briefly` swaps the whole state block; this swaps one span inside the
    /// title. Where the title will not fit, the row says short forms rather
    /// than cutting, so a span whose length is not this program's to choose
    /// can survive a narrow row whole instead of being cut to a head that
    /// names nothing.
    #[must_use]
    pub(crate) fn shortening(mut self, shorter: Vec<Shorter>) -> Self {
        self.shorter = shorter;
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

        let links = self.links;
        let shorter = self.shorter;
        let identity = columns(&self.identity);
        let (spans, linked) = if identity >= width {
            (cut_to(self.identity, width).0, Vec::new())
        } else {
            let room = width - identity;
            let ((title, whole), state) = match self.briefly {
                None => {
                    let (state, _) =
                        fit(self.state, room.saturating_sub(GAP), self.state_or_nothing);
                    let left = room - columns(&state) - if state.is_empty() { 0 } else { GAP };
                    let limit = left.saturating_sub(GAP);
                    (
                        fit(
                            shortened(self.title, &shorter, limit),
                            limit,
                            self.title_or_nothing,
                        ),
                        state,
                    )
                }
                Some(briefly) => {
                    // Whichever form is smaller, rather than the one named
                    // `briefly`: a pane terse enough makes the long form the
                    // short one, and room kept for a form the row will not
                    // use is room taken off the title for nothing.
                    let room_for_state = columns(&briefly).min(columns(&self.state)) + GAP;
                    let limit = room.saturating_sub(GAP + room_for_state);
                    let (title, whole) = fit(
                        shortened(self.title, &shorter, limit),
                        limit,
                        self.title_or_nothing,
                    );
                    let left = room - columns(&title) - if title.is_empty() { 0 } else { GAP };
                    let limit = left.saturating_sub(GAP);
                    let state = if columns(&self.state) <= limit {
                        self.state
                    } else {
                        fit(briefly, limit, self.state_or_nothing).0
                    };
                    ((title, whole), state)
                }
            };

            let linked = surviving(&links, &title, whole, identity + GAP);

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
            (spans, linked)
        };

        Line::from(spans).style(self.whole).render(area, buf);
        for link in linked {
            link.told_to(area, buf);
        }
    }
}

/// A link the row kept whole, at the column it starts on.
struct Kept {
    at: usize,
    width: usize,
    said: String,
    to: String,
}

impl Kept {
    /// The link, told to the terminal in the cell it starts on.
    ///
    /// Everything the link says goes in that one cell, wrapped in the escape
    /// sequences, and the cell reports the width the words alone take. The
    /// diff then skips the columns behind it, so the opening sequence and its
    /// closer are one thing to send or to leave: a redraw carries both or
    /// neither, whatever else on the row changed.
    fn told_to(self, area: Rect, buf: &mut Buffer) {
        let (Ok(at), Some(width)) = (
            u16::try_from(self.at),
            u16::try_from(self.width).ok().and_then(NonZeroU16::new),
        ) else {
            return;
        };
        let Some(said) = hyperlink(&self.said, &self.to) else {
            return;
        };
        if let Some(cell) = buf.cell_mut((area.x + at, area.y)) {
            cell.set_symbol(&said)
                .set_diff_option(CellDiffOption::ForcedWidth(width));
        }
    }
}

/// The links whose span the title block kept whole, each at the column it
/// starts on. A link whose span was cut or dropped is not among them.
fn surviving(links: &[Link], title: &[Span<'static>], whole: usize, starts: usize) -> Vec<Kept> {
    links
        .iter()
        .filter(|link| link.at < whole)
        .map(|link| Kept {
            at: starts + columns(&title[..link.at]),
            width: title[link.at].width(),
            said: title[link.at].content.to_string(),
            to: link.to.clone(),
        })
        .collect()
}

/// Blank the ground a window is about to stand on.
///
/// `Clear` alone is not enough over a row carrying a link. The whole link sits
/// in the cell it starts on, reporting the width of its words, and the diff
/// skips the columns behind that cell whatever they now hold — so a link
/// starting outside the window and reaching under it swallows the window's own
/// left edge, and the border never reaches the terminal. Such a link gives the
/// columns back and stops being a link, and its words are written again into
/// the columns it still has. A badge under a window is a badge the reader
/// cannot see anyway.
pub(crate) fn cover(frame: &mut Frame, window: Rect) {
    frame.render_widget(Clear, window);

    let buf = frame.buffer_mut();
    for y in window.top()..window.bottom() {
        for x in buf.area.left()..window.left() {
            let Some(cell) = buf.cell((x, y)) else {
                continue;
            };
            let CellDiffOption::ForcedWidth(width) = cell.diff_option else {
                continue;
            };
            if x.saturating_add(width.get()) <= window.left() {
                continue;
            }
            let words = words_of(cell.symbol());
            let style = cell.style();
            let room = (window.left() - x) as usize;
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.reset();
            }
            buf.set_stringn(x, y, words, room, style);
        }
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

/// `spans` with short forms swapped in until the run fits in `limit`, or until
/// every span offering one has said it.
///
/// Each short form is dressed in the style of the span it stands in for, so a
/// span the row shortened is the same span saying less. It is swapped rather
/// than cut, so `fit` counts it among the spans the block kept whole and
/// `surviving` keeps its link.
///
/// The widest saving goes first, so that as few spans shorten as will make the
/// run fit: a span shortened where a wider neighbour would have done is columns
/// given up for nothing. Where two save the same, the one nearer the end goes
/// first, that being the end the block gives up anyway.
fn shortened(
    mut spans: Vec<Span<'static>>,
    shorter: &[Shorter],
    limit: usize,
) -> Vec<Span<'static>> {
    let saving = |swap: &Shorter| {
        spans.get(swap.at).map_or(0, |span| {
            span.width()
                .saturating_sub(Span::raw(swap.said.as_str()).width())
        })
    };
    let mut order: Vec<&Shorter> = shorter.iter().collect();
    order.sort_by_key(|swap| (Reverse(saving(swap)), Reverse(swap.at)));

    for swap in order {
        if columns(&spans) <= limit {
            break;
        }
        if let Some(span) = spans.get_mut(swap.at) {
            *span = Span::styled(swap.said.clone(), span.style);
        }
    }
    spans
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
            kept.push(Span::styled(head, unlinked(span.style)));
        }
        break;
    }
    kept.push(Span::raw(CUT.to_string()));
    (kept, whole)
}

/// `style` with the underline that stands for a link taken back off.
///
/// The other half of the rule `surviving` keeps: a span the row cut is not
/// among the links the terminal is told about, so nothing is there to follow.
/// Left underlined it would invite a click that cannot be honoured. Said as
/// what `palette::LINK` adds rather than as the modifier itself, so the two
/// cannot drift apart.
///
/// Asked of every span the row cuts rather than only of the ones a link
/// names, because `cut_to` is told which columns it has and nothing about
/// what it is cutting. A span that never carried the underline is unmoved.
fn unlinked(style: Style) -> Style {
    style.remove_modifier(palette::LINK.add_modifier)
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
    use ratatui::backend::TestBackend;
    use ratatui::widgets::Block;
    use ratatui::Terminal;

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

    /// A row whose state has a short form, for a state whose length is not
    /// this program's to choose.
    fn a_row_saying(title: &str) -> Fitted {
        Fitted::new(
            vec![Span::raw("orb-7")],
            vec![Span::raw(title.to_string())],
            vec![Span::raw("working on the parser")],
        )
        .briefly(vec![Span::raw("working")])
    }

    /// The long form is said only where it costs the title nothing. One column
    /// narrower and the row says the short form whole, rather than saying the
    /// long one over the blanks that keep the two blocks apart and off the end
    /// of the row.
    #[test]
    fn a_state_with_no_room_for_its_long_form_is_said_in_its_short_one() {
        assert_eq!(
            drawn(a_row_saying("a title"), 37),
            "orb-7  a title  working on the parser"
        );
        assert_eq!(
            drawn(a_row_saying("a title"), 36),
            "orb-7  a title               working"
        );
    }

    /// The room kept back for the state is the short form's, so a title long
    /// enough to be cut is cut to exactly what that leaves. Keep back less and
    /// the title takes room the short form was promised, which is a row saying
    /// nothing in part twice over.
    #[test]
    fn a_title_cut_for_width_leaves_the_short_form_the_room_kept_for_it() {
        assert_eq!(
            drawn(a_row_saying("teach the elided run to fold back open"), 40),
            "orb-7  teach the elided run to…  working"
        );
    }

    /// A row whose badge can also say itself as `⇢ #12`, for a badge whose
    /// length is a tracker's to choose rather than this program's.
    fn a_shortenable_row() -> Fitted {
        Fitted::new(
            vec![Span::raw("orb-7")],
            vec![
                Span::raw("a title"),
                Span::raw("  "),
                Span::raw("⇢ awaiting review"),
            ],
            Vec::new(),
        )
        .shortening(vec![Shorter {
            at: 2,
            said: "⇢ #12".to_string(),
        }])
    }

    /// One column narrower than the long form needs and the row says the short
    /// form whole, rather than a cut long one that names no pull request.
    #[test]
    fn a_span_with_no_room_for_its_long_form_is_said_in_its_short_one() {
        assert_eq!(
            drawn(a_shortenable_row(), 33),
            "orb-7  a title  ⇢ awaiting review"
        );
        assert_eq!(
            drawn(a_shortenable_row(), 32),
            "orb-7  a title  ⇢ #12           "
        );
    }

    /// The same row with nothing offered in its place, at the same width.
    #[test]
    fn a_span_offering_no_short_form_is_cut_as_it_always_was() {
        let row = Fitted::new(
            vec![Span::raw("orb-7")],
            vec![
                Span::raw("a title"),
                Span::raw("  "),
                Span::raw("⇢ awaiting review"),
            ],
            Vec::new(),
        );

        assert_eq!(drawn(row, 32), "orb-7  a title  ⇢ awaiting revi…");
    }

    /// A short form that fits is a span the row kept whole, so it keeps the
    /// link. Lose it here and a badge stops being followable at exactly the
    /// widths where it was shortened in order to survive.
    #[test]
    fn a_span_said_in_its_short_form_keeps_the_link_the_long_one_had() {
        let row = a_shortenable_row().linking(vec![Link {
            at: 2,
            to: SOMEWHERE.to_string(),
        }]);

        let said = symbols(&rendered(row, 32));

        assert!(
            said.contains(
                &hyperlink("⇢ #12", SOMEWHERE).expect("this vocabulary holds no control character")
            ),
            "the short form was drawn without the link the long one had: {said:?}"
        );
    }

    /// A row of two spans that could shorten, the second giving back more
    /// columns than the first.
    fn a_row_of_two_shortenable_spans() -> Fitted {
        Fitted::new(
            vec![Span::raw("orb-7")],
            vec![
                Span::raw("a title"),
                Span::raw("  "),
                Span::raw("awaiting review"),
                Span::raw("  "),
                Span::raw("blocked on the tracker"),
            ],
            Vec::new(),
        )
        .shortening(vec![
            Shorter {
                at: 2,
                said: "#12".to_string(),
            },
            Shorter {
                at: 4,
                said: "blocked".to_string(),
            },
        ])
    }

    /// The widest saving is taken first, and taken alone where it is enough.
    /// Sweep in span order instead and the row shortens a span that had room
    /// to be whole, which is columns given up for nothing.
    #[test]
    fn no_more_spans_shorten_than_the_row_has_to_shorten_to_fit() {
        assert_eq!(
            drawn(a_row_of_two_shortenable_spans(), 55),
            "orb-7  a title  awaiting review  blocked on the tracker"
        );
        assert_eq!(
            drawn(a_row_of_two_shortenable_spans(), 40),
            "orb-7  a title  awaiting review  blocked"
        );
        assert_eq!(
            drawn(a_row_of_two_shortenable_spans(), 39),
            "orb-7  a title  #12  blocked           "
        );
    }

    /// A row with both kinds of short form, so which gives way first shows.
    fn a_shortenable_row_saying_briefly() -> Fitted {
        Fitted::new(
            vec![Span::raw("orb-7")],
            vec![
                Span::raw("a title"),
                Span::raw("  "),
                Span::raw("⇢ awaiting review"),
            ],
            vec![Span::raw("working on the parser")],
        )
        .briefly(vec![Span::raw("working")])
        .shortening(vec![Shorter {
            at: 2,
            said: "⇢ #12".to_string(),
        }])
    }

    /// The state says its long form only where it costs the title nothing, and
    /// a badge is part of the title. So the state block swaps first: both
    /// arrangements fit at 55 columns, and the row draws the one that keeps
    /// the title whole.
    #[test]
    fn a_span_keeps_its_long_form_where_the_state_block_can_swap_instead() {
        assert_eq!(
            drawn(a_shortenable_row_saying_briefly(), 56),
            "orb-7  a title  ⇢ awaiting review  working on the parser"
        );
        assert_eq!(
            drawn(a_shortenable_row_saying_briefly(), 55),
            "orb-7  a title  ⇢ awaiting review               working"
        );
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
        a_row_linking("⇢ #12")
    }

    /// The same row, for a badge whose text the caller chooses.
    fn a_row_linking(badge: &str) -> Fitted {
        Fitted::new(
            vec![Span::raw("orb-7")],
            vec![
                Span::raw("a title"),
                Span::raw(" "),
                Span::raw(badge.to_string()),
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
            symbols(&buf).contains(
                &hyperlink("⇢ #12", SOMEWHERE).expect("this vocabulary holds no control character")
            ),
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

    /// What a link needs is its own span whole, rather than the block it sits
    /// in. Here the row is cut after the link, and the link is still a link.
    #[test]
    fn a_link_the_cut_stopped_short_of_is_opened_as_it_always_was() {
        let said = symbols(&rendered(a_linked_row(), 21));

        assert!(
            said.contains(&CUT.to_string()),
            "the row was not cut at all, so it says nothing about a link \
             before the cut: {said:?}"
        );
        assert!(
            said.contains(
                &hyperlink("⇢ #12", SOMEWHERE).expect("this vocabulary holds no control character")
            ),
            "a link the cut stopped short of was dropped: {said:?}"
        );
    }

    /// A URL is a tracker's to write, and what ends the sequence carrying one
    /// is a control character. A link is worth less than a terminal a row can
    /// say anything it likes to.
    #[test]
    fn a_link_carrying_a_control_character_is_not_opened() {
        for to in [
            format!("https://forge.invalid{ST}\x1b]52;c;cGF5bG9hZA=={ST}"),
            "https://forge.invalid/\nfoo".to_string(),
            "https://forge.invalid/\rfoo".to_string(),
        ] {
            let row = Fitted::new(
                vec![Span::raw("orb-7")],
                vec![Span::raw("a title"), Span::raw(" "), Span::raw("⇢ #12")],
                Vec::new(),
            )
            .linking(vec![Link {
                at: 2,
                to: to.clone(),
            }]);

            let said = symbols(&rendered(row, 40));

            assert!(
                !said.contains(ESCAPE),
                "a link naming {to:?} reached the terminal: {said:?}"
            );
            assert!(
                said.contains("⇢ #12"),
                "the badge stopped drawing as well as stopped linking: {said:?}"
            );
        }
    }

    /// A window standing on a row whose link starts outside it. The link is
    /// holding columns the window needs, and the diff cannot reach past it, so
    /// without the hand-back the window's own left edge is never sent and the
    /// badge prints over it.
    ///
    /// Both a badge whose first glyph takes one column and one whose first
    /// glyph takes two, because the columns the link keeps are counted rather
    /// than assumed.
    #[test]
    fn a_window_over_a_link_that_started_outside_it_still_draws_its_own_edge() {
        // A badge starts at column 15, so a window opening at 17 stands on the
        // middle of one.
        const STARTS: u16 = 15;
        let window = Rect::new(17, 0, 10, 3);

        for (badge, glyph) in [("⇢ #12", "⇢"), ("🔗 #12", "🔗")] {
            let mut terminal = Terminal::new(TestBackend::new(40, 3)).expect("a test backend");
            let mut draw_row_and = |window: Option<Rect>| {
                terminal
                    .draw(|frame| {
                        a_row_linking(badge).render(Rect::new(0, 0, 40, 1), frame.buffer_mut());
                        if let Some(window) = window {
                            cover(frame, window);
                            frame.render_widget(Block::bordered(), window);
                        }
                    })
                    .expect("a draw into memory");
            };
            draw_row_and(None);
            draw_row_and(Some(window));

            let screen = terminal.backend().buffer().clone();
            assert_eq!(
                screen[(window.left(), 0)].symbol(),
                "┌",
                "the window's left edge never reached the terminal, over {badge:?}"
            );
            assert_eq!(
                screen[(STARTS, 0)].symbol(),
                glyph,
                "the columns the link handed back say nothing the reader can see"
            );
        }
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
