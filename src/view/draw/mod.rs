//! The forest and the tail, drawn into a ratatui frame.
//!
//! One module for each thing drawn — a project's line, a bead's, the groups
//! below the trees, the foot, the tail — one for the bands they are drawn
//! into, and one for the colours they are drawn in. What is left here is the
//! frame itself: which kind of line each row is, and the few cells that more
//! than one kind draws with.

mod bands;
mod bead;
mod foot;
mod groups;
mod project;
mod tail;
mod tone;

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use ratatui::Frame;

use crate::app::InFlight;
use crate::model::types::PaneStatus;
use crate::view::fitted::{columns, Fitted, GAP};
use crate::view::forest::Forest;
use crate::view::lines::{self, Content, Note, ProjectLine};
use crate::view::phrase;
use crate::view::row::{AGENT, WARNING};
use crate::view::{Freshness, Notice};

pub use bands::{half_screen, line_at, regions};
pub use tail::draw_tail;

use bands::scroll_offset;
use bead::{bead_line, elided_run};
use foot::{notices, status_bar};
use groups::{group_line, item_line};
use project::{project_line, unread_line};
use tone::LOOK_AT_THIS;

/// What every project line's freshness is drawn from: when each project was
/// last read, which projects the collection in flight is reading, and the
/// instant this frame is being drawn at.
///
/// Gathered at the frame rather than held on the lines. A collection starting
/// and ending changes what a project line says without changing the snapshot
/// under it, and the mark turns between two collections' worth of events — so
/// a line that carried its own answer would have to be flattened again to say
/// anything new.
pub(super) struct Reads<'a> {
    read_at: &'a BTreeMap<String, DateTime<Utc>>,
    /// What the collection in flight is reading and when it was asked for,
    /// where one is running. The instant is what tells a collection that is
    /// under way from one that has stopped answering.
    collecting: Option<&'a InFlight>,
    now: DateTime<Utc>,
}

impl<'a> Reads<'a> {
    pub(super) fn new(
        read_at: &'a BTreeMap<String, DateTime<Utc>>,
        collecting: Option<&'a InFlight>,
        now: DateTime<Utc>,
    ) -> Self {
        Self {
            read_at,
            collecting,
            now,
        }
    }

    /// How fresh one project is.
    ///
    /// Whether it is being read now is asked with `Wanted::names`, the same
    /// predicate the collector picks what to read with, so the line and the
    /// collection agree by construction rather than by argument. How the last
    /// collection of it went comes off the line, because it changes only when
    /// the snapshot does.
    fn of(&self, project: &ProjectLine) -> Option<Freshness> {
        Freshness::of(
            self.read_at.get(&project.project).copied(),
            self.collecting
                .filter(|in_flight| in_flight.wanted.names(&project.project)),
            project.every_root_read,
            self.now,
        )
    }
}

/// Draw the forest and the key bar, leaving the tail's band to whoever holds
/// a tail.
///
/// `keys` arrives already named. What a key is called belongs with the
/// mapping that answers it, and this file has never known one.
pub fn draw(
    frame: &mut Frame,
    area: Rect,
    forest: &Forest,
    at_startup: &[Notice],
    collecting: Option<&InFlight>,
    now: DateTime<Utc>,
    keys: &str,
) {
    let bands = regions(area);
    let lines = forest.lines();
    let selected = forest.selected_line();
    let height = bands.forest.height as usize;
    let ids = id_width(lines);
    let reads = Reads::new(&forest.snapshot().read_at, collecting, now);

    for (row, (at, line)) in lines
        .iter()
        .enumerate()
        .skip(scroll_offset(selected, lines.len(), height))
        .take(height)
        .enumerate()
    {
        let drawn = fitted(line, ids, &reads);
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
        status_bar(
            &notices(forest.snapshot().herdr, at_startup),
            keys,
            bands.keys.width as usize,
        ),
        bands.keys,
    );
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
pub(super) fn fitted(line: &lines::Line, id_width: usize, reads: &Reads) -> Fitted {
    match &line.content {
        Content::Project(project) => {
            project_line(project, &line.prefix, reads.of(project), reads.now)
        }
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

/// A line that is one sentence and nothing else.
pub(super) fn sentence(prefix: &str, said: String, colour: Color) -> Fitted {
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

/// How far along something is. A tree and one epic inside it ask the same
/// question of different scopes, so they answer it in the same words.
pub(super) fn done(closed: usize, total: usize) -> String {
    format!("{closed}/{total}")
}

/// Put a cell in a row's state block, beside the ones already there.
///
/// The gap belongs *between* the cells: two that abut read as one that names
/// neither, and a gap in front of the first is spent rather than seen,
/// because the block is set against the row's right edge and the padding
/// swallows it. Every state block on a row is built this way, so there is one
/// place to be right about it rather than one per kind of row.
pub(super) fn beside(state: &mut Vec<Span<'static>>, cell: Span<'static>) {
    if !state.is_empty() {
        state.push(Span::raw(" ".repeat(GAP)));
    }
    state.push(cell);
}

pub(super) fn pane_marker(pane: &str, status: &PaneStatus) -> String {
    format!("{AGENT} {pane} {}", phrase::pane_state(status))
}

/// The box-drawing a line hangs under. It says how the tree is shaped rather
/// than how a bead is going, so it is held at the terminal's default while the
/// row around it dims or brightens. `bd list` leaves its own tree prefix
/// undimmed on a closed row too.
pub(super) fn structure(prefix: &str) -> Span<'static> {
    Span::styled(prefix.to_string(), Style::new().fg(Color::Reset))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use ratatui::style::Modifier;
    use std::collections::BTreeMap;

    use crate::app::Wanted;
    use crate::model::join::{AgentRef, BeadKey, JoinSource};
    use crate::model::snapshot::{
        Counts, Filter, HerdrState, LoosePane, Node, Snapshot, TrackerFailure, TrackerState, Tree,
    };
    use crate::model::types::Status;
    use crate::view::forest::flatten;
    use crate::view::row::{self, Row};
    use crate::view::{Action, Motion};
    use chrono::{DateTime, TimeZone, Utc};

    pub(super) use crate::view::painted::Painted;

    pub(super) const OPEN: &str = "▾ ";
    pub(super) const SHUT: &str = "▸ ";
    pub(super) const NO_FOLD: &str = "  ";
    pub(super) const BRANCH: &str = "  ├── ";
    pub(super) const LAST: &str = "  └── ";

    /// A row says these words.
    ///
    /// The words are written out at the call rather than asked of the code
    /// that drew the row. A test that takes them from `phrase` passes
    /// whatever `phrase` says, the empty string included, so it proves the
    /// words reached the screen and nothing about what they are.
    pub(super) fn says(row: &str, words: &str) {
        assert!(
            !words.is_empty(),
            "every row says nothing, so nothing is asserted"
        );
        assert!(row.contains(words), "{row:?} does not say {words:?}");
    }

    /// A row does not say these words. Inverted, the same guard is needed for
    /// the opposite reason: no row leaves nothing out, so an empty
    /// expectation fails whatever the row says.
    pub(super) fn does_not_say(row: &str, words: &str) {
        assert!(
            !words.is_empty(),
            "no row leaves nothing out, so nothing is asserted"
        );
        assert!(!row.contains(words), "{row:?} says {words:?}");
    }

    /// The whole point of the guard: a phrase emptied at source and passed
    /// straight through would satisfy `contains` on every row ever drawn.
    #[test]
    #[should_panic(expected = "nothing is asserted")]
    fn nothing_is_not_something_a_row_can_say() {
        says("⚠ agents unknown", "");
    }

    /// And its mirror: no row leaves nothing out, so the inverted form has to
    /// refuse the same expectation for the opposite reason.
    #[test]
    #[should_panic(expected = "nothing is asserted")]
    fn nothing_is_not_something_a_row_can_leave_out() {
        does_not_say("⚠ agents unknown", "");
    }

    /// A run of closed siblings, under whichever bead the test likes: the
    /// drawing says the count and nothing about the bead it hangs under.
    pub(super) fn elided(count: usize) -> Content {
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
    pub(super) fn under(prefix: &str, content: Content) -> lines::Line {
        lines::Line {
            prefix: prefix.into(),
            depth: 1,
            folded: None,
            place: None,
            content,
        }
    }

    pub(super) fn counts(
        closed: usize,
        total: usize,
        live_agents: usize,
        anomalies: usize,
    ) -> Counts {
        Counts {
            total,
            closed,
            live_agents,
            anomalies,
        }
    }

    pub(super) fn tree(project: &str, root: &str, title: &str, counts: Counts) -> Tree {
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

    pub(super) fn node(id: &str, title: &str, status: Status) -> Node {
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

    pub(super) fn pane(pane: &str, status: PaneStatus) -> LoosePane {
        LoosePane {
            pane: pane.into(),
            project: "summit-works".into(),
            cwd: "/tmp/bdi-ground/summit-works".into(),
            pane_status: status,
        }
    }

    pub(super) fn row(node: &Node) -> Row {
        row::cells(node, "nix-9670s", None, None)
    }

    /// A project whose roots all read, so its line is its name and its counts
    /// and there are no panes to recover.
    pub(super) fn project(name: &str, counts: Counts) -> ProjectLine {
        ProjectLine {
            project: name.into(),
            counts,
            every_root_read: true,
            recovery: None,
        }
    }

    /// A pane with something on it, which is all most of these rows need to
    /// know about an agent.
    pub(super) fn a_pane() -> AgentRef {
        AgentRef {
            pane: "wCM:p9".into(),
            pane_status: PaneStatus::Working,
            title: None,
            source: JoinSource::AgentPane,
        }
    }

    /// A key row shaped like the real one, without importing the loop's.
    pub(super) const A_KEY_ROW: &str = "Enter focus   a all   ? keys   ^R refresh   q quit";

    #[test]
    fn a_note_leaves_its_box_drawing_in_the_terminals_own_colour() {
        let painted = Painted::of(
            fitted(
                &under(LAST, Content::Note(Note::Dangling(2))),
                0,
                &at_rest(),
            ),
            96,
            1,
        )
        .row(0);

        assert_eq!(painted[0].said, LAST);
        assert_eq!(painted[0].style.fg, Some(Color::Reset));
        assert_eq!(painted[1].style.fg, Some(LOOK_AT_THIS));
    }

    /// A note's count is a count of beads, and the word is what says so. It
    /// is written out here rather than asked of `phrase`, because a count
    /// corrected by renaming what it counts would leave every test that reads
    /// the number alone green.
    #[test]
    fn a_note_names_the_beads_the_tracker_stopped_at() {
        let drawn = Painted::of(
            fitted(
                &under(LAST, Content::Note(Note::Truncated(1))),
                0,
                &at_rest(),
            ),
            96,
            1,
        )
        .rows();

        says(
            &drawn[0],
            "1 bead the tracker stopped at · what hangs beneath it is not in this tree",
        );
    }

    /// Every other note is a fault and wears a warning. Nothing went wrong in
    /// a forest with no work left in it, and a warning over that reads as one
    /// — so it is drawn plain, in one colour the whole way across.
    #[test]
    fn the_line_for_an_empty_forest_is_drawn_in_the_terminals_own_colour() {
        let painted = Painted::of(
            fitted(&under("", Content::Note(Note::NoRoots)), 0, &at_rest()),
            96,
            1,
        )
        .row(0);

        assert_eq!(painted.len(), 1, "{painted:?}");
        assert_eq!(painted[0].style.fg, Some(Color::Reset));
        assert!(!painted[0].said.contains(WARNING), "{painted:?}");
    }

    // ---- the whole frame -------------------------------------------------

    pub(super) fn snapshot(
        trees: Vec<Tree>,
        unattributed: Vec<LoosePane>,
        herdr: HerdrState,
    ) -> Snapshot {
        // The projects a real collection would have named beside these trees,
        // in the order the trees arrive in.
        let mut projects: Vec<String> = Vec::new();
        for tree in &trees {
            if projects.last() != Some(&tree.project) {
                projects.push(tree.project.clone());
            }
        }

        Snapshot {
            generated_at: Utc.with_ymd_and_hms(2026, 8, 30, 10, 22, 14).unwrap(),
            herdr,
            filter: Filter::All,
            collected: trees.clone(),
            trees,
            projects,
            hidden_trees: Vec::new(),
            failed_projects: Vec::new(),
            unattributed,
            unconfigured: Vec::new(),
            conflicts: Vec::new(),
            read_at: BTreeMap::from([("summit-works".to_string(), read_at())]),
        }
    }

    /// When the fixture's tracker was read. Half a minute before the
    /// snapshot was generated, so a frame quoting the wrong one of the two
    /// says so rather than agreeing by coincidence.
    pub(super) fn read_at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 30, 10, 21, 44).unwrap()
    }

    /// The instant a test frame is drawn at: the snapshot's own. The read
    /// behind it is half a minute older, so a project line drawn from it is
    /// half a minute stale.
    pub(super) fn drawn_at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 30, 10, 22, 14).unwrap()
    }

    /// Nothing has been read and nothing is being read, for the lines that
    /// say nothing about either.
    static NOTHING_READ: BTreeMap<String, DateTime<Utc>> = BTreeMap::new();

    pub(super) fn at_rest() -> Reads<'static> {
        Reads::new(&NOTHING_READ, None, drawn_at())
    }

    /// One tree of `children` open beads under an in-flight root.
    pub(super) fn grove(children: usize) -> Tree {
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
    pub(super) fn opened(snapshot: &Snapshot) -> Forest {
        let mut forest = flatten(snapshot);
        forest.apply(Action::ToggleFold);
        forest
    }

    pub(super) fn frame_of(forest: &Forest, width: u16, height: u16) -> Painted {
        frame_collecting(forest, None, width, height)
    }

    /// A collection reading `wanted`, asked for at the instant the frame is
    /// drawn — so it is a collection under way rather than one that has
    /// stopped answering.
    pub(super) fn reading(wanted: Wanted) -> InFlight {
        InFlight {
            wanted,
            asked_at: drawn_at(),
            patience: PATIENCE,
        }
    }

    /// How long the collections these tests build may go unanswered. A round
    /// number the instants are written against, rather than the configured
    /// default: what they assert is which mark a wait produces, not what the
    /// deadline is.
    pub(super) const PATIENCE: chrono::TimeDelta = chrono::TimeDelta::seconds(30);

    /// The same frame with a collection in flight, so a project line the
    /// collection names says so.
    pub(super) fn frame_collecting(
        forest: &Forest,
        collecting: Option<&InFlight>,
        width: u16,
        height: u16,
    ) -> Painted {
        frame_with(forest, &[], collecting, width, height)
    }

    fn frame_with(
        forest: &Forest,
        at_startup: &[Notice],
        collecting: Option<&InFlight>,
        width: u16,
        height: u16,
    ) -> Painted {
        Painted::drawn_by(width, height, |frame| {
            draw(
                frame,
                frame.area(),
                forest,
                at_startup,
                collecting,
                drawn_at(),
                A_KEY_ROW,
            );
        })
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
            frame_of(&forest, 60, 10).rows(),
            vec![
                "▾ summit-works  ✓ 30s ago                                0/3",
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
            frame_with(&forest, &[Notice::NoInboundChannel], None, 80, 10).rows(),
            vec![
                "▾ summit-works  ✓ 30s ago                                                    0/3",
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
        let frame = frame_of(&forest, 24, 10).rows();

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
            let frame = frame_of(&forest, 60, 10).rows();

            assert!(
                frame.iter().any(|row| row.contains(&said)),
                "{motion:?} put line {at} ({said}) off screen: {frame:?}"
            );
        }
    }

    /// Which row the cursor is on, and only that one.
    ///
    /// `Fitted::selected` reverses the row's whole style and moves not one
    /// word, so nothing in the symbols says where the cursor is: a frame that
    /// drew every row selected but the selected one reads the same as a
    /// correct one.
    #[test]
    fn the_row_under_the_cursor_is_the_only_one_drawn_reversed() {
        let mut forest = opened(&snapshot(vec![grove(2)], Vec::new(), HerdrState::Ok));
        forest.apply(Action::Move(Motion::FirstRow));
        forest.apply(Action::Move(Motion::NextRow));
        let selected = forest.selected_line();
        let lines = forest.lines().len();

        let frame = frame_of(&forest, 60, 10);

        for at in 0..lines {
            let reversed = frame
                .row(at)
                .iter()
                .all(|run| run.style.add_modifier.contains(Modifier::REVERSED));

            assert_eq!(
                reversed,
                at == selected,
                "row {at} of {lines}, cursor on {selected}: {:?}",
                frame.row(at)
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
        let frame = frame_of(&forest, 77, 4).rows();

        assert_eq!(
            frame[..2],
            [
                "▾ summit-works  ⚠ 30s ago                                    ◍ wCM:p9 working",
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
        let frame = frame_of(&forest, 90, 5).rows();
        // The project's own line wears the warning too — one of its roots
        // would not read, which is what this fixture is — so the root is
        // found by the warning and its own id together.
        let unread = frame
            .iter()
            .position(|row| row.contains(WARNING) && row.contains("nix-9670s"))
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
}
