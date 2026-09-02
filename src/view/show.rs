//! The bead view: the selected bead shown whole, as `bd show` shows it, in a
//! window over the forest.

use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Clear};
use ratatui::Frame;

use crate::model::edges::Related;
use crate::model::snapshot::Node;
use crate::model::types::Edge;
use crate::view::fitted::{indent, Fitted};
use crate::view::forest::Forest;
use crate::view::phrase;
use crate::view::row::{agent_marker, status_glyph};
use crate::view::Motion;

/// The section names `bd show` prints, verbatim, in the order it prints
/// them. Terminology comes from beads, and a heading is terminology.
const DESCRIPTION: &str = "DESCRIPTION";
const NOTES: &str = "NOTES";
const PARENT: &str = "PARENT";
const DEPENDS_ON: &str = "DEPENDS ON";
const BLOCKS: &str = "BLOCKS";

/// `bd show`'s own arrows: up to the parent, out to what a bead waits on,
/// back from what waits on it.
const UP: char = '↑';
const OUT: char = '→';
const BACK: char = '←';

/// The widest the window is drawn, inside its border. `bd show` wraps its
/// own prose at about this, and a line of prose much wider than it is hard
/// to read back from the end of to the start of the next.
const WIDE: u16 = 80;

/// The rows a bordered window spends on its own edges.
const BORDERS: u16 = 2;

/// Where the bead view is looking: how far down the bead it has scrolled,
/// and how much of it the last frame had room for.
///
/// The room and the total are the frame's to say, and a motion reads them
/// off the last frame drawn — the same arrangement the forest has with its
/// half-screen. A frame is always drawn before a key is answered, so the
/// first motion never reads a window nobody has measured.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Show {
    from: usize,
    room: usize,
    total: usize,
}

impl Show {
    /// Move the view by one motion, reporting whether what it shows changed.
    ///
    /// The last row of the bead is as far as it goes: scrolled past it the
    /// window would hold nothing, and a motion that moved nowhere is not a
    /// change the screen needs redrawing for.
    pub fn scroll(&mut self, motion: Motion) -> bool {
        let furthest = self.total.saturating_sub(self.room);
        let half = (self.room / 2).max(1);
        let to = match motion {
            Motion::PreviousRow => self.from.saturating_sub(1),
            Motion::NextRow => self.from + 1,
            Motion::HalfScreenUp => self.from.saturating_sub(half),
            Motion::HalfScreenDown => self.from + half,
            Motion::FirstRow => 0,
            Motion::LastRow => furthest,
        }
        .min(furthest);
        let moved = to != self.from;
        self.from = to;
        moved
    }

    /// Take the measure of the frame just drawn, so the next motion knows how
    /// far it can go — and come back inside the bead where a frame shorter
    /// than the last has left the view past its end.
    fn fit(&mut self, total: usize, room: usize) {
        self.total = total;
        self.room = room;
        self.from = self.from.min(total.saturating_sub(room));
    }

    /// Whether the last frame was too short for the whole bead.
    fn scrolls(&self) -> bool {
        self.total > self.room
    }
}

/// The bead the selection is on, where it is on one.
///
/// A bead's row and a tree's header both stand for a bead; a project's
/// line, a group and a thing in one stand for none. A root whose tree would
/// not read carries its key and no bead behind it, so it answers none too:
/// there is nothing of it to show.
pub fn selected(forest: &Forest) -> Option<&Node> {
    forest
        .lines()
        .get(forest.selected_line())?
        .bead()
        .and_then(|key| forest.snapshot().node(key))
}

/// Where the window sits: as wide as the screen up to `WIDE` inside the
/// border, as tall as the bead up to the screen, centred over the forest.
fn show_window(area: Rect, rows: usize) -> Rect {
    let height = u16::try_from(rows)
        .unwrap_or(u16::MAX)
        .saturating_add(BORDERS);
    area.centered(
        Constraint::Length(WIDE + BORDERS),
        Constraint::Length(height),
    )
}

/// The bead, one screen row at a time, in `bd show`'s order: the row's own
/// facts, the agent the join put on it, then each section the bead has
/// something in, under the name `bd show` gives it. Prose is wrapped to
/// `width`; every other row is cut to it when drawn, as a row of the forest
/// is.
pub fn said(node: &Node, width: usize) -> Vec<Vec<Span<'static>>> {
    let mut rows = vec![vec![
        Span::raw(format!("{} {}", status_glyph(&node.status), node.id)),
        Span::raw(indent()),
        Span::raw(node.title.clone()),
    ]];

    let mut facts = vec![
        phrase::status_word(&node.status),
        format!("P{}", node.priority),
        node.issue_type.clone(),
    ];
    facts.extend(node.owner.clone());
    rows.push(plain(facts.join(" · ")));
    if let Some(agent) = &node.agent {
        rows.push(plain(agent_marker(agent)));
    }

    let room = width.saturating_sub(indent().len());
    let prose = |text: &str| wrap(text, room).into_iter().map(plain).collect::<Vec<_>>();
    let tied = |arrow: char, related: &[Related]| {
        related
            .iter()
            .map(|related| plain(format!("{arrow} {}", related_row(related))))
            .collect::<Vec<_>>()
    };

    for (heading, body) in [
        (DESCRIPTION, prose(&node.description)),
        (NOTES, prose(&node.notes)),
        (PARENT, tied(UP, node.parent.as_slice())),
        (DEPENDS_ON, tied(OUT, &node.depends_on)),
        (BLOCKS, tied(BACK, &node.blocks)),
    ] {
        if body.is_empty() {
            continue;
        }
        rows.push(Vec::new());
        rows.push(vec![Span::styled(
            heading,
            Style::new().add_modifier(Modifier::BOLD),
        )]);
        rows.extend(body);
    }

    rows
}

/// One row indented under a heading.
fn plain(said: String) -> Vec<Span<'static>> {
    vec![Span::raw(format!("{}{said}", indent()))]
}

/// A related bead as `bd show` lists one: its glyph, id and title, and the
/// kind of edge where the arrow alone would not say. A bead the answer does
/// not hold has no glyph and no title, and says so in their place.
fn related_row(related: &Related) -> String {
    let Some(status) = &related.status else {
        return format!("{}{}{}", related.id, indent(), phrase::not_in_the_answer());
    };
    let mut said = format!(
        "{} {}{}{}",
        status_glyph(status),
        related.id,
        indent(),
        related.title.as_deref().unwrap_or_default()
    );
    if let Edge::Other(kind) = &related.edge {
        said.push_str(&format!(" · {}", phrase::edge_kind(kind)));
    }
    said
}

/// Draw the bead in a window over the forest.
///
/// `Clear` blanks the window first, which is what stops the trees showing
/// through between the rows. The way back is the border's title, so a window
/// too short for a single row still holds it: a reader who cannot see how to
/// leave is stuck in a view they may have opened by accident. Where the bead
/// is taller than the window, the title says how to see the rest.
pub fn show(frame: &mut Frame, area: Rect, node: &Node, view: &mut Show) {
    let width = area.width.min(WIDE + BORDERS).saturating_sub(BORDERS) as usize;
    let rows = said(node, width);
    let window = show_window(area, rows.len());
    if window.is_empty() {
        return;
    }

    let block = Block::bordered();
    let inner = block.inner(window);
    view.fit(rows.len(), inner.height as usize);
    let block = block.title(Span::styled(
        phrase::way_back_from_bead(&node.id, view.scrolls()),
        Style::new().add_modifier(Modifier::BOLD),
    ));
    frame.render_widget(Clear, window);
    frame.render_widget(block, window);

    for (n, row) in rows
        .into_iter()
        .skip(view.from)
        .take(inner.height as usize)
        .enumerate()
    {
        frame.render_widget(
            Fitted::new(row, Vec::new(), Vec::new()),
            Rect {
                y: inner.y + n as u16,
                ..inner
            },
        );
    }
}

/// Prose wrapped to `width` columns, at the spaces. Each line of the text
/// keeps its own leading spaces on every row it wraps onto, so a list keeps
/// its shape; a blank line stays a blank line; and a word too wide for a row
/// is broken between glyphs rather than lost.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut rows = Vec::new();
    for line in text.lines() {
        let lead: String = line.chars().take_while(|c| *c == ' ').collect();
        let body = &line[lead.len()..];
        if body.is_empty() {
            rows.push(String::new());
            continue;
        }
        let room = width.saturating_sub(lead.len()).max(1);
        let mut row = String::new();
        for word in body.split_whitespace() {
            let word = if row.is_empty() {
                word.to_string()
            } else if columns_of(&row) + 1 + columns_of(word) <= room {
                row.push(' ');
                row.push_str(word);
                continue;
            } else {
                rows.push(format!("{lead}{row}"));
                word.to_string()
            };
            row = word;
            while columns_of(&row) > room {
                let (head, rest) = split_at_columns(&row, room);
                rows.push(format!("{lead}{head}"));
                row = rest;
            }
        }
        rows.push(format!("{lead}{row}"));
    }
    rows
}

/// What a piece of text takes up on screen, in columns.
fn columns_of(text: &str) -> usize {
    Span::raw(text).width()
}

/// `text` split after as many glyphs as fit in `limit` columns, never
/// inside one. At least one glyph goes in the head, so a glyph wider than
/// the limit is still drawn rather than looped over for ever.
fn split_at_columns(text: &str, limit: usize) -> (String, String) {
    let mut head = String::new();
    let mut used = 0;
    let mut glyphs = text.chars();
    for glyph in glyphs.by_ref() {
        let width = columns_of(&glyph.to_string());
        if !head.is_empty() && used + width > limit {
            let rest: String = std::iter::once(glyph).chain(glyphs).collect();
            return (head, rest);
        }
        used += width;
        head.push(glyph);
    }
    (head, String::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::edges::Related;
    use crate::model::join::{AgentRef, JoinSource};
    use crate::model::types::{Edge, PaneStatus, Status};
    use crate::view::painted::Painted;
    use pretty_assertions::assert_eq;

    fn related(id: &str, edge: Edge, status: Status, title: &str) -> Related {
        Related {
            id: id.to_string(),
            edge,
            status: Some(status),
            title: Some(title.to_string()),
        }
    }

    /// A bead with something in every section `bd show` prints.
    fn a_bead() -> Node {
        Node {
            id: "orb-7.1".to_string(),
            title: "re-point the dish".to_string(),
            status: Status::InProgress,
            issue_type: "task".to_string(),
            priority: 2,
            ready: false,
            blocked_by: Vec::new(),
            started_at: None,
            closed_at: None,
            badges: Vec::new(),
            agent: Some(AgentRef {
                pane: "w:p1".to_string(),
                pane_status: PaneStatus::Working,
                title: Some("lifting the mast".to_string()),
                source: JoinSource::AgentPane,
            }),
            anomalies: Vec::new(),
            description: "Point it at the new bird.\n\nThe old one is gone.".to_string(),
            notes: "The crane is booked for Tuesday.".to_string(),
            owner: Some("kim".to_string()),
            parent: Some(related(
                "orb-7",
                Edge::ParentChild,
                Status::InProgress,
                "lift the ground station",
            )),
            depends_on: vec![related(
                "orb-7.3",
                Edge::Blocks,
                Status::Closed,
                "lay the feeder cable",
            )],
            blocks: vec![related(
                "orb-7.4",
                Edge::Blocks,
                Status::Open,
                "file the licence",
            )],
        }
    }

    fn drawn(node: &Node, view: &mut Show, width: u16, height: u16) -> Vec<String> {
        Painted::drawn_by(width, height, |frame| show(frame, frame.area(), node, view))
            .rows()
            .into_iter()
            .map(|row| row.trim_end().to_string())
            .collect()
    }

    /// The whole view, character for character: `bd show`'s sections, in its
    /// order and under its names, with the agent from the join beside the
    /// row's own facts, and the way back in the title.
    #[test]
    fn the_bead_is_shown_as_bd_show_shows_it() {
        assert_eq!(
            drawn(&a_bead(), &mut Show::default(), 44, 22),
            vec![
                "┌orb-7.1 · Esc to go back──────────────────┐",
                "│◐ orb-7.1  re-point the dish              │",
                "│  in_progress · P2 · task · kim           │",
                "│  ◍ lifting the mast · working            │",
                "│                                          │",
                "│DESCRIPTION                               │",
                "│  Point it at the new bird.               │",
                "│                                          │",
                "│  The old one is gone.                    │",
                "│                                          │",
                "│NOTES                                     │",
                "│  The crane is booked for Tuesday.        │",
                "│                                          │",
                "│PARENT                                    │",
                "│  ↑ ◐ orb-7  lift the ground station      │",
                "│                                          │",
                "│DEPENDS ON                                │",
                "│  → ✓ orb-7.3  lay the feeder cable       │",
                "│                                          │",
                "│BLOCKS                                    │",
                "│  ← ○ orb-7.4  file the licence           │",
                "└──────────────────────────────────────────┘",
            ]
        );
    }

    /// `bd show` prints nothing for a section the bead has nothing in, and a
    /// heading over nothing would be a claim that something was lost.
    #[test]
    fn sections_the_bead_has_nothing_for_are_left_out() {
        let bare = Node {
            agent: None,
            description: String::new(),
            notes: String::new(),
            owner: None,
            parent: None,
            depends_on: Vec::new(),
            blocks: Vec::new(),
            ..a_bead()
        };

        assert_eq!(
            drawn(&bare, &mut Show::default(), 44, 4),
            vec![
                "┌orb-7.1 · Esc to go back──────────────────┐",
                "│◐ orb-7.1  re-point the dish              │",
                "│  in_progress · P2 · task                 │",
                "└──────────────────────────────────────────┘",
            ]
        );
    }

    /// The description in full is the point of the view, so it wraps to the
    /// window rather than being cut at it like a row is. The cut mark is
    /// what a row leaves, and nothing here may leave one on prose.
    #[test]
    fn the_description_wraps_to_the_window_rather_than_being_cut() {
        let long = Node {
            description: "one two three four five six seven eight nine ten eleven twelve"
                .to_string(),
            ..a_bead()
        };
        let rows = drawn(&long, &mut Show::default(), 30, 30);

        let wrapped: Vec<&str> = rows
            .iter()
            .skip_while(|row| !row.starts_with("│DESCRIPTION"))
            .skip(1)
            .take(3)
            .map(|row| row.trim_matches(['│', ' ']))
            .collect();
        assert_eq!(
            wrapped,
            [
                "one two three four five",
                "six seven eight nine ten",
                "eleven twelve"
            ],
            "{rows:#?}"
        );
        assert!(
            !wrapped.iter().any(|row| row.contains('…')),
            "prose is wrapped, never cut: {rows:#?}"
        );
    }

    /// A row of the view that is not prose — the bead's own line, a related
    /// bead's — is one row whatever its length, cut the way a row of the
    /// forest is.
    #[test]
    fn a_line_that_is_not_prose_is_cut_rather_than_wrapped() {
        let rows = drawn(&a_bead(), &mut Show::default(), 24, 4);

        assert_eq!(rows[1], "│◐ orb-7.1  re-point t…│");
    }

    /// A window too short for the whole bead shows the top of it and says
    /// how to see the rest, and a motion moves it a row at a time.
    #[test]
    fn a_window_too_short_for_the_bead_scrolls_by_motion() {
        let mut view = Show::default();
        let top = drawn(&a_bead(), &mut view, 44, 6);
        assert_eq!(top[0], "┌orb-7.1 · Esc to go back · j, k to scroll─┐");
        assert_eq!(top[1], "│◐ orb-7.1  re-point the dish              │");

        assert!(!view.scroll(Motion::PreviousRow), "already at the top");
        assert!(view.scroll(Motion::NextRow));
        let down_one = drawn(&a_bead(), &mut view, 44, 6);
        assert_eq!(down_one[1], "│  in_progress · P2 · task · kim           │");

        assert!(view.scroll(Motion::LastRow));
    }

    /// The last row of the bead is the furthest the view goes: scrolled past
    /// it there would be nothing on the screen, and a motion that moved the
    /// view nowhere is one the screen need not be redrawn for.
    #[test]
    fn the_view_stops_at_the_last_row_of_the_bead() {
        let mut view = Show::default();
        drawn(&a_bead(), &mut view, 44, 6);

        assert!(view.scroll(Motion::LastRow));
        let bottom = drawn(&a_bead(), &mut view, 44, 6);
        assert_eq!(bottom[4], "│  ← ○ orb-7.4  file the licence           │");
        assert!(!view.scroll(Motion::NextRow), "nothing below the last row");
        assert!(!view.scroll(Motion::LastRow), "already there");

        assert!(view.scroll(Motion::HalfScreenUp));
        assert!(view.scroll(Motion::FirstRow));
        assert_eq!(
            drawn(&a_bead(), &mut view, 44, 6)[1],
            "│◐ orb-7.1  re-point the dish              │"
        );
    }

    /// A bead that fits has nowhere to scroll to, and the title does not
    /// offer a motion that would do nothing.
    #[test]
    fn a_bead_that_fits_the_window_neither_scrolls_nor_says_it_does() {
        let mut view = Show::default();
        drawn(&a_bead(), &mut view, 44, 24);

        assert!(!view.scroll(Motion::NextRow));
        assert!(!view.scroll(Motion::LastRow));
    }

    /// A reader who cannot see how to leave is stuck in a view they may have
    /// opened by accident, so the way back is the one row that survives
    /// every cut.
    #[test]
    fn the_way_back_is_the_windows_title_however_short_the_screen() {
        for height in [1, 2, 3, 8] {
            let rows = drawn(&a_bead(), &mut Show::default(), 44, height);
            assert!(
                rows[0].contains("Esc to go back"),
                "at {height} rows: {:?}",
                rows[0]
            );
        }
    }

    /// Degrade, never disappear: a dependency on a bead the tracker no longer
    /// holds is still listed, with the one thing the answer had of it.
    #[test]
    fn a_related_bead_the_answer_does_not_hold_is_named_as_such() {
        let dangling = Node {
            depends_on: vec![Related {
                id: "orb-9".to_string(),
                edge: Edge::Blocks,
                status: None,
                title: None,
            }],
            ..a_bead()
        };
        let rows = drawn(&dangling, &mut Show::default(), 50, 24);

        assert!(
            rows.contains(&"│  → orb-9  not in the tracker's answer          │".to_string()),
            "{rows:#?}"
        );
    }

    /// An edge of a kind beads may add later is listed with what it waits
    /// on, and says which kind it is, since the arrow alone reads as blocks.
    #[test]
    fn an_edge_of_a_kind_bdi_does_not_know_says_which_kind() {
        let odd = Node {
            depends_on: vec![related(
                "orb-2",
                Edge::Other("relates-to".to_string()),
                Status::Open,
                "the survey",
            )],
            ..a_bead()
        };
        let rows = drawn(&odd, &mut Show::default(), 50, 24);

        assert!(
            rows.contains(&"│  → ○ orb-2  the survey · “relates-to”          │".to_string()),
            "{rows:#?}"
        );
    }

    /// `^D` and `^U` move the view half the window, so a reader paging
    /// through a long bead lands where they expect: two rows down a
    /// four-row window, not four and not one.
    #[test]
    fn a_half_screen_motion_moves_the_view_half_the_window() {
        let mut view = Show::default();
        drawn(&a_bead(), &mut view, 44, 6);

        assert!(view.scroll(Motion::HalfScreenDown));
        assert_eq!(
            drawn(&a_bead(), &mut view, 44, 6)[1],
            "│  ◍ lifting the mast · working            │"
        );
        assert!(view.scroll(Motion::HalfScreenDown));
        assert_eq!(
            drawn(&a_bead(), &mut view, 44, 6)[1],
            "│DESCRIPTION                               │"
        );
        assert!(view.scroll(Motion::HalfScreenUp));
        assert_eq!(
            drawn(&a_bead(), &mut view, 44, 6)[1],
            "│  ◍ lifting the mast · working            │"
        );
    }

    /// The window is never wider than `bd show`'s own prose: on a wide
    /// screen it stops at eighty columns inside its border, centred, and the
    /// prose wraps there rather than at the screen's edge.
    #[test]
    fn on_a_wide_screen_the_window_stops_at_eighty_columns_inside_its_border() {
        let long = Node {
            description: "abcde ".repeat(30).trim().to_string(),
            ..a_bead()
        };
        let rows = drawn(&long, &mut Show::default(), 100, 30);

        let top = rows
            .iter()
            .find(|row| row.contains('┌'))
            .expect("the window's top edge is drawn");
        assert_eq!(top.chars().nth(9), Some('┌'), "{top:?}");
        assert_eq!(top.chars().nth(90), Some('┐'), "{top:?}");
        let first = rows
            .iter()
            .find(|row| row.contains("abcde"))
            .expect("the prose is drawn");
        assert_eq!(
            first.matches("abcde").count(),
            13,
            "thirteen words of five, with a space between, is seventy-seven of the \
             seventy-eight columns left inside the indent: {first:?}"
        );
        assert!(
            !first.contains('…'),
            "prose wrapped to the window is never cut at it: {first:?}"
        );
    }

    // ---- wrapping ----------------------------------------------------------

    /// A word exactly as wide as the row is a word that fits, not one to
    /// break.
    #[test]
    fn a_word_exactly_as_wide_as_the_row_is_not_broken() {
        assert_eq!(wrap("abcd", 4), ["abcd"]);
        assert_eq!(wrap("abcd efgh", 4), ["abcd", "efgh"]);
    }

    #[test]
    fn prose_wraps_at_a_space_and_keeps_its_blank_lines() {
        assert_eq!(
            wrap("one two three\n\nfour", 9),
            ["one two", "three", "", "four"]
        );
    }

    /// A list in a description keeps its shape: the indent a line was
    /// written with is the indent every row it wraps onto gets.
    #[test]
    fn an_indented_line_keeps_its_indent_on_every_row_it_wraps_onto() {
        assert_eq!(
            wrap("  one two three four", 12),
            ["  one two", "  three four"]
        );
    }

    /// A word wider than the window is the one thing that cannot wrap at a
    /// space, and it is broken between glyphs rather than lost.
    #[test]
    fn a_word_wider_than_the_window_is_broken_rather_than_lost() {
        assert_eq!(wrap("abcdefghij", 4), ["abcd", "efgh", "ij"]);
    }

    #[test]
    fn a_window_with_no_room_inside_it_draws_nothing_inside_it() {
        assert_eq!(
            drawn(&a_bead(), &mut Show::default(), 44, 2),
            vec![
                "┌orb-7.1 · Esc to go back · j, k to scroll─┐",
                "└──────────────────────────────────────────┘",
            ]
        );
    }
}
