//! The lines the forest has drawn, addressed by row.
//!
//! Every reader of the forest asks one of a few things of its lines: how many
//! there are, which line is on a row, which row a handle is on, the rows a
//! band has room for, and how wide the widest id is. They ask here rather
//! than of a slice, so that what stands behind the questions can change
//! without the questions moving.

use std::ops::Index;

use ratatui::text::Span;

use crate::view::fitted::columns;
use crate::view::lines::{Content, Line};

use super::handle::{handle_of, Handle};

/// One snapshot's lines in render order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Drawn(Vec<Line>);

#[allow(clippy::len_without_is_empty)]
impl Drawn {
    pub(super) fn new(lines: Vec<Line>) -> Self {
        Self(lines)
    }

    /// How many rows the forest draws.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// The line on a row, where the forest draws that many.
    pub fn get(&self, row: usize) -> Option<&Line> {
        self.0.get(row)
    }

    /// Every line, in render order.
    pub fn iter(&self) -> impl Iterator<Item = &Line> {
        self.0.iter()
    }

    /// The rows a band starting at `from` with room for `height` lines shows,
    /// each with its row.
    pub fn viewport(&self, from: usize, height: usize) -> impl Iterator<Item = (usize, &Line)> {
        self.0.iter().enumerate().skip(from).take(height)
    }

    /// The widest abbreviated id drawn, so every title starts in the same
    /// column and a reader's eye runs down one edge rather than a ragged one.
    pub fn id_width(&self) -> usize {
        self.0
            .iter()
            .filter_map(|line| match &line.content {
                Content::Bead(row) => Some(columns(&[Span::raw(row.id.clone())])),
                _ => None,
            })
            .max()
            .unwrap_or(0)
    }

    /// Which row carries a handle, where one does. A bead reachable more than
    /// once is drawn once per way down to it, and the handle names the way
    /// down, so at most one row carries it.
    pub(super) fn row_of(&self, handle: &Handle) -> Option<usize> {
        self.0
            .iter()
            .position(|line| handle_of(line).as_ref() == Some(handle))
    }
}

impl<'a> IntoIterator for &'a Drawn {
    type Item = &'a Line;
    type IntoIter = std::slice::Iter<'a, Line>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl Index<usize> for Drawn {
    type Output = Line;

    fn index(&self, row: usize) -> &Line {
        &self.0[row]
    }
}
