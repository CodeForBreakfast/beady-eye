//! The groups drawn in the forest, and the things inside them.

use ratatui::style::{Color, Style};
use ratatui::text::Span;

use crate::model::types::PaneStatus;
use crate::view::fitted::{Fitted, GAP};
use crate::view::lines::{Group, GroupKind, Item};
use crate::view::phrase;
use crate::view::row::WARNING;

use super::tone::{LIVE, LOOK_AT_THIS};
use super::{pane_marker, sentence};

/// What lifts the live-agent filter, said beside the trees it is holding back.
const SHOW_ALL: &str = "a to show all";

/// The line a group is drawn as. The hidden trees are the only group nothing
/// went wrong in — the filter put them there and a key takes them back out —
/// so they are the only one drawn without a warning.
pub(super) fn group_line(prefix: &str, group: &Group) -> Fitted {
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
            Style::new().fg(Color::DarkGray),
        )],
    )
}

/// One thing inside such a group.
pub(super) fn item_line(prefix: &str, item: &Item) -> Fitted {
    match item {
        Item::Failed(failed) => sentence(prefix, phrase::failed_project(failed), LOOK_AT_THIS),
        Item::Conflict(conflict) => sentence(prefix, phrase::conflict(conflict), LOOK_AT_THIS),
        Item::Loose(pane) => loose_line(
            prefix,
            &pane.pane.id,
            &pane.pane_status,
            phrase::pane_report(pane.display_agent.as_deref(), pane.title.as_deref()),
            &pane.cwd,
        ),
        Item::Unconfigured(pane) => {
            loose_line(prefix, &pane.pane.id, &pane.pane_status, None, &pane.cwd)
        }
    }
}

/// A live pane in one of the groups: which pane it is, what it reported about
/// itself, and the directory it is working in. The directory is what both
/// groups are asking the reader to look at — one to place the agent, the
/// other to configure the project — and the report is what places the agent
/// without leaving the row, so it comes first and the directory is what a
/// narrow row gives up before it.
fn loose_line(
    prefix: &str,
    pane: &str,
    status: &PaneStatus,
    report: Option<String>,
    cwd: &str,
) -> Fitted {
    let mut title = Vec::new();
    if let Some(report) = report {
        title.push(Span::raw(report));
        title.push(Span::raw(" ".repeat(GAP)));
    }
    title.push(Span::raw(cwd.to_string()));

    Fitted::new(
        vec![Span::styled(
            format!("{prefix}{}", pane_marker(pane, status)),
            Style::new().fg(LIVE),
        )],
        title,
        Vec::new(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::herdr::parse_agent_list;
    use crate::config::Config;
    use crate::model::join;
    use crate::model::snapshot::{a_provider, build, Collected, LoosePane};
    use crate::model::types::testing::A_SESSION;
    use crate::model::types::PaneStatus;
    use chrono::{TimeZone, Utc};
    use pretty_assertions::assert_eq;

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
        let mut claimed = node("nix-9670s.1", "seat the guy wires", Status::InProgress);
        claimed.anomalies = vec![Anomaly::OrphanClaim { refused: None }];
        let beads = vec![node("nix-9670s", "raise the mast", Status::Open), claimed];
        let mut hidden = tree(
            "summit-works",
            "nix-9670s",
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

    /// Every other group is something that went wrong, and is marked as such.
    /// The hidden trees are not: the user asked for them to be hidden.
    #[test]
    fn only_the_group_nothing_went_wrong_in_is_drawn_without_a_warning() {
        for kind in every_kind() {
            let group = Group {
                kind,
                project: None,
                count: 2,
                with_findings: 0,
            };
            let drawn = Painted::of(group_line(SHUT, &group), 80, 1).rows();
            let marked = drawn[0].contains(WARNING);

            assert_eq!(
                marked,
                kind != GroupKind::HiddenTrees,
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
        assert_eq!(painted[1].style.fg, Some(LOOK_AT_THIS));
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
name = "beady-eye"
path = "/tmp/bdi-ground/beady-eye"
"#,
        )
        .expect("the config parses");
        let panes = parse_agent_list(
            A_SESSION,
            include_str!("../../../tests/fixtures/herdr_agent_list.json"),
        )
        .expect("the capture parses");
        let joined = join::resolve(&[], &panes, &cfg);
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

    /// `wCW:p6` stamped a `display_agent`, a title and a label for each
    /// state, and is working: the row says who it says it is, then the label
    /// for the state it is in — the caption rule a bead's agent already gets
    /// — then where it is.
    #[test]
    fn an_unattributed_pane_says_what_herdr_reported_about_it() {
        let frame = frame_over_the_capture();

        assert_eq!(
            row_naming(&frame, "wCW:p6"),
            "      ├── ◍ wCW:p6 working  bdi-3um.5 · writing the parser and its tests  \
             /tmp/bdi-ground/beady-eye"
        );
    }

    /// `wCW:p1` stamped nothing at all, and its row is the one it had.
    #[test]
    fn a_pane_that_reported_nothing_keeps_the_row_it_had() {
        let frame = frame_over_the_capture();

        assert_eq!(
            row_naming(&frame, "wCW:p1"),
            "      ├── ◍ wCW:p1 done  /tmp/bdi-ground/beady-eye"
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
