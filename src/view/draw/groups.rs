//! The groups drawn in the forest, and the things inside them.

use ratatui::text::Span;

use crate::model::types::PaneStatus;
use crate::view::fitted::{Fitted, GAP};
use crate::view::lines::{Group, GroupKind, Item};
use crate::view::palette;
use crate::view::phrase;
use crate::view::row::{AGENT, WARNING};

use super::{pane_marker, sentence};

/// What lifts the live-agent filter, said beside the trees it is holding back.
const SHOW_ALL: &str = "a to show all";

/// What brings back the roots that rooting the forest at one bead put behind
/// a line.
const WHOLE_FOREST: &str = "F for the whole forest";

/// The line a group is drawn as. Two of them hold roots nothing went wrong in
/// — a key put them there and a key takes them back out — so those two are
/// the ones drawn without a warning.
pub(super) fn group_line(prefix: &str, group: &Group) -> Fitted {
    let (said, hidden) = match group.kind {
        GroupKind::FailedProjects => (phrase::failed_projects(group.count), false),
        GroupKind::Conflicts => (phrase::conflicts(group.count), false),
        GroupKind::HiddenTrees => (phrase::hidden_trees(group.count, group.with_findings), true),
        GroupKind::HeldBack => (phrase::held_back(group.count), true),
        GroupKind::Unattributed => (phrase::unattributed(group.count), false),
        GroupKind::Unconfigured => (phrase::unconfigured(group.count), false),
    };

    let (said, style) = if hidden {
        (said, palette::PLAIN)
    } else {
        (format!("{WARNING} {said}"), palette::ATTENTION)
    };
    let state = match group.kind {
        GroupKind::HiddenTrees => vec![Span::styled(SHOW_ALL, palette::QUIET)],
        GroupKind::HeldBack => held_back_state(group),
        _ => Vec::new(),
    };

    let line = Fitted::new(
        vec![Span::raw(prefix.to_string()), Span::styled(said, style)],
        Vec::new(),
        state,
    );
    match group.kind {
        GroupKind::HeldBack => line.briefly(held_back_counts(group)),
        _ => line,
    }
}

/// What a line standing over the roots the mode is holding back says beside
/// itself: the seats in there, the beads wanting looking at, and the key that
/// brings them back.
///
/// The reader asked for one bead, so the roots go; what they did not ask for
/// was to be told there is nobody on them. Said in the words a line resting
/// shut over the same things already uses.
fn held_back_state(group: &Group) -> Vec<Span<'static>> {
    let mut said = held_back_counts(group);
    said.push(Span::styled(WHOLE_FOREST, palette::QUIET));
    said
}

/// The same, where the row cannot afford the key as well.
///
/// The hint goes first and goes whole, because `F for th…` names no key while
/// the counts are the part of this line a reader can read nowhere else.
fn held_back_counts(group: &Group) -> Vec<Span<'static>> {
    let mut said = Vec::new();
    let Some(counts) = &group.held else {
        return said;
    };
    if counts.live_agents > 0 {
        said.push(Span::styled(
            format!("{AGENT} {}", phrase::agents_beneath(counts.live_agents)),
            palette::AGENT,
        ));
    }
    if counts.anomalies > 0 {
        said.push(Span::styled(
            format!("{WARNING} {}", phrase::anomalies_beneath(counts.anomalies)),
            palette::ATTENTION,
        ));
    }
    said
}

/// The scope, where the directory chose it. Nothing went wrong, so it is
/// drawn as the hidden trees are: no warning, and the way to the rest where
/// they keep theirs.
pub(super) fn scoped_line(prefix: &str, project: &str) -> Fitted {
    Fitted::new(
        vec![
            Span::raw(prefix.to_string()),
            Span::raw(phrase::scoped_by_the_directory(project)),
        ],
        Vec::new(),
        vec![Span::styled(
            phrase::all_projects_reads_the_rest(),
            palette::QUIET,
        )],
    )
}

/// One thing inside such a group.
pub(super) fn item_line(prefix: &str, item: &Item) -> Fitted {
    match item {
        Item::Failed(failed) => {
            sentence(prefix, phrase::failed_project(failed), palette::ATTENTION)
        }
        Item::Conflict(conflict) => {
            sentence(prefix, phrase::conflict(conflict), palette::ATTENTION)
        }
        Item::Loose(pane) => loose_line(
            prefix,
            &pane.pane.id,
            &pane.pane_status,
            phrase::pane_report(pane.display_agent.as_deref(), pane.title.as_deref()),
            &pane.cwd,
            pane.claim_refused,
        ),
        Item::Unconfigured(pane) => loose_line(
            prefix,
            &pane.pane.id,
            &pane.pane_status,
            None,
            &pane.cwd,
            false,
        ),
    }
}

/// A live pane in one of the groups: which pane it is, what it reported about
/// itself, and the directory it is working in. The directory is what both
/// groups are asking the reader to look at — one to place the agent, the
/// other to configure the project — and the report is what places the agent
/// without leaving the row, so it comes first and the directory is what a
/// narrow row gives up before it.
///
/// A refused claim is the state rather than the title, so it outlives both on
/// a narrow row. Placing a seat is what the directory is for, and a reader
/// who has taken this pane for one nobody claimed is not placing a seat — the
/// sentence that says so has to reach them first.
fn loose_line(
    prefix: &str,
    pane: &str,
    status: &PaneStatus,
    report: Option<String>,
    cwd: &str,
    claim_refused: bool,
) -> Fitted {
    let mut title = Vec::new();
    if let Some(report) = report {
        title.push(Span::raw(report));
        title.push(Span::raw(" ".repeat(GAP)));
    }
    title.push(Span::raw(cwd.to_string()));

    let state = if claim_refused {
        vec![Span::styled(phrase::claim_refused(), palette::ATTENTION)]
    } else {
        Vec::new()
    };

    Fitted::new(
        vec![Span::styled(
            format!("{prefix}{}", pane_marker(pane, status)),
            palette::AGENT,
        )],
        title,
        state,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::herdr::parse_agent_list;
    use crate::config::Config;
    use crate::model::join::{self, Listed};
    use crate::model::snapshot::{a_provider, build, Collected, LoosePane, UnconfiguredPane};
    use crate::model::types::testing::{key as pane_key, A_SESSION};
    use crate::model::types::PaneStatus;
    use crate::view::fitted::CUT;
    use chrono::{TimeZone, Utc};
    use pretty_assertions::assert_eq;
    use ratatui::style::Color;

    use crate::model::anomaly::Anomaly;
    use crate::model::snapshot::{
        Counts, FailedProject, Filter, HiddenTree, ProviderState, TrackerFailure,
    };
    use crate::model::types::Status;
    use crate::view::draw::tests::*;
    use crate::view::forest::flatten;

    /// Every kind of group there is, whichever side of the trees it is drawn
    /// on: a group's line says the same thing wherever it hangs.
    fn every_kind() -> impl Iterator<Item = GroupKind> {
        GroupKind::BELOW_THE_TREES
            .into_iter()
            .chain(GroupKind::UNDER_A_PROJECT)
    }

    /// The filter hides trees, and it takes their findings with them. Saying
    /// only how many trees are hidden would read as "nothing to see here"
    /// while some of them are broken.
    #[test]
    fn the_hidden_trees_group_admits_that_what_it_hides_is_not_empty() {
        let quiet = Group {
            kind: GroupKind::HiddenTrees,
            project: Some("summit-works".into()),
            count: 4,
            with_findings: 0,
            held: None,
        };
        let broken = Group {
            with_findings: 2,
            ..quiet.clone()
        };

        assert_eq!(
            Painted::of(group_line(SHUT, &quiet), 64, 1).rows(),
            vec!["▸ 4 trees with no live agent                       a to show all"]
        );
        assert_eq!(
            Painted::of(group_line(SHUT, &broken), 64, 1).rows(),
            vec!["▸ 4 trees with no live agent · 2 with findings     a to show all"]
        );
    }

    /// A claim with no pane behind it is a finding, and a tree hidden with one
    /// in it has taken that finding off the screen. No filter today hides such
    /// a tree, so it is hidden here the way the filter hides one: the line
    /// under test is the one that counts, not the one that chooses.
    #[test]
    fn a_hidden_tree_whose_only_finding_is_an_anomaly_is_said_to_have_one() {
        let mut claimed = node("smt-4kd3p.1", "seat the guy wires", Status::InProgress);
        claimed.anomalies = vec![Anomaly::OrphanClaim { refused: None }];
        let beads = vec![node("smt-4kd3p", "raise the mast", Status::Open), claimed];
        let mut hidden = tree(
            "summit-works",
            "smt-4kd3p",
            "raise the mast",
            Counts::over(&beads),
        );
        hidden.children = under_the_root(&beads);
        hidden.beads = beads;

        let mut snapshot = snapshot(vec![hidden.clone()], Vec::new(), ProviderState::Answering);
        snapshot.filter = Filter::LiveAgents;
        snapshot.trees.clear();
        snapshot.hidden_trees = vec![HiddenTree::of(&hidden)];
        let frame = frame_of(&flatten(snapshot), 64, 3).rows();

        assert!(
            frame
                .iter()
                .any(|row| row.contains("1 tree with no live agent · 1 with findings")),
            "{frame:#?}"
        );
    }

    /// Nothing went wrong when the directory chose the scope, so the line
    /// saying so is drawn as the hidden trees are: no warning, and the way to
    /// the rest in the column the hidden trees keep theirs in.
    #[test]
    fn the_scope_the_directory_chose_is_said_with_the_way_to_the_rest() {
        assert_eq!(
            Painted::of(scoped_line("  ", "orbital"), 80, 1).rows(),
            vec![
                "  reading orbital, where bdi was started      --all-projects reads every project"
            ]
        );
    }

    /// The reader rooted the forest at one bead and everything else went
    /// behind this line, open work and the seats on it included. So the line
    /// says how many, rather than leaving a reader to read one number as the
    /// whole truth about what is back there.
    #[test]
    fn a_line_over_held_back_roots_says_its_seats_and_what_wants_looking_at() {
        let group = Group {
            kind: GroupKind::HeldBack,
            project: Some("summit-works".into()),
            count: 3,
            with_findings: 0,
            held: Some(Counts {
                total: 12,
                closed: 4,
                live_agents: 2,
                anomalies: 1,
            }),
        };

        let drawn = Painted::of(group_line(SHUT, &group), 120, 1).rows();

        assert!(drawn[0].contains("3 other roots"), "{drawn:?}");
        assert!(drawn[0].contains("2 agents beneath"), "{drawn:?}");
        assert!(drawn[0].contains("1 bead beneath"), "{drawn:?}");
        assert!(drawn[0].contains("F for the whole forest"), "{drawn:?}");
    }

    /// A row with no room for the way out says the counts without it. The
    /// counts are what a reader can read nowhere else on the screen, and a
    /// key named in part is a key they cannot press.
    #[test]
    fn a_row_too_narrow_for_the_way_out_still_says_what_is_back_there() {
        let group = Group {
            kind: GroupKind::HeldBack,
            project: Some("summit-works".into()),
            count: 3,
            with_findings: 0,
            held: Some(Counts {
                total: 12,
                closed: 4,
                live_agents: 2,
                anomalies: 1,
            }),
        };

        let drawn = Painted::of(group_line(SHUT, &group), 64, 1).rows();

        assert!(drawn[0].contains("3 other roots"), "{drawn:?}");
        assert!(drawn[0].contains("2 agents beneath"), "{drawn:?}");
        assert!(drawn[0].contains("1 bead beneath"), "{drawn:?}");
        assert!(!drawn[0].contains("F for"), "{drawn:?}");
        assert!(!drawn[0].contains(CUT), "{drawn:?}");
    }

    /// And says neither where there is neither. A nought said is a column
    /// spent telling a reader about nothing.
    #[test]
    fn a_line_over_quiet_held_back_roots_says_nothing_of_seats_at_all() {
        let group = Group {
            kind: GroupKind::HeldBack,
            project: Some("summit-works".into()),
            count: 1,
            with_findings: 0,
            held: Some(Counts {
                total: 9,
                closed: 9,
                live_agents: 0,
                anomalies: 0,
            }),
        };

        let drawn = Painted::of(group_line(SHUT, &group), 120, 1).rows();

        assert!(drawn[0].contains("1 other root"), "{drawn:?}");
        assert!(!drawn[0].contains("beneath"), "{drawn:?}");
        assert!(drawn[0].contains("F for the whole forest"), "{drawn:?}");
    }

    /// A group holding something that went wrong is marked as such. The two
    /// that hold whole roots are not: the reader asked for those roots to be
    /// out of the way, one with the filter and one with the key that roots the
    /// forest at a bead.
    #[test]
    fn only_a_group_holding_roots_the_reader_put_away_is_drawn_without_a_warning() {
        for kind in every_kind() {
            let group = Group {
                kind,
                project: None,
                count: 2,
                with_findings: 0,
                held: None,
            };
            let drawn = Painted::of(group_line(SHUT, &group), 80, 1).rows();
            let marked = drawn[0].contains(WARNING);

            assert_eq!(
                marked,
                !matches!(kind, GroupKind::HiddenTrees | GroupKind::HeldBack),
                "{kind:?}: {drawn:?}"
            );
        }
    }

    /// The failed and conflicted items in the bottom groups get their
    /// box-drawing the same way a bead does, so they are tree drawing too.
    #[test]
    fn a_failed_project_leaves_its_box_drawing_in_the_terminals_own_colour() {
        let failed = Item::Failed(FailedProject {
            project: "summit-works".into(),
            tracker: TrackerFailure::Auth,
        });
        let painted = Painted::of(item_line(LAST, &failed), 96, 1).row(0);

        assert_eq!(painted[0].said, LAST);
        assert_eq!(painted[0].style.fg, Some(Color::Reset));
        assert_eq!(painted[1].style.fg, palette::ATTENTION.fg);
    }

    /// The fold arrow is a control rather than a word, and every group has
    /// one. Drawn in the terminal's own foreground the column of arrows reads
    /// as the one control it is, whatever the group beside each says.
    #[test]
    fn a_groups_fold_arrow_is_drawn_in_the_terminals_own_colour() {
        for kind in every_kind() {
            let group = Group {
                kind,
                project: None,
                count: 2,
                with_findings: 0,
                held: None,
            };
            let painted = Painted::of(group_line(SHUT, &group), 80, 1).row(0);

            assert_eq!(
                painted[0].style.fg,
                Some(Color::Reset),
                "{kind:?}: {painted:?}"
            );
            assert!(painted[0].said.starts_with(SHUT), "{kind:?}: {painted:?}");
        }
    }

    // ---- what a loose pane says about itself -----------------------------

    /// The row of a pane no bead claims, drawn from what herdr puts on the
    /// wire: the capture, through the snapshot, to the frame, so the words
    /// on the row are the words herdr sent and not a fixture's paraphrase.
    /// Every pane in the capture whose directory is the project's is loose,
    /// because no tracker rows are read to claim one.
    fn frame_over_the_capture() -> Vec<String> {
        let cfg = Config::from_toml(
            r#"
[[projects]]
name = "orbital"
path = "/tmp/bdi-ground/orbital"
"#,
        )
        .expect("the config parses");
        let panes = parse_agent_list(
            A_SESSION,
            include_str!("../../../tests/fixtures/herdr_agent_list.json"),
        )
        .expect("the capture parses");
        let joined = join::resolve(&[], Listed::all(&panes), &cfg);
        let snapshot = build(
            Collected::default(),
            &panes,
            &joined,
            &cfg,
            a_provider(ProviderState::Answering),
            Filter::All,
            Utc.with_ymd_and_hms(2026, 8, 30, 10, 22, 14).unwrap(),
        );
        frame_of(&flatten(snapshot), 120, 24).rows()
    }

    fn row_naming(frame: &[String], pane: &str) -> String {
        frame
            .iter()
            .find(|row| row.contains(pane))
            .unwrap_or_else(|| panic!("{pane} is on no row of {frame:#?}"))
            .trim_end()
            .to_string()
    }

    /// `wG:p6` stamped a `display_agent`, a title and a label for each
    /// state, and is working: the row says who it says it is, then the label
    /// for the state it is in — the caption rule a bead's agent already gets
    /// — then where it is.
    #[test]
    fn an_unattributed_pane_says_what_herdr_reported_about_it() {
        let frame = frame_over_the_capture();

        assert_eq!(
            row_naming(&frame, "wG:p6"),
            "      ├── ◍ wG:p6 working  orb-2kd.5 · writing the parser and its tests  \
             /tmp/bdi-ground/orbital"
        );
    }

    /// `wG:p1` stamped nothing at all, and its row is the one it had.
    #[test]
    fn a_pane_that_reported_nothing_keeps_the_row_it_had() {
        let frame = frame_over_the_capture();

        assert_eq!(
            row_naming(&frame, "wG:p1"),
            "      ├── ◍ wG:p1 idle  /tmp/bdi-ground/orbital"
        );
    }

    fn reporting(display_agent: Option<&str>, title: Option<&str>) -> LoosePane {
        LoosePane {
            display_agent: display_agent.map(str::to_string),
            title: title.map(str::to_string),
            ..pane("w:p3", PaneStatus::Working)
        }
    }

    /// Either half of the report stands alone, with no separator left over
    /// for the half that is not there.
    #[test]
    fn half_a_report_is_said_without_its_separator() {
        let named = Item::Loose(reporting(Some("orch: core-json"), None));
        let captioned = Item::Loose(reporting(
            None,
            Some("parse bd dep-tree JSON into typed rows"),
        ));

        assert_eq!(
            Painted::of(item_line(LAST, &named), 96, 1).rows()[0].trim_end(),
            "  └── ◍ w:p3 working  orch: core-json  /tmp/bdi-ground/summit-works"
        );
        assert_eq!(
            Painted::of(item_line(LAST, &captioned), 96, 1).rows()[0].trim_end(),
            "  └── ◍ w:p3 working  parse bd dep-tree JSON into typed rows  /tmp/bdi-ground/summit-works"
        );
    }

    /// One row at the width the whole of a refused pane's takes, so the two
    /// rows that carry nothing extra are asserted whole rather than searched
    /// for the absence of something.
    fn said(loose: LoosePane) -> String {
        Painted::of(item_line(LAST, &Item::Loose(loose)), 84, 1).rows()[0]
            .trim_end()
            .to_string()
    }

    /// The three ways a pane comes to be unattributed, drawn side by side.
    ///
    /// Two of them `bdi` cannot tell apart and does not try to: a seat that
    /// finished and cleared its key, and one that has not registered yet,
    /// both leave a pane nothing claims. The third is their opposite — a
    /// claim `bdi` read and would not honour — and it is the one that says
    /// so.
    #[test]
    fn only_the_pane_whose_claim_was_refused_says_a_claim_was_refused() {
        assert_eq!(
            said(LoosePane {
                claim_refused: true,
                ..pane("w:p5", PaneStatus::Working)
            }),
            "  └── ◍ w:p5 working  /tmp/bdi-ground/summit-works  a claim on this pane was refused"
        );
        assert_eq!(
            said(pane("w:p4", PaneStatus::Idle)),
            "  └── ◍ w:p4 idle  /tmp/bdi-ground/summit-works"
        );
        assert_eq!(
            said(pane("w:p6", PaneStatus::Working)),
            "  └── ◍ w:p6 working  /tmp/bdi-ground/summit-works"
        );
    }

    /// A pane in a directory no configured project covers never says a claim
    /// on it was refused. `bdi` was never told the project exists, so there
    /// was no tracker to look in and nothing to refuse — and the group's own
    /// line already gives that reader the honest cause and an action.
    #[test]
    fn a_pane_in_no_configured_project_says_nothing_about_a_refused_claim() {
        let loose = UnconfiguredPane {
            pane: pane_key("w:pF"),
            cwd: "/srv/spike".to_string(),
            pane_status: PaneStatus::Idle,
        };

        assert_eq!(
            Painted::of(item_line(LAST, &Item::Unconfigured(loose)), 81, 1).rows()[0].trim_end(),
            "  └── ◍ w:pF idle  /srv/spike"
        );
    }

    /// The refusal is what a narrow row keeps, because it is the sentence
    /// that says the reader's reading of the row is wrong. The directory
    /// places a seat, which a reader who has misread the row does not want.
    #[test]
    fn a_narrow_row_gives_up_the_directory_before_the_refusal() {
        let refused = LoosePane {
            claim_refused: true,
            ..pane("w:p5", PaneStatus::Working)
        };

        assert_eq!(
            Painted::of(item_line(LAST, &Item::Loose(refused)), 54, 1).rows(),
            vec!["  └── ◍ w:p5 working  a claim on this pane was refused"]
        );
    }

    /// Cut from the right: the directory goes first, then the tail of what
    /// the pane said, and the pane's id and state stay to the end.
    #[test]
    fn a_narrow_row_gives_up_the_directory_before_the_panes_own_words() {
        let item = Item::Loose(reporting(
            Some("bdi-3um.5"),
            Some("writing the parser and its tests"),
        ));

        assert_eq!(
            Painted::of(item_line(LAST, &item), 48, 1).rows(),
            vec!["  └── ◍ w:p3 working  bdi-3um.5 · writing the p…"]
        );
        assert_eq!(
            Painted::of(item_line(LAST, &item), 20, 1).rows(),
            vec!["  └── ◍ w:p3 working"]
        );
    }
}
