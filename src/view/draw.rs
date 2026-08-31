//! The forest and the tail, drawn into a ratatui frame.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::Frame;

use crate::model::snapshot::{Counts, HerdrState, TrackerState};
use crate::model::types::{PaneStatus, Status};
use crate::view::fitted::{columns, indent, Fitted, GAP};
use crate::view::forest::Forest;
use crate::view::lines::{
    self, Content, Group, GroupKind, Item, Note, ProjectLine, Recovery, Unread,
};
use crate::view::phrase;
use crate::view::row::{self, Row, AGENT, WARNING};
use crate::view::tail::{self, Tail};
use crate::view::Notice;

/// What the rule above the tail is drawn from.
const RULE: char = '─';

/// What lifts the live-agent filter, said beside the trees it is holding back.
const SHOW_ALL: &str = "a to show all";

const LIVE: Color = Color::Green;
const LOOK_AT_THIS: Color = Color::Yellow;

/// `bd list`'s own colours for a status, read off `bd` 1.2.2's output. They
/// are literal rather than named because `bd`'s are: it sends 24-bit values
/// that do not move with the terminal's theme, so a named colour here would
/// track the theme away from the tool this is matching.
///
/// `open` is absent on purpose. `bd` sends no escape at all for it, and a
/// glyph that inherits is what lets a row's own brightness reach it.
const IN_PROGRESS: Color = Color::Rgb(255, 180, 84);
const BLOCKED: Color = Color::Rgb(242, 109, 120);
const CLOSED: Color = Color::Rgb(128, 144, 160);

/// `bd` draws a deferred bead's glyph and every cell of a finished row in
/// this one grey, so one name serves both.
const DIM: Color = Color::Rgb(108, 118, 128);

/// The top of the brightness scale, and the one tier `bd list` could not
/// draw: a row a live agent is on. Named rather than literal because it is
/// `bdi`'s own and should follow the reader's terminal, not `bd`'s palette.
const STAFFED: Color = Color::White;

/// Draw the forest and the key bar, leaving the tail's band to whoever holds
/// a tail.
///
/// `keys` arrives already named. What a key is called belongs with the
/// mapping that answers it, and this file has never known one.
pub fn draw(frame: &mut Frame, area: Rect, forest: &Forest, at_startup: &[Notice], keys: &str) {
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

    frame.render_widget(
        status_bar(&notices(forest.snapshot().herdr, at_startup), keys),
        bands.keys,
    );
}

/// Everything the status bar has to say, in the order it should give it up.
///
/// The herdr one is read off the snapshot behind this frame and can change
/// under the reader; the rest were settled before the first collection and
/// hold for the session. Consequence decides the order, not provenance: a
/// herdr nobody can reach empties the agent column, which is what the reader
/// came for, so it is the last thing a narrow screen takes away.
fn notices(herdr: HerdrState, at_startup: &[Notice]) -> Vec<Notice> {
    let collected = match herdr {
        HerdrState::Ok => None,
        HerdrState::Unavailable => Some(Notice::NoHerdr),
    };

    collected
        .into_iter()
        .chain(at_startup.iter().copied())
        .collect()
}

/// The widest abbreviated id on screen, so every title starts in the same
/// column and a reader's eye runs down one edge rather than a ragged one.
fn id_width(lines: &[lines::Line]) -> usize {
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
fn fitted(line: &lines::Line, id_width: usize) -> Fitted {
    match &line.content {
        Content::Project(project) => project_line(project, &line.prefix),
        Content::Unread(unread) => unread_line(unread, &line.prefix, id_width),
        Content::Bead(row) => bead_line(row, &line.prefix, id_width),
        Content::Elided { count, .. } => elided_run(&line.prefix, *count),
        Content::Note(note) => {
            let (said, colour) = finding(*note);
            sentence(&line.prefix, said, colour)
        }
        Content::Group(group) => group_line(&line.prefix, *group),
        Content::Item(item) => item_line(&line.prefix, item),
    }
}

/// A run of closed siblings said as a count, carrying the glyph each of them
/// would carry on a line of its own.
///
/// `lines::split` builds a run out of closed beads and nothing else, so this
/// is not a summary over mixed states — it is the one state every member
/// holds. It goes through `status_glyph` and `status_style` exactly as a
/// bead's does, so a run cannot drift away from the beads it stands for.
fn elided_run(prefix: &str, count: usize) -> Fitted {
    let status = Status::Closed;
    let glyph = row::status_glyph(&status);
    Fitted::new(
        vec![
            structure(prefix),
            Span::styled(glyph.to_string(), status_style(&status)),
            Span::raw(format!(" {}", phrase::elided(count))),
        ],
        Vec::new(),
        Vec::new(),
    )
    .toned(Style::new().fg(DIM))
}

/// A line that is one sentence and nothing else.
fn sentence(prefix: &str, said: String, colour: Color) -> Fitted {
    Fitted::new(
        vec![
            Span::raw(prefix.to_string()),
            Span::styled(said, Style::new().fg(colour)),
        ],
        Vec::new(),
        Vec::new(),
    )
}

/// A finding about the tree above, in `bdi`'s words for it, and the colour
/// it is said in.
///
/// An empty forest is the one note nothing went wrong in — the trackers
/// answered and there was no work — so it alone is drawn plain, the way
/// `group_line` draws the hidden trees.
fn finding(note: Note) -> (String, Color) {
    let said = match note {
        Note::Dangling(count) => phrase::dangling(count),
        Note::Cycle(count) => phrase::cycle(count),
        Note::Truncated(count) => phrase::truncated_nodes(count),
        Note::NoRoots => return (phrase::no_roots().to_string(), Color::Reset),
    };
    (format!("{WARNING} {said}"), LOOK_AT_THIS)
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
        GroupKind::Unconfigured => (phrase::unconfigured(group.count), false),
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
        vec![
            Span::raw(prefix.to_string()),
            Span::styled(said, Style::new().fg(colour)),
        ],
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
        Item::Loose(pane) => loose_line(prefix, &pane.pane, &pane.pane_status, &pane.cwd),
        Item::Unconfigured(pane) => loose_line(prefix, &pane.pane, &pane.pane_status, &pane.cwd),
    }
}

/// A live pane in one of the groups: which pane it is, and the directory it is
/// working in. The directory is what both groups are asking the reader to
/// look at — one to place the agent, the other to configure the project.
fn loose_line(prefix: &str, pane: &str, status: &PaneStatus, cwd: &str) -> Fitted {
    Fitted::new(
        vec![Span::styled(
            format!("{prefix}{}", pane_marker(pane, status)),
            Style::new().fg(LIVE),
        )],
        vec![Span::raw(cwd.to_string())],
        Vec::new(),
    )
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

/// How far `^D` and `^U` move the selection: half the forest's own band,
/// rather than half a screen the key bar and the tail also sit in.
pub fn half_screen(forest: Rect) -> usize {
    (forest.height / 2) as usize
}

/// A project's own line: what it is, how much work it holds, and the live
/// panes recovered for it where a root would not read.
///
/// It says nothing about any one root, because every root below it says that
/// for itself. What is left is what only a project can answer: which project,
/// how much of it there is, and — where a root refused — which panes `bdi`
/// found working here that no bead could be attributed to.
pub fn project_line(project: &ProjectLine, prefix: &str) -> Fitted {
    let identity = vec![
        Span::raw(prefix.to_string()),
        Span::raw(project.project.clone()),
    ];

    let mut state = summary(&project.counts);
    if let Some(found) = &project.recovery {
        if !state.is_empty() {
            state.push(Span::raw(" ".repeat(GAP)));
        }
        state.push(recovered(found));
    }

    Fitted::new(identity, Vec::new(), state)
}

/// A root that drew no row, said where its row would have been.
///
/// It holds the same columns a bead row does — the mark, then the id — so a
/// reader scanning a project's roots meets it in the column the others are in
/// rather than having to find it.
fn unread_line(unread: &Unread, prefix: &str, id_width: usize) -> Fitted {
    let identity = vec![
        structure(prefix),
        Span::styled(WARNING.to_string(), Style::new().fg(LOOK_AT_THIS)),
        Span::raw(format!(" {:id_width$}", unread.root)),
    ];
    let why = match unread.tracker {
        TrackerState::Unreachable(failure) => phrase::tracker_failure(failure),
        TrackerState::Ok => phrase::root_unread(),
    };

    Fitted::new(
        identity,
        vec![Span::styled(why.to_string(), Style::new().fg(LOOK_AT_THIS))],
        Vec::new(),
    )
}

/// How far along something is. A tree and one epic inside it ask the same
/// question of different scopes, so they answer it in the same words.
fn done(closed: usize, total: usize) -> String {
    format!("{closed}/{total}")
}

/// How much of a tree is done, who is on it, and how much of it wants looking
/// at. A count that is zero is left out rather than drawn as a zero: a row of
/// noughts reads as something to check.
fn summary(counts: &Counts) -> Vec<Span<'static>> {
    // Nothing counted means no root here read at all, and `0/0` would say the
    // opposite of what is true — that they were read and hold nothing.
    let mut said = match counts.total {
        0 => Vec::new(),
        total => vec![Span::raw(done(counts.closed, total))],
    };
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

/// The live panes found working in a project no bead could be read to
/// attribute them to. Where they cannot be known to be all of them it says so
/// — a list that is quietly short is the one way this can be read wrongly,
/// because it looks exactly like a complete one.
fn recovered(found: &Recovery) -> Span<'static> {
    let panes = &found.panes;
    let mut said = Vec::new();
    said.push(if panes.is_empty() {
        phrase::no_live_panes().to_string()
    } else {
        panes
            .iter()
            .map(|pane| pane_marker(&pane.pane, &pane.pane_status))
            .collect::<Vec<_>>()
            .join(" · ")
    });
    if !found.complete {
        said.push(phrase::panes_may_be_incomplete().to_string());
    }

    Span::styled(said.join(" · "), Style::new().fg(LOOK_AT_THIS))
}

fn pane_marker(pane: &str, status: &PaneStatus) -> String {
    format!("{AGENT} {pane} {}", phrase::pane_state(status))
}

/// One bead's line, under the box-drawing run its ancestors leave.
///
/// `id_width` is the widest abbreviated id in the tree, so a column of ids
/// lines up under one another and the titles start together.
pub fn bead_line(row: &Row, prefix: &str, id_width: usize) -> Fitted {
    let identity = vec![
        structure(prefix),
        Span::styled(row.glyph.to_string(), status_style(&row.status)),
        Span::raw(format!(" {:id_width$}", row.id)),
    ];

    let mut title = vec![Span::raw(row.title.clone())];
    for badge in &row.badges {
        title.push(Span::raw(" ".repeat(GAP)));
        title.push(Span::raw(badge.clone()));
    }

    let mut state: Vec<Span<'static>> = Vec::new();
    let mut say = |text: &str, colour: Option<Color>| {
        if !state.is_empty() {
            state.push(Span::raw(" ".repeat(GAP)));
        }
        state.push(Span::styled(text.to_string(), fg(colour)));
    };
    if let Some(progress) = row.progress {
        say(&done(progress.closed, progress.total), None);
    }
    if let Some(agent) = &row.agent {
        say(agent, Some(LIVE));
    }
    if let Some(anomalies) = &row.anomalies {
        say(anomalies, Some(LOOK_AT_THIS));
    }
    for note in &row.notes {
        say(note, Some(LOOK_AT_THIS));
    }

    Fitted::new(identity, title, state).toned(tone(row))
}

/// The box-drawing a line hangs under. It says how the tree is shaped rather
/// than how a bead is going, so it is held at the terminal's default while the
/// row around it dims or brightens. `bd list` leaves its own tree prefix
/// undimmed on a closed row too.
fn structure(prefix: &str) -> Span<'static> {
    Span::styled(prefix.to_string(), Style::new().fg(Color::Reset))
}

/// How live a row is, which is the one thing about a bead `bd list` has no
/// way to know — and so the one this scale is spent on.
///
/// | row | drawn |
/// |---|---|
/// | an agent is on it | brighter than the page |
/// | nobody on it, still going | the terminal's default |
/// | finished, nobody on it | the grey `bd` dims a closed row to |
///
/// Finished means what it means to `lines::split`: closed, no agent, no
/// anomaly. A closed bead whose pane is still alive is exactly the row worth
/// looking at, and dimming it is how it would be missed.
fn tone(row: &Row) -> Style {
    if row.agent.is_some() {
        return Style::new().fg(STAFFED);
    }
    let finished = row.status.is_closed() && row.agent.is_none() && row.anomalies.is_none();

    fg(finished.then_some(DIM))
}

/// The colour a bead's status is drawn in.
///
/// Colour is the second channel and never the only one: the glyph already says
/// the status, so a terminal with no colour loses nothing.
fn status_style(status: &Status) -> Style {
    fg(status_colour(status))
}

/// `bd`'s colour for a status, or none where `bd` sends no escape and the
/// glyph should take the brightness of the row it sits on.
fn status_colour(status: &Status) -> Option<Color> {
    match status {
        Status::InProgress => Some(IN_PROGRESS),
        Status::Blocked => Some(BLOCKED),
        Status::Closed => Some(CLOSED),
        Status::Deferred => Some(DIM),
        Status::Open => None,
        // The one status `bd` has no colour for, because it has no such
        // status. It takes the colour of the note already beside it.
        Status::Other(_) => Some(LOOK_AT_THIS),
    }
}

/// A style that says a colour, or one that says nothing and lets the line's
/// own reach the span.
fn fg(colour: Option<Color>) -> Style {
    colour.map_or_else(Style::new, |colour| Style::new().fg(colour))
}

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
    let tail = (tail::LINES + 1).min(rows.saturating_sub(1) / 2);
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

/// Which line the forest draws on one row of the screen, where it draws one.
///
/// The inverse of the skip-and-take in `draw`, and here beside it rather than
/// beside the click that asks the question: the two are one agreement about
/// where a line goes, and the failure they can have is drifting apart.
///
/// The column is not asked for. Every band spans the width of the screen, so
/// a row is the whole of what a pointer names.
pub fn line_at(forest: Rect, selected: usize, lines: usize, row: u16) -> Option<usize> {
    let within = row.checked_sub(forest.y)? as usize;
    if within >= forest.height as usize {
        return None;
    }

    let at = scroll_offset(selected, lines, forest.height as usize) + within;
    (at < lines).then_some(at)
}

/// The row at the foot of the screen: the keys, and every notice the view
/// carries.
///
/// The notices are drawn first and yield last: keys can be rediscovered, and
/// a fact that is silently absent from the one row a reader can neither fold
/// nor scroll away from is a fact they will never learn. Where the screen is
/// too narrow even for those, they yield from the end, so the caller's order
/// is the order they are given up in.
///
/// Nothing here knows what produced a notice. That is the point: a snapshot
/// and this process both reach the screen through the same list, and the next
/// thing that has something to say joins them by being one.
pub fn status_bar(notices: &[Notice], keys: &str) -> Fitted {
    let keys = Span::raw(keys.to_string());
    if notices.is_empty() {
        return Fitted::new(vec![keys], Vec::new(), Vec::new());
    }

    let said = notices
        .iter()
        .map(|notice| format!("{WARNING} {}", phrase::notice(*notice)))
        .collect::<Vec<_>>()
        .join(&" ".repeat(GAP));

    Fitted::new(
        vec![Span::styled(said, Style::new().fg(LOOK_AT_THIS))],
        Vec::new(),
        vec![keys],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use ratatui::backend::TestBackend;
    use ratatui::widgets::Widget;
    use ratatui::Terminal;

    use crate::model::anomaly::Anomaly;
    use crate::model::join::{AgentRef, Badged, BeadKey, JoinSource};
    use crate::model::snapshot::{
        Counts, FailedProject, Filter, LoosePane, Node, Snapshot, TrackerFailure, Tree,
    };
    use crate::model::types::PaneStatus;
    use crate::view::forest::flatten;
    use crate::view::{Action, Motion};
    use chrono::{TimeZone, Utc};

    const OPEN: &str = "▾ ";
    const SHUT: &str = "▸ ";
    const NO_FOLD: &str = "  ";
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

    /// One row of what a widget puts on screen, in runs of a single
    /// foreground colour. `drawn` reads symbols only and cannot see a colour
    /// at all, so a test about which spans a colour reaches asks here.
    fn painted<W: Widget>(widget: W, width: u16) -> Vec<(String, Color)> {
        let mut terminal = Terminal::new(TestBackend::new(width, 1)).expect("a test backend");
        terminal
            .draw(|frame| frame.render_widget(widget, frame.area()))
            .expect("a draw into memory");
        let buffer = terminal.backend().buffer();

        let mut runs: Vec<(String, Color)> = Vec::new();
        for x in 0..width {
            let cell = &buffer[(x, 0)];
            match runs.last_mut() {
                Some((said, colour)) if *colour == cell.fg => said.push_str(cell.symbol()),
                _ => runs.push((cell.symbol().to_string(), cell.fg)),
            }
        }
        runs
    }

    /// A run of closed siblings, under whichever bead the test likes: the
    /// drawing says the count and nothing about the bead it hangs under.
    fn elided(count: usize) -> Content {
        Content::Elided {
            count,
            under: lines::Place::root(BeadKey {
                project: "orbital".into(),
                id: "orb-7".into(),
            }),
        }
    }

    /// One forest line, behind the box-drawing a flatten would have put in
    /// front of it.
    fn under(prefix: &str, content: Content) -> lines::Line {
        lines::Line {
            prefix: prefix.into(),
            depth: 1,
            folded: None,
            place: None,
            content,
        }
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
            cycles: Vec::new(),
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
            project: "summit-works".into(),
            cwd: "/tmp/bdi-ground/summit-works".into(),
            pane_status: status,
        }
    }

    fn row(node: &Node) -> Row {
        row::cells(node, "nix-9670s", None, None)
    }

    // ---- a project's line ------------------------------------------------

    /// A project whose roots all read, so its line is its name and its counts
    /// and there are no panes to recover.
    fn project(name: &str, counts: Counts) -> ProjectLine {
        ProjectLine {
            project: name.into(),
            counts,
            recovery: None,
        }
    }

    /// The same, for a project where a root refused and whose panes had to be
    /// recovered from herdr instead.
    fn recovering(name: &str, panes: &[LoosePane], complete: bool) -> ProjectLine {
        ProjectLine {
            recovery: Some(Recovery {
                panes: panes.to_vec(),
                complete,
            }),
            ..project(name, counts(0, 0, 0, 0))
        }
    }

    fn unread(root: &str, tracker: TrackerState) -> Unread {
        Unread {
            root: root.into(),
            tracker,
        }
    }

    /// The design's own example, at the width it was written for. What is the
    /// project's is here; what is a root's is on the root's own row below.
    #[test]
    fn a_project_line_says_which_project_it_is_and_how_much_of_it_is_done() {
        let counts = counts(8, 21, 3, 3);

        assert_eq!(
            drawn(project_line(&project("summit-works", counts), OPEN), 40, 1),
            vec!["▾ summit-works       8/21  3 agents  ⚠ 3"]
        );
    }

    /// A count of nothing is left out rather than drawn as a nought: a line
    /// reading `0 agents  ⚠ 0` sends a reader looking for rows that are not
    /// there.
    #[test]
    fn a_project_with_no_live_agent_and_nothing_wrong_says_only_how_much_is_done() {
        let counts = counts(2, 7, 0, 0);

        assert_eq!(
            drawn(project_line(&project("homelab", counts), SHUT), 30, 1),
            vec!["▸ homelab                  2/7"]
        );
    }

    #[test]
    fn one_agent_is_not_described_in_the_plural() {
        let counts = counts(2, 7, 1, 0);
        let drawn = drawn(project_line(&project("homelab", counts), SHUT), 40, 1);

        assert!(drawn[0].ends_with("2/7  1 agent"), "{drawn:?}");
    }

    /// Narrower than the identity itself there is nothing left to protect, and
    /// the line is cut like any other.
    #[test]
    fn a_width_too_narrow_for_anything_else_keeps_as_much_of_the_project_as_it_can() {
        let counts = counts(8, 21, 3, 3);

        assert_eq!(
            drawn(project_line(&project("summit-works", counts), OPEN), 10, 1),
            vec!["▾ nixos-c…"]
        );
    }

    /// Width is columns on a screen, not bytes in a string. Every glyph in
    /// this vocabulary is several bytes long, and a cut counted in bytes would
    /// land inside one and put a broken character on the terminal.
    #[test]
    fn a_cut_is_counted_in_columns_and_never_lands_inside_a_glyph() {
        let name = "→→→→→→→→→→→→→→→→→→→→→→→→→→→→→→";
        let drawn = drawn(
            project_line(&project(name, counts(0, 1, 0, 0)), OPEN),
            20,
            1,
        );

        assert_eq!(drawn[0].chars().count(), 20);
        assert!(!drawn[0].contains('\u{fffd}'), "{drawn:?}");
    }

    // ---- a root's own row ------------------------------------------------

    /// `bdi-2bb.25`: a root is a bead like any other, so its status reaches
    /// the screen through the two channels every other bead's does — the
    /// glyph, and the colour that glyph is painted. Before this it was drawn
    /// on a header that spoke a project's language and answered none of it.
    ///
    /// Asked through `status_glyph` and `status_colour` rather than written
    /// out, so the mappings stay in the one place each owns.
    #[test]
    fn a_root_is_drawn_with_its_own_status_glyph_like_any_other_bead() {
        let forest = flatten(&snapshot(vec![grove(2)], Vec::new(), HerdrState::Ok));
        let root = &forest.lines()[1];

        let painted = painted(fitted(root, 12), 60);
        let drawn = drawn(fitted(root, 12), 60, 1);

        assert!(
            drawn[0].contains(&format!(
                "{} nix-9670s",
                row::status_glyph(&Status::InProgress)
            )),
            "{drawn:?}"
        );
        assert!(
            painted.iter().any(|(said, colour)| said
                .contains(row::status_glyph(&Status::InProgress))
                && Some(*colour) == status_colour(&Status::InProgress)),
            "{painted:?}"
        );
    }

    // ---- a root that would not read --------------------------------------

    /// A root `bdi` was told about and could not read has no row to draw, and
    /// leaving it out would lose it as surely as dropping it. It is named
    /// where its row would have been, with the reason beside it.
    #[test]
    fn an_unread_root_is_named_where_its_row_would_have_been_with_the_reason() {
        let unread = unread(
            "nix-9670s",
            TrackerState::Unreachable(TrackerFailure::Unavailable),
        );
        let drawn = drawn(unread_line(&unread, LAST, 9), 60, 1);

        assert!(drawn[0].contains("nix-9670s"), "{drawn:?}");
        assert!(
            drawn[0].contains(phrase::tracker_failure(TrackerFailure::Unavailable)),
            "{drawn:?}"
        );
    }

    /// A tracker that could not be read has no counts, and `0/0` would say the
    /// opposite of what is true — that it was read and holds nothing.
    #[test]
    fn an_unread_root_never_shows_a_count_it_could_not_read() {
        let unread = unread("nix-9670s", TrackerState::Unreachable(TrackerFailure::Auth));
        let drawn = drawn(unread_line(&unread, LAST, 9), 120, 1);

        assert!(!drawn[0].contains("0/0"), "{drawn:?}");
    }

    /// Nothing should reach this: a root that read is a bead row, and one that
    /// did not carries the failure that stopped it. A root that got here
    /// anyway is still a root on the screen, which is the whole point.
    #[test]
    fn a_root_with_no_row_and_no_reason_still_says_it_is_there() {
        let drawn = drawn(
            unread_line(&unread("nix-9670s", TrackerState::Ok), LAST, 9),
            90,
            1,
        );

        assert!(drawn[0].contains("nix-9670s"), "{drawn:?}");
        assert!(drawn[0].contains(phrase::root_unread()), "{drawn:?}");
    }

    /// The design has a project whose roots would not read render its panes.
    /// They are named the way a bead's agent is named, so one reads as the
    /// other.
    #[test]
    fn a_project_with_a_root_it_could_not_read_shows_the_panes_working_in_it() {
        let panes = [
            pane("wCM:p9", PaneStatus::Working),
            pane("wCM:p6", PaneStatus::Idle),
        ];

        assert_eq!(
            drawn(
                project_line(&recovering("summit-works", &panes, true), NO_FOLD),
                80,
                1
            ),
            vec![
                "  summit-works                                  ◍ wCM:p9 working · ◍ wCM:p6 idle"
                    .to_string()
            ]
        );
    }

    #[test]
    fn a_project_with_no_pane_to_show_says_that_rather_than_nothing() {
        let drawn = drawn(
            project_line(&recovering("summit-works", &[], true), OPEN),
            120,
            1,
        );

        assert!(drawn[0].contains(phrase::no_live_panes()), "{drawn:?}");
    }

    /// A pane list that cannot be known to be whole says so. A silently short
    /// list is the one way this line can be read wrongly, because it looks
    /// exactly like a complete one.
    #[test]
    fn a_pane_list_that_may_be_short_says_so_rather_than_reading_as_complete() {
        let panes = [pane("wCM:p9", PaneStatus::Working)];

        let whole = drawn(
            project_line(&recovering("summit-works", &panes, true), OPEN),
            200,
            1,
        );
        let partial = drawn(
            project_line(&recovering("summit-works", &panes, false), OPEN),
            200,
            1,
        );

        assert!(
            !whole[0].contains(phrase::panes_may_be_incomplete()),
            "{whole:?}"
        );
        assert!(
            partial[0].contains(phrase::panes_may_be_incomplete()),
            "{partial:?}"
        );
    }

    /// The identity of a root outlasts everything else on its line: a reader
    /// who cannot tell which root failed learns nothing from knowing one did.
    #[test]
    fn a_narrow_unread_root_keeps_the_root_over_the_reason() {
        let unread = unread("nix-9670s", TrackerState::Unreachable(TrackerFailure::Auth));
        let drawn = drawn(unread_line(&unread, LAST, 9), 24, 1);

        assert!(drawn[0].contains("nix-9670s"), "{drawn:?}");
        assert_eq!(drawn[0].chars().count(), 24);
    }

    // ---- a bead's line ---------------------------------------------------

    #[test]
    fn a_bead_line_says_its_glyph_its_id_and_its_title_in_that_order() {
        let node = node("nix-9670s.20", "wallpaper timer calls dms", Status::Blocked);

        assert_eq!(
            drawn(bead_line(&row(&node), BRANCH, 4), 46, 1),
            vec!["  ├── ● .20   wallpaper timer calls dms       "]
        );
    }

    /// An epic reads like the root above it: how far along, then who is on
    /// it, then what wants looking at. The count leads the state column
    /// because that is the order a header already puts them in.
    #[test]
    fn a_bead_standing_for_a_subtree_says_how_much_of_it_is_done_before_who_is_on_it() {
        let mut epic = row(&node(
            "nix-9670s.2",
            "the noctalia widget",
            Status::InProgress,
        ));
        epic.progress = Some(row::Progress {
            closed: 3,
            total: 8,
        });
        epic.agent = Some(row::agent_marker(&a_pane()));

        let drawn = drawn(bead_line(&epic, BRANCH, 3), 60, 1);

        let count = drawn[0].find("3/8").expect("the count is drawn");
        let agent = drawn[0].find("wCM:p9").expect("the agent is drawn");
        assert!(count < agent, "{drawn:?}");
    }

    /// A leaf stands for itself alone. A fraction over one bead would say
    /// nothing its glyph has not already said, and would spend width a title
    /// needs.
    #[test]
    fn a_bead_standing_only_for_itself_draws_no_count() {
        let leaf = row(&node(
            "nix-9670s.20",
            "wallpaper timer calls dms",
            Status::Open,
        ));

        let drawn = drawn(bead_line(&leaf, BRANCH, 4), 60, 1);

        assert!(!drawn[0].contains('/'), "{drawn:?}");
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
        staffed.agent = Some(a_pane());
        staffed.anomalies = vec![Anomaly::StaleClaim { days: 58 }];
        let drawn = drawn(bead_line(&row(&staffed), LAST, 4), 100, 1);

        assert!(drawn[0].contains("◍ wCM:p9 · working"), "{drawn:?}");
        assert!(drawn[0].contains("58"), "{drawn:?}");
    }

    /// A pane with something on it, which is all most of these rows need to
    /// know about an agent.
    fn a_pane() -> AgentRef {
        AgentRef {
            pane: "wCM:p9".into(),
            pane_status: PaneStatus::Working,
            title: None,
            source: JoinSource::AgentPane,
        }
    }

    fn captioned(caption: &str) -> Node {
        let mut staffed = node(
            "nix-9670s.20",
            "wallpaper timer calls dms",
            Status::InProgress,
        );
        staffed.agent = Some(AgentRef {
            title: Some(caption.into()),
            ..a_pane()
        });
        staffed
    }

    /// A caption is the first unbounded string to reach this cell — a pane id
    /// was short and fixed — so the cut it takes is the one every other cell
    /// takes, and the row is still exactly as wide as it was given.
    #[test]
    fn a_caption_too_long_for_the_row_is_cut_like_every_other_cell() {
        let staffed = captioned("teach the elided run to fold back open on a keypress");

        let drawn = drawn(bead_line(&row(&staffed), LAST, 4), 50, 1);

        assert_eq!(drawn[0].chars().count(), 50, "{drawn:?}");
        assert!(drawn[0].ends_with('…'), "{drawn:?}");
        assert!(!drawn[0].contains("keypress"), "{drawn:?}");
    }

    /// The cell is fitted before the title is, so a caption long enough takes
    /// the room the title would have had. The bead is still named by its id,
    /// which is fitted before either of them and cannot be crowded out.
    #[test]
    fn a_caption_long_enough_takes_the_room_the_title_would_have_had() {
        let staffed = captioned("teach the elided run to fold back open on a keypress");

        let drawn = drawn(bead_line(&row(&staffed), LAST, 4), 80, 1);

        assert!(!drawn[0].contains("wallpaper"), "{drawn:?}");
        assert!(drawn[0].contains(".20"), "{drawn:?}");
    }

    /// Narrow enough and the cell has no room at all. It goes whole rather
    /// than leaving a marker standing for a caption that is not there.
    #[test]
    fn a_caption_with_no_room_left_takes_the_whole_agent_cell_with_it() {
        let staffed = captioned("teach the elided run to fold back open on a keypress");

        let drawn = drawn(bead_line(&row(&staffed), LAST, 4), 14, 1);

        assert!(!drawn[0].contains(row::AGENT), "{drawn:?}");
        assert!(drawn[0].contains(".20"), "{drawn:?}");
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

    /// One of each status, so a loop over them covers the set. The compiler
    /// holds `status_colour` total; this list is only what a test walks.
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

    /// Two statuses sharing a colour would tell one story between them. Only
    /// `open` may arrive without one at all: `bd` sends no escape for it, and
    /// the glyph already says which status it is.
    #[test]
    fn no_colour_is_given_to_two_statuses_and_only_open_goes_without_one() {
        let coloured: Vec<Color> = every_status().iter().filter_map(status_colour).collect();

        for (nth, colour) in coloured.iter().enumerate() {
            assert!(
                !coloured[nth + 1..].contains(colour),
                "{colour:?} is drawn for two statuses"
            );
        }
        assert_eq!(
            coloured.len(),
            every_status().len() - 1,
            "one status goes without a colour and it is open"
        );
        assert_eq!(status_colour(&Status::Open), None);
    }

    // ---- bd's palette, and the brightness only bdi can draw ---------------

    /// Read off `bd` 1.2.2's own output. A reader coming from `bd list` has
    /// already learned these, and a status drawn in a colour `bd` gives to a
    /// different one would be worse than no colour at all.
    #[test]
    fn a_status_glyph_is_painted_the_colour_bd_paints_it() {
        let bds = [
            (Status::InProgress, Color::Rgb(255, 180, 84)),
            (Status::Blocked, Color::Rgb(242, 109, 120)),
            (Status::Closed, Color::Rgb(128, 144, 160)),
            (Status::Deferred, Color::Rgb(108, 118, 128)),
        ];

        for (status, colour) in bds {
            let bead = node("nix-9670s.1", "a bead", status.clone());
            let painted = painted(bead_line(&row(&bead), BRANCH, 3), 60);

            assert_eq!(
                painted[1],
                (row::status_glyph(&status).to_string(), colour),
                "{status:?}: {painted:?}"
            );
        }
    }

    /// `bd` sends no escape at all for an open bead's glyph, and inheriting is
    /// what lets the row's own brightness reach it. A glyph pinned to the
    /// terminal's default would leave a staffed row reading as two colours.
    #[test]
    fn an_open_glyph_takes_the_brightness_of_the_row_it_sits_on() {
        let mut staffed = node("nix-9670s.1", "a bead", Status::Open);
        staffed.agent = Some(a_pane());

        let painted = painted(bead_line(&row(&staffed), BRANCH, 3), 90);

        assert!(painted[1].0.starts_with('○'), "{painted:?}");
        assert_eq!(painted[1].1, Color::White, "{painted:?}");
    }

    /// The tier that earns the screen. `bd list` has no notion of a live
    /// agent, so it has no way to say which row is the one you came for.
    #[test]
    fn a_row_with_an_agent_on_it_is_drawn_brighter_than_one_without() {
        let mut staffed = node("nix-9670s.1", "a bead", Status::Open);
        staffed.agent = Some(a_pane());

        let bright = painted(bead_line(&row(&staffed), BRANCH, 3), 90);
        let plain = painted(
            bead_line(
                &row(&node("nix-9670s.1", "a bead", Status::Open)),
                BRANCH,
                3,
            ),
            90,
        );

        assert_eq!(bright[1].1, Color::White, "{bright:?}");
        assert_eq!(
            plain,
            vec![(plain[0].0.clone(), Color::Reset)],
            "nobody on it, so the whole line is the terminal's own"
        );
    }

    /// What `bd` already does to a closed row, arrived at from the other
    /// side: a finished branch nobody is on falls back into the page.
    #[test]
    fn a_finished_row_nobody_is_on_is_dimmed_to_the_grey_bd_dims_one_to() {
        let painted = painted(
            bead_line(
                &row(&node("nix-9670s.1", "a bead", Status::Closed)),
                BRANCH,
                3,
            ),
            60,
        );

        assert_eq!(
            painted[1].1,
            Color::Rgb(128, 144, 160),
            "the glyph keeps its own status colour: {painted:?}"
        );
        assert_eq!(painted[2].1, Color::Rgb(108, 118, 128), "{painted:?}");
    }

    /// Exactly the row worth looking at, and dimming it is how it would be
    /// missed. `lines::split` leaves it out of a run for the same reason.
    #[test]
    fn a_closed_bead_whose_pane_is_still_alive_is_not_dimmed() {
        let mut alive = node("nix-9670s.1", "a bead", Status::Closed);
        alive.agent = Some(a_pane());
        alive.anomalies = vec![Anomaly::StalePane];

        let painted = painted(bead_line(&row(&alive), BRANCH, 3), 110);

        assert_eq!(painted[2].1, Color::White, "{painted:?}");
    }

    /// Finished means what it means in `lines::split` — closed, no agent, no
    /// anomaly — so an anomaly alone is enough to keep a row out of the dim.
    #[test]
    fn a_closed_bead_with_an_anomaly_against_it_is_not_dimmed() {
        let mut odd = node("nix-9670s.1", "a bead", Status::Closed);
        odd.anomalies = vec![Anomaly::StalePane];

        let painted = painted(bead_line(&row(&odd), BRANCH, 3), 110);

        assert_eq!(painted[2].1, Color::Reset, "{painted:?}");
    }

    /// The box-drawing says how the tree is shaped, not how a bead is going,
    /// so it holds the terminal's default while the row around it moves.
    /// `bd list` leaves its own tree prefix undimmed on a closed row too.
    #[test]
    fn the_box_drawing_a_row_hangs_under_never_takes_the_rows_brightness() {
        let mut staffed = node("nix-9670s.1", "a bead", Status::Open);
        staffed.agent = Some(a_pane());
        let finished = node("nix-9670s.1", "a bead", Status::Closed);

        for bead in [staffed, finished] {
            let painted = painted(bead_line(&row(&bead), BRANCH, 3), 90);

            assert_eq!(
                painted[0],
                (BRANCH.to_string(), Color::Reset),
                "{painted:?}"
            );
        }
    }

    /// A run stands for finished rows and is drawn as one of them, so the two
    /// cannot fall out of step and the palette holds one grey, not two.
    #[test]
    fn an_elided_run_is_dimmed_the_same_grey_a_finished_row_is() {
        let run = painted(elided_run(BRANCH, 4), 60);
        let finished = painted(
            bead_line(
                &row(&node("nix-9670s.1", "a bead", Status::Closed)),
                BRANCH,
                3,
            ),
            60,
        );

        assert_eq!(run[0], (BRANCH.to_string(), Color::Reset), "{run:?}");
        assert_eq!(run[1].1, finished[1].1, "the glyph: {run:?}");
        assert_eq!(run[2].1, finished[2].1, "what follows it: {run:?}");
    }

    /// A project line's counts are its whole project's and not any one bead's,
    /// so the rule that decides a row's tier cannot be asked of it without
    /// quietly changing what it means. It stays off the scale. A root does
    /// not: it is a bead row, and the rule is asked of it like any other.
    #[test]
    fn a_project_line_is_left_off_the_scale_a_bead_row_is_on() {
        let quiet = project("homelab", counts(7, 7, 0, 0));

        let painted = painted(project_line(&quiet, OPEN), 60);

        assert!(
            painted.iter().all(|(_, colour)| *colour == Color::Reset),
            "{painted:?}"
        );
    }

    /// `bd`'s hues belong to `bd`'s concepts. The live agent and the anomaly
    /// are the two things it cannot say, so they are a different axis and
    /// keep a different colour system whatever the row around them does.
    #[test]
    fn the_cells_bd_cannot_draw_keep_their_own_colours_however_bright_the_row() {
        let mut staffed = node("nix-9670s.1", "a bead", Status::InProgress);
        staffed.agent = Some(a_pane());
        staffed.anomalies = vec![Anomaly::StaleClaim { days: 58 }];

        let painted = painted(bead_line(&row(&staffed), BRANCH, 3), 120);

        assert!(
            painted
                .iter()
                .any(|(said, colour)| said.contains(AGENT) && *colour == LIVE),
            "{painted:?}"
        );
        assert!(
            painted
                .iter()
                .any(|(said, colour)| said.contains(WARNING) && *colour == LOOK_AT_THIS),
            "{painted:?}"
        );
    }

    /// The one status `bd` has no colour for, because it has no such status.
    /// It takes the colour of the note already beside it on the row.
    #[test]
    fn a_status_bd_never_had_is_painted_the_colour_of_the_note_beside_it() {
        let odd = node("nix-9670s.1", "a bead", Status::Other("triage".into()));

        let painted = painted(bead_line(&row(&odd), BRANCH, 3), 120);

        assert_eq!(painted[1], ('?'.to_string(), LOOK_AT_THIS), "{painted:?}");
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

    // ---- the line a screen row shows --------------------------------------

    /// The inverse held against the drawing rather than against itself. The
    /// fixture's rows are the header and then `bead number 1` upward, so what
    /// is on a row says which line was drawn there, and a forest taller than
    /// its band is scrolled far enough that an off-by-one in either direction
    /// shows.
    #[test]
    fn every_row_of_the_forest_names_the_line_drawn_on_it() {
        let (width, height) = (60, 24);
        let band = regions(Rect::new(0, 0, width, height)).forest;
        let mut forest = opened(&snapshot(vec![grove(40)], Vec::new(), HerdrState::Ok));
        forest.set_half_screen(half_screen(band));
        forest.apply(Action::Move(Motion::HalfScreenDown));

        let frame = frame_of(&forest, width, height);
        let selected = forest.selected_line();
        let lines = forest.lines().len();

        for row in band.y..band.y + band.height {
            let at = line_at(band, selected, lines, row).expect("the band is full of lines");
            let shown = match at {
                0 => "summit-works".to_string(),
                1 => "lift the ground station".to_string(),
                at => format!("bead number {}", at - 1),
            };
            assert!(
                frame[row as usize].contains(&shown),
                "row {row} shows {:?}, not line {at}",
                frame[row as usize]
            );
        }
    }

    /// The rows under the last line of a short forest are blank, and a click
    /// on blank is a click on nothing.
    #[test]
    fn a_row_past_the_last_line_names_none() {
        let band = Rect::new(0, 0, 60, 16);

        assert_eq!(line_at(band, 0, 3, 2), Some(2));
        for row in 3..16 {
            assert_eq!(line_at(band, 0, 3, row), None, "row {row}");
        }
    }

    /// The tail and the key row are drawn by someone else and hold nothing
    /// the selection can sit on.
    #[test]
    fn a_row_outside_the_forest_band_names_none() {
        let bands = regions(Rect::new(0, 0, 60, 24));
        let (selected, lines) = (0, 100);

        for row in [bands.tail.y, bands.tail.y + 3, bands.keys.y] {
            assert_eq!(
                line_at(bands.forest, selected, lines, row),
                None,
                "row {row}"
            );
        }
    }

    /// A band that starts partway down the screen is the only kind the forest
    /// ever gets when something is drawn above it, and a row measured from
    /// the top of the screen rather than the top of the band would be wrong
    /// by exactly that offset.
    #[test]
    fn a_row_above_the_forest_band_names_none() {
        let band = Rect::new(0, 4, 60, 8);

        assert_eq!(line_at(band, 0, 100, 4), Some(0));
        for row in 0..4 {
            assert_eq!(line_at(band, 0, 100, row), None, "row {row}");
        }
    }

    // ---- the key bar -----------------------------------------------------

    /// A key row shaped like the real one, without importing the loop's.
    const A_KEY_ROW: &str = "Enter focus   a all   ? keys   ^R refresh   q quit";

    /// What the row says is the loop's to decide; the foot's job is to put it
    /// on screen whole where there is room for it.
    #[test]
    fn the_foot_of_the_screen_shows_the_keys_it_is_handed() {
        let drawn = drawn(status_bar(&[], A_KEY_ROW), 60, 1);

        assert!(drawn[0].starts_with(A_KEY_ROW), "{drawn:?}");
    }

    /// With no herdr there is no agent on any row, and a screen that only
    /// stopped showing them would read as a fleet with nobody working in it.
    /// It goes at the foot because that is the one row that cannot be folded
    /// or scrolled away.
    #[test]
    fn a_herdr_that_could_not_be_reached_is_said_where_nothing_can_hide_it() {
        let drawn = drawn(status_bar(&[Notice::NoHerdr], A_KEY_ROW), 90, 1);

        assert!(
            drawn[0].contains(phrase::notice(Notice::NoHerdr)),
            "{drawn:?}"
        );
    }

    /// The bead this row was built for: a `bdi` that could not open its
    /// inbound socket is told nothing when a project changes, so what is on
    /// screen is only as fresh as the last poll. Nothing above the foot could
    /// show that — no row is wrong — so the foot is the only place it can go.
    #[test]
    fn a_bdi_nothing_can_reach_says_so_for_the_life_of_the_session() {
        let drawn = drawn(status_bar(&[Notice::NoInboundChannel], A_KEY_ROW), 90, 1);

        assert!(
            drawn[0].contains(phrase::notice(Notice::NoInboundChannel)),
            "{drawn:?}"
        );
    }

    /// Two notices are two facts and the reader needs both: neither one
    /// implies the other, and a foot that showed only the first would leave
    /// the second unsaid for the whole session.
    #[test]
    fn a_foot_with_room_says_every_notice_it_is_given() {
        let drawn = drawn(
            status_bar(&[Notice::NoHerdr, Notice::NoInboundChannel], A_KEY_ROW),
            200,
            1,
        );

        for said in [Notice::NoHerdr, Notice::NoInboundChannel] {
            assert!(drawn[0].contains(phrase::notice(said)), "{drawn:?}");
        }
    }

    /// The order the caller gives is the order the foot gives up, so a screen
    /// with room for one keeps the one that costs the reader most.
    #[test]
    fn a_narrow_foot_gives_up_the_last_notice_first() {
        let drawn = drawn(
            status_bar(&[Notice::NoHerdr, Notice::NoInboundChannel], A_KEY_ROW),
            60,
            1,
        );

        assert!(
            drawn[0].contains(phrase::notice(Notice::NoHerdr)),
            "{drawn:?}"
        );
        assert!(
            !drawn[0].contains(phrase::notice(Notice::NoInboundChannel)),
            "{drawn:?}"
        );
    }

    /// A frame draws what the snapshot behind it found and what the session
    /// settled at startup through one list, and the snapshot's go first
    /// because a herdr nobody can reach empties the agent column.
    #[test]
    fn the_snapshots_notice_outranks_the_sessions() {
        assert_eq!(
            notices(HerdrState::Unavailable, &[Notice::NoInboundChannel]),
            vec![Notice::NoHerdr, Notice::NoInboundChannel]
        );
    }

    /// A session fact reaches the foot whether or not the collection behind
    /// the frame found anything to say — the two travel by the same road and
    /// neither depends on the other.
    #[test]
    fn a_session_notice_stands_alone_where_the_snapshot_is_well() {
        assert_eq!(
            notices(HerdrState::Ok, &[Notice::NoInboundChannel]),
            vec![Notice::NoInboundChannel]
        );
    }

    #[test]
    fn a_session_with_nothing_wrong_leaves_the_foot_to_the_keys() {
        assert_eq!(notices(HerdrState::Ok, &[]), Vec::new());
    }

    /// Keys can be rediscovered; a herdr that is silently absent cannot. So on
    /// a screen too narrow for both, the keys are what gives way.
    #[test]
    fn a_narrow_foot_gives_up_the_keys_before_the_missing_herdr() {
        let drawn = drawn(status_bar(&[Notice::NoHerdr], A_KEY_ROW), 60, 1);

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

    /// A run stands for closed beads and nothing else — `split` selects on
    /// exactly that — so its glyph is not a summary over mixed states but the
    /// one state every member holds. Resolved through `status_glyph` and
    /// `status_style`, the same two the beads themselves go through, so a run
    /// and the beads it stands for cannot drift apart.
    #[test]
    fn an_elided_run_carries_the_closed_glyph_each_bead_it_stands_for_would() {
        let painted = painted(fitted(&under(BRANCH, elided(15)), 0), 72);

        assert_eq!(
            painted[1],
            (
                row::status_glyph(&Status::Closed).to_string(),
                status_colour(&Status::Closed).expect("closed is one bd colours")
            )
        );
    }

    /// A reader follows the vertical rules down a tree. A sentence that took
    /// its box-drawing into its own colour would break that run wherever it
    /// fell, so the drawing stays in the terminal's own foreground and only
    /// the words beside it are coloured.
    #[test]
    fn an_elided_run_leaves_its_box_drawing_in_the_terminals_own_colour() {
        let painted = painted(fitted(&under(BRANCH, elided(3)), 0), 72);

        assert_eq!(painted[0], (BRANCH.to_string(), Color::Reset));
        assert_eq!(painted[2].1, DIM);
    }

    #[test]
    fn a_note_leaves_its_box_drawing_in_the_terminals_own_colour() {
        let painted = painted(
            fitted(&under(LAST, Content::Note(Note::Dangling(2))), 0),
            96,
        );

        assert_eq!(painted[0], (LAST.to_string(), Color::Reset));
        assert_eq!(painted[1].1, LOOK_AT_THIS);
    }

    /// Every other note is a fault and wears a warning. Nothing went wrong in
    /// a forest with no work left in it, and a warning over that reads as one
    /// — so it is drawn plain, in one colour the whole way across.
    #[test]
    fn the_line_for_an_empty_forest_is_drawn_in_the_terminals_own_colour() {
        let painted = painted(fitted(&under("", Content::Note(Note::NoRoots)), 0), 96);

        assert_eq!(painted.len(), 1, "{painted:?}");
        assert_eq!(painted[0].1, Color::Reset);
        assert!(!painted[0].0.contains(WARNING), "{painted:?}");
    }

    /// The failed and conflicted items in the bottom groups get their
    /// box-drawing the same way a bead does, so they are tree drawing too.
    #[test]
    fn a_failed_project_leaves_its_box_drawing_in_the_terminals_own_colour() {
        let failed = Item::Failed(FailedProject {
            project: "summit-works".into(),
            tracker: TrackerFailure::Auth,
        });
        let painted = painted(item_line(LAST, &failed), 96);

        assert_eq!(painted[0], (LAST.to_string(), Color::Reset));
        assert_eq!(painted[1].1, LOOK_AT_THIS);
    }

    /// The fold arrow is a control rather than a word, and every group has
    /// one. Drawn in the terminal's own foreground the column of arrows reads
    /// as the one control it is, whatever the group beside each says.
    #[test]
    fn a_groups_fold_arrow_is_drawn_in_the_terminals_own_colour() {
        for kind in GroupKind::ALL {
            let group = Group {
                kind,
                count: 2,
                with_findings: 0,
            };
            let painted = painted(group_line(SHUT, group), 80);

            assert_eq!(painted[0].1, Color::Reset, "{kind:?}: {painted:?}");
            assert!(painted[0].0.starts_with(SHUT), "{kind:?}: {painted:?}");
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
            unconfigured: Vec::new(),
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

    /// A forest with its first tree opened by hand. Nothing in these fixtures
    /// is staffed, so the fold default rests every tree as its header, and
    /// what these tests are about is the rows under one.
    fn opened(snapshot: &Snapshot) -> Forest {
        let mut forest = flatten(snapshot);
        forest.apply(Action::ToggleFold);
        forest
    }

    fn frame_of(forest: &Forest, width: u16, height: u16) -> Vec<String> {
        frame_with(forest, &[], width, height)
    }

    fn frame_with(forest: &Forest, at_startup: &[Notice], width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test backend");
        terminal
            .draw(|frame| draw(frame, frame.area(), forest, at_startup, A_KEY_ROW))
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
        let forest = opened(&snapshot(
            vec![grove(2)],
            Vec::new(),
            HerdrState::Unavailable,
        ));

        assert_eq!(
            frame_of(&forest, 60, 10),
            vec![
                "▾ summit-works                                           0/3",
                "  └── ◐ nix-9670s  lift the ground station               0/3",
                "      ├── ○ .1         bead number 1                        ",
                "      └── ○ .2         bead number 2                        ",
                "                                                            ",
                "                                                            ",
                "                                                            ",
                "                                                            ",
                "                                                            ",
                "⚠ no herdr session · which agents are alive is unknown  Ent…",
            ]
        );
    }

    /// The bead's own case, at the whole-frame level: a `bdi` that could not
    /// open its socket says so at the foot, and it is the same row and the
    /// same shape a herdr failure uses. Nothing above the foot changes,
    /// because nothing above the foot is wrong.
    #[test]
    fn a_socket_that_would_not_open_is_said_at_the_foot_of_the_frame() {
        let forest = opened(&snapshot(vec![grove(2)], Vec::new(), HerdrState::Ok));

        assert_eq!(
            frame_with(&forest, &[Notice::NoInboundChannel], 80, 10),
            vec![
                "▾ summit-works                                                               0/3",
                "  └── ◐ nix-9670s  lift the ground station                                   0/3",
                "      ├── ○ .1         bead number 1                                            ",
                "      └── ○ .2         bead number 2                                            ",
                "                                                                                ",
                "                                                                                ",
                "                                                                                ",
                "                                                                                ",
                "                                                                                ",
                "⚠ nothing can tell bdi a project changed · every project is polled instead  Ent…",
            ]
        );
    }

    /// The definition of done's first case, at the whole-frame level: a narrow
    /// screen cuts every row and wraps none, so the row count on screen still
    /// matches the line count in the forest.
    #[test]
    fn a_narrow_frame_cuts_every_row_and_wraps_none() {
        let forest = opened(&snapshot(vec![grove(2)], Vec::new(), HerdrState::Ok));
        let frame = frame_of(&forest, 24, 10);

        assert_eq!(
            frame[..4].to_vec(),
            vec![
                "▾ summit-works       0/3",
                "  └── ◐ nix-9670s    0/3",
                "      ├── ○ .1         …",
                "      └── ○ .2         …",
            ]
        );
        assert!(
            frame[4..9].iter().all(|row| row.trim().is_empty()),
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
                Content::Project(line) => line.project.clone(),
                other => panic!("unexpected line under the selection: {other:?}"),
            };
            let frame = frame_of(&forest, 60, 10);

            assert!(
                frame.iter().any(|row| row.contains(&said)),
                "{motion:?} put line {at} ({said}) off screen: {frame:?}"
            );
        }
    }

    /// The definition of done's third case: a root nobody could read renders
    /// as the root it is, the reason it failed, and — on its project's line —
    /// the panes still working there.
    #[test]
    fn a_root_that_would_not_read_draws_its_reason_and_its_projects_panes() {
        let failed =
            Tree::tracker_unreachable("summit-works", "nix-9670s", TrackerFailure::Unavailable);
        let forest = flatten(&snapshot(
            vec![failed],
            vec![pane("wCM:p9", PaneStatus::Working)],
            HerdrState::Ok,
        ));
        let frame = frame_of(&forest, 77, 4);

        assert_eq!(
            frame[..2],
            [
                "▾ summit-works                                               ◍ wCM:p9 working",
                "  └── ⚠ nix-9670s  the tracker did not answer                                ",
            ]
        );
    }

    /// A root with nothing under it has no fold for a marker to stand for, so
    /// it draws none — and it still starts in the column its siblings start
    /// in, because a reader running down a project's roots finds every one of
    /// them in the same place.
    #[test]
    fn a_root_with_nothing_under_it_draws_no_marker_and_still_lines_up() {
        let unreadable =
            Tree::tracker_unreachable("summit-works", "nix-9670s", TrackerFailure::Unavailable);
        let forest = flatten(&snapshot(
            vec![grove(2), unreadable],
            Vec::new(),
            HerdrState::Ok,
        ));
        let frame = frame_of(&forest, 90, 5);
        let unread = frame
            .iter()
            .position(|row| row.contains(WARNING))
            .expect("the root that would not read");
        let column = |row: &str| {
            let byte = row.find("nix-9670s").expect("the root on the row");
            row[..byte].chars().count()
        };

        assert!(
            !frame[unread].contains(SHUT.trim()),
            "nothing opens this root, so nothing should say it is shut: {:?}",
            frame[unread]
        );
        assert_eq!(
            column(&frame[unread]),
            column(&frame[1]),
            "the mark stands where a status glyph does:\n{}\n{}",
            frame[1],
            frame[unread]
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
