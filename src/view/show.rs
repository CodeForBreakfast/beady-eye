//! The bead view: the selected bead shown whole, as `bd show` shows it, in a
//! window over the forest.

use std::collections::HashMap;

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::{Block, Padding};
use ratatui::Frame;

use crate::model::edges::Related;
use crate::model::join::BeadKey;
use crate::model::snapshot::Node;
use crate::model::types::{Edge, Status};
use crate::view::draw::bead::{badge_style, opens_at};
use crate::view::draw::tone::status_style;
use crate::view::draw::{done, regions};
use crate::view::fitted::{self, cover, indent, Fitted, Link};
use crate::view::forest::Forest;
use crate::view::lines::Content;
use crate::view::markdown;
use crate::view::palette;
use crate::view::phrase;
use crate::view::row::{self, status_glyph, Progress};
use crate::view::{Motion, Notch};

/// The section names `bd show` prints, verbatim, in the order it prints
/// them. Terminology comes from beads, and a heading is terminology.
const DESCRIPTION: &str = "DESCRIPTION";
const NOTES: &str = "NOTES";
const PARENT: &str = "PARENT";
const DEPENDS_ON: &str = "DEPENDS ON";
const BLOCKS: &str = "BLOCKS";

/// `bd show`'s own word for the person a bead is assigned to, lowercased to
/// sit in a row of facts rather than at the head of one.
const ASSIGNEE: &str = "assignee";

/// The words the dates row says each date under, in `design.md`'s order, and
/// the format `bd show` writes a date in.
const CREATED: &str = "created";
const UPDATED: &str = "updated";
const STARTED: &str = "started";
const CLOSED: &str = "closed";
const DATE: &str = "%Y-%m-%d";

/// How `bd show` separates one label from the next.
const BETWEEN_LABELS: &str = ", ";

/// `bd show`'s own arrows: up to the parent, out to what a bead waits on,
/// back from what waits on it.
const UP: char = '↑';
const OUT: char = '→';
const BACK: char = '←';

/// The share of the screen the window takes across: four fifths, so the
/// forest still shows beside it and a bigger terminal gets a bigger window
/// rather than the same window against a wider forest.
const SHARE: (u16, u16) = (4, 5);

/// The rows a bordered window spends on its own edges.
const BORDERS: u16 = 2;

/// The columns the window keeps clear of its border on either side.
const MARGIN: u16 = 1;

/// What that costs the width: one such column at each end.
const MARGINS: u16 = MARGIN * 2;

/// The blank row the window keeps over the head. There is none under the
/// page, which ends with a section.
const BLANK_ROW: u16 = 1;

/// The least the window is offered across: eighty-two columns inside its
/// border, so that the margin is paid for by the frame and the page still
/// gets the eighty columns `bd show` wraps its own prose at. Four fifths of
/// a small screen would be a cramped box for no gain.
const FLOOR_WIDTH: u16 = 80 + MARGINS + BORDERS;

/// `SHARE` of `screen` or `FLOOR_WIDTH`, whichever is more, and never more
/// than the screen: a screen no wider than the floor gives the window the
/// whole of it.
fn offered(screen: u16) -> u16 {
    let share = u32::from(screen) * u32::from(SHARE.0) / u32::from(SHARE.1);
    u16::try_from(share)
        .unwrap_or(u16::MAX)
        .max(FLOOR_WIDTH)
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

    /// Move the view one notch of the wheel, reporting whether what it shows
    /// changed. Further than the motion the same key would make, and the same
    /// distance the config gives a notch over the forest.
    pub fn scrolled(&mut self, notch: Notch, lines: usize) -> bool {
        let to = match notch {
            Notch::Up => self.from.saturating_sub(lines),
            Notch::Down => (self.from + lines).min(self.total.saturating_sub(self.room)),
        };
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
    /// Which spans of which rows the terminal is told point somewhere, for
    /// the rows that hold a link at all.
    links: HashMap<usize, Vec<Link>>,
}

/// How much of what the selected line stands for is done, where the forest
/// has a fraction for it.
///
/// Read off the line rather than worked out here: it counts the whole subtree
/// under the bead, which the bead's own fields say nothing about.
pub fn fraction(forest: &Forest) -> Option<Progress> {
    match &forest.lines().get(forest.selected_line())?.content {
        Content::Bead(row) => row.progress,
        _ => None,
    }
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

/// Where the window falls on a screen and what it holds: the bead's page,
/// the window itself, and the rows inside its border.
///
/// Worked out from the screen and the bead every time rather than kept from
/// the frame that drew: both are the caller's already, and geometry held
/// between frames is geometry that can disagree with the frame on the
/// screen.
struct Laid {
    page: Page,
    window: Rect,
    inner: Rect,
}

/// The window's own frame: the border, the column of margin inside it on
/// either side, and the blank row over the head.
fn window_block() -> Block<'static> {
    Block::bordered().padding(Padding::new(MARGIN, MARGIN, BLANK_ROW, 0))
}

fn lay_out(
    area: Rect,
    node: &Node,
    progress: Option<Progress>,
    followable: &dyn Fn(&Related) -> bool,
) -> Laid {
    let window = show_window(area);
    let inner = window_block().inner(window);
    let page = said(
        node,
        progress,
        inner.width as usize,
        inner.height as usize,
        followable,
    );
    Laid {
        page,
        window,
        inner,
    }
}

/// What the window drew on one row of the screen.
///
/// A row rather than a point, because a click reaches the loop as its row
/// alone. So `Beyond` is off the page up or down the screen — the window's
/// own border, or the foot beneath it — and the forest showing to the left
/// of the window, on a row the window is on, is not something this can tell
/// from the page.
#[cfg_attr(test, derive(Debug, PartialEq))]
pub enum Drawn<'a> {
    /// One of the beads this one names, by its id.
    Related(&'a str),
    /// A row of the page that names none: the bead's own facts, its prose, a
    /// heading, a blank.
    Page,
    /// Not the page: the window's border, or the foot beneath it.
    Beyond,
}

/// What the window drew on one row of the screen, for a pointer that has
/// landed there.
///
/// `view` is what the last frame left behind, and a frame is always drawn
/// before a press is answered, so how far down the bead it had scrolled is
/// how far down the reader was looking.
///
/// `followable` is the page's, not this answer's: a row is drawn where it is
/// drawn whatever colour its id takes, and the page a pointer landed on is
/// the page the reader was looking at.
pub fn drawn_at<'a>(
    area: Rect,
    node: &'a Node,
    progress: Option<Progress>,
    view: &Show,
    row: u16,
    followable: &dyn Fn(&Related) -> bool,
) -> Drawn<'a> {
    let Laid { page, inner, .. } = lay_out(area, node, progress, followable);
    if !(inner.y..inner.bottom()).contains(&row) {
        return Drawn::Beyond;
    }
    let at = view.from + usize::from(row - inner.y);
    related(node)
        .into_iter()
        .zip(page.related)
        .find(|(_, drawn)| *drawn == at)
        .map_or(Drawn::Page, |(named, _)| Drawn::Related(&named.id))
}

/// Where the window sits: a drawer against the right edge, from the screen's
/// first row down to the row above the foot, whatever height the bead is.
///
/// A drawer rather than a box because what a centred window leaves showing
/// either side of it is the ends of the forest's titles — cut words on both
/// sides of a page a reader opened to read — where the right edge leaves the
/// spine, the glyphs and the ids, which read as structure.
///
/// The foot's height is asked of `regions` rather than counted here, so the
/// row it stops above is the row the foot is actually drawn on.
fn show_window(area: Rect) -> Rect {
    let width = offered(area.width);
    Rect {
        x: area.right() - width,
        y: area.y,
        width,
        height: area.height - regions(area).keys.height,
    }
}

/// The bead, one screen row at a time: the head, then each section the bead
/// has something in, under the name `bd show` gives it and in its order.
///
/// The head is the forest row unfolded — every cell the row draws, at its
/// whole width, one row each — so one place decides a cell's words and the
/// row and the head cannot disagree about a bead.
///
/// No `above`, because the head has no row above it for an id to be read
/// against, so the id is drawn whole.
///
/// The name and the prose are both wrapped to `width`; every other row is cut
/// to it when drawn. A row of the forest is cut because a forest is a column
/// of rows that has to line up and the selection's geometry is worked out
/// from one row per bead, and neither holds here: the window draws one bead,
/// at whatever height that bead needs, with nothing lining up against it. The
/// title is also the line the reader opened the window for, so it is the last
/// line in it that should lose its end.
///
/// `window` is what the window has room for down the screen, which the name
/// alone is not allowed to fill — see `title_of`.
///
/// `followable` is whether the forest can take the reader to a bead this one
/// names, which only the forest can say and which decides the colour of that
/// bead's id.
pub fn said(
    node: &Node,
    progress: Option<Progress>,
    width: usize,
    window: usize,
    followable: &dyn Fn(&Related) -> bool,
) -> Page {
    let cells = row::cells(node, None, progress, None);
    let mut name = vec![
        Span::styled(cells.glyph.to_string(), status_style(&cells.status)),
        Span::raw(" "),
        Span::styled(cells.id.clone(), palette::IDENTITY),
        Span::raw(indent()),
    ];
    if !node.labels.is_empty() {
        name.push(Span::styled(
            node.labels.join(BETWEEN_LABELS),
            palette::QUIET,
        ));
        name.push(Span::raw(indent()));
    }
    let where_the_title_starts = name.iter().map(Span::width).sum::<usize>();
    let mut rows: Vec<Vec<Span<'static>>> = Vec::new();
    for line in title_of(
        &cells.title,
        width.saturating_sub(where_the_title_starts),
        window,
    ) {
        let mut row = if rows.is_empty() {
            name.clone()
        } else {
            vec![Span::raw(" ".repeat(where_the_title_starts))]
        };
        row.extend(line);
        rows.push(row);
    }
    let mut facts = vec![format!("P{}", node.priority), node.issue_type.clone()];
    facts.extend(node.created_by.clone());
    facts.extend(
        node.assignee
            .as_ref()
            .map(|assignee| format!("{ASSIGNEE} {assignee}")),
    );
    rows.push(indented(vec![
        Span::styled(
            phrase::status_word(&cells.status),
            status_style(&cells.status),
        ),
        Span::raw(format!(" · {}", facts.join(" · "))),
    ]));
    for dates in dates(node) {
        rows.push(indented(vec![Span::raw(dates)]));
    }
    if let Some(agent) = &cells.agent {
        rows.push(indented(vec![Span::styled(agent.clone(), palette::AGENT)]));
    }
    for anomaly in &cells.anomalies {
        rows.push(indented(vec![Span::styled(
            row::anomaly_alone(anomaly),
            palette::ATTENTION,
        )]));
    }
    let mut links = HashMap::new();
    let mut badges = vec![Span::raw(indent())];
    let mut opened = Vec::new();
    for badge in &cells.badges {
        if badges.len() > 1 {
            badges.push(Span::raw(indent()));
        }
        if let Some(to) = opens_at(badge) {
            opened.push(Link {
                block: fitted::Block::Identity,
                at: badges.len(),
                to: to.to_string(),
            });
        }
        badges.push(Span::styled(
            badge.text.clone(),
            badge_style(badge, &cells.status),
        ));
    }
    if badges.len() > 1 {
        links.insert(rows.len(), opened);
        rows.push(badges);
    }
    if let Some(progress) = cells.progress {
        rows.push(indented(vec![Span::raw(done(
            progress.finished,
            progress.total,
        ))]));
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
                let mut row = vec![Span::styled(format!("{arrow} "), palette::STRUCTURE)];
                row.extend(related_row(related, followable(related)));
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
        rows.push(vec![Span::styled(heading, palette::SECTION)]);
        if names_beads {
            related.extend(rows.len()..rows.len() + body.len());
        }
        rows.extend(body);
    }

    Page {
        rows,
        related,
        links,
    }
}

/// The bead's title in the `room` its own glyph and id leave beside it: as
/// its author wrote it where it need not be broken across rows, and broken
/// where it must be and every row of the break can be seen.
///
/// Breaking a line is what closes up the run of spaces someone typed between
/// two words, because a break is decided between words and a wrap that kept
/// them would carry a run of spaces down to the head of the next row. So a
/// title that fits is left alone: `bd show` prints the title as written and
/// the forest row beside the window draws it as written, and a window that
/// closed up a run of spaces it did not have to would be the one place on
/// the screen saying something else.
///
/// Being seen takes a column to be drawn in and a row to be drawn on, and a
/// narrow window can leave the title without either.
///
/// Without a column, `wrap` keeps itself terminating by taking the one it
/// was not offered, and the title comes back a glyph to a row — each of them
/// drawn past the edge of a window the glyph and the id already fill, so
/// every row of the name says nothing at all.
///
/// Without a row, the name fills the window on its own and the reader is
/// left with the one thing they already knew, a hundred presses above the
/// status, the priority and the prose they opened it for. Cutting is the
/// worse answer to a title in a window that has room for it and the better
/// answer to one that has not: a cut row says less than a wrapped name and
/// it says it where the reader is looking.
fn title_of(title: &str, room: usize, window: usize) -> Vec<Vec<Span<'static>>> {
    let as_written = vec![vec![Span::raw(title.to_string())]];
    if room == 0 {
        return as_written;
    }
    let broken = markdown::wrapped(title, room);
    if broken.len() == 1 || broken.len() >= window {
        return as_written;
    }
    broken
}

/// When the bead was created and updated, and when it was started and
/// closed: each said where the bead has it, on two rows rather than one so
/// that even a bead with all four fits the window at its floor. Joined on
/// one row the four dates need eighty-two columns, two more than the floor
/// offers, so the row was cut and lost the last date's end. A row is left
/// out where the half it says has no date to say.
///
/// `bd show` prints no closed date outside `--long`, and this says one: the
/// head is the row unfolded rather than a transcript, and when a closed bead
/// closed is what a reader opening it came for.
fn dates(node: &Node) -> Vec<String> {
    let half = |pairs: [(&str, Option<chrono::DateTime<chrono::Utc>>); 2]| {
        let said: Vec<String> = pairs
            .into_iter()
            .filter_map(|(word, when)| when.map(|when| format!("{word} {}", when.format(DATE))))
            .collect();
        (!said.is_empty()).then(|| said.join(" · "))
    };
    [
        half([(CREATED, node.created_at), (UPDATED, node.updated_at)]),
        half([(STARTED, node.started_at), (CLOSED, node.closed_at)]),
    ]
    .into_iter()
    .flatten()
    .collect()
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
///
/// The id is drawn in `bd`'s own blue where `followable` says the forest can
/// take the reader to the bead, and is left in the row's own tone where it
/// cannot: blue in this window means a bead you can go to. Only a row whose
/// id is blue is broken up to say so, so a row the reader cannot follow is
/// the row it always was.
fn related_row(related: &Related, followable: bool) -> Vec<Span<'static>> {
    let Some(status) = &related.status else {
        return vec![Span::raw(format!(
            "{}{}{}",
            related.id,
            indent(),
            phrase::not_in_the_answer()
        ))];
    };
    let mut beside = format!(
        "{}{}",
        indent(),
        related.title.as_deref().unwrap_or_default()
    );
    if let Edge::Other(kind) = &related.edge {
        beside.push_str(&format!(" · {}", phrase::edge_kind(kind)));
    }
    let tone = dimmed_if_closed(status);
    if !followable {
        return vec![
            glyph(status),
            Span::styled(format!(" {}{beside}", related.id), tone),
        ];
    }
    vec![
        glyph(status),
        Span::styled(" ", tone),
        Span::styled(related.id.clone(), palette::IDENTITY),
        Span::styled(beside, tone),
    ]
}

/// A related row falls to the finished tier when the bead it names is
/// closed, as a finished row of the forest does, and otherwise says nothing
/// so the page's own tone reaches it.
fn dimmed_if_closed(status: &Status) -> Style {
    if status.is_closed() {
        palette::TIER_FINISHED
    } else {
        Style::new()
    }
}

/// Draw the bead in a window over the forest.
///
/// The ground is blanked first, which is what stops the trees showing
/// through between the rows. The bead is named in the border's title, so a
/// window too short for a single row still says which bead it is over, and
/// where the bead is taller than the window the title says how far down it
/// the reader has got. The keys are on the foot row while the window is up.
///
/// `followable` is whether the forest can take the reader to a bead this one
/// names, which only the forest can say, and the window asks it of every
/// bead named. An id it answers for is drawn in blue and the rest keep the
/// page's tone.
///
/// The bead the window is on is drawn as the forest draws the row the
/// selection is on, and brought inside the window where the last frame left
/// it beyond what there was room for — a ring nobody can see is a ring the
/// reader has lost.
pub fn show(
    frame: &mut Frame,
    area: Rect,
    node: &Node,
    progress: Option<Progress>,
    view: &mut Show,
    followable: &dyn Fn(&Related) -> bool,
) {
    let Laid {
        page,
        window,
        inner,
    } = lay_out(area, node, progress, followable);
    if window.is_empty() {
        return;
    }
    let mut links = page.links;

    let block = window_block();
    view.fit(page.rows.len(), inner.height as usize);
    let on = view
        .on()
        .and_then(|id| at(node, id))
        .and_then(|at| page.related.get(at).copied());
    if let Some(row) = on {
        view.reveal(row);
    }
    let block = block.title(Span::styled(
        phrase::bead_window_title(&node.id, view.from, view.room, view.total),
        palette::TITLE,
    ));
    cover(frame, window);
    frame.render_widget(block, window);

    for (n, row) in page
        .rows
        .into_iter()
        .skip(view.from)
        .take(inner.height as usize)
        .enumerate()
    {
        let at = view.from + n;
        let drawn = Fitted::new(row, Vec::new(), Vec::new())
            .linking(links.remove(&at).unwrap_or_default())
            .toned(palette::PAGE);
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
    use crate::model::anomaly::Anomaly;
    use crate::model::badges::Badged;
    use crate::model::edges::Related;
    use crate::model::join::{AgentRef, JoinSource};
    use crate::model::types::testing::key;
    use crate::model::types::{Edge, PaneStatus, Status};
    use crate::view::fitted::hyperlink;
    use chrono::{DateTime, Utc};
    use ratatui::backend::TestBackend;
    use ratatui::style::Modifier;
    use ratatui::Terminal;

    use crate::view::painted::{symbols, Painted, Run};
    use pretty_assertions::assert_eq;
    use ratatui::style::Color;

    /// A date as a bead carries one, from the day alone: the head says the
    /// day and nothing finer, so the time of day is noise in a fixture.
    fn when(day: &str) -> Option<DateTime<Utc>> {
        Some(
            format!("{day}T00:00:00Z")
                .parse()
                .expect("the day is a date"),
        )
    }

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
            project: "dunwich".to_string(),
            id: "dun-7.1".to_string(),
            title: "re-point the dish".to_string(),
            status: Status::InProgress,
            issue_type: "task".to_string(),
            priority: 2,
            ready: false,
            blocked_by: Vec::new(),
            started_at: None,
            closed_at: None,
            badges: Vec::new(),
            undrawn: Vec::new(),
            agent: Some(AgentRef {
                pane: key("w:p1"),
                pane_status: PaneStatus::Working,
                title: Some("lifting the mast".to_string()),
                source: JoinSource::AgentPane,
            }),
            anomalies: Vec::new(),
            description: "Point it at the new bird.\n\nThe old one is gone.".to_string(),
            notes: "The crane is booked for Tuesday.".to_string(),
            created_by: Some("kim".to_string()),
            assignee: None,
            labels: Vec::new(),
            created_at: None,
            updated_at: None,
            parent: Some(related(
                "dun-7",
                Edge::ParentChild,
                Status::InProgress,
                "lift the ground station",
            )),
            depends_on: vec![related(
                "dun-7.3",
                Edge::Blocks,
                Status::Closed,
                "lay the feeder cable",
            )],
            blocks: vec![related(
                "dun-7.4",
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
        drawn_where(node, None, view, width, height, &nothing_followable)
    }

    /// The same window over a bead the forest has a fraction for.
    fn drawn_with(
        node: &Node,
        progress: Option<Progress>,
        view: &mut Show,
        width: u16,
        height: u16,
    ) -> Vec<String> {
        drawn_where(node, progress, view, width, height, &nothing_followable)
    }

    fn drawn_where(
        node: &Node,
        progress: Option<Progress>,
        view: &mut Show,
        width: u16,
        height: u16,
        followable: &dyn Fn(&Related) -> bool,
    ) -> Vec<String> {
        Painted::drawn_by(width, height, |frame| {
            show(frame, frame.area(), node, progress, view, followable)
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
            drawn(&a_bead(), &mut Show::default(), 44, 24),
            vec![
                "┌dun-7.1───────────────────────────────────┐",
                "│                                          │",
                "│ ◐ dun-7.1  re-point the dish             │",
                "│   in_progress · P2 · task · kim          │",
                "│   ◍ lifting the mast · working           │",
                "│                                          │",
                "│ DESCRIPTION                              │",
                "│   Point it at the new bird.              │",
                "│                                          │",
                "│   The old one is gone.                   │",
                "│                                          │",
                "│ NOTES                                    │",
                "│   The crane is booked for Tuesday.       │",
                "│                                          │",
                "│ PARENT                                   │",
                "│   ↑ ◐ dun-7  lift the ground station     │",
                "│                                          │",
                "│ DEPENDS ON                               │",
                "│   → ✓ dun-7.3  lay the feeder cable      │",
                "│                                          │",
                "│ BLOCKS                                   │",
                "│   ← ○ dun-7.4  file the licence          │",
                "└──────────────────────────────────────────┘",
                "",
            ]
        );
    }

    /// `bd show` prints nothing for a section the bead has nothing in, and a
    /// heading over nothing would be a claim that something was lost. The
    /// same holds of the head's own rows: a bead with no name on it and no
    /// date said of it gets neither row.
    #[test]
    fn sections_the_bead_has_nothing_for_are_left_out() {
        let bare = Node {
            agent: None,
            description: String::new(),
            notes: String::new(),
            created_by: None,
            assignee: None,
            created_at: None,
            updated_at: None,
            parent: None,
            depends_on: Vec::new(),
            blocks: Vec::new(),
            ..a_bead()
        };

        assert_eq!(
            drawn(&bare, &mut Show::default(), 44, 6),
            vec![
                "┌dun-7.1───────────────────────────────────┐",
                "│                                          │",
                "│ ◐ dun-7.1  re-point the dish             │",
                "│   in_progress · P2 · task                │",
                "└──────────────────────────────────────────┘",
                "",
            ]
        );
    }

    /// A bead that has all four dates says created and updated on one row
    /// and started and closed on the next, in the order the head lists them
    /// rather than the order `bd show` prints them. `bdi-d2ra`: joined on one
    /// row the four dates need eighty-two columns, two more than the window
    /// offers at its floor, so the row was cut and lost the last date's end.
    /// Split unconditionally rather than only where the width demands it, so
    /// a reader always finds a date on the same one of two rows.
    #[test]
    fn the_head_says_when_a_bead_was_created_updated_started_and_closed() {
        let closed = Node {
            status: Status::Closed,
            created_at: when("2026-03-14"),
            started_at: when("2026-03-15"),
            closed_at: when("2026-03-20"),
            updated_at: when("2026-03-20"),
            ..a_bead()
        };

        let rows = drawn(&closed, &mut Show::default(), 90, 40);

        assert!(
            rows.iter()
                .any(|row| row.contains("created 2026-03-14 · updated 2026-03-20")),
            "{rows:#?}"
        );
        assert!(
            rows.iter()
                .any(|row| row.contains("started 2026-03-15 · closed 2026-03-20")),
            "{rows:#?}"
        );
        assert!(
            rows.iter().all(|row| !row.contains('…')),
            "a date row was cut: {rows:#?}"
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
            .skip_while(|row| !row.contains(DESCRIPTION))
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

    /// A title from this project's own tracker, long enough to wrap at the
    /// width the window floors at. A short one that wrapped only in a
    /// narrowed window would be a test of the floor.
    const A_LONG_TITLE: &str = "Thirty-three of thirty-four forest rows are the terminal's default, so the three an agent is on are found by reading rather than by looking";

    fn a_bead_with_a_long_title() -> Node {
        Node {
            project: "dunwich".to_string(),
            title: A_LONG_TITLE.to_string(),
            ..a_bead()
        }
    }

    /// The rows of the window the bead's name is drawn on: from the row its
    /// glyph and id are on down to the facts under it. Found rather than
    /// counted, because the window is centred on the screen and a name that
    /// wraps is one of the things that moves it.
    fn the_name_drawn(rows: &[String]) -> Vec<&str> {
        rows.iter()
            .skip_while(|row| !row.contains("◐ dun-7.1"))
            .take_while(|row| !row.contains("in_progress"))
            .map(String::as_str)
            .collect()
    }

    /// The line naming the bead wraps for the reason its description does:
    /// the window is the one place a reader has asked for the bead in full.
    /// The rule that cuts a title is the forest's, and it stays there — a
    /// forest is a column of rows that has to line up, and this is one bead
    /// at its own height with nothing lining up against it.
    #[test]
    fn a_title_too_long_for_the_window_wraps_rather_than_being_cut() {
        let rows = drawn(&a_bead_with_a_long_title(), &mut Show::default(), 44, 30);

        let name = the_name_drawn(&rows)
            .iter()
            .map(|row| row.trim_matches(['│', ' ']))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(
            name.strip_prefix("◐ dun-7.1  "),
            Some(A_LONG_TITLE),
            "{rows:#?}"
        );
    }

    /// The rows a wrapped title takes hang under where the title starts, so
    /// the name is a block the reader sees the extent of rather than a first
    /// line and a paragraph of strays under the glyph.
    #[test]
    fn the_rows_a_wrapped_title_takes_hang_where_the_title_starts() {
        let rows = drawn(&a_bead_with_a_long_title(), &mut Show::default(), 44, 30);
        let name = the_name_drawn(&rows);
        let (first, wrapped) = name.split_first().expect("the bead is named");
        let under = first
            .find("Thirty-three")
            .map(|at| first[..at].chars().count())
            .expect("the title starts on the row the bead is named on");

        assert!(!wrapped.is_empty(), "the title did not wrap: {rows:#?}");
        for row in wrapped {
            assert_eq!(
                row.find(|glyph: char| !matches!(glyph, '│' | ' '))
                    .map(|at| row[..at].chars().count()),
                Some(under),
                "a row of the title does not hang under it: {rows:#?}"
            );
        }
    }

    /// A title the window has room for is drawn as its author wrote it, the
    /// spaces between its words included. The forest row beside the window
    /// draws it that way and `bd show` prints it that way, and the wrap is
    /// what would close a run of them up.
    #[test]
    fn a_title_the_window_has_room_for_is_drawn_as_it_was_written() {
        let spaced = Node {
            title: "re-point   the dish".to_string(),
            ..a_bead()
        };

        assert_eq!(
            drawn(&spaced, &mut Show::default(), 44, 24)[2],
            "│ ◐ dun-7.1  re-point   the dish           │"
        );
    }

    /// A window with rows enough for the name and nothing else cuts the
    /// title too. The reader would otherwise be left with the one thing they
    /// already knew, filling the window, with the status, the priority and
    /// the prose they opened it for below its foot.
    #[test]
    fn a_window_a_wrapped_name_would_fill_cuts_the_title() {
        assert_eq!(
            drawn(&a_bead_with_a_long_title(), &mut Show::default(), 44, 7),
            vec![
                "┌dun-7.1 · 1–3 of 20───────────────────────┐",
                "│                                          │",
                "│ ◐ dun-7.1  Thirty-three of thirty-four … │",
                "│   in_progress · P2 · task · kim          │",
                "│   ◍ lifting the mast · working           │",
                "└──────────────────────────────────────────┘",
                "",
            ]
        );
    }

    /// A window whose width the bead's glyph and id already fill cuts the
    /// title on that row rather than wrapping it into nothing. The wrap comes
    /// back a glyph to a row, and every one of those rows is drawn past the
    /// window's edge — so what a title of seventeen glyphs buys the reader is
    /// seventeen rows that say nothing, with the bead's facts under the last
    /// of them.
    ///
    /// A short title in a tall window is the case that reaches this: a long
    /// one is cut for the other reason, and a short one in a short window is
    /// too.
    #[test]
    fn a_window_with_no_room_beside_the_name_cuts_the_title() {
        let rows = drawn(&a_bead(), &mut Show::default(), 15, 30);
        let named = rows
            .iter()
            .position(|row| row.contains("◐ dun-7.1"))
            .expect("the bead is named");

        assert_eq!(rows[named], "│ ◐ dun-7.1 … │", "{rows:#?}");
        assert_eq!(rows[named + 1], "│   in_progr… │", "{rows:#?}");
    }

    /// A row of the view that is neither prose nor the bead's own name — the
    /// facts under the name, a bead this one points at — is one row whatever
    /// its length, cut the way a row of the forest is. The reader asked for
    /// this bead in full, and a bead it names is somewhere to go rather than
    /// something to read here.
    #[test]
    fn a_line_that_names_another_bead_is_cut_rather_than_wrapped() {
        let named = Node {
            agent: None,
            description: String::new(),
            notes: String::new(),
            depends_on: Vec::new(),
            blocks: Vec::new(),
            ..a_bead()
        };

        assert_eq!(
            drawn(&named, &mut Show::default(), 24, 10),
            vec![
                "┌dun-7.1───────────────┐",
                "│                      │",
                "│ ◐ dun-7.1  re-point  │",
                "│            the dish  │",
                "│   in_progress · P2 … │",
                "│                      │",
                "│ PARENT               │",
                "│   ↑ ◐ dun-7  lift t… │",
                "└──────────────────────┘",
                "",
            ]
        );
    }

    /// A window too short for the whole bead shows the top of it and says
    /// how to see the rest, and a motion moves it a row at a time.
    #[test]
    fn a_window_too_short_for_the_bead_scrolls_by_motion() {
        let mut view = Show::default();
        let top = drawn(&a_bead(), &mut view, 44, 6);
        assert_eq!(top[0], "┌dun-7.1 · 1–2 of 20───────────────────────┐");
        assert_eq!(top[2], "│ ◐ dun-7.1  re-point the dish             │");

        assert!(!view.scroll(Motion::PreviousRow), "already at the top");
        assert!(view.scroll(Motion::NextRow));
        let down_one = drawn(&a_bead(), &mut view, 44, 6);
        assert_eq!(down_one[2], "│   in_progress · P2 · task · kim          │");

        assert!(view.scroll(Motion::LastRow));
    }

    /// The last row of the bead is the furthest the view goes: scrolled past
    /// it there would be nothing on the screen, and a motion that moved the
    /// view nowhere is one the screen need not be redrawn for.
    #[test]
    fn the_view_stops_at_the_last_row_of_the_bead() {
        let mut view = Show::default();
        drawn(&a_bead(), &mut view, 44, 7);

        assert!(view.scroll(Motion::LastRow));
        let bottom = drawn(&a_bead(), &mut view, 44, 7);
        assert_eq!(bottom[4], "│   ← ○ dun-7.4  file the licence          │");
        assert!(!view.scroll(Motion::NextRow), "nothing below the last row");
        assert!(!view.scroll(Motion::LastRow), "already there");

        assert!(view.scroll(Motion::HalfScreenUp));
        assert!(view.scroll(Motion::FirstRow));
        assert_eq!(
            drawn(&a_bead(), &mut view, 44, 7)[2],
            "│ ◐ dun-7.1  re-point the dish             │"
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

    /// Every bead the window names is named again by the row it was drawn on,
    /// which is what a pointer landing there has to be answered with.
    ///
    /// Over a screen the bead fills and one twice as tall, because the row a
    /// reference lands on follows from where the window was put — and a click
    /// worked out from a placement of its own would agree with the frame on
    /// the first screen and answer for a window nobody drew on the second.
    #[test]
    fn the_row_a_reference_was_drawn_on_names_the_bead_it_names() {
        let bead = a_bead();

        for height in [TALL, TALL * 2] {
            let mut view = Show::default();
            let rows = drawn(&bead, &mut view, WIDE, height);

            for (id, drawn_as) in [
                ("dun-7", "dun-7  lift the ground station"),
                ("dun-7.3", "dun-7.3  lay the feeder cable"),
                ("dun-7.4", "dun-7.4  file the licence"),
            ] {
                assert_eq!(
                    drawn_at(
                        over(WIDE, height),
                        &bead,
                        None,
                        &view,
                        drawn_on(&rows, drawn_as),
                        &nothing_followable,
                    ),
                    Drawn::Related(id),
                    "at {height} rows the row {drawn_as:?} was drawn on names \
                     another bead: {rows:#?}"
                );
            }
        }
    }

    /// A row of the page that names no bead is the page and nothing more, so
    /// a reader who aims at a reference and misses by a row keeps the window
    /// they were aiming in.
    #[test]
    fn a_row_of_the_page_that_names_no_bead_is_the_page() {
        let bead = a_bead();
        let mut view = Show::default();
        let rows = drawn(&bead, &mut view, WIDE, TALL);

        for drawn_as in ["PARENT", "Point it at the new bird", "re-point the dish"] {
            assert_eq!(
                drawn_at(
                    over(WIDE, TALL),
                    &bead,
                    None,
                    &view,
                    drawn_on(&rows, drawn_as),
                    &nothing_followable,
                ),
                Drawn::Page,
                "the row {drawn_as:?} was drawn on is not the page: {rows:#?}"
            );
        }
    }

    /// The window's own border is not the page. It is the row the way back is
    /// written on, and the only row off the page a window as tall as the
    /// screen has — so a reader holding a pointer alone could otherwise not
    /// leave one.
    #[test]
    fn the_windows_border_is_not_the_page() {
        let bead = a_bead();
        let mut view = Show::default();
        let rows = drawn(&bead, &mut view, WIDE, TALL);

        assert_eq!(
            drawn_at(
                over(WIDE, TALL),
                &bead,
                None,
                &view,
                drawn_on(&rows, "┌dun-7.1"),
                &nothing_followable,
            ),
            Drawn::Beyond,
            "the title is drawn on the page: {rows:#?}"
        );
        assert_eq!(
            drawn_at(
                over(WIDE, TALL),
                &bead,
                None,
                &view,
                drawn_on(&rows, "└─"),
                &nothing_followable,
            ),
            Drawn::Beyond,
            "the foot of the window is drawn on the page: {rows:#?}"
        );
    }

    /// And neither is the foot's row, which the window stops above and is
    /// the one row of the screen it is not drawn on.
    #[test]
    fn the_row_under_the_window_is_not_the_page() {
        let bead = a_bead();
        let mut view = Show::default();
        let over_a_taller_screen = over(WIDE, TALL * 2);
        let rows = drawn(&bead, &mut view, WIDE, TALL * 2);

        let under = drawn_on(&rows, "└─") + 1;
        assert_eq!(
            under,
            TALL * 2 - 1,
            "the window stops above the foot: {rows:#?}"
        );
        assert_eq!(
            drawn_at(
                over_a_taller_screen,
                &bead,
                None,
                &view,
                under,
                &nothing_followable,
            ),
            Drawn::Beyond,
            "the foot's row is drawn on the page: {rows:#?}"
        );
    }

    /// A window the reader has scrolled answers for the bead now drawn on a
    /// row, not the one that was there before they moved.
    #[test]
    fn a_scrolled_window_names_what_is_drawn_on_a_row_now() {
        let bead = a_bead();
        let mut view = Show::default();
        let short = TALL / 2;
        let before = drawn(&bead, &mut view, WIDE, short);
        assert!(
            !before.iter().any(|row| row.contains("file the licence")),
            "the whole bead fits, so there is nothing to scroll: {before:#?}"
        );

        assert!(view.scroll(Motion::LastRow));
        let after = drawn(&bead, &mut view, WIDE, short);

        assert_eq!(
            drawn_at(
                over(WIDE, short),
                &bead,
                None,
                &view,
                drawn_on(&after, "dun-7.4  file the licence"),
                &nothing_followable,
            ),
            Drawn::Related("dun-7.4"),
            "the row the last reference was scrolled onto names another bead: {after:#?}"
        );
    }

    /// A reference can be the first row of the page a scrolled window
    /// draws, and what is above it is then the blank row over the head
    /// rather than the heading it sits under.
    ///
    /// Which is what a reader aiming at that reference and missing upward
    /// hits, and the blank row is off the page, so it takes the window away.
    /// The trade is deliberate: the alternative leaves a window as tall as
    /// the screen — every window on a screen of twenty-four rows or fewer —
    /// with no row a pointer can close it on at all. A blank row inside a
    /// drawn frame can be seen; a near miss onto one costs the press that
    /// opens the window again.
    #[test]
    fn a_scrolled_window_can_draw_a_reference_under_the_blank_row() {
        let bead = a_bead();
        let mut view = Show::default();
        let barely_taller_than_the_sections = 8;
        drawn(&bead, &mut view, WIDE, barely_taller_than_the_sections);
        assert!(view.scroll(Motion::LastRow));
        let rows = drawn(&bead, &mut view, WIDE, barely_taller_than_the_sections);

        let under_the_blank_row = drawn_on(&rows, "┌dun-7.1") + 2;
        assert_eq!(
            drawn_at(
                over(WIDE, barely_taller_than_the_sections),
                &bead,
                None,
                &view,
                under_the_blank_row,
                &nothing_followable,
            ),
            Drawn::Related("dun-7.3"),
            "no reference is drawn there, so this says nothing: {rows:#?}"
        );
        assert_eq!(
            drawn_at(
                over(WIDE, barely_taller_than_the_sections),
                &bead,
                None,
                &view,
                under_the_blank_row - 1,
                &nothing_followable,
            ),
            Drawn::Beyond,
            "the blank row over the head is not the page: {rows:#?}"
        );
    }

    /// Wide enough for the bead's own rows to be drawn whole, and tall enough
    /// for every section of it.
    const WIDE: u16 = 44;
    const TALL: u16 = 24;

    /// The whole of a screen of that size.
    fn over(width: u16, height: u16) -> Rect {
        Rect::new(0, 0, width, height)
    }

    /// The row of a drawn frame something was drawn on.
    ///
    /// Read back off the frame rather than counted out, because which row
    /// anything lands on follows from the window's height and how far the
    /// view has scrolled — and a test that worked one out would be running
    /// the arithmetic it is checking a second time, green whenever both
    /// copies are wrong the same way.
    fn drawn_on(rows: &[String], drawn_as: &str) -> u16 {
        let on = rows
            .iter()
            .position(|row| row.contains(drawn_as))
            .unwrap_or_else(|| panic!("{drawn_as:?} was drawn on no row of {rows:#?}"));
        u16::try_from(on).expect("no screen is that tall")
    }

    /// A window too short for a single row of the bead still says which bead
    /// it is over: the title is the one row that survives every cut, and a
    /// reader shown a window naming nothing cannot tell what they opened.
    #[test]
    fn the_bead_is_named_in_the_windows_title_however_short_the_screen() {
        for height in [1, 2, 3, 8] {
            let rows = drawn(&a_bead(), &mut Show::default(), 44, height);
            assert!(
                rows[0].contains("dun-7.1"),
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
                id: "dun-9".to_string(),
                edge: Edge::Blocks,
                status: None,
                title: None,
            }],
            ..a_bead()
        };
        let rows = drawn(&dangling, &mut Show::default(), 50, 24);

        assert!(
            rows.contains(&"│   → dun-9  not in the tracker's answer         │".to_string()),
            "{rows:#?}"
        );
    }

    /// An edge of a kind beads may add later is listed with what it waits
    /// on, and says which kind it is, since the arrow alone reads as blocks.
    #[test]
    fn an_edge_of_a_kind_bdi_does_not_know_says_which_kind() {
        let odd = Node {
            depends_on: vec![related(
                "dun-2",
                Edge::Other("relates-to".to_string()),
                Status::Open,
                "the survey",
            )],
            ..a_bead()
        };
        let rows = drawn(&odd, &mut Show::default(), 50, 24);

        assert!(
            rows.contains(&"│   → ○ dun-2  the survey · “relates-to”         │".to_string()),
            "{rows:#?}"
        );
    }

    /// `^D` and `^U` move the view half the window, so a reader paging
    /// through a long bead lands where they expect: two rows down a
    /// four-row window, not four and not one.
    ///
    /// Eight rows of screen are what four rows of window costs: two borders,
    /// the blank row over the head, and the foot the window stops above.
    #[test]
    fn a_half_screen_motion_moves_the_view_half_the_window() {
        let mut view = Show::default();
        drawn(&a_bead(), &mut view, 44, 8);

        assert!(view.scroll(Motion::HalfScreenDown));
        assert_eq!(
            drawn(&a_bead(), &mut view, 44, 8)[2],
            "│   ◍ lifting the mast · working           │"
        );
        assert!(view.scroll(Motion::HalfScreenDown));
        assert_eq!(
            drawn(&a_bead(), &mut view, 44, 8)[2],
            "│ DESCRIPTION                              │"
        );
        assert!(view.scroll(Motion::HalfScreenUp));
        assert_eq!(
            drawn(&a_bead(), &mut view, 44, 8)[2],
            "│   ◍ lifting the mast · working           │"
        );
    }

    /// A notch of the wheel moves the view the distance it is handed, which
    /// is further than the row a key moves it — the same wheel over the same
    /// gesture covers the same ground here as over the forest.
    #[test]
    fn a_notch_moves_the_view_as_far_as_it_is_told() {
        let mut view = Show::default();
        drawn(&a_bead(), &mut view, 44, 6);
        let mut stepped = Show::default();
        drawn(&a_bead(), &mut stepped, 44, 6);

        assert!(view.scrolled(Notch::Down, 3));
        for _ in 0..3 {
            stepped.scroll(Motion::NextRow);
        }
        assert_eq!(
            drawn(&a_bead(), &mut view, 44, 6),
            drawn(&a_bead(), &mut stepped, 44, 6)
        );

        assert!(view.scrolled(Notch::Up, 3));
        assert_eq!(
            drawn(&a_bead(), &mut view, 44, 6)[2],
            "│ ◐ dun-7.1  re-point the dish             │"
        );
        assert!(!view.scrolled(Notch::Up, 3), "already at the top");
    }

    /// The window follows the terminal: on a wide screen it is four fifths of
    /// the width, against the right edge, and the prose wraps to that rather
    /// than to eighty columns or to the screen's edge.
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
        assert_eq!(top.chars().nth(40), Some('┌'), "{top:?}");
        assert_eq!(top.chars().nth(199), Some('┐'), "{top:?}");
        let first = rows
            .iter()
            .find(|row| row.contains("abcde"))
            .expect("the prose is drawn");
        assert_eq!(
            first.matches("abcde").count(),
            25,
            "a twenty-sixth word of five, with the space before it, would be a \
             hundred and fifty-five columns of the hundred and fifty-four left \
             inside the margin and the indent: {first:?}"
        );
        assert!(
            !first.contains('…'),
            "prose wrapped to the window is never cut at it: {first:?}"
        );
    }

    /// Four fifths of a small screen would be a cramped box, so the window
    /// is never offered less than eighty-two columns inside its border: on a
    /// screen a little wider than that, the floor wins over the proportion.
    /// Eighty-two rather than eighty is the margin being paid for by the
    /// frame, so the prose still gets the eighty it wraps at.
    #[test]
    fn a_screen_not_much_wider_than_eighty_two_columns_gives_the_window_eighty_two() {
        let rows = drawn(&a_bead(), &mut Show::default(), 90, 30);
        let top = rows
            .iter()
            .find(|row| row.contains('┌'))
            .expect("the window's top edge is drawn");
        assert_eq!(top.chars().nth(6), Some('┌'), "{top:?}");
        assert_eq!(top.chars().nth(89), Some('┐'), "{top:?}");
    }

    /// The window's three edges: the right border on the screen's last
    /// column, the top on its first row, and the bottom on the row above the
    /// foot. What the window gives up in the middle of the screen it gets
    /// back at the top and the bottom.
    #[test]
    fn the_window_is_a_drawer_against_the_right_edge_above_the_foot() {
        let rows = drawn(&a_bead(), &mut Show::default(), 120, 40);

        let top: Vec<char> = rows[0].chars().collect();
        assert_eq!(top.iter().position(|c| *c == '┌'), Some(24), "{rows:#?}");
        assert_eq!(top.iter().rposition(|c| *c == '┐'), Some(119), "{rows:#?}");
        assert_eq!(drawn_on(&rows, "└─"), 38, "{rows:#?}");
        assert_eq!(rows[39], "", "the foot's row is left clear: {rows:#?}");
    }

    /// And it is that tall whatever the bead is: a bead of four rows leaves
    /// the rest of the page blank rather than pulling the bottom border up to
    /// meet it, so the window a reader opens is the window they closed.
    #[test]
    fn a_bead_shorter_than_the_window_still_gets_the_full_height() {
        let bare = Node {
            agent: None,
            notes: String::new(),
            parent: None,
            depends_on: Vec::new(),
            blocks: Vec::new(),
            ..a_bead()
        };
        let rows = drawn(&bare, &mut Show::default(), 120, 40);

        let bottom = drawn_on(&rows, "└─");
        let last_said = rows[..bottom as usize]
            .iter()
            .rposition(|row| !row.trim_matches(['│', ' ']).is_empty())
            .expect("the bead is drawn");
        assert!(
            last_said + 1 < bottom as usize,
            "the bead has to end well above the border for this to be about \
             the height: {rows:#?}"
        );
        assert_eq!(bottom, 38, "{rows:#?}");
    }

    #[test]
    fn a_window_with_no_room_inside_it_draws_nothing_inside_it() {
        assert_eq!(
            drawn(&a_bead(), &mut Show::default(), 44, 3),
            vec![
                "┌dun-7.1───────────────────────────────────┐",
                "└──────────────────────────────────────────┘",
                "",
            ]
        );
    }

    /// Every row sits one column in from each side of the border, which is
    /// what a reader asked for and what no frame compared character for
    /// character can check: the rows those compare are trimmed of their
    /// trailing spaces, and the right-hand margin is trailing spaces.
    ///
    /// A title long enough to be cut and prose long enough to wrap are what
    /// put a row against a side, and the sweep runs over screens narrower
    /// than the floor, at the floor, and wide enough for the proportion to
    /// win, so a row is cut at some widths and wrapped at others.
    #[test]
    fn no_row_is_drawn_in_the_column_beside_a_border() {
        let bead = Node {
            description: "abcde ".repeat(40).trim().to_string(),
            ..a_bead_with_a_long_title()
        };
        let mut read = 0;

        for width in [16, 30, 44, 90, 200] {
            for height in 4..=26 {
                let mut view = Show::default();
                let painted = Painted::drawn_by(width, height, |frame| {
                    show(
                        frame,
                        frame.area(),
                        &bead,
                        None,
                        &mut view,
                        &nothing_followable,
                    );
                });
                let margins = painted
                    .margins(lay_out(over(width, height), &bead, None, &nothing_followable).window);
                read += margins.chars().count();

                assert_eq!(
                    margins.trim(),
                    "",
                    "at {width} by {height}: {:#?}",
                    painted.rows()
                );
            }
        }

        assert!(read > 0, "the sweep drew no window with an inside to read");
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
        painted_where(node, width, height, &nothing_followable)
    }

    /// The same window over a bead some of whose references the forest
    /// draws, which is what a test about where the blue falls wants.
    fn painted_where(
        node: &Node,
        width: u16,
        height: u16,
        followable: &dyn Fn(&Related) -> bool,
    ) -> Painted {
        Painted::drawn_by(width, height, |frame| {
            show(
                frame,
                frame.area(),
                node,
                None,
                &mut Show::default(),
                followable,
            )
        })
    }

    /// A forest that draws none of them, for the tests that are not about
    /// following one: the title then offers no key it would be pressed for
    /// nothing, and no id is blue.
    fn nothing_followable(_related: &Related) -> bool {
        false
    }

    /// A forest that draws every bead the page names, for the tests that
    /// want the keys offered and do not care which row they land on.
    fn everything_followable(_related: &Related) -> bool {
        true
    }

    /// The forest draws one bead of the page and not the rest, which is what
    /// puts a followable row and an unfollowable one on the same screen.
    fn only(id: &'static str) -> impl Fn(&Related) -> bool {
        move |related: &Related| related.id == id
    }

    /// What every run of the window that answers `chosen` says, in the order
    /// the window draws them, with the border they are written inside taken
    /// off and the blanks dropped.
    fn said_where(painted: &Painted, chosen: impl Fn(&Run) -> bool) -> Vec<String> {
        (0..painted.rows().len())
            .flat_map(|y| painted.row(y))
            .filter(|run| chosen(run))
            .map(|run| {
                run.said
                    .replace(['│', '─', '┌', '┐', '└', '┘'], "")
                    .trim()
                    .to_string()
            })
            .filter(|said| !said.is_empty())
            .collect()
    }

    /// One of each status the forest gives a colour, so a loop over them
    /// covers the palette.
    fn every_coloured_status() -> [Status; 7] {
        [
            Status::InProgress,
            Status::Hooked,
            Status::Blocked,
            Status::Closed,
            Status::Deferred,
            Status::Pinned,
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
            let top = painted(&bead, 44, 24).row(2);

            let glyph = run_saying(&top, &status_glyph(&status).to_string());
            assert_eq!(glyph.said, status_glyph(&status).to_string(), "{top:?}");
            assert_eq!(
                glyph.style.fg,
                status_style(&status).fg,
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
        let top = painted(&open, 44, 24).row(2);

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
        let marker = painted(&a_bead(), 44, 24).row(4);

        assert_eq!(
            run_saying(&marker, "◍ lifting the mast · working").style.fg,
            palette::AGENT.fg,
            "{marker:?}"
        );
    }

    /// Read off `bd show` 1.2.2's own output: the id at the head of the page
    /// is always this blue, whatever the status.
    #[test]
    fn the_id_is_painted_the_blue_bd_show_paints_it() {
        let top = painted(&a_bead(), 44, 24).row(2);

        assert_eq!(
            run_saying(&top, "dun-7.1").style.fg,
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
    /// the type and the owner take no colour of their own and are the page's.
    #[test]
    fn the_facts_row_says_the_status_in_its_colour_and_the_rest_on_the_page() {
        for status in every_coloured_status() {
            let bead = Node {
                status: status.clone(),
                ..a_bead()
            };
            let facts = painted(&bead, 44, 24).row(3);

            let word = run_saying(&facts, &phrase::status_word(&status));
            assert_eq!(
                word.style.fg,
                status_style(&status).fg,
                "{status:?}: {facts:?}"
            );
            assert_eq!(
                run_saying(&facts, "P2 · task · kim").style.fg,
                palette::PAGE.fg,
                "{status:?}: {facts:?}"
            );
        }
    }

    /// A related bead's glyph is the same glyph the forest and the head of
    /// the window paint, and it takes the same colour — or, for an open one,
    /// the brightness of the row it sits on, which on the page is the page's.
    #[test]
    fn a_related_beads_glyph_is_painted_the_colour_of_its_status() {
        let painted = painted(&a_bead(), 44, 24);
        let depends_on = painted.row(18);
        let blocks = painted.row(21);

        assert_eq!(
            run_saying(&depends_on, "✓").style.fg,
            status_style(&Status::Closed).fg,
            "{depends_on:?}"
        );
        assert_eq!(
            run_saying(&blocks, "○").style.fg,
            palette::PAGE.fg,
            "{blocks:?}"
        );
    }

    /// `bd show` dims a closed related bead's id and title to the grey it
    /// dims a finished row to, and leaves the arrow alone; an open one is
    /// the page's, as a row nobody is on is the forest's middle rung.
    #[test]
    fn a_closed_related_bead_is_dimmed_as_bd_show_dims_one() {
        let painted = painted(&a_bead(), 44, 24);
        let depends_on = painted.row(18);
        let blocks = painted.row(21);

        assert_eq!(
            run_saying(&depends_on, "dun-7.3  lay the feeder cable")
                .style
                .fg,
            palette::TIER_FINISHED.fg,
            "{depends_on:?}"
        );
        assert_eq!(
            run_saying(&depends_on, "→").style.fg,
            Some(Color::Reset),
            "the arrow says the edge, not the state: {depends_on:?}"
        );
        assert_eq!(
            run_saying(&blocks, "dun-7.4  file the licence").style.fg,
            palette::PAGE.fg,
            "{blocks:?}"
        );
    }

    /// `bd show` prints a section heading bold and in no colour, and the
    /// window draws it as `bd show` does.
    #[test]
    fn a_heading_is_bold_and_no_colour_as_bd_show_prints_one() {
        let heading = run_saying(&painted(&a_bead(), 44, 24).row(6), DESCRIPTION);

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
                    "dun-7.3",
                    Edge::Blocks,
                    status.clone(),
                    "the cable",
                )],
                ..a_bead()
            };
            let rows = painted(&bead, 44, 24).rows();
            let glyph = status_glyph(&status).to_string();

            assert!(
                rows[2].contains(&glyph),
                "{status:?} lost its glyph: {rows:#?}"
            );
            assert!(
                rows[3].contains(&phrase::status_word(&status)),
                "{status:?} lost its word: {rows:#?}"
            );
            assert!(
                rows[18].contains(&format!("→ {glyph} dun-7.3")),
                "{status:?} lost its glyph on a related row: {rows:#?}"
            );
        }
    }

    /// Weight is the whole of the window's emphasis, so what carries one is
    /// the list of things the reader is meant to navigate by: the border
    /// title, and the name of each section. Nothing else in the window takes it —
    /// the bead's own title stands out by being the row the window opens on
    /// rather than by a treatment.
    #[test]
    fn what_the_window_draws_at_a_weight_is_its_title_and_its_section_names() {
        let bead = a_bead();
        let painted = painted(&bead, 44, 24);

        let own: Vec<String> = said_where(&painted, |run| {
            run.style.add_modifier.contains(Modifier::BOLD)
        });

        assert_eq!(
            own,
            vec![
                bead.id.clone(),
                DESCRIPTION.to_string(),
                NOTES.to_string(),
                PARENT.to_string(),
                DEPENDS_ON.to_string(),
                BLOCKS.to_string(),
            ],
            "{painted:?}"
        );
    }

    /// And what the window colours is what it quotes: `bd`'s own hue for a
    /// status, its blue for an id, its grey for a bead that is finished, and
    /// the one fact `bd` cannot say, which is that an agent is here. Its own
    /// words — the title, the prose, the name of a bead that is still going —
    /// are the terminal's foreground and no tone at all, so the day the page
    /// takes a tone of its own again this list is where it turns up.
    #[test]
    fn the_only_tones_the_window_draws_are_the_ones_it_quotes() {
        let painted = painted(&a_bead(), 44, 24);

        let toned: Vec<String> = said_where(&painted, |run| run.style.fg != Some(Color::Reset));

        assert_eq!(
            toned,
            vec![
                "◐".to_string(),
                "dun-7.1".to_string(),
                phrase::status_word(&Status::InProgress),
                "◍ lifting the mast · working".to_string(),
                "◐".to_string(),
                "✓".to_string(),
                "dun-7.3  lay the feeder cable".to_string(),
            ],
            "{painted:?}"
        );
    }

    /// The page is the terminal's own foreground, as the head above it is:
    /// the prose, and a related bead's id and title. The window spends no
    /// brightness at all, so a reader who learned in the forest that a dim
    /// row is one nobody is on does not then meet a page that is entirely
    /// dim.
    #[test]
    fn the_page_is_drawn_at_the_terminals_own_foreground_as_the_head_is() {
        let painted = painted(&a_bead(), 44, 24);

        for (y, said) in [
            (7, "Point it at the new bird."),
            (12, "The crane is booked for Tuesday."),
            (15, "dun-7  lift the ground station"),
        ] {
            let run = run_saying(&painted.row(y), said);
            assert_eq!(run.style.fg, Some(Color::Reset), "{said}: {run:?}");
        }
    }

    /// Blue means a bead you can go to. An id the forest draws takes `bd`'s
    /// own blue for an id; one it draws nowhere keeps the page's tone, so a
    /// reader sees which rows `Tab` will stop on before they press it.
    #[test]
    fn a_related_beads_id_is_blue_only_where_the_forest_can_be_gone_to_it() {
        let painted = painted_where(&a_bead(), 44, 24, &only("dun-7"));

        let drawn = painted.row(15);
        assert_eq!(
            run_saying(&drawn, "dun-7").style.fg,
            palette::IDENTITY.fg,
            "the id of a bead the forest draws: {drawn:?}"
        );
        assert_eq!(
            run_saying(&drawn, "lift the ground station").style.fg,
            palette::PAGE.fg,
            "the title beside it: {drawn:?}"
        );

        let undrawn = painted.row(21);
        assert_eq!(
            run_saying(&undrawn, "dun-7.4").style.fg,
            palette::PAGE.fg,
            "the id of a bead the forest draws nowhere: {undrawn:?}"
        );
    }

    /// Both things are true of a closed bead the forest draws, and the row
    /// says both: the finished tier on its title, as a finished row of the
    /// forest has, and the blue on its id.
    #[test]
    fn a_closed_bead_the_forest_draws_is_finished_toned_with_a_blue_id() {
        let drawn = painted_where(&a_bead(), 44, 24, &only("dun-7.3")).row(18);

        assert_eq!(
            run_saying(&drawn, "dun-7.3").style.fg,
            palette::IDENTITY.fg,
            "{drawn:?}"
        );
        assert_eq!(
            run_saying(&drawn, "lay the feeder cable").style.fg,
            palette::TIER_FINISHED.fg,
            "{drawn:?}"
        );
    }

    /// A row for a bead the forest draws nowhere is the row it always was,
    /// span for span. Two spans of one style are drawn exactly as the one
    /// span they came from, so the screen cannot say whether a row was split
    /// and the spans are the only place the question is answered.
    #[test]
    fn a_row_the_forest_cannot_be_gone_to_says_its_id_and_its_title_in_one_span() {
        let bead = a_bead();
        let page = said(&bead, None, 60, 60, &nothing_followable);

        assert_eq!(
            page.rows[page.related[0]],
            vec![
                Span::raw(indent()),
                Span::styled(format!("{UP} "), palette::STRUCTURE),
                Span::styled("◐", status_style(&Status::InProgress)),
                Span::styled(" dun-7  lift the ground station", Style::new()),
            ]
        );
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
        let prose = painted(&bead, 44, 24).row(7);

        let bold = run_saying(&prose, "new");
        assert!(
            bold.style.add_modifier.contains(Modifier::BOLD),
            "{prose:?}"
        );
        assert_eq!(bold.style.fg, palette::PAGE.fg, "{prose:?}");
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
            vec!["dun-7", "dun-7.3", "dun-7.4"]
        );
    }

    /// The ordinals the window keeps are into that list, so the row each one
    /// is drawn on has to come back in the same order. Two statements of one
    /// order is what this stops drifting apart.
    #[test]
    fn the_rows_the_page_reports_are_the_beads_it_names_in_that_order() {
        let bead = a_bead();
        let page = said(&bead, None, 60, 60, &nothing_followable);

        assert_eq!(
            page.related.len(),
            super::related(&bead).len(),
            "a row for each bead named"
        );
        // Read the row and check it says the bead of the same ordinal, rather
        // than searching the list for whichever id the row contains: `dun-7`
        // is inside `dun-7.3`, so a search finds the parent on every row and
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

    /// The row the window is on is drawn the way the forest draws the row its
    /// own selection is on, so one reverse means one thing on this screen.
    #[test]
    fn the_bead_the_window_is_on_is_drawn_as_a_selected_row_is() {
        let bead = a_bead();
        let mut view = Show::default();
        view.go_to("dun-7");

        // Found by what the row says rather than by its number: the window is
        // centred, so a page row and a screen row are not the same count.
        let reversed = |said: &str, view: &mut Show| {
            Painted::drawn_by(60, 24, |frame| {
                show(
                    frame,
                    frame.area(),
                    &bead,
                    None,
                    view,
                    &everything_followable,
                )
            })
            .rows()
            .iter()
            .enumerate()
            .find(|(_, row)| row.contains(said))
            .map(|(at, _)| at)
            .map(|at| {
                Painted::drawn_by(60, 24, |frame| {
                    show(
                        frame,
                        frame.area(),
                        &bead,
                        None,
                        view,
                        &everything_followable,
                    )
                })
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
        let page = said(&bead, None, 58, 60, &nothing_followable);
        let last = *page.related.last().expect("a bead names beads");

        view.go_to("dun-7.4");
        let drawn = drawn_where(&bead, None, &mut view, 60, 8, &everything_followable);

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

    /// Somewhere a badge can take a reader, in the invented vocabulary the
    /// fixtures share.
    const SOMEWHERE: &str = "https://example.invalid/dunwich/12";

    /// A bead the forest row has something to say about in every cell it
    /// has: a linked badge and an unlinked one, two anomalies, and an agent
    /// the join inferred rather than confirmed.
    fn a_busy_bead() -> Node {
        Node {
            project: "dunwich".to_string(),
            badges: vec![
                Badged {
                    key: "pr".to_string(),
                    text: "⇢ #12".to_string(),
                    short: None,
                    link: Some(SOMEWHERE.to_string()),
                    colour: None,
                },
                Badged {
                    key: "waiting".to_string(),
                    text: "⏸ waiting".to_string(),
                    short: None,
                    link: None,
                    colour: None,
                },
            ],
            anomalies: vec![Anomaly::StaleClaim { days: 58 }, Anomaly::StalePane],
            agent: Some(AgentRef {
                pane: key("w:p1"),
                pane_status: PaneStatus::Working,
                title: Some("lifting the mast".to_string()),
                source: JoinSource::DisplayAgent,
            }),
            labels: vec!["mast".to_string(), "weather".to_string()],
            created_by: Some("Mira Vance".to_string()),
            assignee: Some("Rowan Ash".to_string()),
            created_at: when("2026-03-14"),
            updated_at: when("2026-03-16"),
            started_at: when("2026-03-15"),
            ..a_bead()
        }
    }

    /// The head: the rows between the blank one the window opens on and the
    /// blank one that opens the first section, without the border either
    /// side of them.
    fn head_of(drawn: &[String]) -> Vec<String> {
        drawn
            .iter()
            .skip(2)
            .map(|row| {
                row.split_once('│')
                    .map_or("", |(_, inside)| inside)
                    .trim_end_matches('│')
                    .trim_end()
                    .to_string()
            })
            .take_while(|row| !row.is_empty())
            .collect()
    }

    /// Every symbol the window drew, escapes and all.
    fn symbols_of(node: &Node, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test backend");
        terminal
            .draw(|frame| {
                show(
                    frame,
                    frame.area(),
                    node,
                    None,
                    &mut Show::default(),
                    &nothing_followable,
                );
            })
            .expect("a draw into memory");
        symbols(terminal.backend().buffer())
    }

    /// `bdi-0d4p`: the head is the forest row unfolded, so everything the row
    /// says about a bead the window says at its whole width — the agent with
    /// its state and the join's caveat, each anomaly on a row of its own, the
    /// badges on one row, and the fraction under them.
    ///
    /// And beside them the facts the row has no width to carry: the labels
    /// between the id and the title, the owner and the assignee by name, and
    /// the dates on two rows of their own under the facts. This is what
    /// fixes their order, which is the one `design.md` lists.
    #[test]
    fn the_head_says_what_the_forest_row_says() {
        let drawn = drawn_with(
            &a_busy_bead(),
            Some(Progress {
                finished: 3,
                total: 7,
            }),
            &mut Show::default(),
            90,
            24,
        );

        assert_eq!(
            head_of(&drawn),
            vec![
                " ◐ dun-7.1  mast, weather  re-point the dish",
                "   in_progress · P2 · task · Mira Vance · assignee Rowan Ash",
                "   created 2026-03-14 · updated 2026-03-16",
                "   started 2026-03-15",
                "   ◍ lifting the mast · working · inferred, not confirmed",
                "   ⚠ claimed · untouched for 58 days",
                "   ⚠ closed · its pane is still alive",
                "   ⇢ #12  ⏸ waiting",
                "   3/7",
            ],
            "{drawn:#?}"
        );
    }

    /// A badge that opens somewhere from the forest row opens there from the
    /// window too: a reader who opened the bead to read it in full should not
    /// have to close it again to follow what it points at.
    #[test]
    fn a_badge_the_forest_links_is_a_link_in_the_head() {
        let said = symbols_of(&a_busy_bead(), 90, 24);

        assert!(
            said.contains(
                &hyperlink("⇢ #12", SOMEWHERE).expect("this vocabulary holds no control character")
            ),
            "the badge opens nowhere from the window: {said:?}"
        );
    }
}
