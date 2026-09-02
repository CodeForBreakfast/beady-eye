//! The description and the notes, rendered as markdown into the rows the
//! window scrolls.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

/// `text` laid out in `width` columns: one row per screen row, styled where
/// the markdown says so.
pub(crate) fn rows(text: &str, width: usize) -> Vec<Vec<Span<'static>>> {
    lines(text, width)
        .into_iter()
        .flat_map(|line| line.wrap(width))
        .collect()
}

/// One line of the rendered text before it is wrapped: what it says, in
/// runs of one style, and what goes in front of its rows.
#[derive(Debug, Default)]
struct Line {
    /// What the first row starts with: the indent, and a list marker where
    /// the line opens an item.
    lead: String,
    /// What every row after the first starts with, so a wrapped item hangs
    /// under its own text rather than under its marker.
    hang: String,
    spans: Vec<Span<'static>>,
    /// Laid out by its author, as a line of code is: every space kept, and
    /// a row too wide broken between glyphs rather than at a space.
    verbatim: bool,
}

impl Line {
    /// The line in rows of `width` columns, wrapped at the spaces. A word too
    /// wide for a row is broken between glyphs rather than lost.
    fn wrap(self, width: usize) -> Vec<Vec<Span<'static>>> {
        let mut rows: Vec<Vec<Span<'static>>> = Vec::new();
        let mut row: Vec<Span<'static>> = Vec::new();
        let room = |rows: &Vec<Vec<Span<'static>>>| {
            let prefix = if rows.is_empty() {
                &self.lead
            } else {
                &self.hang
            };
            width.saturating_sub(columns_of(prefix)).max(1)
        };
        let flush = |rows: &mut Vec<Vec<Span<'static>>>, row: Vec<Span<'static>>| {
            let prefix = if rows.is_empty() {
                &self.lead
            } else {
                &self.hang
            };
            let mut prefixed = vec![Span::raw(prefix.clone())];
            prefixed.extend(row);
            rows.push(prefixed);
        };

        let words = if self.verbatim {
            vec![self.spans.clone()]
        } else {
            words(&self.spans)
        };
        for word in words {
            let mut word = if row.is_empty() {
                word
            } else if columns(&row) + 1 + columns(&word) <= room(&rows) {
                row.push(Span::raw(" "));
                row.extend(word);
                continue;
            } else {
                flush(&mut rows, std::mem::take(&mut row));
                word
            };
            while columns(&word) > room(&rows) {
                let (head, rest) = split_at_columns(word, room(&rows));
                flush(&mut rows, head);
                word = rest;
            }
            row = word;
        }
        flush(&mut rows, row);
        rows
    }
}

/// The words of a line, each in the runs of style it was written in. A word
/// is what sits between spaces, whatever style changes happen inside it.
fn words(spans: &[Span<'static>]) -> Vec<Vec<Span<'static>>> {
    let mut words: Vec<Vec<Span<'static>>> = Vec::new();
    let mut open = false;
    for span in spans {
        for (n, piece) in span.content.split(char::is_whitespace).enumerate() {
            if n > 0 {
                open = false;
            }
            if piece.is_empty() {
                continue;
            }
            if !open {
                words.push(Vec::new());
                open = true;
            }
            words
                .last_mut()
                .expect("a word was just opened")
                .push(Span::styled(piece.to_string(), span.style));
        }
    }
    words
}

/// What a run of spans takes up on screen, in columns.
fn columns(spans: &[Span<'static>]) -> usize {
    spans.iter().map(Span::width).sum()
}

/// What a piece of text takes up on screen, in columns.
fn columns_of(text: &str) -> usize {
    Span::raw(text).width()
}

/// `word` split after as many glyphs as fit in `limit` columns, never
/// inside one. At least one glyph goes in the head, so a glyph wider than
/// the limit is still drawn rather than looped over for ever.
fn split_at_columns(
    word: Vec<Span<'static>>,
    limit: usize,
) -> (Vec<Span<'static>>, Vec<Span<'static>>) {
    let mut head: Vec<Span<'static>> = Vec::new();
    let mut rest: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    for span in word {
        if !rest.is_empty() {
            rest.push(span);
            continue;
        }
        let mut kept = String::new();
        let mut glyphs = span.content.chars();
        for glyph in glyphs.by_ref() {
            let width = columns_of(&glyph.to_string());
            if (!head.is_empty() || !kept.is_empty()) && used + width > limit {
                let left: String = std::iter::once(glyph).chain(glyphs).collect();
                rest.push(Span::styled(left, span.style));
                break;
            }
            used += width;
            kept.push(glyph);
        }
        if !kept.is_empty() {
            head.push(Span::styled(kept, span.style));
        }
    }
    (head, rest)
}

/// The tone a code span or a code block is drawn in, patched over whatever
/// style the text round it has.
const CODE: Color = Color::Cyan;

/// `bd show`'s own bullet.
const BULLET: &str = "• ";

/// The bar down the side of a quoted block.
const QUOTE: &str = "│ ";

/// What a rule is drawn with, across the room it has.
const RULE: char = '─';

/// The text as lines, before wrapping. A rule is as wide as `width`, which
/// is the one thing here that needs to know it.
fn lines(text: &str, width: usize) -> Vec<Line> {
    let mut rendering = Rendering {
        width,
        ..Rendering::default()
    };
    for event in Parser::new_ext(text, Options::empty()) {
        rendering.take(event);
    }
    rendering.close();
    rendering.lines
}

/// A list being rendered: what its next item is marked with.
#[derive(Debug)]
struct List {
    /// The next item's number, or none for a bulleted list.
    next: Option<u64>,
}

/// The lines so far, the one being written, and where the text arriving
/// goes: the styles it is in, innermost last, and what sits in front of it
/// on every row, outermost first.
#[derive(Default)]
struct Rendering {
    width: usize,
    lines: Vec<Line>,
    current: Option<Line>,
    /// The line being written is an item's first, with nothing said on it
    /// yet, so the item's own paragraph joins it rather than opening a block.
    fresh: bool,
    styles: Vec<Style>,
    prefixes: Vec<String>,
    lists: Vec<List>,
    links: Vec<String>,
}

impl Rendering {
    fn take(&mut self, event: Event<'_>) {
        match event {
            Event::Start(Tag::Paragraph) if self.fresh => self.fresh = false,
            Event::Start(Tag::Paragraph | Tag::HtmlBlock) => self.open_block(),
            Event::End(TagEnd::Paragraph | TagEnd::HtmlBlock) => self.close(),
            Event::Start(Tag::Heading { .. }) => {
                self.open_block();
                self.push_style(Modifier::BOLD);
            }
            Event::End(TagEnd::Heading(_)) => {
                self.styles.pop();
                self.close();
            }
            Event::Start(Tag::CodeBlock(_)) => {
                self.open_block();
                self.line().verbatim = true;
                self.styles.push(self.style().fg(CODE));
            }
            Event::End(TagEnd::CodeBlock) => {
                self.styles.pop();
                self.close();
            }
            Event::Start(Tag::BlockQuote(_)) => {
                self.close();
                self.prefixes.push(QUOTE.to_string());
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                self.close();
                self.prefixes.pop();
            }
            Event::Start(Tag::List(first)) => {
                if self.lists.is_empty() {
                    self.separate();
                } else {
                    self.close();
                }
                self.lists.push(List { next: first });
            }
            Event::End(TagEnd::List(_)) => {
                self.close();
                self.lists.pop();
            }
            Event::Start(Tag::Item) => self.open_item(),
            Event::End(TagEnd::Item) => {
                self.close();
                self.prefixes.pop();
            }
            Event::Start(Tag::Emphasis) => self.push_style(Modifier::ITALIC),
            Event::Start(Tag::Strong) => self.push_style(Modifier::BOLD),
            Event::End(TagEnd::Emphasis | TagEnd::Strong) => {
                self.styles.pop();
            }
            Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) => {
                self.push_style(Modifier::UNDERLINED);
                self.links.push(dest_url.to_string());
            }
            Event::End(TagEnd::Link | TagEnd::Image) => {
                self.styles.pop();
                let destination = self.links.pop().unwrap_or_default();
                self.say(&format!(" ({destination})"));
            }
            Event::Code(said) => {
                let style = self.style().fg(CODE);
                self.say_in(&said, style);
            }
            Event::Text(said)
            | Event::Html(said)
            | Event::InlineHtml(said)
            | Event::InlineMath(said)
            | Event::DisplayMath(said)
            | Event::FootnoteReference(said) => self.say(&said),
            Event::SoftBreak | Event::HardBreak => self.break_line(),
            Event::Rule => {
                self.open_block();
                let room = self.width.saturating_sub(columns_of(&self.indent()));
                self.say(&RULE.to_string().repeat(room));
                self.close();
            }
            Event::TaskListMarker(done) => self.say(if done { "[x] " } else { "[ ] " }),
            Event::Start(_) | Event::End(_) => {}
        }
    }

    /// What every row of the text arriving starts with.
    fn indent(&self) -> String {
        self.prefixes.concat()
    }

    /// The style the text arriving is in.
    fn style(&self) -> Style {
        self.styles.last().copied().unwrap_or_default()
    }

    fn push_style(&mut self, modifier: Modifier) {
        self.styles.push(self.style().add_modifier(modifier));
    }

    /// Start a block of its own: a blank line between it and whatever came
    /// before, and a fresh line for it.
    fn open_block(&mut self) {
        self.separate();
        let indent = self.indent();
        self.current = Some(Line {
            lead: indent.clone(),
            hang: indent,
            spans: Vec::new(),
            verbatim: false,
        });
    }

    /// End the line being written, and set a blank line between it and the
    /// block to come — unless nothing has been written yet.
    fn separate(&mut self) {
        self.close();
        if !self.lines.is_empty() {
            let indent = self.indent();
            self.lines.push(Line {
                lead: indent.trim_end().to_string(),
                hang: indent,
                spans: Vec::new(),
                verbatim: false,
            });
        }
    }

    /// Start an item: its marker on the first row, and every row after it
    /// hanging under the text rather than the marker.
    fn open_item(&mut self) {
        self.close();
        let marker = match self.lists.last_mut() {
            Some(List { next: Some(n) }) => {
                *n += 1;
                format!("{}. ", *n - 1)
            }
            _ => BULLET.to_string(),
        };
        let lead = format!("{}{marker}", self.indent());
        self.prefixes.push(" ".repeat(columns_of(&marker)));
        self.current = Some(Line {
            lead,
            hang: self.indent(),
            spans: Vec::new(),
            verbatim: false,
        });
        self.fresh = true;
    }

    /// End the line being written, where there is one.
    fn close(&mut self) {
        if let Some(line) = self.current.take() {
            self.lines.push(line);
        }
        self.fresh = false;
    }

    /// End the line being written and carry on under it, laid out the same
    /// way.
    fn break_line(&mut self) {
        let (hang, verbatim) = {
            let line = self.line();
            (line.hang.clone(), line.verbatim)
        };
        self.close();
        self.current = Some(Line {
            lead: hang.clone(),
            hang,
            spans: Vec::new(),
            verbatim,
        });
    }

    /// The line being written, opened if nothing is.
    fn line(&mut self) -> &mut Line {
        let indent = self.indent();
        self.current.get_or_insert_with(|| Line {
            lead: indent.clone(),
            hang: indent,
            spans: Vec::new(),
            verbatim: false,
        })
    }

    fn say(&mut self, said: &str) {
        let style = self.style();
        self.say_in(said, style);
    }

    /// `said` in `style`, a line break in it a row break on the screen. A
    /// line break at the very end closes the line without opening an empty
    /// one under it, which is how a code block's last line ends.
    fn say_in(&mut self, said: &str, style: Style) {
        let lines: Vec<&str> = said.split('\n').collect();
        for (n, line) in lines.iter().enumerate() {
            let last = n + 1 == lines.len();
            if n > 0 && !(last && line.is_empty()) {
                self.break_line();
            }
            if !line.is_empty() {
                self.line()
                    .spans
                    .push(Span::styled(line.to_string(), style));
                self.fresh = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use ratatui::style::{Color, Modifier};

    /// The words of each row, for a test that is not about style.
    fn words(rows: &[Vec<Span<'static>>]) -> Vec<String> {
        rows.iter()
            .map(|row| row.iter().map(|span| span.content.as_ref()).collect())
            .collect()
    }

    /// Text with no markdown in it draws as it always has: wrapped at the
    /// spaces, a line break in the source a row break on the screen, and a
    /// blank line a blank row.
    #[test]
    fn plain_text_wraps_at_a_space_and_keeps_its_line_breaks_and_blank_lines() {
        assert_eq!(
            words(&rows("one two three\nfour\n\nfive", 9)),
            ["one two", "three", "four", "", "five"]
        );
    }

    /// The style the word was drawn in, wherever it is on the screen.
    fn style_of(rows: &[Vec<Span<'static>>], word: &str) -> Style {
        rows.iter()
            .flatten()
            .find(|span| span.content == word)
            .unwrap_or_else(|| panic!("{word:?} is drawn: {rows:#?}"))
            .style
    }

    /// A word exactly as wide as the row is a word that fits, not one to
    /// break.
    #[test]
    fn a_word_exactly_as_wide_as_the_row_is_not_broken() {
        assert_eq!(words(&rows("abcd", 4)), ["abcd"]);
        assert_eq!(words(&rows("abcd efgh", 4)), ["abcd", "efgh"]);
    }

    /// A word wider than the window is the one thing that cannot wrap at a
    /// space, and it is broken between glyphs rather than lost.
    #[test]
    fn a_word_wider_than_the_window_is_broken_rather_than_lost() {
        assert_eq!(words(&rows("abcdefghij", 4)), ["abcd", "efgh", "ij"]);
    }

    /// A word whose style changes part way through is still one word: it
    /// wraps as a whole, never at the change of style.
    #[test]
    fn a_word_styled_in_parts_wraps_as_one_word() {
        assert_eq!(words(&rows("aa **bb**cc dd", 6)), ["aa", "bbcc", "dd"]);
    }

    /// A word styled in parts that is too wide for the row is broken where
    /// the row runs out, even when that is exactly where its style changes.
    #[test]
    fn a_word_styled_in_parts_is_broken_where_the_row_runs_out() {
        assert_eq!(words(&rows("**ab**cd", 2)), ["ab", "cd"]);
    }

    /// A heading is set apart: its marks are gone, it is bold, and it stands
    /// clear of the prose either side of it.
    #[test]
    fn a_heading_is_bold_without_its_marks_and_stands_clear_of_the_prose() {
        let rows = rows("said\n## Shape\nprose", 20);

        assert_eq!(words(&rows), ["said", "", "Shape", "", "prose"]);
        assert!(style_of(&rows, "Shape")
            .add_modifier
            .contains(Modifier::BOLD));
        assert!(!style_of(&rows, "prose")
            .add_modifier
            .contains(Modifier::BOLD));
    }

    /// A bullet is `bd show`'s own, and an item too wide for a row hangs
    /// under its text rather than under its bullet.
    #[test]
    fn a_list_item_has_a_bullet_and_hangs_under_its_text() {
        assert_eq!(
            words(&rows("- one two three\n- four", 9)),
            ["• one two", "  three", "• four"]
        );
    }

    /// An item written over two lines of source is one item, its second
    /// line under its text.
    #[test]
    fn an_item_broken_over_two_source_lines_hangs_under_its_text() {
        assert_eq!(words(&rows("- one\n  two", 9)), ["• one", "  two"]);
    }

    /// An item with a paragraph of its own under it keeps that paragraph
    /// under its text, a blank row between.
    #[test]
    fn an_items_second_paragraph_sits_under_its_text() {
        assert_eq!(words(&rows("- a\n\n  b", 9)), ["• a", "", "  b"]);
    }

    #[test]
    fn a_nested_list_is_indented_under_its_item() {
        assert_eq!(words(&rows("- a\n  - b", 10)), ["• a", "  • b"]);
    }

    #[test]
    fn a_numbered_list_keeps_its_numbers() {
        assert_eq!(words(&rows("3. a\n4. b", 10)), ["3. a", "4. b"]);
    }

    /// A code span is in a tone of its own, and the words round it are not.
    #[test]
    fn a_code_span_is_drawn_in_its_own_tone() {
        let rows = rows("see `wrap` here", 20);

        assert_eq!(words(&rows), ["see wrap here"]);
        assert_eq!(style_of(&rows, "wrap").fg, Some(Color::Cyan));
        assert_eq!(style_of(&rows, "see").fg, None);
    }

    #[test]
    fn emphasis_is_italic() {
        let rows = rows("a *soft* word", 20);

        assert_eq!(words(&rows), ["a soft word"]);
        assert!(style_of(&rows, "soft")
            .add_modifier
            .contains(Modifier::ITALIC));
        assert!(!style_of(&rows, "word")
            .add_modifier
            .contains(Modifier::ITALIC));
    }

    #[test]
    fn strong_emphasis_is_bold() {
        let rows = rows("a **hard** word", 20);

        assert_eq!(words(&rows), ["a hard word"]);
        assert!(style_of(&rows, "hard")
            .add_modifier
            .contains(Modifier::BOLD));
        assert!(!style_of(&rows, "word")
            .add_modifier
            .contains(Modifier::BOLD));
    }

    /// A code block is drawn line for line, in the code tone.
    #[test]
    fn a_code_block_keeps_its_lines_in_the_code_tone() {
        let rows = rows("said\n\n```\nlet x = 1;\nlet y;\n```", 20);

        assert_eq!(words(&rows), ["said", "", "let x = 1;", "let y;"]);
        assert_eq!(style_of(&rows, "let x = 1;").fg, Some(Color::Cyan));
    }

    /// Code is laid out by its author, so a code block keeps every space it
    /// was written with: its indentation, and the gaps that line things up.
    #[test]
    fn a_code_block_keeps_its_indentation_and_spacing() {
        assert_eq!(
            words(&rows("```\nif x {\n    run();\n}\n```", 20)),
            ["if x {", "    run();", "}"]
        );
        assert_eq!(words(&rows("```\nname   value\n```", 20)), ["name   value"]);
    }

    /// A line of code wider than the window is broken between glyphs, spaces
    /// and all, rather than re-wrapped at its spaces.
    /// A blank line inside a code block is a blank row, and the block's own
    /// last line break does not open an empty row under it.
    #[test]
    fn a_code_block_keeps_a_blank_line_and_ends_without_one() {
        assert_eq!(
            words(&rows("```\nfn a() {}\n\nfn b() {}\n```\nafter", 20)),
            ["fn a() {}", "", "fn b() {}", "", "after"]
        );
    }

    #[test]
    fn a_wide_line_of_code_is_broken_between_glyphs_with_its_spaces_kept() {
        assert_eq!(words(&rows("```\na  b  c\n```", 4)), ["a  b", "  c"]);
    }

    /// A quote keeps its bar, on every row of it.
    #[test]
    fn a_block_quote_is_barred_on_every_row() {
        assert_eq!(words(&rows("> one two three", 9)), ["│ one two", "│ three"]);
    }

    /// A link's destination is text the author wrote, and it is kept.
    #[test]
    fn a_link_keeps_its_destination() {
        assert_eq!(
            words(&rows("[bd](https://x.y) here", 30)),
            ["bd (https://x.y) here"]
        );
    }

    #[test]
    fn a_rule_is_a_row_of_line() {
        assert_eq!(
            words(&rows("a\n\n---\n\nb", 6)),
            ["a", "", "──────", "", "b"]
        );
    }

    /// A renderer that drops text is worse than none: what is not markdown,
    /// or is broken markdown, is drawn as written.
    #[test]
    fn what_is_not_markdown_is_drawn_as_written() {
        for said in [
            "**unclosed and <path> and a | b",
            "bd -C <path> --readonly show <id>",
            "~~struck~~ and snake_case_name and 5 * 3 * 2",
            "<b>tag</b> and a&b and &c",
        ] {
            assert_eq!(words(&rows(said, 60)), [said], "{said:?}");
        }
    }
}
