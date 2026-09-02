//! The tail of the selected bead's pane, shown beneath the forest.

use crate::collect::panes::Panes;
use crate::collect::run::RunFailure;
use crate::model::join::{AgentRef, BeadKey, Conflict};
use crate::model::snapshot::{HerdrState, Snapshot};
use crate::view::forest::Forest;
use crate::view::lines::{Content, Item};
use crate::view::phrase;

/// How many lines of the pane the tail shows. The band reserved for it is
/// this plus the rule that names the pane.
pub const LINES: u16 = 6;

/// What a pane has most recently written, or why there is nothing to show.
///
/// The three are one type because the band under the forest is reserved
/// whatever is happening: there is always something to draw there, and a band
/// left blank would read as a pane sitting quiet rather than as no pane at
/// all. `Reading` is the band between a selection landing on a pane and herdr
/// saying what is on it — a moment on a healthy machine, and for as long as
/// it takes on one where herdr is slow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tail {
    Pane { pane: String, lines: Vec<String> },
    Reading { pane: String },
    Silent(&'static str),
}

/// What the selection points the tail at.
///
/// `Pane` is the pane itself rather than the agent holding it, because a pane
/// in one of the groups below the forest has no agent to hold it: it is live
/// and named, and nothing joined it to a bead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target<'a> {
    Pane(&'a str),
    /// A bead nobody is working.
    NoAgent,
    /// A tree's own line, one of the groups below the forest, or something in
    /// a group that names no pane.
    NotABead,
}

impl Target<'_> {
    pub fn pane(&self) -> Option<&str> {
        match self {
            Target::Pane(pane) => Some(pane),
            Target::NoAgent | Target::NotABead => None,
        }
    }
}

/// The pane the selection points at, where it points at one.
///
/// Two roads reach a pane. A bead's row names one only by way of the join, so
/// it is looked up from the key; a line in one of the groups below the forest
/// carries the pane already, because the thing it stands for is the pane. A
/// root is a bead row and takes the first road with every other bead — a
/// project's own line takes neither, because a project is not a bead.
pub fn target(forest: &Forest) -> Target<'_> {
    let Some(line) = forest.lines().get(forest.selected_line()) else {
        return Target::NotABead;
    };

    match &line.content {
        Content::Bead(_) => line
            .bead()
            .and_then(|key| agent(forest.snapshot(), key))
            .map_or(Target::NoAgent, |agent| Target::Pane(&agent.pane)),
        Content::Item(item) => named_pane(item).map_or(Target::NotABead, Target::Pane),
        Content::Project(_)
        | Content::Unread(_)
        | Content::Elided { .. }
        | Content::Note(_)
        | Content::Group(_)
        | Content::Scoped { .. } => Target::NotABead,
    }
}

/// The pane one of the groups' entries names, where it names exactly one.
///
/// A loose pane and an unconfigured one are panes; that is the whole of what
/// they are. A conflict is not, but two of its four shapes turn on a single
/// pane and name it. The other two name two panes and several, so there is
/// nothing to pick rather than nothing to show.
fn named_pane(item: &Item) -> Option<&str> {
    match item {
        Item::Loose(loose) => Some(&loose.pane),
        Item::Unconfigured(unconfigured) => Some(&unconfigured.pane),
        Item::Conflict(
            Conflict::SeveralBeadsNameOnePane { pane, .. }
            | Conflict::PaneInAnotherProject { pane, .. },
        ) => Some(pane),
        Item::Conflict(
            Conflict::BeadAndPaneDisagree { .. } | Conflict::SeveralPanesNameOneBead { .. },
        )
        | Item::Failed(_)
        | Item::Hidden(_) => None,
    }
}

/// One bead's agent, found the only way a bead can be found across trackers.
fn agent<'a>(snapshot: &'a Snapshot, key: &BeadKey) -> Option<&'a AgentRef> {
    snapshot.node(key)?.agent.as_ref()
}

/// The tail for whatever the selection points at, before herdr has said
/// anything about it.
///
/// Every way this can come back with no pane says so in `bdi`'s own words.
/// The order matters: with no herdr there is no pane on any row, so that is
/// answered before the row is looked at. Where there is a pane, what is left
/// is the reading, and that is the caller's to ask for.
pub fn tail(forest: &Forest) -> Tail {
    if forest.snapshot().herdr == HerdrState::Unavailable {
        return Tail::Silent(phrase::no_herdr_to_tail());
    }

    match target(forest) {
        Target::NotABead => Tail::Silent(phrase::no_bead_to_tail()),
        Target::NoAgent => Tail::Silent(phrase::no_agent_to_tail()),
        Target::Pane(pane) => Tail::Reading {
            pane: pane.to_string(),
        },
    }
}

/// The tail for what herdr said about a pane it was asked to read.
pub fn read(pane: String, read: Result<Vec<String>, RunFailure>) -> Tail {
    match read {
        Ok(lines) => Tail::Pane { pane, lines },
        Err(failure) => Tail::Silent(phrase::pane_unreadable(failure.kind)),
    }
}

/// Whether the tail must be read again for what the selection is on now.
///
/// A pane's rows are the one part of the tail worth keeping, because they are
/// the one part that costs a herdr call. Everything else the tail can say it
/// works out from the selected row alone, so a row that names no pane is
/// always read again and the reading is free.
///
/// So the tail stands only while the selection still names the pane it was
/// read from: scrolling within that pane's rows leaves it where it is, and
/// its rows are re-read on the refresh tick like everything else.
pub fn moved_on(forest: &Forest, showing: Option<&str>) -> bool {
    match target(forest).pane() {
        Some(pane) => showing != Some(pane),
        None => true,
    }
}

/// Ask for the pane the selection points at to be brought to the front,
/// saying nothing where it points at none.
///
/// Most rows carry no pane, so `⏎` on one is not a mistake and there is
/// nothing to report: there is simply nothing to focus. A pane that is named
/// and will not come is a different thing, and says so where the tail is when
/// herdr gets round to answering.
pub fn focus(forest: &Forest, panes: &dyn Panes) {
    if let Some(pane) = target(forest).pane() {
        panes.focus(pane);
    }
}

/// The tail for what herdr said about a pane it was asked to focus, where
/// that is worth saying at all. A focus that worked is its own report — the
/// pane is in front of the reader — so only a refusal reaches the band.
pub fn focused(focused: Result<(), RunFailure>) -> Option<Tail> {
    match focused {
        Ok(()) => None,
        Err(failure) => Some(Tail::Silent(phrase::pane_unreadable(failure.kind))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::run::{FailureKind, RunFailure};
    use crate::config::Scope;
    use crate::model::join::JoinSource;
    use crate::model::snapshot::{
        Counts, FailedProject, Filter, LoosePane, Node, TrackerFailure, TrackerState, Tree,
        UnconfiguredPane,
    };
    use crate::model::tree::Link;
    use crate::model::types::{Edge, PaneStatus, Status};
    use crate::view::forest;
    use crate::view::lines::GroupKind;
    use crate::view::walk::{self, Rows};
    use crate::view::{Action, Motion};
    use chrono::Utc;
    use pretty_assertions::assert_eq;
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    /// A herdr that remembers what it was asked. It answers nothing, because
    /// nothing here waits for an answer: what herdr said arrives later, and
    /// [`read`] and [`focused`] are what turn it into a band.
    #[derive(Default)]
    struct Fake {
        asked: RefCell<Vec<String>>,
        focused: RefCell<Vec<String>>,
    }

    impl Panes for Fake {
        fn read(&self, pane: &str, lines: u16) {
            self.asked.borrow_mut().push(format!("{pane} {lines}"));
        }

        fn focus(&self, pane: &str) {
            self.focused.borrow_mut().push(pane.to_string());
        }
    }

    fn failure(kind: FailureKind) -> RunFailure {
        RunFailure {
            kind,
            program: "herdr".to_string(),
            detail: "the test said so".to_string(),
        }
    }

    fn agent_on(pane: &str) -> AgentRef {
        AgentRef {
            pane: pane.to_string(),
            pane_status: PaneStatus::Working,
            title: None,
            source: JoinSource::AgentPane,
        }
    }

    fn node(id: &str, agent: Option<AgentRef>) -> Node {
        Node {
            id: id.to_string(),
            title: "a bead in the tree".to_string(),
            status: Status::InProgress,
            issue_type: "task".to_string(),
            priority: 2,
            ready: true,
            blocked_by: Vec::new(),
            started_at: None,
            closed_at: None,
            badges: Vec::new(),
            agent,
            anomalies: Vec::new(),
        }
    }

    /// One tree: a root, two beads one agent is on, and two beads nobody is
    /// on. The pairs are what tell a tail that stands apart from one that
    /// must be read again.
    fn snapshot(herdr: HerdrState) -> Snapshot {
        let tree = Arc::new(Tree {
            project: "orbital".to_string(),
            root: "orb-7".to_string(),
            title: "lift the ground station".to_string(),
            counts: Counts {
                total: 5,
                closed: 0,
                live_agents: 2,
                anomalies: 0,
            },
            tracker: TrackerState::Ok,
            beads: vec![
                node("orb-7", None),
                node("orb-7.1", Some(agent_on("w:p1"))),
                node("orb-7.2", None),
                node("orb-7.3", None),
                node("orb-7.4", Some(agent_on("w:p1"))),
            ],
            children: vec![
                (1..5)
                    .map(|bead| Link {
                        bead,
                        edge: Edge::ParentChild,
                        first: true,
                    })
                    .collect(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ],
            dangling: Vec::new(),
            cycles: Vec::new(),
        });

        Snapshot {
            generated_at: Utc::now(),
            herdr,
            filter: Filter::All,
            trees: vec![Arc::clone(&tree)],
            hidden_trees: Vec::new(),
            failed_projects: Vec::new(),
            unattributed: Vec::new(),
            unconfigured: Vec::new(),
            conflicts: Vec::new(),
            read_at: BTreeMap::new(),
            collected: vec![tree],
            projects: vec!["orbital".to_string()],
            scope: Scope::default(),
        }
    }

    /// The forest with the selection moved down `steps` rows from the header
    /// it starts on.
    /// Steps down from where the forest opens, which is its first root.
    fn selecting(steps: usize, herdr: HerdrState) -> Forest {
        let mut forest = forest::flatten(snapshot(herdr));
        for _ in 0..steps {
            forest.apply(Action::Move(Motion::NextRow));
        }
        forest
    }

    /// The one line above the first root: its project.
    fn on_the_project(herdr: HerdrState) -> Forest {
        let mut forest = forest::flatten(snapshot(herdr));
        forest.apply(Action::Move(Motion::FirstRow));
        forest
    }

    /// The band names the pane it is waiting on the moment the selection
    /// lands, which is what it has to draw while herdr is still answering.
    #[test]
    fn the_tail_follows_the_selection() {
        assert_eq!(
            tail(&selecting(1, HerdrState::Ok)),
            Tail::Reading {
                pane: "w:p1".to_string()
            }
        );
        assert_eq!(
            tail(&selecting(2, HerdrState::Ok)),
            Tail::Silent(phrase::no_agent_to_tail()),
            "the row below is a bead nobody is working"
        );
    }

    #[test]
    fn what_herdr_read_is_the_band_under_the_forest() {
        assert_eq!(
            read(
                "w:p1".to_string(),
                Ok(vec!["rebuilt .#thinkpad, generation 541".to_string()])
            ),
            Tail::Pane {
                pane: "w:p1".to_string(),
                lines: vec!["rebuilt .#thinkpad, generation 541".to_string()],
            }
        );
    }

    /// A project is not a bead, so its line names no pane. Its roots do, and
    /// each says so on its own row.
    #[test]
    fn a_project_line_has_no_pane_to_tail() {
        assert_eq!(
            tail(&on_the_project(HerdrState::Ok)),
            Tail::Silent(phrase::no_bead_to_tail())
        );
    }

    #[test]
    fn no_herdr_means_no_pane_to_read() {
        assert_eq!(
            tail(&selecting(1, HerdrState::Unavailable)),
            Tail::Silent(phrase::no_herdr_to_tail()),
            "with no herdr there is no pane on any row, whatever the row says"
        );
    }

    /// A pane that went away between one poll and the next. The band says so
    /// rather than emptying: an empty band reads as a pane with nothing to
    /// say.
    #[test]
    fn a_pane_that_has_gone_degrades_to_a_phrase() {
        assert_eq!(
            read("w:p1".to_string(), Err(failure(FailureKind::Gone))),
            Tail::Silent(phrase::pane_unreadable(FailureKind::Gone))
        );
    }

    #[test]
    fn every_way_a_read_can_fail_is_said_rather_than_drawn_blank() {
        for kind in [
            FailureKind::Auth,
            FailureKind::Unavailable,
            FailureKind::Gone,
            FailureKind::Busy,
            FailureKind::Exec,
            FailureKind::Parse,
        ] {
            assert_eq!(
                read("w:p1".to_string(), Err(failure(kind))),
                Tail::Silent(phrase::pane_unreadable(kind)),
                "for {kind:?}"
            );
        }
    }

    #[test]
    fn enter_focuses_the_pane_the_selection_is_on() {
        let panes = Fake::default();

        focus(&selecting(1, HerdrState::Ok), &panes);

        assert_eq!(*panes.focused.borrow(), ["w:p1"]);
    }

    /// Most rows carry no pane and the mock never shows one on `⏎`. There is
    /// nothing to focus and nothing has gone wrong.
    #[test]
    fn enter_on_a_row_with_no_pane_is_a_no_op_and_not_an_error() {
        let panes = Fake::default();

        for steps in [0, 2] {
            focus(&selecting(steps, HerdrState::Ok), &panes);
        }

        assert!(panes.focused.borrow().is_empty());
    }

    /// A focus that worked is its own report — the pane is in front of the
    /// reader — so only a refusal reaches the band.
    #[test]
    fn a_pane_that_will_not_come_to_the_front_says_so_where_the_tail_is() {
        assert_eq!(focused(Ok(())), None);
        assert_eq!(
            focused(Err(failure(FailureKind::Gone))),
            Some(Tail::Silent(phrase::pane_unreadable(FailureKind::Gone)))
        );
    }

    #[test]
    fn the_tail_is_read_again_only_where_the_selection_has_left_the_pane() {
        assert!(
            !moved_on(&selecting(1, HerdrState::Ok), Some("w:p1")),
            "the selection is still on the pane the tail is showing"
        );
        assert!(moved_on(&selecting(2, HerdrState::Ok), Some("w:p1")));
        assert!(moved_on(&selecting(1, HerdrState::Ok), None));
    }

    /// A header and a bead nobody is working name no pane between them, and
    /// say different things. The tail read for the one is the wrong tail for
    /// the other.
    #[test]
    fn leaving_a_header_for_a_bead_nobody_is_working_is_a_move() {
        assert!(moved_on(&selecting(2, HerdrState::Ok), None));
    }

    #[test]
    fn moving_between_two_beads_nobody_is_working_is_a_move() {
        assert!(moved_on(&selecting(3, HerdrState::Ok), None));
    }

    /// One agent can be on more than one bead, and the pane its rows share is
    /// already on screen. This is what the comparison exists for.
    #[test]
    fn two_beads_on_one_pane_do_not_read_it_twice() {
        assert!(!moved_on(&selecting(4, HerdrState::Ok), Some("w:p1")));
    }

    /// The property stated rather than sampled: wherever `moved_on` says the
    /// tail stands, the tail the new row calls for *is* the tail on screen.
    /// So no phrase can outlive the row it was said for.
    #[test]
    fn a_tail_that_stands_is_the_tail_the_new_row_calls_for() {
        for from in 0..5 {
            let was = selecting(from, HerdrState::Ok);
            let showing = target(&was).pane().map(str::to_string);
            let on_screen = tail(&was);

            for onto in 0..5 {
                let now = selecting(onto, HerdrState::Ok);
                if !moved_on(&now, showing.as_deref()) {
                    assert_eq!(
                        tail(&now),
                        on_screen,
                        "the tail read on row {from} was left standing on row {onto}"
                    );
                }
            }
        }
    }

    fn key(id: &str) -> BeadKey {
        BeadKey {
            project: "orbital".to_string(),
            id: id.to_string(),
        }
    }

    fn loose() -> LoosePane {
        LoosePane {
            pane: "w:p2".to_string(),
            project: "orbital".to_string(),
            cwd: "/tmp/bdi-ground/orbital".to_string(),
            pane_status: PaneStatus::Working,
        }
    }

    /// A second pane in the same group, so that a group can come back from a
    /// refresh in a different order than it went in.
    fn another_loose() -> LoosePane {
        LoosePane {
            pane: "w:p10".to_string(),
            project: "orbital".to_string(),
            cwd: "/tmp/bdi-ground/orbital".to_string(),
            pane_status: PaneStatus::Idle,
        }
    }

    fn unconfigured() -> UnconfiguredPane {
        UnconfiguredPane {
            pane: "w:p3".to_string(),
            cwd: "/tmp/bdi-ground/lander".to_string(),
            pane_status: PaneStatus::Working,
        }
    }

    fn pane_in_another_project() -> Conflict {
        Conflict::PaneInAnotherProject {
            bead: key("orb-7.2"),
            pane: "w:p4".to_string(),
            pane_project: None,
        }
    }

    fn several_beads_name_one_pane() -> Conflict {
        Conflict::SeveralBeadsNameOnePane {
            pane: "w:p5".to_string(),
            caption: None,
            beads: vec![key("orb-7.2"), key("orb-7.3")],
        }
    }

    fn bead_and_pane_disagree() -> Conflict {
        Conflict::BeadAndPaneDisagree {
            bead: key("orb-7.2"),
            named_by_bead: "w:p6".to_string(),
            named_by_pane: "w:p7".to_string(),
        }
    }

    fn several_panes_name_one_bead() -> Conflict {
        Conflict::SeveralPanesNameOneBead {
            bead: key("orb-7.3"),
            panes: vec!["w:p8".to_string(), "w:p9".to_string()],
        }
    }

    fn failed_project() -> FailedProject {
        FailedProject {
            project: "lander".to_string(),
            tracker: TrackerFailure::Unavailable,
        }
    }

    /// The same tree, and beneath it the groups: three live panes no bead
    /// claims, all four shapes of conflict, and a project whose tracker never
    /// answered. Five of those lines turn on one pane and four turn on none,
    /// which is what tells a row the tail can follow from a row it cannot.
    fn snapshot_with_groups(herdr: HerdrState) -> Snapshot {
        Snapshot {
            failed_projects: vec![failed_project()],
            unattributed: vec![loose(), another_loose()],
            unconfigured: vec![unconfigured()],
            conflicts: vec![
                pane_in_another_project(),
                several_beads_name_one_pane(),
                bead_and_pane_disagree(),
                several_panes_name_one_bead(),
            ],
            ..snapshot(herdr)
        }
    }

    /// The kind of the first group still drawn shut, where one is.
    fn shut_group(forest: &Forest) -> Option<GroupKind> {
        forest.lines().iter().find_map(|line| match &line.content {
            Content::Group(group) if line.folded == Some(false) => Some(group.kind),
            _ => None,
        })
    }

    /// That forest with every group open, as a reader who pressed the key on
    /// each of them in turn would have it.
    ///
    /// The walk's bound holds for every snapshot rather than for this
    /// fixture, and it rests on `layout::draw_groups`: it pushes a group's
    /// line before its `if !open { continue; }`, so a shut group is already a
    /// drawn line, and it puts a group's items in as `Content::Item`, so
    /// opening one can never produce another group to open.
    fn with_groups_open(herdr: HerdrState) -> Forest {
        let mut forest = forest::flatten(snapshot_with_groups(herdr));
        walk::until(
            &mut forest,
            |forest| shut_group(forest).is_none(),
            |forest| {
                let shut = shut_group(forest).expect("the walk stops when none is shut");
                step_onto(
                    forest,
                    |content| matches!(content, Content::Group(group) if group.kind == shut),
                );
                forest.apply(Action::ExpandOrChild);
            },
            |_| "a group was still shut after the key was pressed on it".to_string(),
        );
        forest
    }

    /// Move the selection down to the row `wanted` picks out, by pressing
    /// down until it is there.
    ///
    /// A row is named by what is on it rather than by where it sits, because
    /// where it sits moves: a tree folds shut as the selection leaves it, and
    /// every line below it shifts up under a test still holding the old
    /// number.
    fn step_onto(forest: &mut Forest, wanted: impl Fn(&Content) -> bool) {
        forest.apply(Action::Move(Motion::FirstRow));
        walk::until(
            forest,
            |forest| wanted(&forest.lines()[forest.selected_line()].content),
            |forest| {
                forest.apply(Action::Move(Motion::NextRow));
            },
            |_| "pressing down from the top never reached the row".to_string(),
        );
    }

    fn onto(wanted: &Item) -> impl Fn(&Content) -> bool + use<'_> {
        move |content| matches!(content, Content::Item(item) if item == wanted)
    }

    /// The forest with the selection `steps` rows below the top, every group
    /// open and the row reached by pressing down.
    fn stepping(steps: usize, herdr: HerdrState) -> Forest {
        let mut forest = with_groups_open(herdr);
        forest.apply(Action::Move(Motion::FirstRow));
        for _ in 0..steps {
            forest.apply(Action::Move(Motion::NextRow));
        }
        forest
    }

    /// How many rows pressing down from the top reaches, which is every row
    /// drawn or the walk below says which one it stopped at.
    fn rows(herdr: HerdrState) -> usize {
        let mut forest = with_groups_open(herdr);
        forest.apply(Action::Move(Motion::FirstRow));
        to_the_last_row(&mut forest);
        forest.rows()
    }

    /// Press down until the selection is on the last row drawn.
    fn to_the_last_row(forest: &mut Forest) {
        walk::until(
            forest,
            |forest| forest.selected_line() + 1 == forest.rows(),
            |forest| {
                forest.apply(Action::Move(Motion::NextRow));
            },
            |forest| {
                format!(
                    "pressing down from the top stopped at row {} of {}",
                    forest.selected_line(),
                    forest.rows()
                )
            },
        );
    }

    /// One line of every kind that names a pane no bead holds. Each is a live
    /// agent doing work, and the one thing `bdi` would not do was show you
    /// what it was doing.
    fn pane_bearing() -> [(Item, &'static str); 4] {
        [
            (Item::Loose(loose()), "w:p2"),
            (Item::Unconfigured(unconfigured()), "w:p3"),
            (Item::Conflict(pane_in_another_project()), "w:p4"),
            (Item::Conflict(several_beads_name_one_pane()), "w:p5"),
        ]
    }

    #[test]
    fn a_pane_no_bead_claims_can_be_tailed() {
        let mut forest = with_groups_open(HerdrState::Ok);

        for (item, pane) in pane_bearing() {
            step_onto(&mut forest, onto(&item));

            assert_eq!(
                tail(&forest),
                Tail::Reading {
                    pane: pane.to_string()
                },
                "on {item:?}"
            );
        }
    }

    #[test]
    fn enter_focuses_a_pane_no_bead_claims() {
        let panes = Fake::default();
        let mut forest = with_groups_open(HerdrState::Ok);

        for (item, _) in pane_bearing() {
            step_onto(&mut forest, onto(&item));

            focus(&forest, &panes);
        }
        assert_eq!(*panes.focused.borrow(), ["w:p2", "w:p3", "w:p4", "w:p5"]);
    }

    /// A conflict can be tailed exactly where it turns on one pane. Two of the
    /// four do not: one names the two panes that disagree and the other names
    /// every pane that claimed the bead, so there is nothing to pick rather
    /// than nothing to show. A tracker that could not be read names no pane at
    /// all.
    #[test]
    fn a_line_in_a_group_that_turns_on_no_one_pane_is_unchanged() {
        let panes = Fake::default();
        let mut forest = with_groups_open(HerdrState::Ok);

        for item in [
            Item::Conflict(bead_and_pane_disagree()),
            Item::Conflict(several_panes_name_one_bead()),
            Item::Failed(failed_project()),
        ] {
            step_onto(&mut forest, onto(&item));

            assert_eq!(
                tail(&forest),
                Tail::Silent(phrase::no_bead_to_tail()),
                "on {item:?}"
            );
            focus(&forest, &panes);
        }
        assert!(panes.focused.borrow().is_empty());
    }

    /// A group stands for everything under it, panes included, and names no
    /// one of them. Opening it is how a reader gets to a pane; the group's own
    /// line is not one.
    #[test]
    fn a_groups_own_line_names_no_pane() {
        let panes = Fake::default();
        let mut forest = with_groups_open(HerdrState::Ok);
        let kinds: Vec<GroupKind> = forest
            .lines()
            .iter()
            .filter_map(|line| match &line.content {
                Content::Group(group) => Some(group.kind),
                _ => None,
            })
            .collect();

        assert_eq!(kinds.len(), 4, "the fixture fills four of the five groups");
        for kind in kinds {
            step_onto(
                &mut forest,
                |content| matches!(content, Content::Group(group) if group.kind == kind),
            );

            assert_eq!(
                tail(&forest),
                Tail::Silent(phrase::no_bead_to_tail()),
                "on the {kind:?} group"
            );
            focus(&forest, &panes);
        }
        assert!(panes.focused.borrow().is_empty());
    }

    /// The sequence a reader drives, rather than a state set by hand: onto a
    /// pane no bead claims, its rows read, off it, and back onto it. The tail
    /// stands while the selection is still on the pane and is read again once
    /// it has left, which for a loose pane is the rule it already was for a
    /// bead's.
    #[test]
    fn the_tail_follows_the_selection_onto_a_loose_pane_and_off_it() {
        let mut forest = with_groups_open(HerdrState::Ok);
        let pane = Item::Loose(loose());

        step_onto(&mut forest, onto(&pane));
        assert!(
            moved_on(&forest, None),
            "arriving on the pane, nothing on screen was read for it"
        );
        let on_arrival = tail(&forest);
        assert_eq!(
            on_arrival,
            Tail::Reading {
                pane: "w:p2".to_string()
            }
        );
        assert!(
            !moved_on(&forest, Some("w:p2")),
            "the selection has not left the pane, so its rows stand"
        );

        forest.apply(Action::Move(Motion::PreviousRow));
        assert!(
            moved_on(&forest, Some("w:p2")),
            "the row above is the group's own line and names no pane"
        );

        step_onto(&mut forest, onto(&pane));
        assert_eq!(tail(&forest), on_arrival);
    }

    /// The property restated over a forest with panes no bead claims in it.
    /// `moved_on` is where widening `target` could quietly cost a herdr call
    /// on every keypress, or leave a phrase standing over a row that calls for
    /// a pane. Over every pair of rows rather than a sample, neither can
    /// happen unseen.
    #[test]
    fn a_tail_that_stands_holds_over_the_groups_too() {
        let rows = rows(HerdrState::Ok);

        for from in 0..rows {
            let was = stepping(from, HerdrState::Ok);
            let showing = target(&was).pane().map(str::to_string);
            let on_screen = tail(&was);

            for onto in 0..rows {
                let now = stepping(onto, HerdrState::Ok);
                if !moved_on(&now, showing.as_deref()) {
                    assert_eq!(
                        tail(&now),
                        on_screen,
                        "the tail read on row {from} was left standing on row {onto}"
                    );
                }
            }
        }
    }

    /// With no herdr there is no pane on any row, and a row that names one in
    /// its own text is no exception.
    #[test]
    fn no_herdr_means_no_pane_on_a_loose_row_either() {
        let mut forest = with_groups_open(HerdrState::Unavailable);
        step_onto(&mut forest, onto(&Item::Loose(loose())));

        assert_eq!(tail(&forest), Tail::Silent(phrase::no_herdr_to_tail()));
    }

    /// A refresh that reorders a group around the selection leaves the
    /// selection on the same *pane*, not merely on some pane.
    ///
    /// `moved_on` compares pane ids, so a selection that slid onto a
    /// neighbouring pane on a refresh tick would re-read the tail and look,
    /// from outside, like a tail refusing to stand — a defect in this file
    /// caused by what the forest identifies an item by. Neither the row a
    /// refresh holds nor the tail that stands over it says this on its own.
    #[test]
    fn a_refresh_that_reorders_a_group_leaves_the_selection_on_the_same_pane() {
        let mut forest = with_groups_open(HerdrState::Ok);
        step_onto(&mut forest, onto(&Item::Loose(loose())));
        assert_eq!(target(&forest).pane(), Some("w:p2"));

        let mut reordered = snapshot_with_groups(HerdrState::Ok);
        reordered.unattributed.reverse();
        reordered.conflicts.reverse();
        forest.refresh(reordered);

        assert_eq!(
            target(&forest).pane(),
            Some("w:p2"),
            "the group came back in a different order and took the selection with it"
        );
        assert!(
            !moved_on(&forest, Some("w:p2")),
            "the pane selected is the pane on screen, so its rows stand"
        );
    }
}
