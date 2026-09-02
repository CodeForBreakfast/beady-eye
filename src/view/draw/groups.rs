//! The groups drawn below the trees, and the things inside them.

use ratatui::style::{Color, Style};
use ratatui::text::Span;

use crate::model::types::PaneStatus;
use crate::view::fitted::Fitted;
use crate::view::lines::{Group, GroupKind, Item};
use crate::view::phrase;
use crate::view::row::WARNING;

use super::tone::{LIVE, LOOK_AT_THIS};
use super::{pane_marker, sentence};

/// What lifts the live-agent filter, said beside the trees it is holding back.
const SHOW_ALL: &str = "a to show all";

/// One of the groups below the trees. The hidden trees are the only group
/// nothing went wrong in — the filter put them there and a key takes them
/// back out — so they are the only one drawn without a warning.
pub(super) fn group_line(prefix: &str, group: Group) -> Fitted {
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

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    use crate::model::anomaly::Anomaly;
    use crate::model::snapshot::{
        Counts, FailedProject, Filter, HerdrState, HiddenTree, TrackerFailure,
    };
    use crate::model::types::Status;
    use crate::view::draw::tests::*;
    use crate::view::forest::flatten;

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
            Painted::of(group_line(SHUT, quiet), 64, 1).rows(),
            vec!["▸ 4 trees with no live agent                       a to show all"]
        );
        assert_eq!(
            Painted::of(group_line(SHUT, broken), 64, 1).rows(),
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

        let mut snapshot = snapshot(Vec::new(), Vec::new(), HerdrState::Ok);
        snapshot.filter = Filter::LiveAgents;
        snapshot.hidden_trees = vec![HiddenTree::of(&hidden)];
        let frame = frame_of(&flatten(snapshot), 64, 2).rows();

        assert!(
            frame[0].contains("1 tree with no live agent · 1 with findings"),
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
        for kind in GroupKind::ALL {
            let group = Group {
                kind,
                count: 2,
                with_findings: 0,
            };
            let drawn = Painted::of(group_line(SHUT, group), 80, 1).rows();
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
        for kind in GroupKind::ALL {
            let group = Group {
                kind,
                count: 2,
                with_findings: 0,
            };
            let painted = Painted::of(group_line(SHUT, group), 80, 1).row(0);

            assert_eq!(
                painted[0].style.fg,
                Some(Color::Reset),
                "{kind:?}: {painted:?}"
            );
            assert!(painted[0].said.starts_with(SHUT), "{kind:?}: {painted:?}");
        }
    }
}
