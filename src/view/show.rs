//! The bead view: the selected bead shown whole, as `bd show` shows it, in a
//! window over the forest.

use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Clear};
use ratatui::Frame;

use crate::model::edges::Related;
use crate::model::join::BeadKey;
use crate::model::snapshot::Node;
use crate::model::types::{Edge, Status};
use crate::view::draw::tone::{fg, status_style, DIM, LIVE, PAGE};
use crate::view::fitted::{indent, Fitted};
use crate::view::forest::Forest;
use crate::view::markdown;
use crate::view::phrase;
use crate::view::row::{agent_marker, status_glyph};
use crate::view::Motion;

/// `bd show`'s own colour for the id at the head of the page, read off `bd`
/// 1.2.2's output. Literal rather than named for the reason `tone.rs`'s
/// are: `bd` sends a 24-bit value that does not move with the theme.
const ID: Color = Color::Rgb(89, 194, 255);

/// The terminal's own foreground, held under the page's tone by the few
/// things meant to stand out from a page of text: a heading, and an arrow
/// that says the edge as the forest's box-drawing says the shape.
const STANDS_OUT: Style = Style::new().fg(Color::Reset);

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

/// The share of the screen the window takes on either side: four fifths,
/// so the forest still shows round it and a bigger terminal gets a bigger
/// window rather than the same box in the middle of a bigger forest.
const SHARE: (u16, u16) = (4, 5);

/// The rows a bordered window spends on its own edges.
const BORDERS: u16 = 2;

/// The least the window is offered across: eighty columns inside its
/// border, which is about where `bd show` wraps its own prose. Four fifths
/// of a small screen would be a cramped box for no gain.
const FLOOR_WIDTH: u16 = 80 + BORDERS;

/// The least the window is offered down: a classic terminal's twenty-four
/// rows, for the same reason.
const FLOOR_HEIGHT: u16 = 24;

/// `SHARE` of `screen` or `floor`, whichever is more, and never more than
/// the screen: a screen no bigger than the floor gives the window the whole
/// of itself.
fn offered(screen: u16, floor: u16) -> u16 {
    let share = u32::from(screen) * u32::from(SHARE.0) / u32::from(SHARE.1);
    u16::try_from(share)
        .unwrap_or(u16::MAX)
        .max(floor)
        .min(screen)
}

/// Where the bead view is looking: how far down the bead it has scrolled,
/// and how much of it the last frame had room for.
///
/// The room and the total are the frame's to say, and a motion reads them
/// off the last frame drawn — the same arrangement the forest has with its
/// half-screen. A frame is always drawn before a key is answered, so the
/// first motion never reads a window nobody has measured.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Show {
    from: usize,
    room: usize,
    total: usize,
    /// The bead this one names that the window is on, by its id, where the
    /// reader has moved to one.
    ///
    /// The bead rather than a row of the page, because a narrower window
    /// rewraps the prose above the sections and moves every row under it
    /// while moving none of the beads. And rather than an ordinal into
    /// `related`'s list, because a collection can rewrite what a bead names
    /// while the reader is still on it: an ordinal survives that and comes to
    /// stand for a different bead, so the ring moves without moving and
    /// `Enter` goes somewhere nobody pointed at. Which row it is drawn on is
    /// asked of the page each time it is drawn.
    on: Option<String>,
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

    /// The bead this one names that the window is on.
    pub fn on(&self) -> Option<&str> {
        self.on.as_deref()
    }

    /// Put the window on one of the beads this one names.
    pub fn go_to(&mut self, on: &str) {
        self.on = Some(on.to_string());
    }

    /// Take the window off whatever bead it was on, for a bead this one no
    /// longer names or can no longer be gone to.
    pub fn go_nowhere(&mut self) {
        self.on = None;
    }

    /// Bring `row` inside the window, where the last frame left it above or
    /// below what there was room for.
    fn reveal(&mut self, row: usize) {
        if row < self.from {
            self.from = row;
        } else if self.room > 0 && row >= self.from + self.room {
            self.from = row + 1 - self.room;
        }
    }
}

/// The bead the window moves to when the reader steps on from the one it is
/// on, out of the `count` beads this one names — coming round to the first
/// past the last, and passing over any the forest cannot take them to.
/// Nothing where none of them can be followed, which is the answer for a bead
/// that names none at all.
///
/// A step from nowhere lands on the first, which is where one would land
/// coming round from off the end.
pub fn stepped(
    from: Option<usize>,
    count: usize,
    followable: impl Fn(usize) -> bool,
) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let at = from.unwrap_or(count - 1);
    (1..=count)
        .map(|step| (at + step) % count)
        .find(|at| followable(*at))
}

/// Where a bead this one names sits in that list, by its id — which is what
/// turns the bead the window is on back into a step of the ring.
///
/// Nothing where this bead no longer names it, which is a collection having
/// rewritten what it names while the reader was reading it.
pub fn at(node: &Node, id: &str) -> Option<usize> {
    related(node).iter().position(|named| named.id == id)
}

/// Every bead this one names, in the order the window draws them: its
/// parent, then what it waits on, then what waits on it, which is `bd
/// show`'s own order and the order `said` builds its sections in.
pub fn related(node: &Node) -> Vec<&Related> {
    node.parent
        .iter()
        .chain(&node.depends_on)
        .chain(&node.blocks)
        .collect()
}

/// The key a bead named in a section stands for: its bare id, in the project
/// the selection is in.
///
/// `Related` carries no project and the key here is `(project, id)`. Within
/// one tracker the project is the source bead's, which is the project of the
/// tree the selection was drawn in — a tree does not span projects. A
/// dependency naming another tracker is `bdi-0gf` and is not this.
pub fn key_of(forest: &Forest, related: &Related) -> Option<BeadKey> {
    Some(BeadKey {
        project: forest.place()?.tree.project.clone(),
        id: related.id.clone(),
    })
}

/// Whether the forest can take the reader to a bead named in a section.
///
/// A bead the tracker's answer does not hold is not one — there is nothing
/// to go to, and the row says so where it is drawn. Neither is one no tree
/// the forest draws holds, which `bd list --all` makes an ordinary case: the
/// answer is the whole project, and the trees are what hangs under the roots
/// the config names. Both are asked of the snapshot as one question, so
/// neither depends on how a root comes to be one.
pub fn followable(forest: &Forest, related: &Related) -> bool {
    key_of(forest, related).is_some_and(|key| forest.draws(&key))
}

/// The bead as the window draws it: its rows, and the row each bead it names
/// is drawn on.
pub struct Page {
    pub rows: Vec<Vec<Span<'static>>>,
    /// The row each of `related`'s beads was drawn on, in that order.
    pub related: Vec<usize>,
}

/// The bead the selection is on, where it is on one.
///
/// A bead's row and a tree's header both stand for a bead, wherever the
/// header is drawn; a project's line, a group and a thing in one stand for
/// none. A root whose tree would not read carries its key and no bead
/// behind it, so it answers none too: there is nothing of it to show.
pub fn selected(forest: &Forest) -> Option<&Node> {
    forest
        .lines()
        .get(forest.selected_line())?
        .bead()
        .and_then(|key| forest.snapshot().node(key))
}

/// Where the window sits: `width` across, as tall as the bead up to what
/// the screen offers, centred over the forest.
fn show_window(area: Rect, width: u16, rows: usize) -> Rect {
    let wanted = u16::try_from(rows)
        .unwrap_or(u16::MAX)
        .saturating_add(BORDERS);
    area.centered(
        Constraint::Length(width),
        Constraint::Length(wanted.min(offered(area.height, FLOOR_HEIGHT))),
    )
}

/// The bead, one screen row at a time, in `bd show`'s order: the row's own
/// facts, the agent the join put on it, then each section the bead has
/// something in, under the name `bd show` gives it. Prose is wrapped to
/// `width`; every other row is cut to it when drawn, as a row of the forest
/// is.
pub fn said(node: &Node, width: usize) -> Page {
    let mut rows = vec![vec![
        glyph(&node.status),
        Span::raw(" "),
        Span::styled(node.id.clone(), Style::new().fg(ID)),
        Span::raw(indent()),
        Span::raw(node.title.clone()),
    ]];

    let mut facts = vec![format!("P{}", node.priority), node.issue_type.clone()];
    facts.extend(node.owner.clone());
    rows.push(indented(vec![
        Span::styled(
            phrase::status_word(&node.status),
            status_style(&node.status),
        ),
        Span::raw(format!(" · {}", facts.join(" · "))),
    ]));
    if let Some(agent) = &node.agent {
        rows.push(indented(vec![Span::styled(
            agent_marker(agent),
            Style::new().fg(LIVE),
        )]));
    }

    let room = width.saturating_sub(indent().len());
    let prose = |text: &str| {
        markdown::rows(text, room)
            .into_iter()
            .map(indented)
            .collect::<Vec<_>>()
    };
    let tied = |arrow: char, related: &[Related]| {
        related
            .iter()
            .map(|related| {
                let mut row = vec![Span::styled(format!("{arrow} "), STANDS_OUT)];
                row.extend(related_row(related));
                indented(row)
            })
            .collect::<Vec<_>>()
    };

    let mut related = Vec::new();
    for (heading, body, names_beads) in [
        (DESCRIPTION, prose(&node.description), false),
        (NOTES, prose(&node.notes), false),
        (PARENT, tied(UP, node.parent.as_slice()), true),
        (DEPENDS_ON, tied(OUT, &node.depends_on), true),
        (BLOCKS, tied(BACK, &node.blocks), true),
    ] {
        if body.is_empty() {
            continue;
        }
        rows.push(Vec::new());
        rows.push(vec![Span::styled(
            heading,
            STANDS_OUT.add_modifier(Modifier::BOLD),
        )]);
        if names_beads {
            related.extend(rows.len()..rows.len() + body.len());
        }
        rows.extend(body);
    }

    Page { rows, related }
}

/// What a row of the window is drawn in under the spans that name no colour
/// of their own. The head — the bead's glyph, id and title — is the
/// terminal's own, as the row the reader came for is in the forest; the page
/// beneath it sits one rung below, so what the page holds at the default
/// reads as emphasis rather than as the page.
fn tone_of(row: usize) -> Style {
    if row == 0 {
        STANDS_OUT
    } else {
        Style::new().fg(PAGE)
    }
}

/// One row indented under a heading.
fn indented(row: Vec<Span<'static>>) -> Vec<Span<'static>> {
    let mut said = vec![Span::raw(indent())];
    said.extend(row);
    said
}

/// A status glyph in the colour the forest paints it, so the same thing is
/// the same colour on both sides of the border.
fn glyph(status: &Status) -> Span<'static> {
    Span::styled(status_glyph(status).to_string(), status_style(status))
}

/// A related bead as `bd show` lists one: its glyph, id and title, and the
/// kind of edge where the arrow alone would not say. A closed one is dimmed
/// the way `bd show` dims it, the glyph aside. A bead the answer does not
/// hold has no glyph and no title, and says so in their place.
fn related_row(related: &Related) -> Vec<Span<'static>> {
    let Some(status) = &related.status else {
        return vec![Span::raw(format!(
            "{}{}{}",
            related.id,
            indent(),
            phrase::not_in_the_answer()
        ))];
    };
    let mut said = format!(
        " {}{}{}",
        related.id,
        indent(),
        related.title.as_deref().unwrap_or_default()
    );
    if let Edge::Other(kind) = &related.edge {
        said.push_str(&format!(" · {}", phrase::edge_kind(kind)));
    }
    vec![
        glyph(status),
        Span::styled(said, fg(status.is_closed().then_some(DIM))),
    ]
}

/// Draw the bead in a window over the forest.
///
/// `Clear` blanks the window first, which is what stops the trees showing
/// through between the rows. The way back is the border's title, so a window
/// too short for a single row still holds it: a reader who cannot see how to
/// leave is stuck in a view they may have opened by accident. Where the bead
/// is taller than the window, the title says how to see the rest.
///
/// `follows` is whether the forest can take the reader to any of the beads
/// this one names, which only the forest can say. The title offers the keys
/// that do it where it can, and says nothing about them where every bead
/// named is one the reader would press Enter on for nothing.
///
/// The bead the window is on is drawn as the forest draws the row the
/// selection is on, and brought inside the window where the last frame left
/// it beyond what there was room for — a ring nobody can see is a ring the
/// reader has lost.
pub fn show(frame: &mut Frame, area: Rect, node: &Node, view: &mut Show, follows: bool) {
    let width = offered(area.width, FLOOR_WIDTH);
    let page = said(node, width.saturating_sub(BORDERS) as usize);
    let window = show_window(area, width, page.rows.len());
    if window.is_empty() {
        return;
    }

    let block = Block::bordered();
    let inner = block.inner(window);
    view.fit(page.rows.len(), inner.height as usize);
    let on = view
        .on()
        .and_then(|id| at(node, id))
        .and_then(|at| page.related.get(at).copied());
    if let Some(row) = on {
        view.reveal(row);
    }
    let block = block.title(Span::styled(
        phrase::way_back_from_bead(&node.id, view.scrolls(), follows),
        Style::new().add_modifier(Modifier::BOLD),
    ));
    frame.render_widget(Clear, window);
    frame.render_widget(block, window);

    for (n, row) in page
        .rows
        .into_iter()
        .skip(view.from)
        .take(inner.height as usize)
        .enumerate()
    {
        let at = view.from + n;
        let drawn = Fitted::new(row, Vec::new(), Vec::new()).toned(tone_of(at));
        let drawn = if on == Some(at) {
            drawn.selected()
        } else {
            drawn
        };
        frame.render_widget(
            drawn,
            Rect {
                y: inner.y + n as u16,
                ..inner
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::edges::Related;
    use crate::model::join::{AgentRef, JoinSource};
    use crate::model::types::testing::key;
    use crate::model::types::{Edge, PaneStatus, Status};
    use crate::view::draw::tone::{status_colour, DIM, LIVE, PAGE};
    use crate::view::painted::{Painted, Run};
    use pretty_assertions::assert_eq;
    use ratatui::style::Color;

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
                pane: key("w:p1"),
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

    /// The window over a bead none of whose references the forest draws,
    /// which is what a test that is not about following one wants: the title
    /// then offers no key it would be pressed for nothing.
    fn drawn(node: &Node, view: &mut Show, width: u16, height: u16) -> Vec<String> {
        drawn_where(node, view, width, height, false)
    }

    fn drawn_where(
        node: &Node,
        view: &mut Show,
        width: u16,
        height: u16,
        follows: bool,
    ) -> Vec<String> {
        Painted::drawn_by(width, height, |frame| {
            show(frame, frame.area(), node, view, follows)
        })
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

    /// The window follows the terminal: on a wide screen it is four fifths of
    /// the width, centred, and the prose wraps to that rather than to eighty
    /// columns or to the screen's edge.
    #[test]
    fn on_a_wide_screen_the_window_is_four_fifths_of_it_and_the_prose_wraps_there() {
        let long = Node {
            description: "abcde ".repeat(60).trim().to_string(),
            ..a_bead()
        };
        let rows = drawn(&long, &mut Show::default(), 200, 60);

        let top = rows
            .iter()
            .find(|row| row.contains('┌'))
            .expect("the window's top edge is drawn");
        assert_eq!(top.chars().nth(20), Some('┌'), "{top:?}");
        assert_eq!(top.chars().nth(179), Some('┐'), "{top:?}");
        let first = rows
            .iter()
            .find(|row| row.contains("abcde"))
            .expect("the prose is drawn");
        assert_eq!(
            first.matches("abcde").count(),
            26,
            "twenty-six words of five, with a space between, is a hundred and \
             fifty-five of the hundred and fifty-six columns left inside the \
             indent: {first:?}"
        );
        assert!(
            !first.contains('…'),
            "prose wrapped to the window is never cut at it: {first:?}"
        );
    }

    /// Four fifths of a small screen would be a cramped box, so the window
    /// is never offered less than eighty columns inside its border: on a
    /// screen a little wider than that, the floor wins over the proportion.
    #[test]
    fn a_screen_not_much_wider_than_eighty_columns_gives_the_window_eighty() {
        let rows = drawn(&a_bead(), &mut Show::default(), 90, 30);
        let top = rows
            .iter()
            .find(|row| row.contains('┌'))
            .expect("the window's top edge is drawn");
        assert_eq!(top.chars().nth(4), Some('┌'), "{top:?}");
        assert_eq!(top.chars().nth(85), Some('┐'), "{top:?}");
    }

    /// The floor holds for the height too: on a screen a little taller than
    /// twenty-four rows, a bead taller than that gets twenty-four rows of
    /// window rather than four fifths of the screen.
    #[test]
    fn a_screen_not_much_taller_than_twenty_four_rows_gives_the_window_twenty_four() {
        let tall = Node {
            description: (1..=40)
                .map(|n| format!("line {n}"))
                .collect::<Vec<_>>()
                .join("\n"),
            ..a_bead()
        };
        let rows = drawn(&tall, &mut Show::default(), 44, 28);
        let top = rows
            .iter()
            .position(|row| row.contains('┌'))
            .expect("the window's top edge is drawn");
        let bottom = rows
            .iter()
            .position(|row| row.contains('└'))
            .expect("the window's bottom edge is drawn");
        assert_eq!((top, bottom), (2, 25));
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

    // ---- colour ----------------------------------------------------------

    /// The run a word is drawn in, found by what it says rather than where it
    /// falls: a run's place on a row moves with the width and the border.
    fn run_saying(painted: &[Run], said: &str) -> Run {
        painted
            .iter()
            .find(|run| run.said.contains(said))
            .unwrap_or_else(|| panic!("{said:?} is drawn: {painted:?}"))
            .clone()
    }

    fn painted(node: &Node, width: u16, height: u16) -> Painted {
        Painted::drawn_by(width, height, |frame| {
            show(frame, frame.area(), node, &mut Show::default(), false)
        })
    }

    /// One of each status the forest gives a colour, so a loop over them
    /// covers the palette.
    fn every_coloured_status() -> [Status; 5] {
        [
            Status::InProgress,
            Status::Blocked,
            Status::Closed,
            Status::Deferred,
            Status::Other("triage".into()),
        ]
    }

    /// The same thing is the same colour on both sides of the border: the
    /// glyph at the top of the window goes through the rule the forest's
    /// glyph goes through.
    #[test]
    fn the_glyph_is_painted_the_colour_the_forest_paints_it() {
        for status in every_coloured_status() {
            let bead = Node {
                status: status.clone(),
                ..a_bead()
            };
            let top = painted(&bead, 44, 22).row(1);

            let glyph = run_saying(&top, &status_glyph(&status).to_string());
            assert_eq!(glyph.said, status_glyph(&status).to_string(), "{top:?}");
            assert_eq!(
                glyph.style.fg,
                status_colour(&status),
                "{status:?}: {top:?}"
            );
        }
    }

    /// `bd` sends no escape for an open bead, so the glyph inherits its row's
    /// brightness as the forest's does — and the head of the page is the
    /// terminal's own.
    #[test]
    fn an_open_glyph_at_the_head_of_the_page_is_the_terminals_own() {
        let open = Node {
            status: Status::Open,
            ..a_bead()
        };
        let top = painted(&open, 44, 22).row(1);

        assert_eq!(
            run_saying(&top, "○").style.fg,
            Some(Color::Reset),
            "{top:?}"
        );
    }

    /// The agent is the one thing on the page `bd` cannot say, and it keeps
    /// the colour the forest gives it.
    #[test]
    fn the_agent_marker_is_painted_live_as_the_forest_paints_it() {
        let marker = painted(&a_bead(), 44, 22).row(3);

        assert_eq!(
            run_saying(&marker, "◍ lifting the mast · working").style.fg,
            Some(LIVE),
            "{marker:?}"
        );
    }

    /// Read off `bd show` 1.2.2's own output: the id at the head of the page
    /// is always this blue, whatever the status.
    #[test]
    fn the_id_is_painted_the_blue_bd_show_paints_it() {
        let top = painted(&a_bead(), 44, 22).row(1);

        assert_eq!(
            run_saying(&top, "orb-7.1").style.fg,
            Some(Color::Rgb(89, 194, 255)),
            "{top:?}"
        );
        assert_eq!(
            run_saying(&top, "re-point the dish").style.fg,
            Some(Color::Reset),
            "the title is the terminal's own: {top:?}"
        );
    }

    /// `bd show` says the status word in the status colour; the priority,
    /// the type and the owner are the page's, one rung below the head.
    #[test]
    fn the_facts_row_says_the_status_in_its_colour_and_the_rest_on_the_page() {
        for status in every_coloured_status() {
            let bead = Node {
                status: status.clone(),
                ..a_bead()
            };
            let facts = painted(&bead, 44, 22).row(2);

            let word = run_saying(&facts, &phrase::status_word(&status));
            assert_eq!(
                word.style.fg,
                status_colour(&status),
                "{status:?}: {facts:?}"
            );
            assert_eq!(
                run_saying(&facts, "P2 · task · kim").style.fg,
                Some(PAGE),
                "{status:?}: {facts:?}"
            );
        }
    }

    /// A related bead's glyph is the same glyph the forest and the head of
    /// the window paint, and it takes the same colour — or, for an open one,
    /// the brightness of the row it sits on, which on the page is the page's.
    #[test]
    fn a_related_beads_glyph_is_painted_the_colour_of_its_status() {
        let painted = painted(&a_bead(), 44, 22);
        let depends_on = painted.row(17);
        let blocks = painted.row(20);

        assert_eq!(
            run_saying(&depends_on, "✓").style.fg,
            status_colour(&Status::Closed),
            "{depends_on:?}"
        );
        assert_eq!(run_saying(&blocks, "○").style.fg, Some(PAGE), "{blocks:?}");
    }

    /// `bd show` dims a closed related bead's id and title to the grey it
    /// dims a finished row to, and leaves the arrow alone; an open one is
    /// the page's, as a row nobody is on is the forest's middle rung.
    #[test]
    fn a_closed_related_bead_is_dimmed_as_bd_show_dims_one() {
        let painted = painted(&a_bead(), 44, 22);
        let depends_on = painted.row(17);
        let blocks = painted.row(20);

        assert_eq!(
            run_saying(&depends_on, "orb-7.3  lay the feeder cable")
                .style
                .fg,
            Some(DIM),
            "{depends_on:?}"
        );
        assert_eq!(
            run_saying(&depends_on, "→").style.fg,
            Some(Color::Reset),
            "the arrow says the edge, not the state: {depends_on:?}"
        );
        assert_eq!(
            run_saying(&blocks, "orb-7.4  file the licence").style.fg,
            Some(PAGE),
            "{blocks:?}"
        );
    }

    /// `bd show` prints a section heading bold and in no colour, and the
    /// window draws it as `bd show` does.
    #[test]
    fn a_heading_is_bold_and_no_colour_as_bd_show_prints_one() {
        let heading = run_saying(&painted(&a_bead(), 44, 22).row(5), DESCRIPTION);

        assert!(
            heading.style.add_modifier.contains(Modifier::BOLD),
            "{heading:?}"
        );
        assert_eq!(heading.style.fg, Some(Color::Reset), "{heading:?}");
    }

    /// Colour is the second channel and never the only one: the glyph and
    /// the status word say the status, so a terminal that drops colour loses
    /// nothing on either the bead's own rows or a related bead's.
    #[test]
    fn nothing_in_the_window_is_told_apart_by_colour_alone() {
        for status in every_coloured_status().into_iter().chain([Status::Open]) {
            let bead = Node {
                status: status.clone(),
                depends_on: vec![related(
                    "orb-7.3",
                    Edge::Blocks,
                    status.clone(),
                    "the cable",
                )],
                ..a_bead()
            };
            let rows = painted(&bead, 44, 22).rows();
            let glyph = status_glyph(&status).to_string();

            assert!(
                rows[1].contains(&glyph),
                "{status:?} lost its glyph: {rows:#?}"
            );
            assert!(
                rows[2].contains(&phrase::status_word(&status)),
                "{status:?} lost its word: {rows:#?}"
            );
            assert!(
                rows[17].contains(&format!("→ {glyph} orb-7.3")),
                "{status:?} lost its glyph on a related row: {rows:#?}"
            );
        }
    }

    /// White is the exception. Every non-blank run the window draws on the
    /// terminal's own foreground — or on `White` — is one of the few things
    /// meant to stand out from a page of text: the way back, the bead's own
    /// title, a heading, an arrow. A span dropped back to `Span::raw` lands
    /// on this list and turns it red.
    #[test]
    fn what_stands_out_from_the_page_is_the_way_back_the_title_the_headings_and_the_arrows() {
        let bead = a_bead();
        let painted = painted(&bead, 44, 22);

        let own: Vec<String> = (0..22)
            .flat_map(|y| painted.row(y))
            .filter(|run| matches!(run.style.fg, Some(Color::Reset | Color::White)))
            .map(|run| {
                run.said
                    .replace(['│', '─', '┌', '┐', '└', '┘'], "")
                    .trim()
                    .to_string()
            })
            .filter(|said| !said.is_empty())
            .collect();

        assert_eq!(
            own,
            vec![
                phrase::way_back_from_bead(&bead.id, false, false),
                bead.title.clone(),
                DESCRIPTION.to_string(),
                NOTES.to_string(),
                PARENT.to_string(),
                UP.to_string(),
                DEPENDS_ON.to_string(),
                OUT.to_string(),
                BLOCKS.to_string(),
                BACK.to_string(),
            ],
            "{painted:?}"
        );
    }

    /// The page under the head sits one rung below the terminal's own, at
    /// the tone the forest draws a row nobody is on: the prose, and a related
    /// bead's id and title.
    #[test]
    fn the_page_is_drawn_at_the_tone_the_forest_draws_a_row_nobody_is_on() {
        let painted = painted(&a_bead(), 44, 22);

        for (y, said) in [
            (6, "Point it at the new bird."),
            (11, "The crane is booked for Tuesday."),
            (14, "orb-7  lift the ground station"),
        ] {
            let run = run_saying(&painted.row(y), said);
            assert_eq!(run.style.fg, Some(Color::DarkGray), "{said}: {run:?}");
        }
    }

    /// Emphasis in the prose is by weight, not by white: a bold word keeps
    /// the page's tone under its modifier, and a code span keeps the colour
    /// markdown gives it.
    #[test]
    fn emphasis_in_the_prose_is_by_weight_and_a_code_span_by_its_own_colour() {
        let bead = Node {
            description: "Point it at the **new** bird, `now`.\n\nThe old one is gone.".to_string(),
            ..a_bead()
        };
        let prose = painted(&bead, 44, 22).row(6);

        let bold = run_saying(&prose, "new");
        assert!(
            bold.style.add_modifier.contains(Modifier::BOLD),
            "{prose:?}"
        );
        assert_eq!(bold.style.fg, Some(PAGE), "{prose:?}");
        assert_eq!(
            run_saying(&prose, "now").style.fg,
            Some(Color::Cyan),
            "{prose:?}"
        );
    }

    /// Everything can be followed, for the ring tests that are about stepping
    /// rather than about skipping.
    fn all(_at: usize) -> bool {
        true
    }

    /// The order the window draws them in, which is `bd show`'s: the parent,
    /// then what it waits on, then what waits on it.
    #[test]
    fn the_beads_a_bead_names_are_listed_in_the_order_the_window_draws_them() {
        assert_eq!(
            super::related(&a_bead())
                .iter()
                .map(|it| it.id.as_str())
                .collect::<Vec<_>>(),
            vec!["orb-7", "orb-7.3", "orb-7.4"]
        );
    }

    /// The ordinals the window keeps are into that list, so the row each one
    /// is drawn on has to come back in the same order. Two statements of one
    /// order is what this stops drifting apart.
    #[test]
    fn the_rows_the_page_reports_are_the_beads_it_names_in_that_order() {
        let bead = a_bead();
        let page = said(&bead, 60);

        assert_eq!(
            page.related.len(),
            super::related(&bead).len(),
            "a row for each bead named"
        );
        // Read the row and check it says the bead of the same ordinal, rather
        // than searching the list for whichever id the row contains: `orb-7`
        // is inside `orb-7.3`, so a search finds the parent on every row and
        // an order that had drifted would still line up.
        for (at, named) in super::related(&bead).iter().enumerate() {
            let said: String = page.rows[page.related[at]]
                .iter()
                .map(|span| span.content.as_ref())
                .collect();
            assert!(
                said.contains(named.id.as_str()),
                "the row reported for {} says {said:?}",
                named.id
            );
        }
    }

    /// A step from nowhere lands where one coming round from off the end
    /// would, so the first press does what a reader reading downwards means
    /// by it.
    #[test]
    fn a_first_step_lands_on_the_first_bead_the_page_names() {
        assert_eq!(stepped(None, 3, all), Some(0));
    }

    #[test]
    fn stepping_past_the_last_comes_round_to_the_first() {
        assert_eq!(stepped(Some(2), 3, all), Some(0));
    }

    #[test]
    fn a_step_moves_one_bead_at_a_time() {
        assert_eq!(stepped(Some(0), 3, all), Some(1));
        assert_eq!(stepped(Some(1), 3, all), Some(2));
    }

    /// A bead the forest cannot take the reader to is drawn and says so, and
    /// the ring passes over it: a ring anyone can see is an Enter that goes
    /// somewhere.
    #[test]
    fn a_bead_the_forest_cannot_go_to_is_stepped_over() {
        let only_the_last = |at: usize| at == 2;

        assert_eq!(stepped(None, 3, only_the_last), Some(2));
        assert_eq!(stepped(Some(2), 3, only_the_last), Some(2));
        assert_eq!(stepped(Some(0), 3, only_the_last), Some(2));
    }

    /// Where none of them can be followed the ring never comes up, which is
    /// also the answer for a bead that names none at all.
    #[test]
    fn a_bead_with_nowhere_to_go_has_no_ring() {
        assert_eq!(stepped(None, 3, |_| false), None);
        assert_eq!(stepped(None, 0, all), None);
        assert_eq!(stepped(Some(1), 0, all), None);
    }

    /// The title names the keys only where a press would do something. A
    /// reader who is told about `Tab` on a bead whose every reference is
    /// undrawn presses it and nothing happens.
    #[test]
    fn the_title_offers_the_keys_that_follow_only_where_something_can_be() {
        let bead = a_bead();

        let says = |drawn: Vec<String>, said: &str| drawn.iter().any(|row| row.contains(said));

        assert!(
            !says(drawn(&bead, &mut Show::default(), 60, 24), "to follow"),
            "keys offered over a bead nothing can be followed from"
        );
        assert!(
            says(
                drawn_where(&bead, &mut Show::default(), 60, 24, true),
                "Tab, Enter to follow"
            ),
            "no keys offered where a reference can be followed"
        );
    }

    /// The row the window is on is drawn the way the forest draws the row its
    /// own selection is on, so one reverse means one thing on this screen.
    #[test]
    fn the_bead_the_window_is_on_is_drawn_as_a_selected_row_is() {
        let bead = a_bead();
        let mut view = Show::default();
        view.go_to("orb-7");

        // Found by what the row says rather than by its number: the window is
        // centred, so a page row and a screen row are not the same count.
        let reversed = |said: &str, view: &mut Show| {
            Painted::drawn_by(60, 24, |frame| show(frame, frame.area(), &bead, view, true))
                .rows()
                .iter()
                .enumerate()
                .find(|(_, row)| row.contains(said))
                .map(|(at, _)| at)
                .map(|at| {
                    Painted::drawn_by(60, 24, |frame| show(frame, frame.area(), &bead, view, true))
                        .row(at)
                        .iter()
                        .any(|run| run.style.add_modifier.contains(Modifier::REVERSED))
                })
                .unwrap_or_else(|| panic!("{said:?} is drawn"))
        };

        assert!(
            reversed("lift the ground station", &mut view),
            "the bead the window is on is not drawn as the selected row is"
        );
        assert!(
            !reversed("file the licence", &mut view),
            "a bead the window is not on is drawn as selected"
        );
    }

    /// A ring nobody can see is a ring the reader has lost: stepping on to a
    /// bead below what the window has room for scrolls it into view.
    #[test]
    fn stepping_on_to_a_bead_below_the_window_brings_it_into_view() {
        let bead = a_bead();
        let mut view = Show::default();
        let page = said(&bead, 58);
        let last = *page.related.last().expect("a bead names beads");

        view.go_to("orb-7.4");
        let drawn = drawn_where(&bead, &mut view, 60, 8, true);

        assert!(
            drawn
                .iter()
                .any(|row| row.contains(&super::related(&bead)[2].id)),
            "the bead the window is on is off the page it drew: {drawn:#?}"
        );
        assert!(
            last + 2 > drawn.len(),
            "a window with room for the whole bead does not test scrolling"
        );
    }

    /// The window scrolled to a row, with `room` rows to draw in and `total`
    /// rows of bead to draw — the state a frame leaves behind it.
    fn looking(from: usize, room: usize, total: usize) -> Show {
        let mut view = Show::default();
        view.fit(total, room);
        view.from = from;
        view
    }

    /// Scrolling up to a row above the window puts that row at the top and
    /// goes no further: a jump that overshot would move the reader past the
    /// thing they asked to see.
    #[test]
    fn a_row_above_the_window_is_brought_to_its_top() {
        let mut view = looking(10, 5, 40);

        view.reveal(3);

        assert_eq!(view.from, 3);
    }

    /// Scrolling down puts it on the last row there is room for, so the rows
    /// a reader has just read stay on the screen above it.
    #[test]
    fn a_row_below_the_window_is_brought_to_its_foot() {
        let mut view = looking(0, 5, 40);

        view.reveal(9);

        assert_eq!(view.from, 5, "row 9 is the last of rows 5 to 9");
    }

    /// The row one past the last drawn is below the window; the last drawn is
    /// not. The boundary is where an off-by-one lives, and either side of it
    /// is one row of scroll.
    #[test]
    fn the_last_row_the_window_draws_is_not_below_it() {
        let mut already = looking(0, 5, 40);
        already.reveal(4);
        assert_eq!(already.from, 0, "row 4 is the last of rows 0 to 4");

        let mut just_past = looking(0, 5, 40);
        just_past.reveal(5);
        assert_eq!(just_past.from, 1);
    }

    /// The first row the window draws is not above it either, which is the
    /// same boundary at the other end.
    #[test]
    fn the_first_row_the_window_draws_is_not_above_it() {
        let mut already = looking(3, 5, 40);
        already.reveal(3);
        assert_eq!(already.from, 3);

        let mut just_before = looking(3, 5, 40);
        just_before.reveal(2);
        assert_eq!(just_before.from, 2);
    }

    /// A window with no room to draw in has nowhere to bring a row, and the
    /// arithmetic that would run there subtracts a room of nothing from a row
    /// and scrolls to one past it.
    #[test]
    fn a_window_with_no_room_scrolls_nowhere() {
        let mut view = looking(0, 0, 40);

        view.reveal(9);

        assert_eq!(view.from, 0);
    }
}
