//! The rules that say a bead's claim and the pane behind it have come apart,
//! or that a bead waits with nobody watching it.

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::config::Anomalies;
use crate::model::join::{AgentRef, Conflict};
use crate::model::snapshot::ProviderState;
use crate::model::types::{Bead, Status};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "rule", rename_all = "kebab-case")]
pub enum Anomaly {
    /// `in_progress` and untouched for longer than the configured window.
    StaleClaim { days: i64 },
    /// `in_progress` with no pane behind it or behind any bead above it.
    ///
    /// Where the bead named a live pane the join would not award it, that
    /// refusal is the reason and travels with the rule; where it named
    /// nothing, or nothing live, there is no reason to carry.
    OrphanClaim {
        #[serde(skip_serializing_if = "Option::is_none")]
        refused: Option<Conflict>,
    },
    /// Closed, but its pane is still there.
    StalePane,
    /// Not closed, selected as wanting an agent to watch it, and watched by
    /// none, so nothing is woken when it moves.
    Unwatched,
}

/// Every anomaly rule that fires on one bead, in a fixed order.
///
/// A node carries all of them, not the first: an old claim whose agent has
/// died is both an orphan claim and a stale one, and the age is the part that
/// says whether to care. `now` is a parameter so the age rule is testable.
///
/// `agents` is how the run went for panes, because only one of these rules
/// reads bd alone. The rest need a pane, and where nothing answered for
/// panes there is no pane fact to read either way.
///
/// `pane_out_of_reach` is the same ignorance one bead at a time: the run
/// answered for panes, and the pane or agent this bead named is in a session
/// that did not. It is asked per bead rather than per run because the
/// sessions that did answer are still worth reading — a run that fell silent
/// about every claim because one session hiccuped would throw away what it
/// does know to avoid saying what it does not.
///
/// `seat_above` says a seat sits on a bead above this one in the tree, or on
/// one whose pane the run could not ask about. A seat names its pane on the
/// lowest bead covering its work and sets the beads beneath to `in_progress`
/// without naming it on them, so a claim under a seat has its agent.
///
/// A claim whose lease is live has a holder renewing it, perhaps from another
/// machine whose panes this run never reads, so it is not an orphan either.
#[allow(clippy::too_many_arguments)]
pub fn detect(
    bead: &Bead,
    agent: Option<&AgentRef>,
    refused: Option<&Conflict>,
    agents: ProviderState,
    pane_out_of_reach: bool,
    seat_above: bool,
    cfg: &Anomalies,
    now: DateTime<Utc>,
) -> Vec<Anomaly> {
    if bead.status.is_closed() {
        return agent.map(|_| Anomaly::StalePane).into_iter().collect();
    }

    if bead.status != Status::InProgress {
        // An agent parked on blocked or open work is ordinary, not an anomaly.
        return Vec::new();
    }

    let mut fired = Vec::new();

    if agents.answered()
        && agent.is_none()
        && !pane_out_of_reach
        && !seat_above
        && !bead.claim_is_live(now)
    {
        fired.push(Anomaly::OrphanClaim {
            refused: refused.cloned(),
        });
    }

    let untouched_for = bead.updated_at.map(|updated| (now - updated).num_days());
    if let Some(days) = untouched_for.filter(|days| *days >= cfg.stale_claim_days) {
        fired.push(Anomaly::StaleClaim { days });
    }

    fired
}

/// The `unwatched` rule on `bead`, which `project`'s tracker holds.
///
/// `anyone_watching` is what [`crate::model::watching::anyone_watches`] says
/// of the bead, which counts the agent on its claim. Where nothing could say
/// who watches, the rule cannot say nobody does.
pub fn unwatched(
    project: &str,
    bead: &Bead,
    anyone_watching: Option<bool>,
    cfg: &Anomalies,
) -> Option<Anomaly> {
    let nobody_watches = anyone_watching == Some(false);
    (nobody_watches && !bead.status.is_closed() && cfg.wants_watched(project, bead))
        .then_some(Anomaly::Unwatched)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::bd::parse_beads;
    use crate::model::join::JoinSource;
    use crate::model::types::testing::key;
    use crate::model::types::PaneStatus;
    use pretty_assertions::assert_eq;

    const NOW: &str = "2026-08-30T12:00:00Z";
    const SIXTY_DAYS_AGO: &str = "2026-07-01T12:00:00Z";
    const THIRTY_DAYS_AGO: &str = "2026-07-31T12:00:00Z";
    const TWENTY_NINE_DAYS_AGO: &str = "2026-08-01T12:00:00Z";
    const YESTERDAY: &str = "2026-08-29T12:00:00Z";

    /// A bead as bd writes it, so the fields the rules read come through the
    /// real parser rather than a struct literal that can drift from it.
    fn bead(status: &str, updated_at: &str) -> Bead {
        parse_beads(&format!(
            r#"[{{"id":"p-1","title":"work","status":"{status}",
                  "updated_at":"{updated_at}"}}]"#
        ))
        .expect("the row parses")
        .remove(0)
    }

    fn pane(pane_status: PaneStatus) -> AgentRef {
        AgentRef {
            pane: key("w:p1"),
            pane_status,
            title: None,
            source: JoinSource::AgentPane,
            agent_session: None,
        }
    }

    fn live() -> AgentRef {
        pane(PaneStatus::Working)
    }

    fn now() -> DateTime<Utc> {
        NOW.parse().expect("the clock parses")
    }

    /// A claim with nothing behind it and no refusal to explain it.
    fn orphan() -> Anomaly {
        Anomaly::OrphanClaim { refused: None }
    }

    // ---- closed beads: stale-pane ---------------------------------------

    #[test]
    fn a_closed_bead_whose_pane_is_still_there_is_a_stale_pane() {
        let got = detect(
            &bead("closed", YESTERDAY),
            Some(&live()),
            None,
            ProviderState::Answering,
            false,
            false,
            &Anomalies::default(),
            now(),
        );
        assert_eq!(got, vec![Anomaly::StalePane]);
    }

    /// A stale pane is a claim that outlived the bead somebody closed. bd
    /// keeps a pinned bead indefinitely and never closes it, so a pane on one
    /// has outlived nothing.
    #[test]
    fn a_pane_on_a_pinned_bead_is_not_a_stale_pane() {
        let got = detect(
            &bead("pinned", YESTERDAY),
            Some(&live()),
            None,
            ProviderState::Answering,
            false,
            false,
            &Anomalies::default(),
            now(),
        );
        assert_eq!(got, Vec::new());
    }

    #[test]
    fn a_pane_that_has_finished_still_outlived_its_closed_bead() {
        let got = detect(
            &bead("closed", YESTERDAY),
            Some(&pane(PaneStatus::Done)),
            None,
            ProviderState::Answering,
            false,
            false,
            &Anomalies::default(),
            now(),
        );
        assert_eq!(
            got,
            vec![Anomaly::StalePane],
            "the rule keys on a pane being there, not on what it is doing"
        );
    }

    #[test]
    fn a_closed_bead_with_no_pane_is_clean() {
        let got = detect(
            &bead("closed", YESTERDAY),
            None,
            None,
            ProviderState::Answering,
            false,
            false,
            &Anomalies::default(),
            now(),
        );
        assert_eq!(got, Vec::new());
    }

    #[test]
    fn a_long_closed_bead_is_never_a_stale_claim() {
        let got = detect(
            &bead("closed", SIXTY_DAYS_AGO),
            Some(&live()),
            None,
            ProviderState::Answering,
            false,
            false,
            &Anomalies::default(),
            now(),
        );
        assert_eq!(
            got,
            vec![Anomaly::StalePane],
            "the age rule is about claims, and a closed bead holds none"
        );
    }

    // ---- claims: orphan-claim and stale-claim ---------------------------

    /// The other side of the gate: a provider that answered and awarded the
    /// bead no pane has said something, and the rule reads it.
    #[test]
    fn a_fresh_claim_with_no_pane_is_an_orphan_claim() {
        let got = detect(
            &bead("in_progress", YESTERDAY),
            None,
            None,
            ProviderState::Answering,
            false,
            false,
            &Anomalies::default(),
            now(),
        );
        assert_eq!(got, vec![orphan()]);
    }

    /// A claimed bead whose lease runs out at `expires`.
    fn leased_until(expires: &str) -> Bead {
        parse_beads(&format!(
            r#"[{{"id":"p-1","title":"work","status":"in_progress",
                  "updated_at":"{YESTERDAY}","assignee":"Rowan Ash",
                  "lease_expires_at":"{expires}"}}]"#
        ))
        .expect("the row parses")
        .remove(0)
    }

    /// A lease is renewed only while its holder runs, wherever it runs, so a
    /// live one says a seat is working the bead even where no pane here does.
    #[test]
    fn a_claim_whose_lease_is_live_is_not_an_orphan() {
        let got = detect(
            &leased_until("2026-08-30T12:05:00Z"),
            None,
            None,
            ProviderState::Answering,
            false,
            false,
            &Anomalies::default(),
            now(),
        );
        assert_eq!(got, Vec::new());
    }

    /// A lease that ran out is a holder that stopped renewing it, and the
    /// instant itself is the first one past the lease.
    #[test]
    fn a_claim_whose_lease_has_run_out_is_an_orphan() {
        let got = detect(
            &leased_until(NOW),
            None,
            None,
            ProviderState::Answering,
            false,
            false,
            &Anomalies::default(),
            now(),
        );
        assert_eq!(got, vec![orphan()]);
    }

    #[test]
    fn a_claim_untouched_past_the_window_is_a_stale_claim() {
        let got = detect(
            &bead("in_progress", SIXTY_DAYS_AGO),
            Some(&live()),
            None,
            ProviderState::Answering,
            false,
            false,
            &Anomalies::default(),
            now(),
        );
        assert_eq!(got, vec![Anomaly::StaleClaim { days: 60 }]);
    }

    #[test]
    fn an_old_claim_whose_agent_has_died_is_both_and_keeps_its_age() {
        let got = detect(
            &bead("in_progress", SIXTY_DAYS_AGO),
            None,
            None,
            ProviderState::Answering,
            false,
            false,
            &Anomalies::default(),
            now(),
        );
        assert_eq!(
            got,
            vec![orphan(), Anomaly::StaleClaim { days: 60 }],
            "reporting only the orphan throws away how long it has sat there"
        );
    }

    /// A claim whose pane the run could not ask about is not a claim whose
    /// pane is gone, and only the second is an orphan. The stale-claim rule
    /// reads bd alone, so it still fires on the same bead — a run that knows
    /// nothing about panes still knows how long the row has sat there.
    #[test]
    fn a_claim_whose_pane_is_out_of_reach_is_not_orphaned_and_is_still_aged() {
        let got = detect(
            &bead("in_progress", SIXTY_DAYS_AGO),
            None,
            None,
            ProviderState::Answering,
            true,
            false,
            &Anomalies::default(),
            now(),
        );
        assert_eq!(got, vec![Anomaly::StaleClaim { days: 60 }]);
    }

    /// The seat convention: the pane is named on the bead covering the work,
    /// and the steps beneath it are in progress with nothing of their own to
    /// name. The age rule reads bd alone, so a covered claim still goes stale.
    #[test]
    fn a_claim_beneath_a_seat_is_not_orphaned_and_is_still_aged() {
        let covered = detect(
            &bead("in_progress", YESTERDAY),
            None,
            None,
            ProviderState::Answering,
            false,
            true,
            &Anomalies::default(),
            now(),
        );
        assert_eq!(covered, Vec::new());

        let neglected = detect(
            &bead("in_progress", SIXTY_DAYS_AGO),
            None,
            None,
            ProviderState::Answering,
            false,
            true,
            &Anomalies::default(),
            now(),
        );
        assert_eq!(neglected, vec![Anomaly::StaleClaim { days: 60 }]);
    }

    #[test]
    fn a_recent_claim_with_a_pane_is_clean() {
        let got = detect(
            &bead("in_progress", YESTERDAY),
            Some(&live()),
            None,
            ProviderState::Answering,
            false,
            false,
            &Anomalies::default(),
            now(),
        );
        assert_eq!(got, Vec::new());
    }

    #[test]
    fn the_window_fires_on_the_day_it_is_reached() {
        let inside = detect(
            &bead("in_progress", TWENTY_NINE_DAYS_AGO),
            Some(&live()),
            None,
            ProviderState::Answering,
            false,
            false,
            &Anomalies::default(),
            now(),
        );
        assert_eq!(inside, Vec::new(), "29 days is inside a 30-day window");

        let reached = detect(
            &bead("in_progress", THIRTY_DAYS_AGO),
            Some(&live()),
            None,
            ProviderState::Answering,
            false,
            false,
            &Anomalies::default(),
            now(),
        );
        assert_eq!(reached, vec![Anomaly::StaleClaim { days: 30 }]);
    }

    #[test]
    fn the_window_is_configurable() {
        let wide = Anomalies {
            stale_claim_days: 90,
            ..Anomalies::default()
        };
        let got = detect(
            &bead("in_progress", SIXTY_DAYS_AGO),
            Some(&live()),
            None,
            ProviderState::Answering,
            false,
            false,
            &wide,
            now(),
        );
        assert_eq!(got, Vec::new(), "60 days is inside a 90-day window");

        let narrow = Anomalies {
            stale_claim_days: 7,
            ..Anomalies::default()
        };
        let got = detect(
            &bead("in_progress", SIXTY_DAYS_AGO),
            Some(&live()),
            None,
            ProviderState::Answering,
            false,
            false,
            &narrow,
            now(),
        );
        assert_eq!(got, vec![Anomaly::StaleClaim { days: 60 }]);
    }

    #[test]
    fn a_claim_bd_gives_no_update_time_for_has_no_age_to_judge() {
        let mut b = bead("in_progress", SIXTY_DAYS_AGO);
        b.updated_at = None;

        let got = detect(
            &b,
            Some(&live()),
            None,
            ProviderState::Answering,
            false,
            false,
            &Anomalies::default(),
            now(),
        );
        assert_eq!(got, Vec::new());
    }

    // ---- the statuses that hold no claim --------------------------------

    #[test]
    fn a_blocked_bead_with_a_live_pane_is_never_flagged() {
        let got = detect(
            &bead("blocked", SIXTY_DAYS_AGO),
            Some(&live()),
            None,
            ProviderState::Answering,
            false,
            false,
            &Anomalies::default(),
            now(),
        );
        assert_eq!(got, Vec::new(), "an agent parked on blocked work is normal");
    }

    #[test]
    fn a_blocked_bead_with_no_pane_is_never_flagged() {
        let got = detect(
            &bead("blocked", SIXTY_DAYS_AGO),
            None,
            None,
            ProviderState::Answering,
            false,
            false,
            &Anomalies::default(),
            now(),
        );
        assert_eq!(got, Vec::new());
    }

    /// A hooked bead is a claim, but `bd stale` leaves it out, and a hook can
    /// outlive the session of the agent it belongs to.
    #[test]
    fn a_hooked_bead_with_no_pane_is_never_flagged() {
        let got = detect(
            &bead("hooked", SIXTY_DAYS_AGO),
            None,
            None,
            ProviderState::Answering,
            false,
            false,
            &Anomalies::default(),
            now(),
        );
        assert_eq!(got, Vec::new());
    }

    #[test]
    fn an_open_bead_is_never_flagged() {
        let got = detect(
            &bead("open", SIXTY_DAYS_AGO),
            None,
            None,
            ProviderState::Answering,
            false,
            false,
            &Anomalies::default(),
            now(),
        );
        assert_eq!(got, Vec::new());
    }

    #[test]
    fn a_deferred_bead_is_never_flagged() {
        let got = detect(
            &bead("deferred", SIXTY_DAYS_AGO),
            Some(&live()),
            None,
            ProviderState::Answering,
            false,
            false,
            &Anomalies::default(),
            now(),
        );
        assert_eq!(got, Vec::new());
    }

    // ---- beads that want watching: unwatched ----------------------------

    /// Every bead labelled `human`, and every gate labelled `human-move`.
    fn waiting_on_someone() -> Anomalies {
        crate::config::Config::from_toml(
            r#"
[[projects]]
name = "dunwich"
path = "/srv/work/dunwich"

[[anomalies.unwatched]]
when = { labels = "human" }

[[anomalies.unwatched]]
when = { labels = "human-move", issue_type = "gate" }
"#,
        )
        .expect("the config parses")
        .anomalies
    }

    fn labelled(issue_type: &str, status: &str, label: &str) -> Bead {
        parse_beads(&format!(
            r#"[{{"id":"dun-4","title":"which dish","status":"{status}",
                  "issue_type":"{issue_type}","labels":["{label}"],
                  "updated_at":"{YESTERDAY}"}}]"#
        ))
        .expect("the row parses")
        .remove(0)
    }

    fn question() -> Bead {
        labelled("task", "open", "human")
    }

    fn merge_gate() -> Bead {
        labelled("gate", "open", "human-move")
    }

    /// What `watching::anyone_watches` says of a bead nobody watches, of one
    /// some agent watches, and where nothing could say.
    const NOBODY: Option<bool> = Some(false);
    const SOMEONE: Option<bool> = Some(true);
    const UNKNOWN: Option<bool> = None;

    #[test]
    fn a_bead_the_config_selects_that_nobody_watches_is_unwatched() {
        let got = unwatched("dunwich", &question(), NOBODY, &waiting_on_someone());
        assert_eq!(got, Some(Anomaly::Unwatched));
    }

    #[test]
    fn a_gate_the_config_selects_that_nobody_watches_is_unwatched() {
        let got = unwatched("dunwich", &merge_gate(), NOBODY, &waiting_on_someone());
        assert_eq!(got, Some(Anomaly::Unwatched));
    }

    #[test]
    fn a_selected_bead_an_agent_watches_is_not_unwatched() {
        let config = waiting_on_someone();
        assert_eq!(unwatched("dunwich", &question(), SOMEONE, &config), None);
        assert_eq!(unwatched("dunwich", &merge_gate(), SOMEONE, &config), None);
    }

    /// Where no watcher answered, nothing can say the bead is unwatched.
    #[test]
    fn a_selected_bead_whose_watches_are_unknown_is_not_flagged() {
        let got = unwatched("dunwich", &question(), UNKNOWN, &waiting_on_someone());
        assert_eq!(got, None);
    }

    #[test]
    fn a_bead_the_config_does_not_select_is_never_unwatched() {
        let config = waiting_on_someone();
        let unlabelled = labelled("task", "open", "chore");
        assert_eq!(unwatched("dunwich", &unlabelled, NOBODY, &config), None);
        let plain = labelled("task", "open", "human-move");
        assert_eq!(
            unwatched("dunwich", &plain, NOBODY, &config),
            None,
            "the gate rule selects gates alone"
        );
    }

    #[test]
    fn with_no_rule_configured_no_bead_is_unwatched() {
        let got = unwatched("dunwich", &question(), NOBODY, &Anomalies::default());
        assert_eq!(got, None);
    }

    /// A closed bead waits on nobody, so nobody need watch it.
    #[test]
    fn a_closed_bead_the_config_selects_is_never_unwatched() {
        let answered = labelled("task", "closed", "human");
        let got = unwatched("dunwich", &answered, NOBODY, &waiting_on_someone());
        assert_eq!(got, None);
    }

    // ---- the wire ------------------------------------------------------

    #[test]
    fn unwatched_is_written_as_its_rule_alone() {
        assert_eq!(
            serde_json::to_value(Anomaly::Unwatched).expect("an anomaly serialises"),
            serde_json::json!({ "rule": "unwatched" })
        );
    }
}
