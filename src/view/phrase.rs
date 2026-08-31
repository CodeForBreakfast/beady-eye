//! `bdi`'s own words for what the model found.
//!
//! Every phrase here is written by `bdi`. Nothing bd or herdr wrote reaches
//! the screen: a failure is classified at the collector's boundary and the
//! text that classified it is dropped there, so a phrase is handed the reason
//! and never the tool's account of it.
//!
//! The exceptions are deliberate and none of them is error text: herdr's own
//! agent states; a status or state neither project's vocabulary covers; and
//! what a pane says of itself, where that is the only thing on the screen
//! that can settle the finding. The last two are quoted, so they read as
//! somebody else's words rather than as `bdi`'s.

use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};

use crate::collect::run::FailureKind;
use crate::model::anomaly::Anomaly;
use crate::model::join::{BeadKey, Conflict, JoinSource};
use crate::model::snapshot::{FailedProject, TrackerFailure};
use crate::model::types::{PaneStatus, Status};
use crate::view::{Freshness, Mark, Notice};

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
        Notice::AnotherBdiHadTheInboundChannel => {
            "another bdi held the inbound channel · every project is polled instead"
        }
    }
}

/// The same fact in the fewest words that still carry it, for a foot with no
/// room to say it in full.
///
/// The narrowest supported screen is forty columns and neither full phrase
/// fits in one on its own, so a foot without these has nothing to fall back
/// to but a cut — and a cut takes the end, which is where both phrases keep
/// what the fact costs the reader.
pub fn brief_notice(notice: Notice) -> &'static str {
    match notice {
        Notice::NoHerdr => "agents unknown",
        Notice::NoInboundChannel => "polled, not reported",
        // The cause is what survives the cut, not the cost. A reader who
        // keeps only *polled* has what the notice this one replaced already
        // gave them, and still nothing to do about it.
        Notice::AnotherBdiHadTheInboundChannel => "another bdi had it",
    }
}

/// How long one frame of the collecting mark is on the screen.
///
/// Public because it is how often the mark turns, which is something about
/// `bdi` that a caller driving the binary can otherwise only find out by
/// watching. The screen can redraw sooner — an age beside the mark expires
/// on a clock of its own — but never later while a collection runs.
pub const FRAME: Duration = Duration::from_millis(80);

/// The same length in the unit the clock arithmetic counts in, derived rather
/// than written out again: a mark that turned on one length and expired on
/// another would fall a little further behind every frame.
const FRAME_MS: i64 = FRAME.as_millis() as i64;

/// The frames the collecting mark turns through.
const TURNING: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// The mark a project wears when the last collection read every root of it.
///
/// As quiet as a glyph gets: it holds the column the turning mark turns in,
/// and says the collection came back whole. A project where nothing is wrong
/// should not be drawing the eye.
const READ: &str = "✓";

/// The mark a project wears when the last collection met a root it could not
/// read. `bdi`'s own word for something to look at, which this is: the rows
/// under the name are short of that root's.
const REFUSED: &str = "⚠";

/// The mark beside a project's name: how far the collection reading it has
/// turned, or how the last one went.
///
/// One column in every state, so the cell does not change width for a
/// collection starting or ending. It changing shape rather than content was
/// the whole of what made it jump.
pub fn mark(freshness: Freshness, now: DateTime<Utc>) -> &'static str {
    match freshness.mark {
        Mark::Collecting => turning(now),
        Mark::Read => READ,
        Mark::Refused => REFUSED,
    }
}

/// How long ago a project's rows were read, said beside its name.
///
/// A duration rather than a time of day. The reader's question is how stale
/// the rows in front of them are, and a clock makes them subtract one time
/// from another to answer it.
///
/// Said while a collection runs as well as at rest. The rows on the screen
/// during a collection are the previous collection's rows, and this is the
/// only thing that says so — a cell that gave it up for the mark left a
/// reader watching a mark turn over rows of unknown age.
///
/// Nothing at all before the first collection of a project comes back: there
/// is no read to date the rows to, and no rows either.
pub fn last_read(freshness: Freshness, now: DateTime<Utc>) -> Option<String> {
    freshness.read_at.map(|at| format!("{} ago", age(now - at)))
}

/// Which frame the collecting mark is on at this instant.
///
/// Taken from the clock rather than counted, so every redraw inside one
/// collection agrees about which frame it is. A keystroke redraws the screen
/// too, and a counter the redraw advanced would make the mark jump forward
/// for a key that has nothing to do with the collection.
fn turning(now: DateTime<Utc>) -> &'static str {
    let frame = now.timestamp_millis().div_euclid(FRAME_MS);
    TURNING[frame.rem_euclid(TURNING.len() as i64) as usize]
}

/// How long what the cell beside a project's name says goes on being true.
///
/// An age is a duration, so it stops being true with nothing having happened:
/// a line saying `0s ago` is wrong a second later, under a reader who has not
/// touched anything. The clock this replaced was true for as long as it was on
/// the screen, and a screen that never changed by itself needed nothing to
/// redraw it.
///
/// So this is what the loop sleeps until. Both halves are drawn from a clock
/// by dividing it, so both hold until that clock's own next boundary rather
/// than for a whole unit from whenever they were last drawn — a project read
/// four days ago is redrawn when it becomes five, and a keystroke part way
/// through a frame does not leave the mark late for every frame after it.
///
/// The cell says two things at once and holds only as long as the shorter of
/// them: a frame of the turning mark is 80 ms and an age can be a day. A mark
/// at rest is not one of them — it changes when a collection does something
/// and never on its own — so a project nothing is reading holds for its age
/// alone, and one never read and not being read holds for nothing at all.
pub fn holds_for(freshness: Freshness, now: DateTime<Utc>) -> Option<Duration> {
    // The mark's frame is cut from the clock itself, so its boundaries are
    // the clock's.
    let turning = matches!(freshness.mark, Mark::Collecting)
        .then(|| until_the_next(FRAME_MS, now.timestamp_millis()));
    let ageing = freshness.read_at.map(|at| {
        let elapsed = (now - at).num_milliseconds().max(0);
        until_the_next(unit_of(elapsed), elapsed)
    });

    turning.into_iter().chain(ageing).min()
}

const SECOND: i64 = 1_000;
const MINUTE: i64 = 60 * SECOND;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

/// The unit `age` says an elapsed time in.
fn unit_of(elapsed: i64) -> i64 {
    match elapsed {
        0..MINUTE => SECOND,
        MINUTE..HOUR => MINUTE,
        HOUR..DAY => HOUR,
        _ => DAY,
    }
}

/// How long until `clock` crosses its next multiple of `unit`.
///
/// Never nothing: a deadline of no time at all would wake the loop, find
/// what is drawn unchanged, and ask for no time again.
fn until_the_next(unit: i64, clock: i64) -> Duration {
    Duration::from_millis((unit - clock.rem_euclid(unit)) as u64)
}

/// How long ago a read was, in the coarsest unit that still says it.
///
/// One unit and no more: the question is how much to trust the rows, and a
/// reader answers it from the order of magnitude. Seconds of precision on an
/// hour-old read is precision about a number nobody is going to act on.
///
/// A read stamped ahead of the frame's own clock reads as this instant rather
/// than as a negative age. Both times come from this process, so a step
/// backwards in the system clock is the only thing that produces one, and
/// `-3s ago` would say the rows arrive in the future.
fn age(since: TimeDelta) -> String {
    let elapsed = since.num_milliseconds().max(0);
    let unit = unit_of(elapsed);
    let said = elapsed / unit;
    match unit {
        SECOND => format!("{said}s"),
        MINUTE => format!("{said}m"),
        HOUR => format!("{said}h"),
        _ => format!("{said}d"),
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
        // Neither of these ever gets here. A refusal sends the reader to the
        // disagreement's row to find the pane the claim was for, and these two
        // name no one pane, so `join::resolve` refuses no claim with either.
        Some(Conflict::BeadAndPaneDisagree { .. } | Conflict::SeveralPanesNameOneBead { .. })
        | None => "claimed · no pane".to_string(),
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
        Conflict::SeveralBeadsNameOnePane {
            pane,
            caption,
            beads,
        } => format!(
            "pane {pane}{}: {} beads name it — {} — so none holds it",
            caption.as_deref().map(saying).unwrap_or_default(),
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

/// A root that drew no row and whose tracker named no reason. Nothing should
/// reach this, and a root that quietly left the screen would be the one kind
/// of wrong answer `bdi` exists to prevent.
pub fn root_unread() -> &'static str {
    "this root drew no rows, and nothing said why"
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

/// The agents at work behind a line resting shut over them.
///
/// Said as a count where the line's own agent is said by name, because the
/// two answer different questions: who is on this bead, and how much is going
/// on out of sight. "beneath" is the word keeping a reader from adding the
/// named one to the number.
///
/// A state cell rather than a note, so it is said in the register the
/// fraction beside it is said in. Drawn out to a sentence it took the title
/// off the row at the widths this is read at.
pub fn agents_beneath(count: usize) -> String {
    let agent = if count == 1 { "agent" } else { "agents" };
    format!("{count} {agent} beneath")
}

/// The beads behind a line resting shut over them that want looking at.
///
/// Beads rather than rules fired, which is what the number counts and what a
/// reader opening the fold would find rows of. What is wrong with them is on
/// their own rows and not summarised here — the count's whole job is to say
/// that opening this is worth it.
pub fn anomalies_beneath(count: usize) -> String {
    let bead = if count == 1 { "bead" } else { "beads" };
    format!("{count} {bead} beneath")
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

/// Beads naming something they depend on that this tree does not hold. One
/// left with nowhere else to sit hangs off the root.
pub fn dangling(count: usize) -> String {
    let bead = if count == 1 { "bead" } else { "beads" };
    format!(
        "{count} {bead} waiting on work outside this tree · no bead by the id each names is in it"
    )
}

/// Beads whose own descendants lead back to them, each drawn where the loop
/// was cut.
pub fn cycle(count: usize) -> String {
    let bead = if count == 1 { "bead" } else { "beads" };
    format!(
        "{count} {bead} that must finish before themselves · a chain of dependencies that loops"
    )
}

/// Why the whole forest is empty. `bd` is asked for unfinished work and
/// every root is climbed from what it names, so nothing unfinished anywhere
/// is the only thing that leaves no root to draw — a tracker that refused is
/// a failed project or an unreadable tree, and either draws a line of its
/// own. The clause after the separator is the point: it says the trackers
/// answered, which is what a reader must not mistake a blank screen for.
pub fn no_roots() -> &'static str {
    "no unfinished work anywhere · every tracker answered, and none of them had a root to draw"
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

/// That the pane the selection points at is being read and has not answered
/// yet. The rule above the band names the pane already, so what is left to
/// say is that `bdi` is waiting rather than that the pane is quiet.
pub fn pane_being_read() -> &'static str {
    "reading that pane"
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

/// What a pane says of itself, set into a sentence `bdi` is saying about it.
///
/// It sits directly after the pane it belongs to, ahead of anything else the
/// sentence has to say, because a sentence too long for the width is cut from
/// the right.
fn saying(caption: &str) -> String {
    format!(" {}", quoted(caption))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::run::{Env, RealRunner, Runner};
    use crate::view::fitted::columns;
    use pretty_assertions::assert_eq;
    use ratatui::text::Span;

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

        for fact in [
            Notice::NoHerdr,
            Notice::NoInboundChannel,
            Notice::AnotherBdiHadTheInboundChannel,
        ] {
            said.push(notice(fact).to_string());
            said.push(brief_notice(fact).to_string());
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
                    caption: None,
                    beads: vec![key("nix-9670s.20"), key("nix-9670s.1")],
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
                caption: None,
                beads: vec![key("nix-9670s.20"), key("nix-9670s.1")],
            },
            Conflict::SeveralBeadsNameOnePane {
                pane: "wCM:p9".into(),
                caption: Some("nix-9670s.1: rebuild the installer image".into()),
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
        said.push(pane_being_read().to_string());
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
            said.push(agents_beneath(count));
            said.push(anomalies_beneath(count));
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
        said.push(cycle(1));
        said.push(cycle(3));

        for source in [JoinSource::AgentPane, JoinSource::DisplayAgent] {
            said.extend(join_caveat(source).map(str::to_string));
        }

        // Every frame of the mark and every unit an age is said in: the
        // instant decides which of each is drawn, so the whole vocabulary
        // only appears if this walks them.
        for frame in 0..TURNING.len() as i64 {
            said.push(
                mark(
                    collecting(),
                    an_instant() + TimeDelta::milliseconds(frame * FRAME_MS),
                )
                .to_string(),
            );
        }
        for at_rest in [Mark::Read, Mark::Refused] {
            said.push(mark(resting(at_rest), an_instant()).to_string());
        }
        for ago in [1, 90, 5_000, 200_000] {
            said.extend(last_read(
                resting(Mark::Read),
                an_instant() + TimeDelta::seconds(ago),
            ));
        }

        said
    }

    fn an_instant() -> chrono::DateTime<chrono::Utc> {
        use chrono::TimeZone;
        chrono::Utc
            .with_ymd_and_hms(2026, 8, 30, 10, 22, 14)
            .unwrap()
    }

    /// A project being read now, with the read it is replacing still under
    /// it.
    fn collecting() -> Freshness {
        Freshness {
            mark: Mark::Collecting,
            read_at: Some(an_instant()),
        }
    }

    /// A project nothing is reading, wearing the mark its last collection
    /// earned.
    fn resting(at_rest: Mark) -> Freshness {
        Freshness {
            mark: at_rest,
            read_at: Some(an_instant()),
        }
    }

    /// The whole cell, as one string, for a test about when it changes
    /// rather than about what it says.
    fn cell(freshness: Freshness, now: chrono::DateTime<chrono::Utc>) -> String {
        match last_read(freshness, now) {
            Some(age) => format!("{} {age}", mark(freshness, now)),
            None => mark(freshness, now).to_string(),
        }
    }

    /// Graeme asked for "ago" language, and the reason it is better than the
    /// clock it replaces is that the reader does no arithmetic: the line says
    /// how stale the rows are, not what time it was when they arrived.
    #[test]
    fn a_collected_project_says_how_long_ago_rather_than_at_what_time() {
        let said = last_read(resting(Mark::Read), an_instant() + TimeDelta::seconds(9));

        assert_eq!(said.as_deref(), Some("9s ago"));
    }

    /// The bead. A collection running says the rows are about to be replaced;
    /// it does not say how old the ones being replaced are, and those are the
    /// rows in front of the reader for as long as it runs.
    #[test]
    fn a_project_being_read_still_says_how_old_the_rows_under_it_are() {
        let said = last_read(collecting(), an_instant() + TimeDelta::seconds(9));

        assert_eq!(said.as_deref(), Some("9s ago"));
    }

    /// The startup frame is the one state with no age: nothing has come back,
    /// so there is no read to date the rows to and no rows either.
    #[test]
    fn a_project_read_by_nothing_yet_has_no_age_to_say() {
        let starting = Freshness {
            mark: Mark::Collecting,
            read_at: None,
        };

        assert_eq!(last_read(starting, an_instant()), None);
    }

    /// One unit, chosen by how old the read is. A minute is where seconds
    /// stop being worth counting, an hour where minutes do, and a day where
    /// hours do.
    #[test]
    fn an_age_is_said_in_the_coarsest_unit_that_still_says_it() {
        let said = |seconds| {
            last_read(
                resting(Mark::Read),
                an_instant() + TimeDelta::seconds(seconds),
            )
            .expect("a project that has been read has an age")
        };

        assert_eq!(
            [
                said(0),
                said(59),
                said(60),
                said(3_599),
                said(3_600),
                said(86_399),
                said(86_400)
            ],
            ["0s ago", "59s ago", "1m ago", "59m ago", "1h ago", "23h ago", "1d ago"]
        );
    }

    /// A system clock stepping backwards between the read and the frame is
    /// the one thing that can produce this, and `-3s ago` would say the rows
    /// arrive in the future.
    #[test]
    fn a_read_stamped_ahead_of_the_frame_reads_as_this_instant() {
        let said = last_read(resting(Mark::Read), an_instant() - TimeDelta::seconds(30));

        assert_eq!(said.as_deref(), Some("0s ago"));
    }

    /// Graeme, on the mark at rest: *"when not collecting, the spinner can be
    /// replaced with something to indicate success/failure so that it doesn't
    /// jump around"*. Three states, three marks, and the mark is what tells
    /// them apart — the age beside it says the same kind of thing in all
    /// three.
    #[test]
    fn each_state_of_a_collection_wears_its_own_mark() {
        assert_eq!(
            [
                mark(collecting(), an_instant()),
                mark(resting(Mark::Read), an_instant()),
                mark(resting(Mark::Refused), an_instant()),
            ],
            ["⠴", "✓", "⚠"]
        );
    }

    /// The bead: the cell changed shape rather than content every time a
    /// collection started and ended, so the words beside it jumped. Every
    /// mark is one column, so nothing after it moves.
    #[test]
    fn every_mark_is_one_column_so_the_cell_never_changes_width() {
        for state in [Mark::Collecting, Mark::Read, Mark::Refused] {
            for frame in 0..TURNING.len() as i64 {
                let at = an_instant() + TimeDelta::milliseconds(frame * FRAME_MS);
                let drawn = mark(resting_or_turning(state), at);

                // Measured the way `Fitted` measures, so the claim is about
                // the columns the layout will give it rather than about
                // bytes or characters.
                assert_eq!(
                    columns(&[Span::raw(drawn)]),
                    1,
                    "{state:?} draws {drawn:?} at frame {frame}"
                );
            }
        }
    }

    /// A `Freshness` in whichever state `at_rest` names, turning included.
    fn resting_or_turning(at_rest: Mark) -> Freshness {
        Freshness {
            mark: at_rest,
            read_at: Some(an_instant()),
        }
    }

    /// The whole of what `holds_for` promises, over every state the cell can
    /// be in: what is drawn now is still what would be drawn at any instant
    /// before it runs out, and is not what would be drawn the instant it
    /// does. A deadline longer than that draws a line that has stopped being
    /// true; a shorter one wakes the loop to redraw what is already there.
    #[test]
    fn what_is_drawn_holds_exactly_as_long_as_holds_for_says() {
        for offset in [0, 1, 37, 79, 80, 500, 999, 1_500, 61_000, 3_601_000] {
            let now = an_instant() + TimeDelta::milliseconds(offset);
            for state in [collecting(), resting(Mark::Read), resting(Mark::Refused)] {
                let held = holds_for(state, now)
                    .expect("a project that has been read says something that expires")
                    .as_millis() as i64;
                let still = now + TimeDelta::milliseconds(held - 1);
                let over = now + TimeDelta::milliseconds(held);

                assert_eq!(
                    cell(state, now),
                    cell(state, still),
                    "{state:?} at +{offset}ms changed before its {held}ms was up"
                );
                assert_ne!(
                    cell(state, now),
                    cell(state, over),
                    "{state:?} at +{offset}ms said the same after its {held}ms was up"
                );
            }
        }
    }

    /// An age says a different thing a second later with nothing having
    /// happened, so the screen has to be redrawn for it. Held to the unit's
    /// own boundary rather than a whole unit from now: a read four days old
    /// is redrawn when it becomes five, not at some arbitrary offset from it.
    #[test]
    fn an_age_holds_only_until_its_own_units_next_boundary() {
        let held = |seconds, millis| {
            holds_for(
                resting(Mark::Read),
                an_instant() + TimeDelta::seconds(seconds) + TimeDelta::milliseconds(millis),
            )
        };

        assert_eq!(
            [
                held(0, 250),
                held(59, 0),
                held(60, 0),
                held(3_599, 0),
                held(3_600, 0),
                held(86_400, 0),
            ],
            [
                Some(Duration::from_millis(750)),
                Some(Duration::from_secs(1)),
                Some(Duration::from_secs(60)),
                Some(Duration::from_secs(1)),
                Some(Duration::from_secs(3_600)),
                Some(Duration::from_secs(86_400)),
            ]
        );
    }

    /// Never nothing. A deadline of no time at all would wake the loop, find
    /// the words unchanged, and ask for no time again.
    #[test]
    fn an_age_always_holds_for_some_time_however_the_clocks_stand() {
        for seconds in [-30, 0, 1, 59, 60, 3_600, 86_400, 500_000] {
            let held = holds_for(
                resting(Mark::Read),
                an_instant() + TimeDelta::seconds(seconds),
            );
            assert!(held > Some(Duration::ZERO), "{seconds}s: {held:?}");
        }
    }

    /// The bead's own warning. The cell says two things and holds only as
    /// long as the shorter: a project read half a second ago and being read
    /// again is due a redraw when the age turns over, well inside the frame
    /// the mark is on. A deadline of a whole frame would leave `0s ago` on
    /// the screen into its second second.
    #[test]
    fn a_cell_saying_two_things_holds_only_as_long_as_the_shorter_of_them() {
        let now = an_instant() + TimeDelta::milliseconds(960);

        assert_eq!(
            holds_for(collecting(), now),
            Some(Duration::from_millis(40)),
            "the age is 40ms from turning over and the frame is further off"
        );
        assert_eq!(
            holds_for(resting(Mark::Read), now),
            Some(Duration::from_millis(40))
        );
    }

    /// The other way round: an age that is not going to change for another
    /// day leaves the turning mark deciding when the screen is next due.
    #[test]
    fn a_day_old_project_being_read_is_redrawn_for_the_mark_rather_than_the_age() {
        let now = an_instant() + TimeDelta::seconds(86_400);

        assert_eq!(holds_for(collecting(), now), Some(FRAME));
        assert_eq!(
            holds_for(resting(Mark::Read), now),
            Some(Duration::from_secs(86_400))
        );
    }

    /// A mark at rest changes when a collection does something and never on
    /// its own, so a project with no age under it puts no deadline on the
    /// screen at all.
    #[test]
    fn a_resting_mark_over_a_project_never_read_asks_for_no_deadline() {
        let never_read = Freshness {
            mark: Mark::Read,
            read_at: None,
        };

        assert_eq!(holds_for(never_read, an_instant()), None);
    }

    /// A mark that is turning is one frame from being out of date whatever
    /// else is on the screen.
    #[test]
    fn a_turning_mark_holds_for_exactly_one_frame() {
        let starting = Freshness {
            mark: Mark::Collecting,
            read_at: None,
        };

        assert_eq!(holds_for(starting, an_instant()), Some(FRAME));
    }

    /// The mark turns, which is the whole of why it is a spinner and not a
    /// word: a still mark says a collection is running and says nothing about
    /// whether `bdi` is still alive.
    #[test]
    fn the_collecting_mark_is_on_a_different_frame_one_frame_later() {
        let frame = |at| mark(collecting(), at);

        assert_ne!(
            frame(an_instant()),
            frame(an_instant() + TimeDelta::milliseconds(FRAME_MS))
        );
    }

    /// Every frame is drawn before any is drawn twice, so the mark turns
    /// evenly rather than resting on one of them.
    #[test]
    fn the_mark_turns_through_every_frame_before_it_comes_round_again() {
        let frames: Vec<&str> = (0..TURNING.len() as i64)
            .map(|frame| {
                mark(
                    collecting(),
                    an_instant() + TimeDelta::milliseconds(frame * FRAME_MS),
                )
            })
            .collect();

        let mut distinct = frames.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(distinct.len(), frames.len(), "{frames:?}");
        assert_eq!(
            mark(
                collecting(),
                an_instant() + TimeDelta::milliseconds(TURNING.len() as i64 * FRAME_MS)
            ),
            frames[0]
        );
    }

    /// Two redraws inside one frame's worth of time show the same frame. A
    /// keystroke redraws the screen, and a mark counted per redraw would jump
    /// forward for one.
    #[test]
    fn a_redraw_within_one_frame_shows_the_frame_already_on_the_screen() {
        let frame = |at| mark(collecting(), at);
        let at = an_instant() + TimeDelta::milliseconds(FRAME_MS / 2);

        assert_eq!(frame(at), frame(at + TimeDelta::milliseconds(1)));
    }

    /// A mark at rest is a still glyph and stays on whichever one its last
    /// collection earned: a resting mark that turned would read as a
    /// collection running.
    #[test]
    fn a_mark_at_rest_is_the_same_glyph_a_frame_later() {
        for at_rest in [Mark::Read, Mark::Refused] {
            let frame = |at| mark(resting(at_rest), at);

            assert_eq!(
                frame(an_instant()),
                frame(an_instant() + TimeDelta::milliseconds(FRAME_MS))
            );
        }
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
    ///
    /// The notices are not among them any more: one of them names the pid of
    /// the process holding the inbound channel, which no `&'static str` can
    /// carry. What it carries instead is a `u32` the kernel gave, so nothing
    /// a tool wrote can reach it by any path — and both notices are in
    /// `every_phrase`, which is where the guarantee is now made.
    #[test]
    fn the_failure_phrases_are_static() {
        let _: fn(TrackerFailure) -> &'static str = tracker_failure;
        let _: fn() -> &'static str = truncated;
        let _: fn() -> &'static str = no_live_panes;
        let _: fn() -> &'static str = panes_may_be_incomplete;
        let _: fn(JoinSource) -> Option<&'static str> = join_caveat;
        let _: fn() -> &'static str = no_bead_to_tail;
        let _: fn() -> &'static str = no_agent_to_tail;
        let _: fn() -> &'static str = no_herdr_to_tail;
        let _: fn() -> &'static str = pane_being_read;
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

    /// `bdi-7ao.61`: a reader who knows only that they are polled cannot act.
    /// Knowing it is another `bdi` is what they can act on, because it is the
    /// only cause of this that closing something puts right.
    #[test]
    fn a_socket_another_bdi_holds_says_so_rather_than_only_what_it_cost() {
        let said = notice(Notice::AnotherBdiHadTheInboundChannel);

        assert!(said.contains("another bdi"), "{said}");
        assert!(said.contains("polled"), "{said}");
    }

    /// The cause is what survives the cut, not the cost. A reader left with
    /// only *polled* has what the notice this one replaced already gave them,
    /// and still nothing to do about it.
    #[test]
    fn the_brief_words_keep_the_cause_and_give_up_the_cost() {
        let said = brief_notice(Notice::AnotherBdiHadTheInboundChannel);

        assert!(said.contains("another bdi"), "{said}");
        assert!(
            columns(&[Span::raw(said)])
                <= columns(&[Span::raw(brief_notice(Notice::NoInboundChannel))]),
            "the brief words are what fit a forty-column foot: {said}"
        );
    }

    /// Two ways of losing the channel that want different things of the
    /// reader: one is theirs to fix by closing a process, the other is not.
    #[test]
    fn losing_the_channel_to_another_bdi_reads_differently_from_never_having_one() {
        assert_ne!(
            notice(Notice::NoInboundChannel),
            notice(Notice::AnotherBdiHadTheInboundChannel)
        );
        assert_ne!(
            brief_notice(Notice::NoInboundChannel),
            brief_notice(Notice::AnotherBdiHadTheInboundChannel)
        );
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
                caption: None,
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
            cycle(1),
            elided(1),
            unfinished_beneath(1),
            agents_beneath(1),
            anomalies_beneath(1),
            truncated_nodes(1),
            failed_projects(1),
            conflicts(1),
            hidden_trees(1, 0),
            hidden_trees(1, 1),
            unattributed(1),
            unconfigured(1),
            anomaly(&Anomaly::StaleClaim { days: 1 }),
        ] {
            for plural in [
                "beads",
                "days",
                "projects",
                "trees",
                "panes",
                "conflicts",
                "agents",
            ] {
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

    /// A contested pane is awarded to nobody, so nothing else on the screen
    /// says what it is working on — and that is the one thing that tells the
    /// live claim from the stale ones. It is said in the pane's own words:
    /// `bdi` reads no bead id out of it and picks no winner.
    #[test]
    fn a_contested_pane_says_what_it_is_working_on_in_its_own_words() {
        let said = conflict(&Conflict::SeveralBeadsNameOnePane {
            pane: "wCM:p9".into(),
            caption: Some("nix-9670s.1: rebuild the installer image".into()),
            beads: vec![key("nix-9670s.20"), key("nix-9670s.1")],
        });

        assert!(
            said.contains("nix-9670s.1: rebuild the installer image"),
            "{said}"
        );
        assert!(
            said.contains('\u{201c}'),
            "the pane's words are marked as its own: {said}"
        );
    }

    /// The sentence is cut from the right at the width it is drawn in, so the
    /// part that settles which claim is live has to come before the roll of
    /// claims, which is the part a reader can lose and still act.
    #[test]
    fn a_contested_panes_own_words_come_before_the_claims_on_it() {
        let said = conflict(&Conflict::SeveralBeadsNameOnePane {
            pane: "wCM:p9".into(),
            caption: Some("nix-9670s.1: rebuild the installer image".into()),
            beads: vec![key("nix-9670s.20"), key("nix-9670s.1")],
        });

        let words = said
            .find("rebuild the installer image")
            .expect("the pane's own words are in the sentence");
        let claims = said
            .find("beads name it")
            .expect("so is the roll of claims on it");

        assert!(words < claims, "{said}");
    }

    /// A pane herdr reports nothing about still contests, and the sentence
    /// says what it has rather than leaving an empty pair of quotes standing
    /// for words nobody wrote.
    #[test]
    fn a_contested_pane_with_nothing_to_say_is_described_without_it() {
        let said = conflict(&Conflict::SeveralBeadsNameOnePane {
            pane: "wCM:p9".into(),
            caption: None,
            beads: vec![key("nix-9670s.20"), key("nix-9670s.1")],
        });

        assert!(!said.contains('\u{201c}'), "{said}");
        assert!(said.contains("wCM:p9"), "{said}");
        assert!(said.contains('2'), "{said}");
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
