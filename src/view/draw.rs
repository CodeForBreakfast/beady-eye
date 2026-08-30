//! The forest and the tail, drawn into a ratatui frame.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use ratatui::Frame;

use crate::model::snapshot::{Counts, HerdrState, LoosePane, TrackerFailure, TrackerState, Tree};
use crate::model::types::Status;
use crate::view::forest::{self, Content, Forest, Group, GroupKind, Item, Note};
use crate::view::phrase;
use crate::view::row::{self, Row, AGENT, WARNING};
use crate::view::tail::Tail;

/// The mark left where a line ran out of width, so a cut line reads as cut
/// rather than as one that had nothing more to say.
const CUT: char = '…';

/// What the rule above the tail is drawn from.
const RULE: char = '─';

/// The blank columns that keep two blocks from reading as one.
const GAP: usize = 2;

/// The keys the view answers to, in the order the design lists them.
const KEYS: &str = "⏎ focus   a all   ^R refresh   q quit";

/// What lifts the live-agent filter, said beside the trees it is holding back.
const SHOW_ALL: &str = "a to show all";

const LIVE: Color = Color::Green;
const LOOK_AT_THIS: Color = Color::Yellow;

/// Draw the forest and the key bar, leaving the tail's band to whoever holds
/// a tail.
pub fn draw(frame: &mut Frame, area: Rect, forest: &Forest) {
    let bands = regions(area);
    let lines = forest.lines();
    let selected = forest.selected_line();
    let height = bands.forest.height as usize;
    let ids = id_width(lines);

    for (row, (at, line)) in lines
        .iter()
        .enumerate()
        .skip(scroll_offset(selected, lines.len(), height))
        .take(height)
        .enumerate()
    {
        let drawn = fitted(line, ids);
        let drawn = if at == selected {
            drawn.selected()
        } else {
            drawn
        };
        frame.render_widget(
            drawn,
            Rect {
                y: bands.forest.y + row as u16,
                height: 1,
                ..bands.forest
            },
        );
    }

    frame.render_widget(status_bar(forest.snapshot().herdr), bands.keys);
}

/// The widest abbreviated id on screen, so every title starts in the same
/// column and a reader's eye runs down one edge rather than a ragged one.
fn id_width(lines: &[forest::Line]) -> usize {
    lines
        .iter()
        .filter_map(|line| match &line.content {
            Content::Bead(row) => Some(columns(&[Span::raw(row.id.clone())])),
            _ => None,
        })
        .max()
        .unwrap_or(0)
}

/// One line of the forest, whatever kind it is.
fn fitted(line: &forest::Line, id_width: usize) -> Fitted {
    match &line.content {
        Content::Tree(head) => header(&head.tree, &line.prefix, &head.panes, head.panes_complete),
        Content::Bead(row) => bead_line(row, &line.prefix, id_width),
        Content::Elided { count } => {
            sentence(&line.prefix, phrase::elided(*count), Color::DarkGray)
        }
        Content::Note(note) => sentence(&line.prefix, finding(*note), LOOK_AT_THIS),
        Content::Group(group) => group_line(&line.prefix, *group),
        Content::Item(item) => item_line(&line.prefix, item),
    }
}

/// A line that is one sentence and nothing else.
fn sentence(prefix: &str, said: String, colour: Color) -> Fitted {
    Fitted::new(
        vec![Span::styled(
            format!("{prefix}{said}"),
            Style::new().fg(colour),
        )],
        Vec::new(),
        Vec::new(),
    )
}

/// A finding about the tree above, in `bdi`'s words for it.
fn finding(note: Note) -> String {
    let said = match note {
        Note::Dangling(count) => phrase::dangling(count),
        Note::Unreachable(count) => phrase::unreachable(count),
        Note::Truncated(count) => phrase::truncated_nodes(count),
    };
    format!("{WARNING} {said}")
}

/// One of the groups below the trees. The hidden trees are the only group
/// nothing went wrong in — the filter put them there and a key takes them
/// back out — so they are the only one drawn without a warning.
fn group_line(prefix: &str, group: Group) -> Fitted {
    let (said, hidden) = match group.kind {
        GroupKind::FailedProjects => (phrase::failed_projects(group.count), false),
        GroupKind::Conflicts => (phrase::conflicts(group.count), false),
        GroupKind::HiddenTrees => (phrase::hidden_trees(group.count, group.with_findings), true),
        GroupKind::Unattributed => (phrase::unattributed(group.count), false),
    };

    let (said, colour) = if hidden {
        (said, Color::Reset)
    } else {
        (format!("{WARNING} {said}"), LOOK_AT_THIS)
    };
    let state = if hidden {
        vec![Span::styled(SHOW_ALL, Style::new().fg(Color::DarkGray))]
    } else {
        Vec::new()
    };

    Fitted::new(
        vec![Span::styled(
            format!("{prefix}{said}"),
            Style::new().fg(colour),
        )],
        Vec::new(),
        state,
    )
}

/// One thing inside such a group.
fn item_line(prefix: &str, item: &Item) -> Fitted {
    match item {
        Item::Failed(failed) => sentence(prefix, phrase::failed_project(failed), LOOK_AT_THIS),
        Item::Conflict(conflict) => sentence(prefix, phrase::conflict(conflict), LOOK_AT_THIS),
        Item::Hidden(hidden) => Fitted::new(
            vec![Span::raw(format!(
                "{prefix}{} · {}",
                hidden.project, hidden.root
            ))],
            vec![Span::raw(hidden.title.clone())],
            Vec::new(),
        ),
        Item::Loose(pane) => Fitted::new(
            vec![Span::styled(
                format!("{prefix}{}", pane_marker(pane)),
                Style::new().fg(LIVE),
            )],
            vec![Span::raw(pane.cwd.clone())],
            Vec::new(),
        ),
    }
}

/// Draw the tail into the band `regions` reserved for it: a rule naming the
/// pane, and as much of what that pane last wrote as fits beneath it.
///
/// The newest lines are the ones kept. A pane's last line is what it is
/// doing now, and a tail that dropped it to keep older ones would be
/// answering yesterday's question.
pub fn draw_tail(frame: &mut Frame, area: Rect, tail: &Tail) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let row = |n: usize| Rect {
        y: area.y + n as u16,
        height: 1,
        ..area
    };
    let room = area.height as usize - 1;

    match tail {
        Tail::Pane { pane, lines } => {
            frame.render_widget(rule(Some(pane), area.width as usize), row(0));
            for (n, said) in lines
                .iter()
                .skip(lines.len().saturating_sub(room))
                .enumerate()
            {
                frame.render_widget(sentence(&indent(), said.clone(), Color::Reset), row(n + 1));
            }
        }
        Tail::Silent(why) => {
            frame.render_widget(rule(None, area.width as usize), row(0));
            if room > 0 {
                frame.render_widget(
                    sentence(&indent(), (*why).to_string(), Color::DarkGray),
                    row(1),
                );
            }
        }
    }
}

/// The rule between the forest and the tail, with the pane the tail is
/// showing named in the middle of it.
///
/// A tail with no pane draws the rule alone. The band is reserved either
/// way, and the rule is what says where the forest stopped.
fn rule(pane: Option<&str>, width: usize) -> Line<'static> {
    let drawn =
        |n: usize| Span::styled(RULE.to_string().repeat(n), Style::new().fg(Color::DarkGray));

    let named = match pane {
        Some(pane) => format!(" {pane} "),
        None => return Line::from(drawn(width)),
    };
    let taken = columns(&[Span::raw(named.clone())]);
    if taken >= width {
        return Line::from(drawn(width));
    }

    let left = (width - taken) / 2;
    Line::from(vec![
        drawn(left),
        Span::raw(named),
        drawn(width - taken - left),
    ])
}

fn indent() -> String {
    " ".repeat(GAP)
}

/// How far `^D` and `^U` move the selection: half the forest's own band,
/// rather than half a screen the key bar and the tail also sit in.
pub fn half_screen(forest: Rect) -> usize {
    (forest.height / 2) as usize
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
    background: Style,
}

impl Fitted {
    fn new(
        identity: Vec<Span<'static>>,
        title: Vec<Span<'static>>,
        state: Vec<Span<'static>>,
    ) -> Self {
        Self {
            identity,
            title,
            state,
            background: Style::new(),
        }
    }

    /// The row under the cursor, drawn so the eye finds it without reading it.
    #[must_use]
    pub fn selected(mut self) -> Self {
        self.background = Style::new().add_modifier(Modifier::REVERSED);
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

        Line::from(spans).style(self.background).render(area, buf);
    }
}

/// One tree's own line: where it is, what it is, and how much of it is done.
///
/// A tree whose tracker could not be read has no title and no counts. Its
/// line is the only place that failure is said, so it says why, and shows the
/// live panes recovered for it, where the counts would be. It is not given a
/// phrase in place of the title: the reason beside it is already the reason
/// the title is missing, and saying it twice would cost the columns the panes
/// need.
pub fn header(tree: &Tree, prefix: &str, panes: &[LoosePane], panes_complete: bool) -> Fitted {
    let identity = vec![Span::raw(format!(
        "{prefix}{} · {}",
        tree.project.clone(),
        tree.root.clone()
    ))];
    let state = match tree.tracker {
        TrackerState::Ok => summary(&tree.counts),
        TrackerState::Unreachable(failure) => unreadable(failure, panes, panes_complete),
    };

    Fitted::new(identity, vec![Span::raw(tree.title.clone())], state)
}

/// How much of a tree is done, who is on it, and how much of it wants looking
/// at. A count that is zero is left out rather than drawn as a zero: a row of
/// noughts reads as something to check.
fn summary(counts: &Counts) -> Vec<Span<'static>> {
    let mut said = vec![Span::raw(format!("{}/{}", counts.closed, counts.total))];
    if counts.live_agents > 0 {
        let agent = if counts.live_agents == 1 {
            "agent"
        } else {
            "agents"
        };
        said.push(Span::raw(" ".repeat(GAP)));
        said.push(Span::styled(
            format!("{} {agent}", counts.live_agents),
            Style::new().fg(LIVE),
        ));
    }
    if counts.anomalies > 0 {
        said.push(Span::raw(" ".repeat(GAP)));
        said.push(Span::styled(
            format!("{WARNING} {}", counts.anomalies),
            Style::new().fg(LOOK_AT_THIS),
        ));
    }
    said
}

/// Why a tree's tracker never answered, and whatever live panes could still
/// be found for it. Where those panes cannot be known to be all of them it
/// says so — a list that is quietly short is the one way this line can be
/// read wrongly, because it looks exactly like a complete one.
fn unreadable(failure: TrackerFailure, panes: &[LoosePane], complete: bool) -> Vec<Span<'static>> {
    let mut said = vec![format!("{WARNING} {}", phrase::tracker_failure(failure))];
    said.push(if panes.is_empty() {
        phrase::no_live_panes().to_string()
    } else {
        panes
            .iter()
            .map(pane_marker)
            .collect::<Vec<_>>()
            .join(" · ")
    });
    if !complete {
        said.push(phrase::panes_may_be_incomplete().to_string());
    }

    vec![Span::styled(
        said.join(" · "),
        Style::new().fg(LOOK_AT_THIS),
    )]
}

fn pane_marker(pane: &LoosePane) -> String {
    format!(
        "{AGENT} {} {}",
        pane.pane,
        phrase::pane_state(&pane.pane_status)
    )
}

/// One bead's line, under the box-drawing run its ancestors leave.
///
/// `id_width` is the widest abbreviated id in the tree, so a column of ids
/// lines up under one another and the titles start together.
pub fn bead_line(row: &Row, prefix: &str, id_width: usize) -> Fitted {
    let identity = vec![
        Span::raw(prefix.to_string()),
        Span::styled(row.glyph.to_string(), status_style(row.glyph)),
        Span::raw(format!(" {:id_width$}", row.id)),
    ];

    let mut title = vec![Span::raw(row.title.clone())];
    for badge in &row.badges {
        title.push(Span::raw(" ".repeat(GAP)));
        title.push(Span::raw(badge.clone()));
    }

    let mut state: Vec<Span<'static>> = Vec::new();
    let mut say = |text: &str, colour: Color| {
        if !state.is_empty() {
            state.push(Span::raw(" ".repeat(GAP)));
        }
        state.push(Span::styled(text.to_string(), Style::new().fg(colour)));
    };
    if let Some(agent) = &row.agent {
        say(agent, LIVE);
    }
    if let Some(anomalies) = &row.anomalies {
        say(anomalies, LOOK_AT_THIS);
    }
    for note in &row.notes {
        say(note, LOOK_AT_THIS);
    }

    Fitted::new(identity, title, state)
}

/// The colour a bead's status is drawn in.
///
/// Keyed by the glyph, because a `Row` carries the glyph and not the status it
/// came from, and resolved through `row::status_glyph` so the glyphs
/// themselves are written down in one place only. Colour is the second channel
/// and never the only one: the glyph already says the status, so a terminal
/// with no colour loses nothing.
fn status_style(glyph: char) -> Style {
    let colour = every_status()
        .into_iter()
        .find(|status| row::status_glyph(status) == glyph)
        .map(|status| status_colour(&status));

    colour.map_or_else(Style::new, |colour| Style::new().fg(colour))
}

fn status_colour(status: &Status) -> Color {
    match status {
        Status::InProgress => Color::Cyan,
        Status::Blocked => Color::Yellow,
        Status::Open => Color::Reset,
        Status::Deferred => Color::DarkGray,
        Status::Closed => Color::Green,
        Status::Other(_) => Color::Magenta,
    }
}

/// One of each status, which is what makes the glyph-to-colour lookup total.
fn every_status() -> [Status; 6] {
    [
        Status::InProgress,
        Status::Blocked,
        Status::Open,
        Status::Deferred,
        Status::Closed,
        Status::Other(String::new()),
    ]
}

/// The lines of its pane the tail shows where the screen can spare them. The
/// band it is given is one more than this: the rule that names the pane is
/// part of the tail and not part of the forest above it.
const TAIL_LINES: u16 = 6;

/// The three bands of the screen, top to bottom.
///
/// Named rather than returned from `draw` because the tail is drawn by
/// whoever holds one, and `draw` is handed a forest and no tail. Both sides
/// ask here instead of agreeing a number twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Regions {
    pub forest: Rect,
    pub tail: Rect,
    pub keys: Rect,
}

/// Divide the screen between the forest, the tail and the key bar.
///
/// The tail gives up its rows before the forest gives up any, and the forest
/// is never left with none: a `bdi` with no tree on screen is not showing the
/// thing it exists to show.
pub fn regions(area: Rect) -> Regions {
    let mut rows = area.height;
    let keys = if rows >= 2 { 1 } else { 0 };
    rows -= keys;
    let tail = (TAIL_LINES + 1).min(rows.saturating_sub(1) / 2);
    let forest = rows - tail;

    Regions {
        forest: Rect {
            height: forest,
            ..area
        },
        tail: Rect {
            y: area.y + forest,
            height: tail,
            ..area
        },
        keys: Rect {
            y: area.y + forest + tail,
            height: keys,
            ..area
        },
    }
}

/// The first visible line, so that the selection is on screen.
///
/// A pure function of the selection, which is what lets the renderer hold no
/// scroll state of its own: the selection moves, the window follows it, and
/// there is no third thing to keep in step with the other two.
pub fn scroll_offset(selected: usize, lines: usize, height: usize) -> usize {
    if lines <= height || height == 0 {
        return 0;
    }
    selected.saturating_sub(height / 2).min(lines - height)
}

/// The row at the foot of the screen: the keys, and anything true of the
/// whole session rather than of any row above.
///
/// A herdr that could not be reached belongs here because it changes what
/// every row above it means — none of them can show an agent — and this is
/// the one row a reader can neither fold nor scroll away from. It is drawn
/// first and yields last: keys can be rediscovered, and a herdr that is
/// silently absent reads as a fleet with nobody working in it.
pub fn status_bar(herdr: HerdrState) -> Fitted {
    match phrase::herdr_state(herdr) {
        None => Fitted::new(vec![Span::raw(KEYS)], Vec::new(), Vec::new()),
        Some(said) => Fitted::new(
            vec![Span::styled(
                format!("{WARNING} {said}"),
                Style::new().fg(LOOK_AT_THIS),
            )],
            Vec::new(),
            vec![Span::raw(KEYS)],
        ),
    }
}

/// What a run of spans takes up on screen, in columns rather than in bytes:
/// every glyph in this vocabulary is several bytes long, and a width counted
/// in bytes would put a cut inside one.
fn columns(spans: &[Span<'static>]) -> usize {
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

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use crate::collect::herdr::PaneStatus;
    use crate::model::anomaly::Anomaly;
    use crate::model::join::{AgentRef, Badged, JoinSource};
    use crate::model::snapshot::{Filter, Node, Snapshot, TrackerFailure};
    use crate::view::forest::flatten;
    use crate::view::{Action, Motion};
    use chrono::{TimeZone, Utc};

    const OPEN: &str = "▾ ";
    const SHUT: &str = "▸ ";
    const BRANCH: &str = "  ├── ";
    const LAST: &str = "  └── ";

    /// What a widget puts on screen, one string per row.
    fn drawn<W: Widget>(widget: W, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test backend");
        terminal
            .draw(|frame| frame.render_widget(widget, frame.area()))
            .expect("a draw into memory");
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
            .collect()
    }

    fn counts(closed: usize, total: usize, live_agents: usize, anomalies: usize) -> Counts {
        Counts {
            total,
            closed,
            live_agents,
            anomalies,
        }
    }

    fn tree(project: &str, root: &str, title: &str, counts: Counts) -> Tree {
        Tree {
            project: project.into(),
            root: root.into(),
            title: title.into(),
            counts,
            tracker: TrackerState::Ok,
            nodes: Vec::new(),
            dangling: Vec::new(),
            unreachable: Vec::new(),
        }
    }

    fn node(id: &str, title: &str, status: Status) -> Node {
        Node {
            id: id.into(),
            title: title.into(),
            status,
            issue_type: "task".into(),
            priority: 2,
            depth: 1,
            edge: None,
            ready: false,
            blocked_by: Vec::new(),
            started_at: None,
            closed_at: None,
            badges: Vec::new(),
            agent: None,
            anomalies: Vec::new(),
            truncated: false,
        }
    }

    fn pane(pane: &str, status: PaneStatus) -> LoosePane {
        LoosePane {
            pane: pane.into(),
            project: Some("summit-works".into()),
            cwd: "/tmp/bdi-ground/summit-works".into(),
            pane_status: status,
        }
    }

    fn row(node: &Node) -> Row {
        row::cells(node, "nix-9670s")
    }

    // ---- the header ------------------------------------------------------

    /// The design's own example, at the width it was written for.
    #[test]
    fn a_header_says_where_a_tree_is_what_it_is_and_how_much_of_it_is_done() {
        let tree = tree(
            "summit-works",
            "nix-9670s",
            "DMS → noctalia v5",
            counts(8, 21, 3, 3),
        );

        assert_eq!(
            drawn(header(&tree, OPEN, &[], true), 78, 1),
            vec!["▾ summit-works · nix-9670s  DMS → noctalia v5              8/21  3 agents  ⚠ 3"]
        );
    }

    /// A count of nothing is left out rather than drawn as a nought: a header
    /// reading `0 agents  ⚠ 0` sends a reader looking for rows that are not
    /// there.
    #[test]
    fn a_tree_with_no_live_agent_and_nothing_wrong_says_only_how_much_is_done() {
        let tree = tree(
            "homelab",
            "hl-sgqyv",
            "heartbeat cadence",
            counts(2, 7, 0, 0),
        );

        assert_eq!(
            drawn(header(&tree, SHUT, &[], true), 60, 1),
            vec!["▸ homelab · hl-sgqyv  heartbeat cadence                  2/7"]
        );
    }

    #[test]
    fn one_agent_is_not_described_in_the_plural() {
        let tree = tree(
            "homelab",
            "hl-sgqyv",
            "heartbeat cadence",
            counts(2, 7, 1, 0),
        );

        assert_eq!(
            drawn(header(&tree, SHUT, &[], true), 60, 1),
            vec!["▸ homelab · hl-sgqyv  heartbeat cadence         2/7  1 agent"]
        );
    }

    /// A tree row is one row. A title that will not fit is cut, and the cut is
    /// marked so it reads as cut rather than as a title that is simply short.
    #[test]
    fn a_title_too_long_for_the_width_is_cut_rather_than_wrapped() {
        let tree = tree(
            "summit-works",
            "nix-9670s",
            "Switch the thinkpad's session shell from DMS to noctalia v5",
            counts(8, 21, 3, 3),
        );

        assert_eq!(
            drawn(header(&tree, OPEN, &[], true), 60, 2),
            vec![
                "▾ summit-works · nix-9670s  Switch the…  8/21  3 agents  ⚠ 3",
                "                                                            ",
            ]
        );
    }

    /// Narrower than the identity itself there is nothing left to protect, and
    /// the line is cut like any other.
    #[test]
    fn a_width_too_narrow_for_anything_else_keeps_as_much_of_the_tree_as_it_can() {
        let tree = tree(
            "summit-works",
            "nix-9670s",
            "DMS → noctalia v5",
            counts(8, 21, 3, 3),
        );

        assert_eq!(
            drawn(header(&tree, OPEN, &[], true), 12, 1),
            vec!["▾ nixos-con…"]
        );
    }

    /// Width is columns on a screen, not bytes in a string. Every glyph in
    /// this vocabulary is several bytes long, and a cut counted in bytes would
    /// land inside one and put a broken character on the terminal.
    #[test]
    fn a_cut_is_counted_in_columns_and_never_lands_inside_a_glyph() {
        let tree = tree(
            "summit-works",
            "nix-9670s",
            "→→→→→→→→→→→→→→→→→→→→→→→→→→→→→→",
            counts(0, 1, 0, 0),
        );
        let drawn = drawn(header(&tree, OPEN, &[], true), 40, 1);

        assert_eq!(drawn[0].chars().count(), 40);
        assert!(!drawn[0].contains('\u{fffd}'), "{drawn:?}");
        assert!(drawn[0].ends_with("0/1"), "{drawn:?}");
    }

    // ---- a tree whose tracker never answered -----------------------------

    /// The design has such a tree render as a header, a marker and its live
    /// panes. All three are here, and the panes are named the way a bead's
    /// agent is named so one reads as the other.
    #[test]
    fn an_unreachable_tree_renders_its_header_its_marker_and_its_panes() {
        let tree =
            Tree::tracker_unreachable("summit-works", "nix-9670s", TrackerFailure::Unavailable);
        let panes = [
            pane("wCM:p9", PaneStatus::Working),
            pane("wCM:p6", PaneStatus::Idle),
        ];

        assert_eq!(
            drawn(header(&tree, OPEN, &panes, true), 100, 1),
            vec!["▾ summit-works · nix-9670s           ⚠ the tracker did not answer · ◍ wCM:p9 working · ◍ wCM:p6 idle"
                .to_string()]
        );
    }

    /// A tracker that could not be read has no counts, and `0/0` would say the
    /// opposite of what is true — that it was read and holds nothing.
    #[test]
    fn an_unreachable_tree_never_shows_a_count_it_could_not_read() {
        let tree = Tree::tracker_unreachable("summit-works", "nix-9670s", TrackerFailure::Auth);
        let drawn = drawn(header(&tree, OPEN, &[], true), 120, 1);

        assert!(!drawn[0].contains("0/0"), "{drawn:?}");
    }

    #[test]
    fn an_unreachable_tree_with_no_pane_to_show_says_that_rather_than_nothing() {
        let tree = Tree::tracker_unreachable("summit-works", "nix-9670s", TrackerFailure::Auth);
        let drawn = drawn(header(&tree, OPEN, &[], true), 120, 1);

        assert!(drawn[0].contains(phrase::no_live_panes()), "{drawn:?}");
    }

    /// A pane list that cannot be known to be whole says so. A silently short
    /// list is the one way this line can be read wrongly, because it looks
    /// exactly like a complete one.
    #[test]
    fn a_pane_list_that_may_be_short_says_so_rather_than_reading_as_complete() {
        let tree =
            Tree::tracker_unreachable("summit-works", "nix-9670s", TrackerFailure::Unavailable);
        let panes = [pane("wCM:p9", PaneStatus::Working)];

        let whole = drawn(header(&tree, OPEN, &panes, true), 200, 1);
        let partial = drawn(header(&tree, OPEN, &panes, false), 200, 1);

        assert!(
            !whole[0].contains(phrase::panes_may_be_incomplete()),
            "{whole:?}"
        );
        assert!(
            partial[0].contains(phrase::panes_may_be_incomplete()),
            "{partial:?}"
        );
    }

    /// The identity of a tree outlasts everything else on its line: a reader
    /// who cannot tell which tree failed learns nothing from knowing that one
    /// did.
    #[test]
    fn a_narrow_unreachable_header_keeps_the_tree_and_the_reason_over_the_panes() {
        let tree = Tree::tracker_unreachable("summit-works", "nix-9670s", TrackerFailure::Auth);
        let panes = [pane("wCM:p9", PaneStatus::Working)];
        let drawn = drawn(header(&tree, OPEN, &panes, false), 80, 1);

        assert!(
            drawn[0].starts_with("▾ summit-works · nix-9670s"),
            "{drawn:?}"
        );
        assert!(
            drawn[0].contains(phrase::tracker_failure(TrackerFailure::Auth)),
            "{drawn:?}"
        );
        assert_eq!(drawn[0].chars().count(), 80);
    }

    // ---- a bead's line ---------------------------------------------------

    #[test]
    fn a_bead_line_says_its_glyph_its_id_and_its_title_in_that_order() {
        let node = node("nix-9670s.20", "wallpaper timer calls dms", Status::Blocked);

        assert_eq!(
            drawn(bead_line(&row(&node), BRANCH, 4), 46, 1),
            vec!["  ├── ◐ .20   wallpaper timer calls dms       "]
        );
    }

    /// Ids are padded to the widest in the tree so the titles start together;
    /// a column that did not line up would be read as a tree shape it is not.
    #[test]
    fn ids_are_padded_so_the_titles_below_one_another_start_together() {
        let short = node("nix-9670s.1", "wire the niri theme include", Status::Open);
        let long = node("nix-9670s.20", "wallpaper timer calls dms", Status::Open);

        let short = drawn(bead_line(&row(&short), BRANCH, 4), 60, 1);
        let long = drawn(bead_line(&row(&long), BRANCH, 4), 60, 1);

        assert_eq!(
            short[0].find("wire the"),
            long[0].find("wallpaper timer"),
            "{short:?} {long:?}"
        );
    }

    #[test]
    fn a_bead_line_carries_its_agent_and_its_anomalies() {
        let mut staffed = node(
            "nix-9670s.20",
            "wallpaper timer calls dms",
            Status::InProgress,
        );
        staffed.agent = Some(AgentRef {
            pane: "wCM:p9".into(),
            pane_status: PaneStatus::Working,
            title: None,
            source: JoinSource::AgentPane,
        });
        staffed.anomalies = vec![Anomaly::StaleClaim { days: 58 }];
        let drawn = drawn(bead_line(&row(&staffed), LAST, 4), 100, 1);

        assert!(drawn[0].contains("◍ wCM:p9 working"), "{drawn:?}");
        assert!(drawn[0].contains("58"), "{drawn:?}");
    }

    #[test]
    fn a_bead_lines_badges_are_drawn_in_the_order_they_were_configured() {
        let mut badged = node("nix-9670s.20", "a bead", Status::Blocked);
        badged.badges = vec![
            Badged {
                key: "delivery_pr".into(),
                text: "⇢ #12".into(),
            },
            Badged {
                key: "blocked_on".into(),
                text: "⏸ waiting".into(),
            },
        ];
        let drawn = drawn(bead_line(&row(&badged), BRANCH, 4), 100, 1);
        let first = drawn[0].find("⇢ #12").expect("the first badge");
        let second = drawn[0].find("⏸ waiting").expect("the second badge");

        assert!(first < second, "{drawn:?}");
    }

    /// A row bd stopped at means what hangs beneath it is not in the tree at
    /// all, which is the silent partial answer this tool exists to avoid — so
    /// it survives all the way to the screen.
    #[test]
    fn a_bead_the_tracker_stopped_at_says_so_on_screen() {
        let mut stopped = node("nix-9670s.20", "a bead", Status::Open);
        stopped.truncated = true;
        let drawn = drawn(bead_line(&row(&stopped), BRANCH, 4), 120, 1);

        assert!(drawn[0].contains(phrase::truncated()), "{drawn:?}");
    }

    #[test]
    fn a_bead_line_too_long_for_the_width_is_cut_rather_than_wrapped() {
        let long = node("nix-9670s.20", &"wallpaper ".repeat(20), Status::Open);
        let drawn = drawn(bead_line(&row(&long), BRANCH, 4), 40, 3);

        assert_eq!(drawn[0], "  ├── ○ .20   wallpaper wallpaper wallp…");
        assert_eq!(drawn[1].trim(), "");
        assert_eq!(drawn[2].trim(), "");
    }

    // ---- styling ---------------------------------------------------------

    /// Colour is the second channel and never the only one: the glyph already
    /// says the status, so a terminal that drops colour must lose nothing.
    #[test]
    fn no_status_is_told_apart_by_colour_alone() {
        for status in every_status() {
            let node = node("nix-9670s.1", "a bead", status.clone());
            let drawn = drawn(bead_line(&row(&node), BRANCH, 3), 40, 1);

            assert!(
                drawn[0].contains(row::status_glyph(&status)),
                "{status:?} lost its glyph: {drawn:?}"
            );
        }
    }

    /// A status that arrived without a colour would be drawn in whatever the
    /// terminal defaults to, which is the same as `open`'s — two statuses
    /// telling the same story is exactly what the glyph test above forbids.
    #[test]
    fn every_status_has_a_colour_reachable_from_the_glyph_it_is_drawn_with() {
        for status in every_status() {
            assert_eq!(
                status_style(row::status_glyph(&status)),
                Style::new().fg(status_colour(&status)),
                "{status:?}"
            );
        }
    }

    // ---- the bands of the screen -----------------------------------------

    #[test]
    fn a_full_screen_gives_the_forest_most_of_it_the_tail_a_look_and_the_keys_a_row() {
        let bands = regions(Rect::new(0, 0, 80, 24));

        assert_eq!(bands.forest, Rect::new(0, 0, 80, 16));
        assert_eq!(
            bands.tail,
            Rect::new(0, 16, 80, 7),
            "six lines of pane, under the rule that names it"
        );
        assert_eq!(bands.keys, Rect::new(0, 23, 80, 1));
    }

    /// The tail yields first, because the forest is the thing this tool is
    /// for and a screen showing no tree is showing nothing.
    #[test]
    fn a_short_screen_takes_the_rows_from_the_tail_and_not_from_the_forest() {
        let bands = regions(Rect::new(0, 0, 80, 10));

        assert_eq!(bands.forest.height, 5);
        assert_eq!(bands.tail.height, 4);
        assert_eq!(bands.keys.height, 1);
    }

    #[test]
    fn the_forest_keeps_a_row_however_little_room_there_is() {
        for height in 1..=8 {
            let bands = regions(Rect::new(0, 0, 80, height));
            assert!(bands.forest.height >= 1, "{height} rows: {bands:?}");
        }
    }

    /// The two smallest screens that still show something, pinned so the rule
    /// that produces them cannot be simplified into one that does not.
    #[test]
    fn the_smallest_screens_spend_their_rows_on_the_forest_first() {
        assert_eq!(
            regions(Rect::new(0, 0, 80, 2)),
            Regions {
                forest: Rect::new(0, 0, 80, 1),
                tail: Rect::new(0, 1, 80, 0),
                keys: Rect::new(0, 1, 80, 1),
            }
        );
        assert_eq!(
            regions(Rect::new(0, 0, 80, 1)),
            Regions {
                forest: Rect::new(0, 0, 80, 1),
                tail: Rect::new(0, 1, 80, 0),
                keys: Rect::new(0, 1, 80, 0),
            }
        );
    }

    /// The three bands are the screen: a gap between them would draw whatever
    /// the last frame left there, and an overlap would draw two things at once.
    #[test]
    fn the_three_bands_tile_the_screen_exactly() {
        for height in 0..40 {
            let area = Rect::new(3, 7, 80, height);
            let bands = regions(area);

            assert_eq!(bands.forest.y, area.y, "{height}");
            assert_eq!(
                bands.tail.y,
                bands.forest.y + bands.forest.height,
                "{height}"
            );
            assert_eq!(bands.keys.y, bands.tail.y + bands.tail.height, "{height}");
            assert_eq!(
                bands.forest.height + bands.tail.height + bands.keys.height,
                area.height,
                "{height}"
            );
        }
    }

    // ---- the scroll offset -----------------------------------------------

    #[test]
    fn a_forest_that_fits_the_viewport_never_scrolls() {
        for selected in 0..5 {
            assert_eq!(scroll_offset(selected, 5, 10), 0, "{selected}");
        }
    }

    #[test]
    fn the_selection_is_always_inside_the_viewport() {
        let (lines, height) = (100, 10);
        for selected in 0..lines {
            let offset = scroll_offset(selected, lines, height);
            assert!(
                (offset..offset + height).contains(&selected),
                "row {selected} fell outside {offset}..{}",
                offset + height
            );
        }
    }

    /// Scrolling past the end would draw blank rows under the last one, which
    /// reads as a forest that has run out rather than one that has ended.
    #[test]
    fn the_last_row_is_reachable_without_scrolling_past_the_end() {
        assert_eq!(scroll_offset(99, 100, 10), 90);
        assert_eq!(scroll_offset(0, 100, 10), 0);
    }

    /// A viewport with no rows in it has nowhere to scroll to, and the
    /// arithmetic that finds the offset would run off the bottom of `usize`.
    #[test]
    fn a_viewport_with_no_room_asks_for_no_offset() {
        assert_eq!(scroll_offset(40, 100, 0), 0);
    }

    // ---- the key bar -----------------------------------------------------

    #[test]
    fn the_foot_of_the_screen_names_every_key_the_view_answers_to() {
        let drawn = drawn(status_bar(HerdrState::Ok), 60, 1);

        for key in ["⏎", "a", "^R", "q"] {
            assert!(drawn[0].contains(key), "{key} missing from {drawn:?}");
        }
    }

    /// With no herdr there is no agent on any row, and a screen that only
    /// stopped showing them would read as a fleet with nobody working in it.
    /// It goes at the foot because that is the one row that cannot be folded
    /// or scrolled away.
    #[test]
    fn a_herdr_that_could_not_be_reached_is_said_where_nothing_can_hide_it() {
        let drawn = drawn(status_bar(HerdrState::Unavailable), 90, 1);

        assert!(
            drawn[0].contains(phrase::herdr_state(HerdrState::Unavailable).expect("a notice")),
            "{drawn:?}"
        );
    }

    /// Keys can be rediscovered; a herdr that is silently absent cannot. So on
    /// a screen too narrow for both, the keys are what gives way.
    #[test]
    fn a_narrow_foot_gives_up_the_keys_before_the_missing_herdr() {
        let drawn = drawn(status_bar(HerdrState::Unavailable), 60, 1);

        assert!(drawn[0].contains("no herdr session"), "{drawn:?}");
        assert_eq!(drawn[0].chars().count(), 60);
    }

    // ---- the groups below the trees --------------------------------------

    /// The filter hides trees, and it takes their findings with them. Saying
    /// only how many trees are hidden would read as "nothing to see here"
    /// while some of them are broken.
    #[test]
    fn the_hidden_trees_group_admits_that_what_it_hides_is_not_empty() {
        let quiet = Group {
            kind: GroupKind::HiddenTrees,
            count: 4,
            with_findings: 0,
        };
        let broken = Group {
            with_findings: 2,
            ..quiet
        };

        assert_eq!(
            drawn(group_line(SHUT, quiet), 64, 1),
            vec!["▸ 4 trees with no live agent                       a to show all"]
        );
        assert_eq!(
            drawn(group_line(SHUT, broken), 64, 1),
            vec!["▸ 4 trees with no live agent · 2 with findings     a to show all"]
        );
    }

    /// Every other group is something that went wrong, and is marked as such.
    /// The hidden trees are not: the user asked for them to be hidden.
    #[test]
    fn only_the_group_nothing_went_wrong_in_is_drawn_without_a_warning() {
        for kind in GroupKind::ALL {
            let group = Group {
                kind,
                count: 2,
                with_findings: 0,
            };
            let drawn = drawn(group_line(SHUT, group), 80, 1);
            let marked = drawn[0].contains(WARNING);

            assert_eq!(
                marked,
                kind != GroupKind::HiddenTrees,
                "{kind:?}: {drawn:?}"
            );
        }
    }

    // ---- the whole frame -------------------------------------------------

    fn snapshot(trees: Vec<Tree>, unattributed: Vec<LoosePane>, herdr: HerdrState) -> Snapshot {
        Snapshot {
            generated_at: Utc.with_ymd_and_hms(2026, 8, 30, 10, 22, 14).unwrap(),
            herdr,
            filter: Filter::All,
            collected: trees.clone(),
            trees,
            hidden_trees: Vec::new(),
            failed_projects: Vec::new(),
            unattributed,
            conflicts: Vec::new(),
        }
    }

    /// One tree of `children` open beads under an in-flight root.
    fn grove(children: usize) -> Tree {
        let mut nodes = vec![Node {
            depth: 0,
            ..node("nix-9670s", "lift the ground station", Status::InProgress)
        }];
        for child in 1..=children {
            nodes.push(node(
                &format!("nix-9670s.{child}"),
                &format!("bead number {child}"),
                Status::Open,
            ));
        }

        Tree {
            counts: counts(0, nodes.len(), 0, 0),
            nodes,
            ..tree(
                "summit-works",
                "nix-9670s",
                "lift the ground station",
                counts(0, 0, 0, 0),
            )
        }
    }

    fn frame_of(forest: &Forest, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test backend");
        terminal
            .draw(|frame| draw(frame, frame.area(), forest))
            .expect("a draw into memory");
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
            .collect()
    }

    /// The whole screen, character for character: five rows of forest, four of
    /// reserved tail, and the foot.
    #[test]
    fn a_frame_is_the_forest_the_tails_reserved_band_and_the_foot() {
        let forest = flatten(&snapshot(
            vec![grove(2)],
            Vec::new(),
            HerdrState::Unavailable,
        ));

        assert_eq!(
            frame_of(&forest, 60, 10),
            vec![
                "▾ summit-works · nix-9670s  lift the ground station      0/3",
                "  ├── ○ .1  bead number 1                                   ",
                "  └── ○ .2  bead number 2                                   ",
                "                                                            ",
                "                                                            ",
                "                                                            ",
                "                                                            ",
                "                                                            ",
                "                                                            ",
                "⚠ no herdr session · which agents are alive is unknown  ⏎ f…",
            ]
        );
    }

    /// The definition of done's first case, at the whole-frame level: a narrow
    /// screen cuts every row and wraps none, so the row count on screen still
    /// matches the line count in the forest.
    #[test]
    fn a_narrow_frame_cuts_every_row_and_wraps_none() {
        let forest = flatten(&snapshot(vec![grove(2)], Vec::new(), HerdrState::Ok));
        let frame = frame_of(&forest, 24, 10);

        assert_eq!(
            frame[..3].to_vec(),
            vec![
                "▾ summit-works · nix-96…",
                "  ├── ○ .1  bead number…",
                "  └── ○ .2  bead number…",
            ]
        );
        assert!(
            frame[3..9].iter().all(|row| row.trim().is_empty()),
            "{frame:?}"
        );
    }

    /// The definition of done's second case: however far the selection moves,
    /// the row it is on is drawn.
    #[test]
    fn the_selected_row_is_drawn_wherever_the_selection_has_moved_to() {
        let mut forest = flatten(&snapshot(vec![grove(40)], Vec::new(), HerdrState::Ok));

        for motion in [Motion::LastRow, Motion::FirstRow, Motion::HalfScreenDown] {
            forest.apply(Action::Move(motion));
            let at = forest.selected_line();
            let said = match &forest.lines()[at].content {
                Content::Bead(row) => row.title.clone(),
                Content::Tree(header) => header.tree.title.clone(),
                other => panic!("unexpected line under the selection: {other:?}"),
            };
            let frame = frame_of(&forest, 60, 10);

            assert!(
                frame.iter().any(|row| row.contains(&said)),
                "{motion:?} put line {at} ({said}) off screen: {frame:?}"
            );
        }
    }

    /// The definition of done's third case: a tree nobody could read renders
    /// as its header, the reason it failed, and the panes still working in it.
    #[test]
    fn an_unreachable_tree_draws_its_header_its_reason_and_its_panes() {
        let failed =
            Tree::tracker_unreachable("summit-works", "nix-9670s", TrackerFailure::Unavailable);
        let forest = flatten(&snapshot(
            vec![failed],
            vec![pane("wCM:p9", PaneStatus::Working)],
            HerdrState::Ok,
        ));

        assert_eq!(
            frame_of(&forest, 75, 4)[0],
            "▾ summit-works · nix-9670s  ⚠ the tracker did not answer · ◍ wCM:p9 working"
        );
    }

    // ---- the tail ---------------------------------------------------------

    /// The tail drawn into a band that starts partway down the screen, which
    /// is the only place it ever is.
    fn tail_frame(tail: &Tail, width: u16, height: u16, at: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test backend");
        terminal
            .draw(|frame| draw_tail(frame, Rect::new(0, at, width, height - at), tail))
            .expect("a draw into memory");
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
            .collect()
    }

    fn tailing(pane: &str, lines: &[&str]) -> Tail {
        Tail::Pane {
            pane: pane.into(),
            lines: lines.iter().map(|line| (*line).to_string()).collect(),
        }
    }

    #[test]
    fn the_tail_fills_the_band_it_is_given_and_no_row_above_it() {
        let tail = tailing("wCM:p9", &["rebuilt .#thinkpad, generation 541"]);

        assert_eq!(
            tail_frame(&tail, 44, 5, 2),
            vec![
                "                                            ",
                "                                            ",
                "────────────────── wCM:p9 ──────────────────",
                "  rebuilt .#thinkpad, generation 541        ",
                "                                            ",
            ]
        );
    }

    /// A pane writes more than the band holds. What it wrote last is what it
    /// is doing now.
    #[test]
    fn a_pane_with_more_to_say_than_there_is_room_shows_the_newest_of_it() {
        let tail = tailing("w:p1", &["one", "two", "three", "four"]);

        assert_eq!(
            tail_frame(&tail, 20, 3, 0),
            vec![
                "─────── w:p1 ───────",
                "  three             ",
                "  four              "
            ]
        );
    }

    #[test]
    fn a_tail_with_no_pane_says_why_rather_than_leaving_the_band_blank() {
        assert_eq!(
            tail_frame(&Tail::Silent(phrase::no_agent_to_tail()), 40, 3, 0),
            vec![
                "────────────────────────────────────────",
                "  no pane · nobody is working this bead ",
                "                                        ",
            ]
        );
    }

    #[test]
    fn a_pane_line_too_wide_for_the_screen_is_cut_rather_than_wrapped() {
        let tail = tailing("w:p1", &["a line with a great deal more to say than this"]);

        assert_eq!(
            tail_frame(&tail, 20, 2, 0),
            vec!["─────── w:p1 ───────", "  a line with a gre…"]
        );
    }

    /// A pane named so widely there is no rule left to draw around it, and a
    /// band with no room under its rule. Neither draws outside itself.
    #[test]
    fn a_tail_with_barely_any_room_draws_what_it_can() {
        let tail = tailing("a-very-long-pane-identifier", &["never seen"]);

        assert_eq!(tail_frame(&tail, 10, 1, 0), vec!["──────────"]);
        assert_eq!(
            tail_frame(&Tail::Silent(phrase::no_bead_to_tail()), 10, 1, 0),
            vec!["──────────"]
        );
    }

    #[test]
    fn a_band_with_no_rows_draws_nothing() {
        let mut terminal = Terminal::new(TestBackend::new(20, 1)).expect("a test backend");
        terminal
            .draw(|frame| draw_tail(frame, Rect::new(0, 0, 20, 0), &tailing("w:p1", &["x"])))
            .expect("a draw into memory");

        let buffer = terminal.backend().buffer();
        assert_eq!(
            (0..20).map(|x| buffer[(x, 0)].symbol()).collect::<String>(),
            " ".repeat(20)
        );
    }

    /// `^D` and `^U` move by half the band the trees are in, not half a
    /// screen the keys and the tail also sit in.
    #[test]
    fn a_half_screen_is_half_the_forest_and_not_half_the_frame() {
        assert_eq!(half_screen(regions(Rect::new(0, 0, 80, 24)).forest), 8);
        assert_eq!(half_screen(Rect::new(0, 0, 80, 1)), 0);
    }
}
