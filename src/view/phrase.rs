//! `bdi`'s own words for what the model found.
//!
//! Every phrase here is written by `bdi`. Nothing bd or herdr wrote reaches
//! the screen: a failure is classified at the collector's boundary and the
//! text that classified it is dropped there, so a phrase is handed the reason
//! and never the tool's account of it.
//!
//! The two exceptions are deliberate and are not error text: herdr's own
//! agent states, and a status or state neither project's vocabulary covers,
//! which is quoted so it reads as a foreign word rather than as `bdi`'s.

use crate::collect::run::FailureKind;
use crate::model::anomaly::Anomaly;
use crate::model::join::{BeadKey, Conflict, JoinSource};
use crate::model::snapshot::{FailedProject, TrackerFailure};
use crate::model::types::{PaneStatus, Status};
use crate::view::Notice;

pub fn tracker_failure(failure: TrackerFailure) -> &'static str {
    match failure {
        TrackerFailure::Auth => "the tracker refused the credential it was given",
        TrackerFailure::Unavailable => "the tracker did not answer",
        TrackerFailure::Exec => "bd could not be run",
        TrackerFailure::Parse => "bd answered with something bdi cannot read",
    }
}

/// A fact about the whole view, said at the foot of the screen.
///
/// Each of these is written so a reader can tell what it costs them: what
/// they can no longer see, or how stale what they are looking at may be.
pub fn notice(notice: Notice) -> &'static str {
    match notice {
        Notice::NoHerdr => "no herdr session · which agents are alive is unknown",
        Notice::NoInboundChannel => {
            "nothing can tell bdi a project changed · every project is polled instead"
        }
    }
}

pub fn failed_project(failed: &FailedProject) -> String {
    format!("{}: {}", failed.project, tracker_failure(failed.tracker))
}

pub fn anomaly(anomaly: &Anomaly) -> String {
    match anomaly {
        Anomaly::OrphanClaim { refused } => orphan_claim(refused.as_ref()),
        Anomaly::StalePane => "closed · its pane is still alive".to_string(),
        Anomaly::StaleClaim { days } => {
            let day = if *days == 1 { "day" } else { "days" };
            format!("claimed · untouched for {days} {day}")
        }
    }
}

/// Why a claim has no pane, in the words of the disagreement that refused it.
///
/// The bead's own row is the first place a reader looks, so the reason belongs
/// on it rather than only in the conflicts group at the foot of the forest.
/// Each phrase says what to change: a directory no project covers is a config
/// entry, and a pane several beads name is a key one of them should have
/// cleared.
fn orphan_claim(refused: Option<&Conflict>) -> String {
    match refused {
        Some(Conflict::PaneInAnotherProject { pane_project, .. }) => format!(
            "claimed · its pane is in {}",
            pane_project.as_deref().unwrap_or("no configured project")
        ),
        Some(Conflict::SeveralBeadsNameOnePane { beads, .. }) => {
            format!("claimed · {} beads name its pane", beads.len())
        }
        Some(Conflict::SeveralPanesNameOneBead { panes, .. }) => {
            format!("claimed · {} panes name it", panes.len())
        }
        Some(Conflict::BeadAndPaneDisagree { .. }) | None => "claimed · no pane".to_string(),
    }
}

pub fn conflict(conflict: &Conflict) -> String {
    match conflict {
        Conflict::BeadAndPaneDisagree {
            bead,
            named_by_bead,
            named_by_pane,
        } => format!(
            "{}: the bead names pane {named_by_bead}, and pane {named_by_pane} names the bead",
            bead_key(bead)
        ),
        Conflict::SeveralPanesNameOneBead { bead, panes } => format!(
            "{}: {} panes name this bead — {} — so none holds it",
            bead_key(bead),
            panes.len(),
            panes.join(", ")
        ),
        Conflict::SeveralBeadsNameOnePane { pane, beads } => format!(
            "pane {pane}: {} beads name it — {} — so none holds it",
            beads.len(),
            beads.iter().map(bead_key).collect::<Vec<_>>().join(", ")
        ),
        Conflict::PaneInAnotherProject {
            bead,
            pane,
            pane_project,
        } => format!(
            "{}: pane {pane} is working in {}, so it joins nothing here",
            bead_key(bead),
            pane_project.as_deref().unwrap_or("no configured project")
        ),
    }
}

/// A tree whose tracker could not be read, and no live pane naming its
/// project to show in place of the beads.
pub fn no_live_panes() -> &'static str {
    "no live pane names this project"
}

/// The live panes shown for a tree whose tracker could not be read are the
/// ones naming its project. A pane working outside every configured project
/// names none, so it could belong to this tree and there is no way to tell.
pub fn panes_may_be_incomplete() -> &'static str {
    "and possibly more · a live pane under no configured project could belong here"
}

/// A node bd stopped at, so what hangs beneath it is not in this tree.
pub fn truncated() -> &'static str {
    "more beneath this · the tracker stopped at its depth limit"
}

/// A run of closed siblings nobody is working, drawn as a count rather than
/// as rows of its own.
pub fn elided(count: usize) -> String {
    let bead = if count == 1 { "bead" } else { "beads" };
    format!("{count} more {bead} · closed, and nobody on them")
}

/// Work still to do behind a closed line resting shut over it.
///
/// A bead's children here are the work closing it unblocked, so the row above
/// them says done while they are not, and its fraction says the same thing in
/// arithmetic a reader has to do. This says it in words, where the line is
/// shut and the beads are therefore nowhere else on the screen.
pub fn unfinished_beneath(count: usize) -> String {
    let bead = if count == 1 { "bead" } else { "beads" };
    format!("{count} unfinished {bead} beneath this")
}

/// Beads bd stopped at, counted for the tree they sit in.
pub fn truncated_nodes(count: usize) -> String {
    let (bead, them) = if count == 1 {
        ("bead", "it")
    } else {
        ("beads", "them")
    };
    format!("{count} {bead} the tracker stopped at · what hangs beneath {them} is not in this tree")
}

/// Projects whose tracker could not be read at all, so they have no root to
/// hang anything on.
pub fn failed_projects(count: usize) -> String {
    let project = if count == 1 { "project" } else { "projects" };
    format!("{count} {project} whose tracker could not be read")
}

/// Beads and panes that name each other in ways that cannot all be true.
pub fn conflicts(count: usize) -> String {
    let conflict = if count == 1 { "conflict" } else { "conflicts" };
    format!("{count} {conflict} nothing could settle")
}

/// Trees the live-agent filter is holding back, and how many of those carry
/// findings that are therefore not on screen.
///
/// A group that said only how many trees it hides would read as "nothing to
/// see here" while hiding broken ones. The findings stay hidden — the user
/// asked for that — but the group admits they exist.
pub fn hidden_trees(count: usize, with_findings: usize) -> String {
    let tree = if count == 1 { "tree" } else { "trees" };
    let hidden = format!("{count} {tree} with no live agent");
    if with_findings == 0 {
        return hidden;
    }
    format!("{hidden} · {with_findings} with findings")
}

/// Live panes that resolved to no bead.
pub fn unattributed(count: usize) -> String {
    let pane = if count == 1 { "pane" } else { "panes" };
    format!("{count} unattributed {pane}")
}

/// Live panes working somewhere `bdi` was never told about. The finding is
/// about the configuration rather than the pane, so the sentence is too, and
/// each directory below it is the one a `[[projects]]` entry would name.
pub fn unconfigured(count: usize) -> String {
    if count == 1 {
        return "1 pane in a directory no configured project covers".to_string();
    }
    format!("{count} panes in directories no configured project covers")
}

/// Beads whose declared parent was absent, now hanging off the root.
pub fn dangling(count: usize) -> String {
    let bead = if count == 1 { "bead" } else { "beads" };
    format!("{count} {bead} re-parented onto the root · no parent by the id each declares is in this tree")
}

/// Beads no walk down from the root reaches, because their parent chain loops.
pub fn unreachable(count: usize) -> String {
    let bead = if count == 1 { "bead" } else { "beads" };
    format!("{count} {bead} hanging off the root · a parent chain that loops")
}

/// Why the tail is showing no pane, where the selection points at none.
///
/// The band under the forest is reserved whether or not there is a pane to
/// fill it, and a band left blank reads as a pane with nothing to say rather
/// than as no pane at all. So each of these is said out loud.
pub fn no_bead_to_tail() -> &'static str {
    "no pane · select a bead to see what is on it"
}

pub fn no_agent_to_tail() -> &'static str {
    "no pane · nobody is working this bead"
}

pub fn no_herdr_to_tail() -> &'static str {
    "no herdr session · there is no pane to read"
}

/// Why the pane the selection points at could not be read. A pane that went
/// away between one poll and the next is the ordinary one of these: an agent
/// finishing is not a fault.
pub fn pane_unreadable(kind: FailureKind) -> &'static str {
    match kind {
        FailureKind::Gone => "that pane has gone",
        FailureKind::Busy => "that pane is too busy to be read",
        FailureKind::Auth | FailureKind::Unavailable | FailureKind::Exec | FailureKind::Parse => {
            "that pane could not be read"
        }
    }
}

/// What an agent was resolved by, where that is worth saying: an agent the
/// bead named is confirmed, one a pane's free text named is not.
pub fn join_caveat(source: JoinSource) -> Option<&'static str> {
    match source {
        JoinSource::AgentPane => None,
        JoinSource::DisplayAgent => Some("inferred, not confirmed"),
    }
}

/// A bead, named the only way a bead can be named across trackers.
pub fn bead_key(key: &BeadKey) -> String {
    format!("{} · {}", key.project, key.id)
}

/// herdr's word for what a pane is doing.
///
/// Read verbatim except for `blocked`, which is a TTY prompt waiting and is
/// one of three different things this tool calls blocked; a bare "blocked" on
/// screen would be unreadable, so it is said in full.
pub fn pane_state(state: &PaneStatus) -> String {
    match state {
        PaneStatus::Idle => "idle".to_string(),
        PaneStatus::Working => "working".to_string(),
        PaneStatus::Done => "done".to_string(),
        PaneStatus::Blocked => "waiting at a prompt".to_string(),
        PaneStatus::Other(state) => quoted(state),
    }
}

/// A status outside bd's own set, said rather than swallowed.
pub fn unrecognised_status(status: &Status) -> Option<String> {
    match status {
        Status::Open | Status::InProgress | Status::Blocked | Status::Closed | Status::Deferred => {
            None
        }
        Status::Other(status) => Some(format!(
            "a status bdi does not recognise: {}",
            quoted(status)
        )),
    }
}

/// Vocabulary from bd or herdr that neither project's own set covers: marked
/// as theirs rather than said in `bdi`'s voice.
fn quoted(word: &str) -> String {
    format!("“{word}”")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::run::{Env, RealRunner, Runner};
    use pretty_assertions::assert_eq;

    /// The two shapes bd writes when it cannot open a tracker, measured
    /// against this repo's own tracker on 2026-08-30. Both name a database, a
    /// host and a user, and none of it may reach a phrase.
    const REFUSED: &str = r#"Error: failed to open database: failed to check if database "atlas" exists on server db.example.invalid:3306: Error 1045 (28000): Access denied for user 'atlas'"#;
    const UNREACHABLE: &str = "Error: failed to open database: Dolt server unreachable at nosuchhost.invalid:3306: dial tcp: lookup nosuchhost.invalid: no such host";

    fn key(id: &str) -> BeadKey {
        BeadKey {
            project: "summit-works".into(),
            id: id.into(),
        }
    }

    /// Every phrase this module can produce, over every variant of every enum
    /// it takes. The enums are finite and the matches are exhaustive, so this
    /// is the whole of what can ever appear on screen from here.
    fn every_phrase() -> Vec<String> {
        let mut said: Vec<String> = Vec::new();

        for failure in [
            TrackerFailure::Auth,
            TrackerFailure::Unavailable,
            TrackerFailure::Exec,
            TrackerFailure::Parse,
        ] {
            said.push(tracker_failure(failure).to_string());
            said.push(failed_project(&FailedProject {
                project: "summit-works".into(),
                tracker: failure,
            }));
        }

        for fact in [Notice::NoHerdr, Notice::NoInboundChannel] {
            said.push(notice(fact).to_string());
        }

        for rule in [
            Anomaly::OrphanClaim { refused: None },
            Anomaly::OrphanClaim {
                refused: Some(Conflict::PaneInAnotherProject {
                    bead: key("nix-9670s.20"),
                    pane: "wCM:pD".into(),
                    pane_project: None,
                }),
            },
            Anomaly::OrphanClaim {
                refused: Some(Conflict::PaneInAnotherProject {
                    bead: key("nix-9670s.20"),
                    pane: "wCM:p9".into(),
                    pane_project: Some("homelab".into()),
                }),
            },
            Anomaly::OrphanClaim {
                refused: Some(Conflict::SeveralBeadsNameOnePane {
                    pane: "wCM:p9".into(),
                    beads: vec![key("nix-9670s.20"), key("nix-9670s.1")],
                }),
            },
            Anomaly::OrphanClaim {
                refused: Some(Conflict::SeveralPanesNameOneBead {
                    bead: key("nix-9670s.20"),
                    panes: vec!["wCM:p9".into(), "wCM:p6".into()],
                }),
            },
            Anomaly::StalePane,
            Anomaly::StaleClaim { days: 1 },
            Anomaly::StaleClaim { days: 58 },
        ] {
            said.push(anomaly(&rule));
        }

        for clash in [
            Conflict::BeadAndPaneDisagree {
                bead: key("nix-9670s.20"),
                named_by_bead: "wCM:p9".into(),
                named_by_pane: "wCM:p6".into(),
            },
            Conflict::SeveralPanesNameOneBead {
                bead: key("nix-9670s.20"),
                panes: vec!["wCM:p9".into(), "wCM:p6".into()],
            },
            Conflict::SeveralBeadsNameOnePane {
                pane: "wCM:p9".into(),
                beads: vec![key("nix-9670s.20"), key("nix-9670s.1")],
            },
            Conflict::PaneInAnotherProject {
                bead: key("nix-9670s.20"),
                pane: "wCM:p9".into(),
                pane_project: Some("homelab".into()),
            },
            Conflict::PaneInAnotherProject {
                bead: key("nix-9670s.20"),
                pane: "wCM:pD".into(),
                pane_project: None,
            },
        ] {
            said.push(conflict(&clash));
        }

        said.push(no_bead_to_tail().to_string());
        said.push(no_agent_to_tail().to_string());
        said.push(no_herdr_to_tail().to_string());
        for kind in [
            FailureKind::Auth,
            FailureKind::Unavailable,
            FailureKind::Gone,
            FailureKind::Busy,
            FailureKind::Exec,
            FailureKind::Parse,
        ] {
            said.push(pane_unreadable(kind).to_string());
        }
        said.push(no_live_panes().to_string());
        said.push(panes_may_be_incomplete().to_string());
        said.push(truncated().to_string());
        for count in [1, 3] {
            said.push(elided(count));
            said.push(unfinished_beneath(count));
            said.push(truncated_nodes(count));
            said.push(failed_projects(count));
            said.push(conflicts(count));
            for with_findings in [0, 1, count] {
                said.push(hidden_trees(count, with_findings));
            }
            said.push(unattributed(count));
            said.push(unconfigured(count));
        }
        said.push(dangling(1));
        said.push(dangling(3));
        said.push(unreachable(1));
        said.push(unreachable(3));

        for source in [JoinSource::AgentPane, JoinSource::DisplayAgent] {
            said.extend(join_caveat(source).map(str::to_string));
        }

        said
    }

    /// The collector's own account of a command that failed with `text`.
    fn detail_of(text: &str) -> String {
        RealRunner
            .run(
                "sh",
                &["-c", "printf '%s' \"$1\" >&2; exit 1", "sh", text],
                None,
                &Env::new(),
            )
            .expect_err("the command exits non-zero")
            .to_string()
    }

    /// The constraint, end to end: a real command fails with the text bd
    /// really writes, and none of it — nor the words the collector wrote
    /// about it — can be found in anything this module can say.
    #[test]
    fn nothing_a_tool_wrote_reaches_a_phrase() {
        let mut poison: Vec<String> = [
            "atlas",
            "db.example.invalid",
            "nosuchhost.invalid",
            "Access denied",
            "1045",
            "dial tcp",
            "no such host",
            "Dolt",
        ]
        .iter()
        .map(|token| token.to_string())
        .collect();
        poison.push(REFUSED.to_string());
        poison.push(UNREACHABLE.to_string());
        poison.push(detail_of(REFUSED));
        poison.push(detail_of(UNREACHABLE));

        let said = every_phrase();
        let leaked: Vec<&String> = poison
            .iter()
            .filter(|text| said.iter().any(|phrase| phrase.contains(text.as_str())))
            .collect();

        assert_eq!(leaked, Vec::<&String>::new());
    }

    /// A `&'static str` cannot hold text a tool produced at runtime, so the
    /// phrases that are one are clean by construction rather than by test.
    #[test]
    fn the_failure_phrases_are_static() {
        let _: fn(TrackerFailure) -> &'static str = tracker_failure;
        let _: fn(Notice) -> &'static str = notice;
        let _: fn() -> &'static str = truncated;
        let _: fn() -> &'static str = no_live_panes;
        let _: fn() -> &'static str = panes_may_be_incomplete;
        let _: fn(JoinSource) -> Option<&'static str> = join_caveat;
        let _: fn() -> &'static str = no_bead_to_tail;
        let _: fn() -> &'static str = no_agent_to_tail;
        let _: fn() -> &'static str = no_herdr_to_tail;
        let _: fn(FailureKind) -> &'static str = pane_unreadable;
    }

    #[test]
    fn every_phrase_says_something() {
        assert!(every_phrase()
            .iter()
            .all(|phrase| !phrase.trim().is_empty()));
    }

    #[test]
    fn the_four_tracker_failures_are_told_apart() {
        let said = [
            tracker_failure(TrackerFailure::Auth),
            tracker_failure(TrackerFailure::Unavailable),
            tracker_failure(TrackerFailure::Exec),
            tracker_failure(TrackerFailure::Parse),
        ];
        let mut distinct = said.to_vec();
        distinct.sort_unstable();
        distinct.dedup();

        assert_eq!(distinct.len(), said.len());
    }

    #[test]
    fn a_confirmed_agent_has_nothing_to_say() {
        assert_eq!(join_caveat(JoinSource::AgentPane), None);
    }

    /// Neither notice can be acted on without knowing which one it is: one
    /// says the agents are missing, the other that the beads may be stale.
    #[test]
    fn the_two_notices_are_told_apart() {
        assert_ne!(notice(Notice::NoHerdr), notice(Notice::NoInboundChannel));
    }

    /// The reader cannot open the socket from in here, so the notice is
    /// written about what it costs them rather than about what failed.
    #[test]
    fn a_bdi_nothing_can_reach_says_the_view_is_polled_rather_than_reported() {
        let said = notice(Notice::NoInboundChannel);

        assert!(said.contains("polled"), "{said}");
    }

    #[test]
    fn an_agent_named_only_by_its_panes_free_text_is_marked_as_inferred() {
        assert_eq!(
            join_caveat(JoinSource::DisplayAgent),
            Some("inferred, not confirmed")
        );
    }

    #[test]
    fn a_failed_project_is_named_alongside_its_reason() {
        let said = failed_project(&FailedProject {
            project: "summit-works".into(),
            tracker: TrackerFailure::Auth,
        });

        assert!(said.contains("summit-works"));
        assert!(said.contains(tracker_failure(TrackerFailure::Auth)));
    }

    /// bdi-9vm: every claimed bead on a live screen read `claimed · no pane`
    /// while the panes it named were alive and working. Where the join refused
    /// a claim, the row says which refusal rather than reporting a dead agent.
    #[test]
    fn a_refused_claim_says_why_rather_than_that_there_is_no_pane() {
        let bare = anomaly(&Anomaly::OrphanClaim { refused: None });

        let outside = anomaly(&Anomaly::OrphanClaim {
            refused: Some(Conflict::PaneInAnotherProject {
                bead: key("nix-9670s.20"),
                pane: "wCM:pD".into(),
                pane_project: None,
            }),
        });
        assert_ne!(outside, bare);
        assert!(outside.contains("no configured project"), "{outside}");

        let elsewhere = anomaly(&Anomaly::OrphanClaim {
            refused: Some(Conflict::PaneInAnotherProject {
                bead: key("nix-9670s.20"),
                pane: "wCM:p9".into(),
                pane_project: Some("homelab".into()),
            }),
        });
        assert!(elsewhere.contains("homelab"), "{elsewhere}");

        let shared = anomaly(&Anomaly::OrphanClaim {
            refused: Some(Conflict::SeveralBeadsNameOnePane {
                pane: "wCM:p9".into(),
                beads: vec![key("nix-9670s.20"), key("nix-9670s.1")],
            }),
        });
        assert!(shared.contains('2'), "{shared}");
        assert_ne!(shared, bare);
    }

    #[test]
    fn a_stale_claim_says_how_long_it_has_sat() {
        assert!(anomaly(&Anomaly::StaleClaim { days: 58 }).contains("58"));
    }

    /// The whole of what this sentence is for: the fold is shut over the
    /// beads, so the number is the only thing about them a reader gets.
    #[test]
    fn work_behind_a_shut_line_is_counted_rather_than_merely_admitted_to() {
        assert!(unfinished_beneath(7).contains('7'));
    }

    #[test]
    fn one_of_a_thing_is_not_described_in_the_plural() {
        for said in [
            dangling(1),
            unreachable(1),
            elided(1),
            unfinished_beneath(1),
            truncated_nodes(1),
            failed_projects(1),
            conflicts(1),
            hidden_trees(1, 0),
            hidden_trees(1, 1),
            unattributed(1),
            unconfigured(1),
            anomaly(&Anomaly::StaleClaim { days: 1 }),
        ] {
            for plural in ["beads", "days", "projects", "trees", "panes", "conflicts"] {
                assert!(!said.contains(plural), "{said}");
            }
        }
    }

    #[test]
    fn both_sides_of_a_disagreement_are_named() {
        let said = conflict(&Conflict::BeadAndPaneDisagree {
            bead: key("nix-9670s.20"),
            named_by_bead: "wCM:p9".into(),
            named_by_pane: "wCM:p6".into(),
        });

        assert!(said.contains("wCM:p9"), "{said}");
        assert!(said.contains("wCM:p6"), "{said}");
        assert!(said.contains("nix-9670s.20"), "{said}");
    }

    /// A pane under no configured project has no project to name, and saying
    /// nothing at all there would read as a pane in the same project.
    #[test]
    fn a_pane_belonging_to_no_project_still_says_where_it_is() {
        let said = conflict(&Conflict::PaneInAnotherProject {
            bead: key("nix-9670s.20"),
            pane: "wCM:pD".into(),
            pane_project: None,
        });

        assert!(said.contains("wCM:pD"), "{said}");
        assert!(said.contains("no configured project"), "{said}");
    }

    /// A bead id alone does not name a bead: prefixes are per-tracker and
    /// uncoordinated, so the project travels with it.
    #[test]
    fn a_bead_is_named_by_its_project_and_its_id() {
        let said = bead_key(&key("nix-9670s.20"));

        assert!(said.contains("summit-works"), "{said}");
        assert!(said.contains("nix-9670s.20"), "{said}");
    }

    #[test]
    fn herdrs_own_states_are_read_verbatim() {
        assert_eq!(pane_state(&PaneStatus::Idle), "idle");
        assert_eq!(pane_state(&PaneStatus::Working), "working");
        assert_eq!(pane_state(&PaneStatus::Done), "done");
    }

    /// Three different things are called blocked — a bead's status, an unmet
    /// dependency, and a TTY prompt. Only the last of them is this one.
    #[test]
    fn a_pane_waiting_at_a_prompt_is_never_a_bare_blocked() {
        assert_ne!(pane_state(&PaneStatus::Blocked), "blocked");
        assert!(pane_state(&PaneStatus::Blocked).contains("prompt"));
    }

    #[test]
    fn a_state_herdr_invented_is_quoted_rather_than_swallowed() {
        let said = pane_state(&PaneStatus::Other("compacting".into()));

        assert!(said.contains("compacting"), "{said}");
        assert_ne!(said, "compacting");
    }

    #[test]
    fn a_status_bd_invented_is_quoted_rather_than_swallowed() {
        let said = unrecognised_status(&Status::Other("triage".into()))
            .expect("a status outside bd's own set is worth saying");

        assert!(said.contains("triage"), "{said}");
    }

    #[test]
    fn a_status_bd_already_has_a_glyph_for_needs_no_words() {
        for status in [
            Status::Open,
            Status::InProgress,
            Status::Blocked,
            Status::Closed,
            Status::Deferred,
        ] {
            assert_eq!(unrecognised_status(&status), None, "{status:?}");
        }
    }
}
