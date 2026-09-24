//! What has been typed into the search prompt, and where the cursor stands in
//! it.

use ratatui::text::Span;

use crate::view::fitted::columns;
use crate::view::Edit;

/// The search prompt's query and its cursor.
///
/// The cursor is a byte offset that always sits on a character boundary, so
/// the text either side of it can be sliced off without a check.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    typed: String,
    cursor: usize,
}

/// What an edit came to: whether the query is different, or only the cursor
/// has moved, or neither.
///
/// A search is for the query, so only a changed one is searched afresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edited {
    Changed,
    Moved,
    Nothing,
}

impl Query {
    pub fn typed(&self) -> &str {
        &self.typed
    }

    /// The columns the query takes up before the cursor, which is how far
    /// into it the cursor is drawn.
    pub fn columns_before_cursor(&self) -> usize {
        columns(&[Span::raw(self.before().to_string())])
    }

    pub fn edit(&mut self, edit: Edit) -> Edited {
        let (was, cursor) = (self.typed.clone(), self.cursor);
        match edit {
            Edit::Character(glyph) => {
                self.typed.insert(self.cursor, glyph);
                self.cursor += glyph.len_utf8();
            }
            Edit::Back => self.cursor = self.previous(),
            Edit::Forward => self.cursor = self.next(),
            Edit::Start => self.cursor = 0,
            Edit::End => self.cursor = self.typed.len(),
            Edit::RubOut => self.rub_out_to(self.previous()),
            Edit::Delete => self.typed.replace_range(self.cursor..self.next(), ""),
            Edit::RubOutWord => self.rub_out_to(self.word_start()),
            Edit::RubOutToStart => self.rub_out_to(0),
            Edit::DeleteToEnd => self.typed.truncate(self.cursor),
        }
        if self.typed != was {
            Edited::Changed
        } else if self.cursor != cursor {
            Edited::Moved
        } else {
            Edited::Nothing
        }
    }

    fn before(&self) -> &str {
        &self.typed[..self.cursor]
    }

    fn rub_out_to(&mut self, from: usize) {
        self.typed.replace_range(from..self.cursor, "");
        self.cursor = from;
    }

    /// Where the character before the cursor starts.
    fn previous(&self) -> usize {
        self.before()
            .char_indices()
            .next_back()
            .map_or(0, |(at, _)| at)
    }

    /// Where the character at the cursor ends.
    fn next(&self) -> usize {
        self.typed[self.cursor..]
            .chars()
            .next()
            .map_or(self.cursor, |glyph| self.cursor + glyph.len_utf8())
    }

    /// Where the word before the cursor starts, past any spaces between it
    /// and the cursor.
    fn word_start(&self) -> usize {
        self.before()
            .trim_end_matches(' ')
            .rfind(' ')
            .map_or(0, |space| space + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// A query with `typed` in it and the cursor at its end, as typing it
    /// leaves one.
    fn typed(text: &str) -> Query {
        let mut query = Query::default();
        for glyph in text.chars() {
            query.edit(Edit::Character(glyph));
        }
        query
    }

    /// `query` drawn as the reader sees it, with `|` where the cursor is.
    fn drawn(query: &Query) -> String {
        format!(
            "{}|{}",
            &query.typed[..query.cursor],
            &query.typed[query.cursor..]
        )
    }

    fn after(mut query: Query, edits: &[Edit]) -> Query {
        for edit in edits {
            query.edit(*edit);
        }
        query
    }

    #[test]
    fn a_character_goes_in_at_the_cursor() {
        let query = after(typed("bd-1"), &[Edit::Start, Edit::Character('x')]);
        assert_eq!(drawn(&query), "x|bd-1");

        let query = after(
            typed("bd-1"),
            &[Edit::Back, Edit::Back, Edit::Character('x')],
        );
        assert_eq!(drawn(&query), "bdx|-1");
    }

    #[test]
    fn start_and_end_move_the_cursor_to_either_end_of_the_query() {
        assert_eq!(drawn(&after(typed("bd-1"), &[Edit::Start])), "|bd-1");
        assert_eq!(
            drawn(&after(typed("bd-1"), &[Edit::Start, Edit::End])),
            "bd-1|"
        );
    }

    #[test]
    fn back_and_forward_move_the_cursor_one_character() {
        assert_eq!(drawn(&after(typed("bd-1"), &[Edit::Back])), "bd-|1");
        assert_eq!(
            drawn(&after(typed("bd-1"), &[Edit::Start, Edit::Forward])),
            "b|d-1"
        );
    }

    /// A character several bytes long is still one step.
    #[test]
    fn a_step_is_one_character_however_many_bytes_it_takes() {
        let query = after(typed("aé"), &[Edit::Back]);
        assert_eq!(drawn(&query), "a|é");
        assert_eq!(query.columns_before_cursor(), 1);

        let query = after(query, &[Edit::Delete]);
        assert_eq!(drawn(&query), "a|");
    }

    #[test]
    fn rub_out_takes_the_character_before_the_cursor() {
        let query = after(typed("bd-1"), &[Edit::Back, Edit::RubOut]);
        assert_eq!(drawn(&query), "bd|1");
    }

    #[test]
    fn delete_takes_the_character_at_the_cursor() {
        let query = after(typed("bd-1"), &[Edit::Start, Edit::Delete]);
        assert_eq!(drawn(&query), "|d-1");
    }

    /// A word is as far back as a space, as a shell's `^W` takes it, so the
    /// whole of an id goes at once, and the spaces just before the cursor go
    /// with the word before them.
    #[test]
    fn rub_out_word_takes_back_to_the_space_before_the_word() {
        let query = after(typed("fix bd-1.2  "), &[Edit::RubOutWord]);
        assert_eq!(drawn(&query), "fix |");

        let query = after(query, &[Edit::RubOutWord]);
        assert_eq!(drawn(&query), "|");
    }

    #[test]
    fn rub_out_word_leaves_what_is_after_the_cursor() {
        let query = after(
            typed("fix bd-1"),
            &[Edit::Back, Edit::Back, Edit::RubOutWord],
        );
        assert_eq!(drawn(&query), "fix |-1");
    }

    #[test]
    fn rub_out_to_start_and_delete_to_end_take_either_side_of_the_cursor() {
        let at_the_dash = [Edit::Start, Edit::Forward, Edit::Forward];

        let query = after(
            typed("bd-1"),
            &[&at_the_dash[..], &[Edit::RubOutToStart]].concat(),
        );
        assert_eq!(drawn(&query), "|-1");

        let query = after(
            typed("bd-1"),
            &[&at_the_dash[..], &[Edit::DeleteToEnd]].concat(),
        );
        assert_eq!(drawn(&query), "bd|");
    }

    /// Only an edit that changes the query is searched afresh, so each says
    /// which it did — and one that does neither, at either end of the query,
    /// says so too.
    #[test]
    fn an_edit_says_whether_it_changed_the_query_or_only_moved_the_cursor() {
        let mut query = typed("bd");
        assert_eq!(query.edit(Edit::Back), Edited::Moved);
        assert_eq!(query.edit(Edit::Character('x')), Edited::Changed);
        assert_eq!(query.edit(Edit::Delete), Edited::Changed);

        let mut at_the_end = typed("bd");
        for edit in [Edit::Forward, Edit::End, Edit::Delete, Edit::DeleteToEnd] {
            assert_eq!(at_the_end.edit(edit), Edited::Nothing, "{edit:?}");
        }

        let mut at_the_start = after(typed("bd"), &[Edit::Start]);
        for edit in [
            Edit::Back,
            Edit::Start,
            Edit::RubOut,
            Edit::RubOutWord,
            Edit::RubOutToStart,
        ] {
            assert_eq!(at_the_start.edit(edit), Edited::Nothing, "{edit:?}");
        }
    }
}
