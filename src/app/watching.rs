//! What a consumer watches on the listener's socket, and the lines that tell
//! it what it watches.
//!
//! Every line sent about a watch is one JSON object whose `line` says which
//! kind it is. `docs/design.md`'s *Watching* is the protocol.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::json;

use crate::collect::listened::PROTOCOL;
use crate::config::Reach;
use crate::model::snapshot::{TrackerFailure, TrackerState};

use super::listener::Held;

/// What one line asks the listener to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Watch {
    /// `watch`: every project the listener reads.
    Everything,
    /// `watch <project>`, or `watch-all <project>`, which starts from the
    /// closed beads too.
    Project { project: String, closed_too: bool },
    /// `watch <project> <id>`.
    Bead { project: String, id: String },
}

/// Why a watch line was not served.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Refusal {
    /// A project the listener does not read.
    UnknownProject,
    /// A `watch` or `watch-all` in none of the four forms.
    Malformed,
}

const WATCH: &str = "watch";
const WATCH_ALL: &str = "watch-all";

/// What `line` asks, where it is a watch line at all. Any other line is a
/// producer's.
pub fn asked(line: &str) -> Option<Result<Watch, Refusal>> {
    let mut words = line.split_whitespace();
    let first = words.next()?;
    let rest: Vec<&str> = words.collect();
    let watch = match (first, rest.as_slice()) {
        (WATCH, []) => Watch::Everything,
        (WATCH, [project]) => Watch::Project {
            project: project.to_string(),
            closed_too: false,
        },
        (WATCH_ALL, [project]) => Watch::Project {
            project: project.to_string(),
            closed_too: true,
        },
        (WATCH, [project, id]) => Watch::Bead {
            project: project.to_string(),
            id: id.to_string(),
        },
        (WATCH | WATCH_ALL, _) => return Some(Err(Refusal::Malformed)),
        _ => return None,
    };
    Some(Ok(watch))
}

/// What one connection watches in one project, and what it has been told of
/// each bead there.
#[derive(Debug, Default)]
pub struct Interest {
    /// Every bead in the project, rather than only those `named`.
    whole: bool,
    /// Where the whole project is watched, whether it starts from the closed
    /// beads too.
    closed_too: bool,
    named: BTreeSet<String>,
    known: BTreeMap<String, Known>,
    /// The beads the connection was told are gone and that have not been
    /// held since, so a later line does not tell it again.
    told_gone: BTreeSet<String>,
    /// A watch line has widened this since the project's beads were last
    /// caught up with, so the next beads it is given are where that watch
    /// starts rather than a change.
    starting: bool,
}

/// One bead as a connection knows it.
#[derive(Debug)]
struct Known {
    held: Held,
    /// Whether the connection was sent it. A closed bead a `watch` started
    /// past is known untold, so a later change to it is still sent.
    told: bool,
}

impl Interest {
    pub fn widen(&mut self, watch: &Watch) {
        match watch {
            Watch::Everything => self.whole = true,
            Watch::Project { closed_too, .. } => {
                self.whole = true;
                self.closed_too |= closed_too;
            }
            Watch::Bead { id, .. } => {
                self.named.insert(id.clone());
            }
        }
        self.starting = true;
    }

    /// The bead and gone lines that bring this connection up to `beads`, the
    /// whole of what `project` holds.
    pub fn catch_up(&mut self, project: &str, beads: &BTreeMap<String, Held>) -> Vec<String> {
        let starting = std::mem::take(&mut self.starting);
        let mut lines = Vec::new();

        for (id, held) in beads {
            if !self.covers(id) {
                continue;
            }
            let starts_from = self.starts_from(id, held);
            let (tell, told_before) = match self.known.get(id) {
                None => (!starting || starts_from, false),
                Some(known) => (
                    differs(&known.held, held) || (!known.told && starts_from),
                    known.told,
                ),
            };
            if tell {
                lines.push(bead_line(project, held));
            }
            let known = Known {
                held: held.clone(),
                told: tell || told_before,
            };
            self.known.insert(id.clone(), known);
        }

        let mut gone = BTreeSet::new();
        self.known.retain(|id, known| {
            let held = beads.contains_key(id);
            if !held && known.told {
                gone.insert(id.clone());
            }
            held
        });
        if starting {
            gone.extend(
                self.named
                    .iter()
                    .filter(|id| !beads.contains_key(*id) && !self.told_gone.contains(*id))
                    .cloned(),
            );
        }
        lines.extend(gone.iter().map(|id| gone_line(project, id)));
        self.told_gone.retain(|id| !beads.contains_key(id));
        self.told_gone.extend(gone);
        lines
    }

    fn covers(&self, id: &str) -> bool {
        self.whole || self.named.contains(id)
    }

    /// Whether a watch starting now is sent this bead.
    fn starts_from(&self, id: &str, held: &Held) -> bool {
        self.closed_too || self.named.contains(id) || !is_closed(held)
    }
}

/// The fields a claim's heartbeat writes, which nothing a consumer acts on
/// reads.
const HEARTBEAT: [&str; 2] = ["lease_expires_at", "heartbeat_at"];

fn differs(was: &Held, is: &Held) -> bool {
    was.ready != is.ready
        || was.blocked_by != is.blocked_by
        || was.bd != is.bd
        || (was.row != is.row && acted_on(was) != acted_on(is))
}

fn acted_on(held: &Held) -> Option<Vec<(&String, &serde_json::Value)>> {
    held.row.as_deref().map(|row| {
        row.iter()
            .filter(|(field, _)| !HEARTBEAT.contains(&field.as_str()))
            .collect()
    })
}

fn is_closed(held: &Held) -> bool {
    held.row
        .as_ref()
        .and_then(|row| row.get("status"))
        .and_then(serde_json::Value::as_str)
        == Some("closed")
}

fn bead_line(project: &str, held: &Held) -> String {
    json!({
        "line": "bead",
        "project": project,
        "ready": held.ready,
        "blocked_by": held.blocked_by,
        "bd": held.bd,
        "row": held.row,
    })
    .to_string()
}

fn gone_line(project: &str, id: &str) -> String {
    json!({ "line": "gone", "project": project, "id": id }).to_string()
}

/// How current `project`'s beads are: as of `as_of`, and whether the last
/// attempt to reach its tracker failed, which the listener reaches as
/// `reach` says. It carries the protocol every line about a watch is written
/// in.
pub fn freshness_line(
    project: &str,
    reach: &Reach,
    as_of: Option<DateTime<Utc>>,
    unreachable: Option<&TrackerFailure>,
) -> String {
    let tracker = unreachable.map_or(TrackerState::Ok, |failure| {
        TrackerState::Unreachable(failure.clone())
    });
    json!({
        "line": "freshness",
        "project": project,
        "as_of": as_of,
        "tracker": tracker,
        "events": "off",
        "protocol": PROTOCOL,
        "reach": reach,
    })
    .to_string()
}

pub fn refused_line(asked: &str, refusal: Refusal) -> String {
    json!({ "line": "refused", "asked": asked.trim(), "reason": refusal }).to_string()
}

pub const ALIVE_LINE: &str = r#"{"line":"alive"}"#;

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::Value;

    use super::*;
    use crate::app::listener::BeadReadiness;

    fn a_bead(id: &str, status: &str) -> (String, Held) {
        let row = json!({ "id": id, "status": status });
        let Value::Object(row) = row else {
            unreachable!("a row is an object")
        };
        (
            id.to_string(),
            Held {
                row: Some(Arc::new(row)),
                ready: false,
                blocked_by: Vec::new(),
                bd: BeadReadiness::default(),
            },
        )
    }

    fn beads(of: &[(&str, &str)]) -> BTreeMap<String, Held> {
        of.iter().map(|(id, status)| a_bead(id, status)).collect()
    }

    fn watching(watches: &[Watch]) -> Interest {
        let mut interest = Interest::default();
        for watch in watches {
            interest.widen(watch);
        }
        interest
    }

    fn project(closed_too: bool) -> Watch {
        Watch::Project {
            project: "dunwich".to_string(),
            closed_too,
        }
    }

    fn bead(id: &str) -> Watch {
        Watch::Bead {
            project: "dunwich".to_string(),
            id: id.to_string(),
        }
    }

    /// What each line says, as `<kind> <id>`.
    fn said(lines: &[String]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                let line: Value = serde_json::from_str(line).expect("each line is JSON");
                let id = line["row"]["id"].as_str().or(line["id"].as_str());
                format!(
                    "{} {}",
                    line["line"].as_str().unwrap_or_default(),
                    id.unwrap_or_default()
                )
            })
            .collect()
    }

    #[test]
    fn each_of_the_four_forms_is_a_watch() {
        assert_eq!(asked("watch\n"), Some(Ok(Watch::Everything)));
        assert_eq!(asked("watch dunwich"), Some(Ok(project(false))));
        assert_eq!(asked("watch-all dunwich"), Some(Ok(project(true))));
        assert_eq!(asked("watch dunwich dun-1"), Some(Ok(bead("dun-1"))));
    }

    #[test]
    fn a_watch_in_none_of_the_forms_is_malformed() {
        for line in [
            "watch-all",
            "watch-all dunwich dun-1",
            "watch dunwich dun-1 dun-2",
        ] {
            assert_eq!(asked(line), Some(Err(Refusal::Malformed)), "{line}");
        }
    }

    #[test]
    fn any_other_line_is_a_producers() {
        for line in ["dunwich", "covered dunwich", "watching", ""] {
            assert_eq!(asked(line), None, "{line}");
        }
    }

    #[test]
    fn a_watch_starts_from_the_beads_that_are_not_closed() {
        let mut interest = watching(&[project(false)]);

        let lines = interest.catch_up("dunwich", &beads(&[("dun-1", "open"), ("dun-2", "closed")]));

        assert_eq!(said(&lines), ["bead dun-1"]);
    }

    #[test]
    fn a_watch_all_starts_from_every_bead() {
        let mut interest = watching(&[project(true)]);

        let lines = interest.catch_up("dunwich", &beads(&[("dun-1", "open"), ("dun-2", "closed")]));

        assert_eq!(said(&lines), ["bead dun-1", "bead dun-2"]);
    }

    #[test]
    fn a_closed_bead_a_watch_started_past_is_sent_when_it_reopens() {
        let mut interest = watching(&[project(false)]);
        interest.catch_up("dunwich", &beads(&[("dun-2", "closed")]));

        let lines = interest.catch_up("dunwich", &beads(&[("dun-2", "open")]));

        assert_eq!(said(&lines), ["bead dun-2"]);
    }

    #[test]
    fn a_bead_created_after_the_watch_started_is_sent_whatever_its_status() {
        let mut interest = watching(&[project(false)]);
        interest.catch_up("dunwich", &beads(&[]));

        let lines = interest.catch_up("dunwich", &beads(&[("dun-3", "closed")]));

        assert_eq!(said(&lines), ["bead dun-3"]);
    }

    #[test]
    fn a_bead_whose_row_changes_is_sent_again() {
        let mut interest = watching(&[project(false)]);
        interest.catch_up("dunwich", &beads(&[("dun-1", "open")]));

        let lines = interest.catch_up("dunwich", &beads(&[("dun-1", "in_progress")]));

        assert_eq!(said(&lines), ["bead dun-1"]);
    }

    /// `watch` starts past closed beads and still sends every change after.
    #[test]
    fn a_closed_bead_that_changes_and_stays_closed_is_sent() {
        let mut interest = watching(&[project(false)]);
        interest.catch_up("dunwich", &beads(&[("dun-2", "closed")]));
        let mut commented = beads(&[("dun-2", "closed")]);
        let row = Arc::make_mut(
            commented
                .get_mut("dun-2")
                .and_then(|held| held.row.as_mut())
                .expect("a row"),
        );
        row.insert("comment_count".to_string(), json!(1));

        assert_eq!(
            said(&interest.catch_up("dunwich", &commented)),
            ["bead dun-2"]
        );
    }

    #[test]
    fn a_blocker_arriving_is_a_change() {
        let mut interest = watching(&[project(false)]);
        interest.catch_up("dunwich", &beads(&[("dun-1", "open")]));
        let mut blocked = beads(&[("dun-1", "open")]);
        blocked.get_mut("dun-1").expect("held").blocked_by = vec!["fer-4".to_string()];

        assert_eq!(
            said(&interest.catch_up("dunwich", &blocked)),
            ["bead dun-1"]
        );
    }

    /// bd's own readiness is what a run reading part of the tracker takes, so
    /// it moving is a change even where `bdi`'s does not.
    #[test]
    fn bd_naming_a_blocker_is_a_change() {
        let mut interest = watching(&[project(false)]);
        interest.catch_up("dunwich", &beads(&[("dun-1", "open")]));
        let mut blocked = beads(&[("dun-1", "open")]);
        blocked.get_mut("dun-1").expect("held").bd.blocked_by = vec!["dun-4".to_string()];

        assert_eq!(
            said(&interest.catch_up("dunwich", &blocked)),
            ["bead dun-1"]
        );
    }

    #[test]
    fn a_bead_line_carries_bds_readiness_beside_bdis() {
        let mut held = beads(&[("dun-1", "open")]);
        let dun_1 = held.get_mut("dun-1").expect("held");
        dun_1.blocked_by = vec!["fer-4".to_string()];
        dun_1.bd = BeadReadiness {
            ready: true,
            blocked_by: Vec::new(),
        };
        let lines = watching(&[project(false)]).catch_up("dunwich", &held);
        let line: Value = serde_json::from_str(&lines[0]).expect("JSON");

        assert_eq!(
            line,
            json!({ "line": "bead", "project": "dunwich", "ready": false, "blocked_by": ["fer-4"], "bd": { "ready": true, "blocked_by": [] }, "row": { "id": "dun-1", "status": "open" } })
        );
    }

    #[test]
    fn a_refusal_names_the_line_and_why() {
        let line: Value =
            serde_json::from_str(&refused_line("watch ferry\n", Refusal::UnknownProject))
                .expect("JSON");

        assert_eq!(
            line,
            json!({ "line": "refused", "asked": "watch ferry", "reason": "unknown-project" })
        );
    }

    #[test]
    fn a_bead_nothing_changed_is_not_sent_again() {
        let mut interest = watching(&[project(false)]);
        interest.catch_up("dunwich", &beads(&[("dun-1", "open")]));

        assert_eq!(
            said(&interest.catch_up("dunwich", &beads(&[("dun-1", "open")]))),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_heartbeat_alone_is_not_a_change() {
        let mut interest = watching(&[project(false)]);
        interest.catch_up("dunwich", &beads(&[("dun-1", "open")]));
        let mut beating = beads(&[("dun-1", "open")]);
        let row = Arc::make_mut(
            beating
                .get_mut("dun-1")
                .and_then(|held| held.row.as_mut())
                .expect("a row"),
        );
        row.insert("heartbeat_at".to_string(), json!("2026-08-30T10:21:02Z"));
        row.insert(
            "lease_expires_at".to_string(),
            json!("2026-08-30T10:31:02Z"),
        );

        assert_eq!(
            said(&interest.catch_up("dunwich", &beating)),
            Vec::<String>::new()
        );
    }

    #[test]
    fn readiness_changing_is_a_change() {
        let mut interest = watching(&[project(false)]);
        interest.catch_up("dunwich", &beads(&[("dun-1", "open")]));
        let mut ready = beads(&[("dun-1", "open")]);
        ready.get_mut("dun-1").expect("held").ready = true;

        assert_eq!(said(&interest.catch_up("dunwich", &ready)), ["bead dun-1"]);
    }

    #[test]
    fn a_bead_sent_and_then_no_longer_held_is_gone() {
        let mut interest = watching(&[project(false)]);
        interest.catch_up("dunwich", &beads(&[("dun-1", "open"), ("dun-2", "closed")]));

        let lines = interest.catch_up("dunwich", &beads(&[]));

        assert_eq!(said(&lines), ["gone dun-1"]);
    }

    #[test]
    fn a_bead_named_on_its_own_line_is_sent_whatever_its_status() {
        let mut interest = watching(&[bead("dun-2")]);

        let lines = interest.catch_up("dunwich", &beads(&[("dun-1", "open"), ("dun-2", "closed")]));

        assert_eq!(said(&lines), ["bead dun-2"]);
    }

    #[test]
    fn a_bead_named_and_never_held_is_gone_once() {
        let mut interest = watching(&[bead("dun-9")]);

        let first = interest.catch_up("dunwich", &beads(&[("dun-1", "open")]));
        let then = interest.catch_up("dunwich", &beads(&[("dun-1", "open")]));

        assert_eq!(said(&first), ["gone dun-9"]);
        assert_eq!(said(&then), Vec::<String>::new());
    }

    #[test]
    fn a_bead_told_gone_is_not_told_again_by_a_later_line() {
        let held = beads(&[("dun-1", "open")]);
        let mut interest = watching(&[bead("dun-9")]);
        interest.catch_up("dunwich", &held);

        interest.widen(&bead("dun-1"));
        let lines = interest.catch_up("dunwich", &held);

        assert_eq!(said(&lines), ["bead dun-1"]);
    }

    #[test]
    fn a_bead_told_gone_that_comes_and_goes_again_is_gone_again() {
        let mut interest = watching(&[bead("dun-9")]);
        interest.catch_up("dunwich", &beads(&[]));
        interest.catch_up("dunwich", &beads(&[("dun-9", "open")]));

        let lines = interest.catch_up("dunwich", &beads(&[]));

        assert_eq!(said(&lines), ["gone dun-9"]);
    }

    /// A bead two lines name is sent once, and a closed bead `watch` started
    /// past is sent when a later line asks for it.
    #[test]
    fn a_later_line_is_sent_only_what_the_connection_was_not() {
        let held = beads(&[("dun-1", "open"), ("dun-2", "closed")]);
        let mut interest = watching(&[project(false)]);
        interest.catch_up("dunwich", &held);

        interest.widen(&bead("dun-1"));
        let named = interest.catch_up("dunwich", &held);
        interest.widen(&project(true));
        let everything = interest.catch_up("dunwich", &held);

        assert_eq!(said(&named), Vec::<String>::new());
        assert_eq!(said(&everything), ["bead dun-2"]);
    }

    #[test]
    fn freshness_says_the_tracker_could_not_be_reached_and_why() {
        let line: Value = serde_json::from_str(&freshness_line(
            "dunwich",
            &Reach::default(),
            None,
            Some(&TrackerFailure::Auth),
        ))
        .expect("JSON");

        assert_eq!(
            line["tracker"],
            json!({ "unreachable": { "reason": "auth" } })
        );
    }

    #[test]
    fn freshness_says_how_the_listener_reaches_the_tracker() {
        let reach = Reach {
            path: "/srv/work/dunwich".into(),
            environment_command: Some(vec![
                "direnv".to_string(),
                "exec".to_string(),
                ".".to_string(),
            ]),
        };
        let line: Value =
            serde_json::from_str(&freshness_line("dunwich", &reach, None, None)).expect("JSON");

        assert_eq!(
            line,
            json!({ "line": "freshness", "project": "dunwich", "as_of": null, "tracker": "ok", "events": "off", "protocol": 1, "reach": { "path": "/srv/work/dunwich", "environment_command": ["direnv", "exec", "."] } })
        );
    }
}
