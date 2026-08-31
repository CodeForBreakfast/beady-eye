//! The rules that say a bead's claim and the pane behind it have come apart.

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::config::Anomalies;
use crate::model::join::{AgentRef, Conflict};
use crate::model::types::{Bead, Status};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "rule", rename_all = "kebab-case")]
pub enum Anomaly {
    /// `in_progress` and untouched for longer than the configured window.
    StaleClaim { days: i64 },
    /// `in_progress` with no pane behind it.
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
}

/// Every anomaly rule that fires on one bead, in a fixed order.
///
/// A node carries all of them, not the first: an old claim whose agent has
/// died is both an orphan claim and a stale one, and the age is the part that
/// says whether to care. `now` is a parameter so the age rule is testable.
pub fn detect(
    bead: &Bead,
    agent: Option<&AgentRef>,
    refused: Option<&Conflict>,
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

    if agent.is_none() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::bd::parse_beads;
    use crate::model::join::JoinSource;
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
            pane: "w:p1".into(),
            pane_status,
            title: None,
            source: JoinSource::AgentPane,
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
            &Anomalies::default(),
            now(),
        );
        assert_eq!(got, vec![Anomaly::StalePane]);
    }

    #[test]
    fn a_pane_that_has_finished_still_outlived_its_closed_bead() {
        let got = detect(
            &bead("closed", YESTERDAY),
            Some(&pane(PaneStatus::Done)),
            None,
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

    #[test]
    fn a_fresh_claim_with_no_pane_is_an_orphan_claim() {
        let got = detect(
            &bead("in_progress", YESTERDAY),
            None,
            None,
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
            &Anomalies::default(),
            now(),
        );
        assert_eq!(
            got,
            vec![orphan(), Anomaly::StaleClaim { days: 60 }],
            "reporting only the orphan throws away how long it has sat there"
        );
    }

    #[test]
    fn a_recent_claim_with_a_pane_is_clean() {
        let got = detect(
            &bead("in_progress", YESTERDAY),
            Some(&live()),
            None,
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
            &Anomalies::default(),
            now(),
        );
        assert_eq!(inside, Vec::new(), "29 days is inside a 30-day window");

        let reached = detect(
            &bead("in_progress", THIRTY_DAYS_AGO),
            Some(&live()),
            None,
            &Anomalies::default(),
            now(),
        );
        assert_eq!(reached, vec![Anomaly::StaleClaim { days: 30 }]);
    }

    #[test]
    fn the_window_is_configurable() {
        let wide = Anomalies {
            stale_claim_days: 90,
        };
        let got = detect(
            &bead("in_progress", SIXTY_DAYS_AGO),
            Some(&live()),
            None,
            &wide,
            now(),
        );
        assert_eq!(got, Vec::new(), "60 days is inside a 90-day window");

        let narrow = Anomalies {
            stale_claim_days: 7,
        };
        let got = detect(
            &bead("in_progress", SIXTY_DAYS_AGO),
            Some(&live()),
            None,
            &narrow,
            now(),
        );
        assert_eq!(got, vec![Anomaly::StaleClaim { days: 60 }]);
    }

    #[test]
    fn a_claim_bd_gives_no_update_time_for_has_no_age_to_judge() {
        let mut b = bead("in_progress", SIXTY_DAYS_AGO);
        b.updated_at = None;

        let got = detect(&b, Some(&live()), None, &Anomalies::default(), now());
        assert_eq!(got, Vec::new());
    }

    // ---- the statuses that hold no claim --------------------------------

    #[test]
    fn a_blocked_bead_with_a_live_pane_is_never_flagged() {
        let got = detect(
            &bead("blocked", SIXTY_DAYS_AGO),
            Some(&live()),
            None,
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
            &Anomalies::default(),
            now(),
        );
        assert_eq!(got, Vec::new());
    }

    // ---- the wire ------------------------------------------------------

    #[test]
    fn the_rules_serialise_under_the_names_the_contract_publishes() {
        let json = serde_json::to_string(&vec![
            Anomaly::StalePane,
            orphan(),
            Anomaly::StaleClaim { days: 60 },
        ])
        .expect("the anomalies serialise");

        assert_eq!(
            json,
            r#"[{"rule":"stale-pane"},{"rule":"orphan-claim"},{"rule":"stale-claim","days":60}]"#
        );
    }
}
