//! Which agents watch a bead, found in herdr's panes by the agent sessions the
//! watcher names.

use std::collections::{BTreeMap, BTreeSet};

use crate::model::types::Pane;

/// What herdr reports of a watching agent, read off the pane it reports the
/// agent's session in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchingAgent {
    /// The agent herdr detected in the pane, by herdr's own name for it.
    pub agent: String,
    pub display_agent: Option<String>,
    /// The pane's state label for the state it is in, or its title.
    pub caption: Option<String>,
}

/// The agent sessions watching one bead, each with what herdr reports of its
/// agent where herdr reports the session in a live pane.
pub type Watchers = BTreeMap<String, Option<WatchingAgent>>;

/// Who watches a bead, besides the agent on its claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Watching {
    /// No watcher answered for the bead's project, so nobody can say who
    /// watches it.
    Unknown,
    Known {
        /// Each watching agent herdr reports in a live pane.
        agents: Vec<WatchingAgent>,
        /// How many watching agent sessions herdr reports in no live pane.
        unreported: usize,
    },
}

impl Watching {
    /// Known to be watched by nobody besides its claim.
    pub fn nobody() -> Self {
        Watching::Known {
            agents: Vec::new(),
            unreported: 0,
        }
    }

    /// Who watches bead `id`, of the watchers of each bead in its project,
    /// leaving out `claimed_by`, the agent session of the agent on its
    /// claim. Unknown where nothing could say who watches the project's
    /// beads.
    pub fn of(
        project: Option<&BTreeMap<String, Watchers>>,
        id: &str,
        claimed_by: Option<&str>,
    ) -> Self {
        let Some(project) = project else {
            return Watching::Unknown;
        };
        let others = project
            .get(id)
            .into_iter()
            .flatten()
            .filter(|(session, _)| Some(session.as_str()) != claimed_by);
        let mut agents = Vec::new();
        let mut unreported = 0;
        for (_, agent) in others {
            match agent {
                Some(agent) => agents.push(agent.clone()),
                None => unreported += 1,
            }
        }
        Watching::Known { agents, unreported }
    }

    /// Whether some agent besides the one on its claim watches the bead.
    pub fn is_watched(&self) -> bool {
        match self {
            Watching::Unknown => false,
            Watching::Known { agents, unreported } => !agents.is_empty() || *unreported > 0,
        }
    }
}

/// Each bead's watchers, found among `panes`. herdr keeps a pane's agent
/// session after its agent leaves, so only a pane with a live agent places
/// one.
pub fn in_panes(
    watches: BTreeMap<String, BTreeSet<String>>,
    panes: &[Pane],
) -> BTreeMap<String, Watchers> {
    watches
        .into_iter()
        .map(|(id, sessions)| {
            let watchers = sessions
                .into_iter()
                .map(|session| {
                    let agent = panes
                        .iter()
                        .find(|pane| pane.live_agent_session() == Some(session.as_str()))
                        .map(reported);
                    (session, agent)
                })
                .collect();
            (id, watchers)
        })
        .collect()
}

fn reported(pane: &Pane) -> WatchingAgent {
    WatchingAgent {
        agent: pane.agent.clone().unwrap_or_default(),
        display_agent: pane.display_agent.clone(),
        caption: pane.caption().map(str::to_string),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::model::types::{AgentSession, PaneStatus};

    fn pane_of(session: &str, agent: Option<&str>, display_agent: &str) -> Pane {
        let mut pane = Pane::answered(
            "default".to_string(),
            "w1:p4".to_string(),
            PathBuf::from("/srv/work/dunwich"),
            PaneStatus::Working,
        );
        pane.agent = agent.map(str::to_string);
        pane.display_agent = Some(display_agent.to_string());
        pane.title = Some("re-pointing the dish".to_string());
        pane.agent_session = Some(AgentSession {
            agent: "claude".to_string(),
            kind: "id".to_string(),
            source: "hook".to_string(),
            value: session.to_string(),
        });
        pane
    }

    fn watched_by(sessions: &[&str]) -> BTreeMap<String, BTreeSet<String>> {
        BTreeMap::from([(
            "dun-7".to_string(),
            sessions.iter().map(|session| session.to_string()).collect(),
        )])
    }

    #[test]
    fn a_watcher_is_what_herdr_reports_of_the_live_pane_holding_its_session() {
        let panes = [pane_of("4f1c-keeper", Some("claude"), "dish crew")];

        let found = in_panes(watched_by(&["4f1c-keeper"]), &panes);

        assert_eq!(
            found["dun-7"]["4f1c-keeper"],
            Some(WatchingAgent {
                agent: "claude".to_string(),
                display_agent: Some("dish crew".to_string()),
                caption: Some("re-pointing the dish".to_string()),
            })
        );
    }

    /// herdr keeps the session on a pane its agent has left, so that pane
    /// says nothing about where the watching agent is now.
    #[test]
    fn a_pane_whose_agent_has_left_places_no_watcher() {
        let panes = [pane_of("4f1c-keeper", None, "dish crew")];

        let found = in_panes(watched_by(&["4f1c-keeper", "9a2e-ferryman"]), &panes);

        assert_eq!(found["dun-7"]["4f1c-keeper"], None);
        assert_eq!(found["dun-7"]["9a2e-ferryman"], None);
    }

    #[test]
    fn the_agent_on_the_claim_is_not_counted_among_the_watchers() {
        let panes = [pane_of("4f1c-keeper", Some("claude"), "dish crew")];
        let found = in_panes(watched_by(&["4f1c-keeper"]), &panes);

        let watching = Watching::of(Some(&found), "dun-7", Some("4f1c-keeper"));

        assert_eq!(watching, Watching::nobody());
        assert!(!watching.is_watched());
    }

    #[test]
    fn a_watcher_herdr_reports_in_no_pane_still_watches() {
        let found = in_panes(watched_by(&["4f1c-keeper", "9a2e-ferryman"]), &[]);

        let watching = Watching::of(Some(&found), "dun-7", Some("4f1c-keeper"));

        assert_eq!(
            watching,
            Watching::Known {
                agents: Vec::new(),
                unreported: 1
            }
        );
        assert!(watching.is_watched());
    }

    #[test]
    fn who_watches_is_unknown_where_nothing_could_say() {
        let watching = Watching::of(None, "dun-7", None);

        assert_eq!(watching, Watching::Unknown);
        assert!(!watching.is_watched());
    }
}
