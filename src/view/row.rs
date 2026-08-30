//! The cells of one bead's row.
//!
//! Indentation, the box-drawing prefix, width, colour and where each cell
//! lands are the renderer's. This is what there is to say about one bead.

use crate::model::anomaly::Anomaly;
use crate::model::join::AgentRef;
use crate::model::snapshot::Node;
use crate::model::types::Status;
use crate::view::phrase;

/// A live agent. The mock's marker, kept in the role the mock gave it.
pub const AGENT: char = '◍';

/// Something the reader should look at.
pub const WARNING: char = '⚠';

/// How much of what a line stands for is done, closed beads over all of them.
///
/// Counted over the whole subtree with the line's own bead among them, which
/// is the count a root already carries for a tree: one rule at every depth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub closed: usize,
    pub total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub glyph: char,
    pub id: String,
    pub title: String,
    pub badges: Vec<String>,
    /// How far along what hangs off this bead is, where anything does. A leaf
    /// stands for itself alone, so a fraction over it would only repeat the
    /// glyph.
    pub progress: Option<Progress>,
    pub agent: Option<String>,
    pub anomalies: Option<String>,
    /// What is true of this bead beyond its own fields: a subtree the tracker
    /// stopped at, a status outside bd's own set.
    pub notes: Vec<String>,
}

pub fn cells(node: &Node, root: &str, progress: Option<Progress>) -> Row {
    let mut notes = Vec::new();
    if node.truncated {
        notes.push(phrase::truncated().to_string());
    }
    notes.extend(phrase::unrecognised_status(&node.status));

    Row {
        glyph: status_glyph(&node.status),
        id: abbreviate(&node.id, root).to_string(),
        title: node.title.clone(),
        badges: node.badges.iter().map(|b| b.text.clone()).collect(),
        progress,
        agent: node.agent.as_ref().map(agent_marker),
        anomalies: anomaly_marker(&node.anomalies),
        notes,
    }
}

/// The glyph is the bead's own status and nothing else. Liveness has its own
/// cell, and one glyph meaning both would make neither readable.
///
/// | status | glyph |
/// |---|---|
/// | `in_progress` | `●` |
/// | `blocked` | `◐` |
/// | `open` | `○` |
/// | `deferred` | `◌` |
/// | `closed` | `✓` |
/// | anything else | `?` |
pub fn status_glyph(status: &Status) -> char {
    match status {
        Status::InProgress => '●',
        Status::Blocked => '◐',
        Status::Open => '○',
        Status::Deferred => '◌',
        Status::Closed => '✓',
        Status::Other(_) => '?',
    }
}

/// A node's id with its root's prefix dropped, which is what makes a column of
/// ids readable. A node that does not carry the root's prefix keeps its whole
/// id: a bare suffix would place it under a root it does not belong to, and
/// the dangling and re-parented nodes are exactly the ones that would lie.
pub fn abbreviate<'a>(id: &'a str, root: &str) -> &'a str {
    id.strip_prefix(root)
        .filter(|rest| rest.starts_with('.'))
        .unwrap_or(id)
}

pub fn agent_marker(agent: &AgentRef) -> String {
    let mut said = format!(
        "{AGENT} {} {}",
        agent.pane,
        phrase::pane_state(&agent.pane_status)
    );
    if let Some(caveat) = phrase::join_caveat(agent.source) {
        said.push_str(" · ");
        said.push_str(caveat);
    }
    said
}

/// Every rule that fired, not the first: an old claim whose agent has died is
/// both, and the age is the part that says whether to care.
pub fn anomaly_marker(anomalies: &[Anomaly]) -> Option<String> {
    if anomalies.is_empty() {
        return None;
    }
    let said: Vec<String> = anomalies.iter().map(phrase::anomaly).collect();
    Some(format!("{WARNING} {}", said.join(" · ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::herdr::PaneStatus;
    use crate::model::join::{Badged, JoinSource};
    use pretty_assertions::assert_eq;

    const ROOT: &str = "nix-9670s";

    fn node(id: &str, status: Status) -> Node {
        Node {
            id: id.into(),
            title: "wallpaper timer calls dms".into(),
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

    fn agent(source: JoinSource) -> AgentRef {
        AgentRef {
            pane: "wCM:p9".into(),
            pane_status: PaneStatus::Working,
            title: Some("shell selector".into()),
            source,
        }
    }

    #[test]
    fn the_glyph_is_the_beads_own_status_and_no_two_statuses_share_one() {
        let statuses = [
            Status::InProgress,
            Status::Blocked,
            Status::Open,
            Status::Deferred,
            Status::Closed,
            Status::Other("triage".into()),
        ];
        let mut glyphs: Vec<char> = statuses.iter().map(status_glyph).collect();
        let drawn = glyphs.len();
        glyphs.sort_unstable();
        glyphs.dedup();

        assert_eq!(glyphs.len(), drawn);
    }

    /// The glyph says what the bead's status is, never whether an agent is on
    /// it: liveness has its own cell, and the mock's `◐` row is `in_progress`
    /// without a pane, which under the other reading would have no glyph left.
    #[test]
    fn the_glyph_does_not_move_when_an_agent_arrives() {
        let mut staffed = node("nix-9670s.20", Status::InProgress);
        staffed.agent = Some(agent(JoinSource::AgentPane));

        assert_eq!(
            cells(&staffed, ROOT, None).glyph,
            cells(&node("nix-9670s.20", Status::InProgress), ROOT, None).glyph
        );
    }

    #[test]
    fn a_node_under_the_root_shows_only_what_it_adds_to_it() {
        assert_eq!(abbreviate("nix-9670s.20", ROOT), ".20");
        assert_eq!(abbreviate("nix-9670s.1.4", ROOT), ".1.4");
    }

    /// A dangling or re-parented node is drawn under a root it does not
    /// descend from, and a bare suffix there would say it does.
    #[test]
    fn a_node_that_does_not_descend_from_the_root_keeps_its_whole_id() {
        assert_eq!(abbreviate("hl-sgqyv.3", ROOT), "hl-sgqyv.3");
        assert_eq!(abbreviate("nix-9670sX.3", ROOT), "nix-9670sX.3");
    }

    #[test]
    fn the_root_keeps_its_whole_id() {
        assert_eq!(abbreviate(ROOT, ROOT), ROOT);
    }

    #[test]
    fn a_live_agent_is_marked_with_its_pane_and_what_that_pane_is_doing() {
        let said = agent_marker(&agent(JoinSource::AgentPane));

        assert_eq!(said, "◍ wCM:p9 working");
    }

    #[test]
    fn an_agent_the_bead_never_named_is_marked_as_inferred() {
        let said = agent_marker(&agent(JoinSource::DisplayAgent));

        assert!(said.contains("wCM:p9"), "{said}");
        assert!(said.contains("inferred, not confirmed"), "{said}");
    }

    #[test]
    fn a_row_carries_every_anomaly_that_fired_rather_than_the_first() {
        let said = anomaly_marker(&[Anomaly::OrphanClaim, Anomaly::StaleClaim { days: 58 }])
            .expect("two rules fired");

        assert!(said.contains("no pane"), "{said}");
        assert!(said.contains("58"), "{said}");
    }

    #[test]
    fn a_bead_with_nothing_wrong_carries_no_marker_at_all() {
        let row = cells(&node("nix-9670s.20", Status::Open), ROOT, None);

        assert_eq!(row.anomalies, None);
        assert_eq!(row.agent, None);
        assert_eq!(row.notes, Vec::<String>::new());
    }

    /// A truncated node means the tree shown is incomplete, which is the kind
    /// of silent partial answer this tool exists to avoid.
    #[test]
    fn a_node_the_tracker_stopped_at_says_so_on_its_row() {
        let mut stopped = node("nix-9670s.20", Status::Open);
        stopped.truncated = true;

        assert_eq!(cells(&stopped, ROOT, None).notes, vec![phrase::truncated()]);
    }

    #[test]
    fn a_status_outside_bds_own_set_leaves_the_word_bd_used_on_the_row() {
        let odd = node("nix-9670s.20", Status::Other("triage".into()));
        let row = cells(&odd, ROOT, None);

        assert_eq!(row.glyph, '?');
        assert!(
            row.notes.iter().any(|note| note.contains("triage")),
            "{row:?}"
        );
    }

    #[test]
    fn badges_are_drawn_in_the_order_they_were_configured() {
        let mut badged = node("nix-9670s.20", Status::Blocked);
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

        assert_eq!(
            cells(&badged, ROOT, None).badges,
            vec!["⇢ #12", "⏸ waiting"]
        );
    }

    #[test]
    fn a_row_says_what_the_bead_says() {
        let row = cells(&node("nix-9670s.20", Status::Blocked), ROOT, None);

        assert_eq!(row.glyph, '◐');
        assert_eq!(row.id, ".20");
        assert_eq!(row.title, "wallpaper timer calls dms");
    }
}
