//! The cells of one bead's row.
//!
//! Indentation, the box-drawing prefix, width, colour and where each cell
//! lands are the renderer's. This is what there is to say about one bead.

use crate::model::anomaly::Anomaly;
use crate::model::join::AgentRef;
use crate::model::snapshot::{Counts, Node};
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
    pub status: Status,
    /// What the reader sees for `status`.
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
    /// The work this line is shut over, where it is shut over any: the beads
    /// its fold hides, counted once each.
    ///
    /// `progress` counts this line's own bead among its total; this does not,
    /// and the difference is not an inconsistency. A fraction is the only
    /// thing on the row saying how much work the line stands for, so it must
    /// take the bead in. The agent on this bead is already on the row by
    /// name, so a count taking it in would have a reader add the name to the
    /// number and come out one too many.
    pub shut_over: Option<Counts>,
    /// What is true of this bead beyond its own fields: a subtree the tracker
    /// stopped at, unfinished work the line is shut over, a status outside
    /// bd's own set.
    pub notes: Vec<String>,
}

/// `shut_over` is what this line's fold hides, where it hides anything. The
/// caller knows the branch and the fold; the bead's own fields say nothing
/// about either.
///
/// Both of the things a shut line says come off that one set of counts. A
/// closed line's glyph says done, and the unfinished beads it rests over are
/// nowhere else on the screen to say otherwise — so it says how many, in
/// words, beside the fraction saying it in arithmetic.
pub fn cells(
    node: &Node,
    root: &str,
    progress: Option<Progress>,
    shut_over: Option<Counts>,
) -> Row {
    let mut notes = Vec::new();
    if node.truncated {
        notes.push(phrase::truncated().to_string());
    }
    notes.extend(
        shut_over
            .as_ref()
            .filter(|_| node.status.is_closed())
            .map(Counts::unfinished)
            .filter(|unfinished| *unfinished > 0)
            .map(phrase::unfinished_beneath),
    );
    notes.extend(phrase::unrecognised_status(&node.status));

    Row {
        status: node.status.clone(),
        glyph: status_glyph(&node.status),
        id: abbreviate(&node.id, root).to_string(),
        title: node.title.clone(),
        badges: node.badges.iter().map(|b| b.text.clone()).collect(),
        progress,
        agent: node.agent.as_ref().map(agent_marker),
        anomalies: anomaly_marker(&node.anomalies),
        shut_over,
        notes,
    }
}

/// The glyph is the bead's own status and nothing else. Liveness has its own
/// cell, and one glyph meaning both would make neither readable.
///
/// Every one of these is the glyph `bd list` prints beside that word in the
/// legend at the foot of its own output. Terminology comes from beads and a
/// glyph is terminology, so there is nothing here to improve on — only a
/// reader's existing habit to keep or to break.
///
/// | status | glyph |
/// |---|---|
/// | `open` | `○` |
/// | `in_progress` | `◐` |
/// | `blocked` | `●` |
/// | `closed` | `✓` |
/// | `deferred` | `❄` |
/// | anything else | `?` — a status `bd` has no legend for |
pub fn status_glyph(status: &Status) -> char {
    match status {
        Status::Open => '○',
        Status::InProgress => '◐',
        Status::Blocked => '●',
        Status::Closed => '✓',
        Status::Deferred => '❄',
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

/// A live agent, said as what it is doing rather than as which pane it sits
/// in. herdr shows a pane id nowhere a reader can look one up, so an id here
/// spends the row's widest cell on a handle nobody can follow.
///
/// A pane that has said nothing keeps its id, which is then the only thing
/// left that tells one live agent from another.
///
/// The caption is free text and a state can be several words, so the row's own
/// `·` goes between them; run together they read as one sentence. The state
/// stays between the caption and the join caveat, which is about which pane
/// this is and not about the work, and beside the caption would read as doubt
/// about what the agent is doing.
pub fn agent_marker(agent: &AgentRef) -> String {
    let doing = agent.title.as_deref().unwrap_or(&agent.pane);
    let mut said = vec![
        format!("{AGENT} {doing}"),
        phrase::pane_state(&agent.pane_status),
    ];
    said.extend(phrase::join_caveat(agent.source).map(str::to_string));
    said.join(" · ")
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
    use crate::model::badges::Badged;
    use crate::model::join::JoinSource;
    use crate::model::types::PaneStatus;
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

    fn unlabelled(source: JoinSource) -> AgentRef {
        AgentRef {
            title: None,
            ..agent(source)
        }
    }

    /// `bd list` prints this legend at the foot of every listing, so a reader
    /// arriving from `bd` has already learned which glyph means what. A glyph
    /// given to a different status here would be read backwards, and no
    /// amount of doing better elsewhere would undo that.
    #[test]
    fn every_glyph_is_the_one_bd_lists_beside_that_status_in_its_own_legend() {
        assert_eq!(status_glyph(&Status::Open), '○');
        assert_eq!(status_glyph(&Status::InProgress), '◐');
        assert_eq!(status_glyph(&Status::Blocked), '●');
        assert_eq!(status_glyph(&Status::Closed), '✓');
        assert_eq!(status_glyph(&Status::Deferred), '❄');
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
            cells(&staffed, ROOT, None, None).glyph,
            cells(&node("nix-9670s.20", Status::InProgress), ROOT, None, None).glyph
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
    fn a_live_agent_is_marked_with_what_it_is_doing_and_not_with_its_pane() {
        let said = agent_marker(&agent(JoinSource::AgentPane));

        assert_eq!(said, "◍ shell selector · working");
    }

    /// herdr shows a pane id nowhere a reader can look one up, so on a row it
    /// costs the widest columns of the cell and answers nothing.
    #[test]
    fn a_pane_that_says_what_it_is_doing_never_shows_its_id() {
        let said = agent_marker(&agent(JoinSource::DisplayAgent));

        assert!(!said.contains("wCM:p9"), "{said}");
    }

    /// The last thing left that identifies the pane. A cell saying only that
    /// something is working would drop the agent the row exists to show.
    #[test]
    fn a_pane_that_has_said_nothing_falls_back_to_the_id_it_cannot_lose() {
        let said = agent_marker(&unlabelled(JoinSource::AgentPane));

        assert_eq!(said, "◍ wCM:p9 · working");
    }

    #[test]
    fn an_agent_the_bead_never_named_is_marked_as_inferred() {
        let said = agent_marker(&agent(JoinSource::DisplayAgent));

        assert!(said.contains("inferred, not confirmed"), "{said}");
    }

    /// The caveat is about the join, not about the work: it says the pane was
    /// matched to this bead by inference. Left against the caption it would
    /// read as doubt about what the agent is doing, so the state stays between
    /// them and the caveat keeps the place it has always had.
    #[test]
    fn the_join_caveat_follows_the_state_rather_than_the_caption() {
        let said = agent_marker(&agent(JoinSource::DisplayAgent));

        assert_eq!(said, "◍ shell selector · working · inferred, not confirmed");
    }

    /// A caption is free text and a state can be several words, so run
    /// together they read as one sentence and neither can be picked out.
    #[test]
    fn a_caption_is_kept_apart_from_the_state_that_follows_it() {
        let waiting = AgentRef {
            pane_status: PaneStatus::Blocked,
            ..agent(JoinSource::AgentPane)
        };

        assert_eq!(
            agent_marker(&waiting),
            "◍ shell selector · waiting at a prompt"
        );
    }

    #[test]
    fn a_row_carries_every_anomaly_that_fired_rather_than_the_first() {
        let said = anomaly_marker(&[
            Anomaly::OrphanClaim { refused: None },
            Anomaly::StaleClaim { days: 58 },
        ])
        .expect("two rules fired");

        assert!(said.contains("no pane"), "{said}");
        assert!(said.contains("58"), "{said}");
    }

    #[test]
    fn a_bead_with_nothing_wrong_carries_no_marker_at_all() {
        let row = cells(&node("nix-9670s.20", Status::Open), ROOT, None, None);

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

        assert_eq!(
            cells(&stopped, ROOT, None, None).notes,
            vec![phrase::truncated()]
        );
    }

    #[test]
    fn a_status_outside_bds_own_set_leaves_the_word_bd_used_on_the_row() {
        let odd = node("nix-9670s.20", Status::Other("triage".into()));
        let row = cells(&odd, ROOT, None, None);

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
            cells(&badged, ROOT, None, None).badges,
            vec!["⇢ #12", "⏸ waiting"]
        );
    }

    #[test]
    fn a_row_says_what_the_bead_says() {
        let row = cells(&node("nix-9670s.20", Status::Blocked), ROOT, None, None);

        assert_eq!(row.status, Status::Blocked);
        assert_eq!(row.glyph, '●');
        assert_eq!(row.id, ".20");
        assert_eq!(row.title, "wallpaper timer calls dms");
    }
}
