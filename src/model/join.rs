//! Which pane is working which bead, read from the two directions that can
//! disagree about it.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::config::{Config, Project};
use crate::model::tree::{self, Assembled};
use crate::model::types::Bead;
use crate::model::types::{Pane, PaneKey, PaneStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JoinSource {
    /// The bead named the agent's session, and herdr reported it in the pane.
    /// Exact, and it follows the agent from pane to pane.
    AgentSession,
    /// The bead named the pane. Exact.
    AgentPane,
    /// The pane named the bead. Inferred from free text.
    DisplayAgent,
}

/// A bead, across every tracker `bdi` reads. Prefixes are per-tracker and
/// uncoordinated, so an id on its own does not name a bead.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct BeadKey {
    pub project: String,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentRef {
    pub pane: PaneKey,
    pub pane_status: PaneStatus,
    pub title: Option<String>,
    pub source: JoinSource,
    /// The agent session herdr reports for the agent in `pane`, where it
    /// reports one.
    pub agent_session: Option<String>,
}

/// A disagreement between the two directions of the join. Each is reported
/// rather than resolved: picking a winner would hide exactly the drift `bdi`
/// exists to surface.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "conflict", rename_all = "kebab-case")]
pub enum Conflict {
    /// The bead's own key and a pane's `display_agent` name different panes.
    BeadAndPaneDisagree {
        bead: BeadKey,
        named_by_bead: PaneKey,
        named_by_pane: PaneKey,
    },
    /// Several panes name one bead. None of them wins it.
    SeveralPanesNameOneBead { bead: BeadKey, panes: Vec<PaneKey> },
    /// Several beads name one pane. None of them gets it.
    ///
    /// `caption` is the pane's own account of what it is working on, carried
    /// here because it is the one thing that tells a live claim from a stale
    /// one and it is nowhere else on the screen: a caption is drawn off the
    /// agent a pane was awarded, and a contested pane is awarded to nobody.
    SeveralBeadsNameOnePane {
        pane: PaneKey,
        caption: Option<String>,
        beads: Vec<BeadKey>,
    },
    /// A pane and the bead naming it, or named by it, sit in different
    /// projects. No join. `pane_project` is absent when the pane's `cwd` is
    /// under no configured project at all.
    PaneInAnotherProject {
        bead: BeadKey,
        pane: PaneKey,
        pane_project: Option<String>,
    },
    /// The bead's own key names a pane id that several sessions each hold a
    /// pane under. A seat writes the id alone, so the key cannot say which,
    /// and the bead gets none of them: picking would draw the wrong seat on
    /// the bead as surely as the right one.
    PaneIdInSeveralSessions {
        bead: BeadKey,
        pane_id: String,
        sessions: Vec<String>,
    },
    /// Several beads name one agent session, which one live pane holds.
    /// None of them gets it.
    ///
    /// `caption` is carried for the reason `SeveralBeadsNameOnePane` carries
    /// it: the pane is awarded to nobody, so nothing else says what it is
    /// working on.
    SeveralBeadsNameOneAgentSession {
        agent_session: String,
        pane: PaneKey,
        caption: Option<String>,
        beads: Vec<BeadKey>,
    },
    /// The bead names an agent session that several live panes hold. None of
    /// them wins it.
    AgentSessionInSeveralPanes {
        bead: BeadKey,
        agent_session: String,
        panes: Vec<PaneKey>,
    },
}

impl Conflict {
    /// The panes this disagreement left holding nothing.
    ///
    /// A candidate rather than a verdict: `BeadAndPaneDisagree` is settled in
    /// the bead's favour, and the panes of `SeveralPanesNameOneBead` may each
    /// hold some other bead by the exact direction. So a caller wanting the
    /// panes that came away empty takes this against the ones the join
    /// awarded — which is what lets the set be generous without a row saying
    /// of a working pane that its claim was refused.
    pub fn refused_panes(&self) -> Vec<PaneKey> {
        match self {
            Conflict::BeadAndPaneDisagree { named_by_pane, .. } => vec![named_by_pane.clone()],
            Conflict::SeveralPanesNameOneBead { panes, .. } => panes.clone(),
            Conflict::SeveralBeadsNameOnePane { pane, .. }
            | Conflict::SeveralBeadsNameOneAgentSession { pane, .. } => vec![pane.clone()],
            Conflict::AgentSessionInSeveralPanes { panes, .. } => panes.clone(),
            Conflict::PaneInAnotherProject { pane, .. } => vec![pane.clone()],
            Conflict::PaneIdInSeveralSessions {
                pane_id, sessions, ..
            } => sessions
                .iter()
                .map(|session| PaneKey {
                    session: session.clone(),
                    id: pane_id.clone(),
                })
                .collect(),
        }
    }
}

/// One project's assembled rows, as the join reads them.
pub struct ProjectRows<'a> {
    pub project: &'a str,
    pub rows: &'a [Arc<Bead>],
}

/// What one run holds about panes: every pane a session answered with, and
/// what it knows it is missing.
///
/// The second is not the first's complement, and nothing could enumerate
/// that: a pane id nothing answered for is usually a pane that has gone.
/// It is what a session that has since gone quiet was holding when it last
/// answered, which is the only thing that can place a pane id inside a
/// session this run cannot ask — a seat writes the id alone, and an id
/// names a pane only within its session.
///
/// So a rule that reads a pane's absence can be evaluated over the first and
/// not over the second, and the two are handed over together because a run
/// that has one without the other cannot tell which it is holding.
pub struct Listed<'a> {
    pub panes: &'a [Pane],
    pub out_of_reach: &'a OutOfReach,
}

/// What the sessions a run could not ask were holding when they last
/// answered: the ids of their panes, and the agent sessions live in them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OutOfReach {
    pub pane_ids: BTreeSet<String>,
    pub agent_sessions: BTreeSet<String>,
}

impl OutOfReach {
    /// Add what a session that has gone quiet held in `last_answered`, the
    /// panes it last answered with.
    pub fn extend(&mut self, last_answered: &[Pane]) {
        for pane in last_answered {
            self.pane_ids.insert(pane.pane_id.clone());
            self.agent_sessions
                .extend(pane.live_agent_session().map(str::to_string));
        }
    }
}

/// A run every session of which answered, so there is nothing it is missing.
#[cfg(feature = "testing")]
static NOTHING_MISSED: OutOfReach = OutOfReach {
    pane_ids: BTreeSet::new(),
    agent_sessions: BTreeSet::new(),
};

/// Behind the feature rather than `cfg(test)` because the tests under
/// `tests/` resolve a join too, and they link the library.
#[cfg(feature = "testing")]
impl<'a> Listed<'a> {
    /// Every pane there is, from a run that asked each session and was
    /// answered by all of them.
    ///
    /// A test's shorthand and no run's: a collection always knows which of
    /// its sessions went quiet, so nothing in production has this to say.
    pub fn all(panes: &'a [Pane]) -> Self {
        Self {
            panes,
            out_of_reach: &NOTHING_MISSED,
        }
    }
}

/// The agents the join could account for, and every disagreement it could not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Joined {
    pub agents: BTreeMap<BeadKey, AgentRef>,
    /// Beads whose own key named a live pane the join would not award them,
    /// and the disagreement that refused it. Only the exact direction is here:
    /// a bead that named nothing has no claim to refuse, and a pane naming an
    /// id another tracker happens to reuse says nothing about that tracker's
    /// bead.
    pub refused: BTreeMap<BeadKey, Conflict>,
    /// Beads whose own key named a pane or an agent session that is out of
    /// this run's reach, so nothing it holds says whether the seat behind the
    /// claim is alive. A rule reading that absence has no absence to read.
    pub out_of_reach: BTreeSet<BeadKey>,
    /// Beads sitting beneath one that holds an agent, one whose pane is out
    /// of reach, or one whose claim is live, in any tree the run drew. Filled
    /// by `cover`, once for every tree, before any tree is built from this.
    pub under_a_seat: BTreeSet<BeadKey>,
    pub conflicts: Vec<Conflict>,
}

impl Joined {
    /// Note every bead beneath a seat in `assembled`, a tree of `project`'s.
    ///
    /// A tree holds everything beneath its root, so every bead above another
    /// is in a tree that holds both. A bead drawn in several trees is covered
    /// in all of them by a seat in any one.
    pub fn cover(&mut self, project: &str, assembled: &Assembled, now: DateTime<Utc>) {
        let key_of = |at: usize| BeadKey {
            project: assembled
                .external
                .get(&at)
                .map_or(project, String::as_str)
                .to_string(),
            id: assembled.beads[at].id.clone(),
        };
        for seat in 0..assembled.beads.len() {
            let key = key_of(seat);
            if self.agents.contains_key(&key)
                || self.out_of_reach.contains(&key)
                || assembled.beads[seat].claim_is_live(now)
            {
                self.under_a_seat.extend(
                    tree::beneath(&assembled.children, seat, &[])
                        .into_iter()
                        .map(key_of),
                );
            }
        }
    }
}

/// The project a pane sits in: the one holding its directory, or, where none
/// does, the one holding the same place in the main working tree of the
/// repository the directory is in. `None` when no project's does.
///
/// The directory comes first because a config may name a project by its
/// place in a linked worktree. The main working tree is asked second, for a
/// pane in a linked worktree of a project this run never asked where it is
/// worked — one the scope left out — which is held by nothing as it stands.
pub fn project_of<'a>(pane: &Pane, projects: &'a [Project]) -> Option<&'a Project> {
    project_holding(&pane.cwd, projects)
        .or_else(|| project_holding(pane.cwd_in_the_main_working_tree()?, projects))
}

/// The project a path sits in: the one whose deepest working tree contains
/// it. `None` when no project's does.
fn project_holding<'a>(path: &Path, projects: &'a [Project]) -> Option<&'a Project> {
    projects
        .iter()
        .filter_map(|p| Some((p.holds(path)?, p)))
        .max_by_key(|(depth, _)| *depth)
        .map(|(_, project)| project)
}

/// Join live panes onto beads, scoped to each pane's own project.
///
/// Bidirectional, because each direction alone has a hole: a bead's configured
/// key names its pane exactly, and a pane's `display_agent` names its bead by
/// inference. A bead takes its agent from the exact direction when that
/// direction resolved uncontested, otherwise from the inferred direction when
/// exactly one pane names it, otherwise none — and wherever the two directions
/// name different live panes, the disagreement is reported however it went.
///
/// The exact direction names a pane by id alone, which is what a seat has to
/// write, and an id names a pane only within a session. So it is matched
/// across every session: held by one, that is the pane; held by several,
/// the claim is refused and the disagreement reported, because nothing the
/// bead wrote says which.
pub fn resolve(trees: &[ProjectRows<'_>], listed: Listed<'_>, cfg: &Config) -> Joined {
    let Listed {
        panes,
        out_of_reach: panes_out_of_reach,
    } = listed;
    let mut live_under: BTreeMap<&str, Vec<&Pane>> = BTreeMap::new();
    for pane in panes {
        live_under
            .entry(pane.pane_id.as_str())
            .or_default()
            .push(pane);
    }
    let live: BTreeMap<PaneKey, &Pane> = panes.iter().map(|p| (p.key(), p)).collect();
    let mut live_agents: BTreeMap<&str, Vec<&Pane>> = BTreeMap::new();
    for pane in panes {
        if let Some(session) = pane.live_agent_session() {
            live_agents.entry(session).or_default().push(pane);
        }
    }

    // Every pane is placed in a project before any bead is looked at, so a
    // colliding id in another tracker never reaches the join at all. Placed
    // against every configured project, read or not: a pane in a project
    // this run left out is in that project, not in a directory nobody
    // configured.
    let pane_project: BTreeMap<PaneKey, Option<&str>> = panes
        .iter()
        .map(|p| {
            (
                p.key(),
                project_of(p, &cfg.projects).map(|q| q.name.as_str()),
            )
        })
        .collect();

    // Which projects hold a bead of each id, so a pane naming one outside its
    // own project is told apart from a pane naming nothing at all.
    let mut projects_holding: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for tree in trees {
        for row in tree.rows {
            projects_holding
                .entry(row.id.as_str())
                .or_default()
                .insert(tree.project);
        }
    }

    let mut conflicts: Vec<Conflict> = Vec::new();
    let mut refused: BTreeMap<BeadKey, Conflict> = BTreeMap::new();
    let mut out_of_reach: BTreeSet<BeadKey> = BTreeSet::new();

    // The exact direction: the bead names its agent's session, or its pane.
    let mut claims: BTreeMap<PaneKey, BTreeMap<BeadKey, JoinSource>> = BTreeMap::new();
    let mut claimed_pane: BTreeMap<BeadKey, PaneKey> = BTreeMap::new();
    let mut named_session: BTreeMap<BeadKey, &str> = BTreeMap::new();
    for tree in trees {
        for row in tree.rows {
            let bead = BeadKey {
                project: tree.project.to_string(),
                id: row.id.clone(),
            };
            let session = row.metadata.get(&cfg.join.agent_session_key);
            if let Some(session) = session {
                named_session.insert(bead.clone(), session.as_str());
            }
            let holding_session = session.map(|session| {
                let holding = live_agents.get(session.as_str());
                (session, holding.map_or(&[][..], Vec::as_slice))
            });
            let (pane, source) = match holding_session {
                Some((_, [pane])) => (*pane, JoinSource::AgentSession),
                Some((session, several @ [_, _, ..])) => {
                    let ambiguous = Conflict::AgentSessionInSeveralPanes {
                        bead: bead.clone(),
                        agent_session: session.clone(),
                        panes: several.iter().map(|pane| pane.key()).collect(),
                    };
                    conflicts.push(ambiguous.clone());
                    refused.insert(bead, ambiguous);
                    continue;
                }
                // The agent may be live in a session that has gone quiet, in
                // a pane other than the one the bead names.
                Some((session, [])) if panes_out_of_reach.agent_sessions.contains(session) => {
                    out_of_reach.insert(bead);
                    continue;
                }
                Some((_, [])) | None => {
                    let Some(named) = row.metadata.get(&cfg.join.pane_key) else {
                        continue;
                    };
                    let pane = match pane_named(named, &live_under, &panes_out_of_reach.pane_ids) {
                        Named::Live(pane) => pane,
                        Named::OutOfReach => {
                            out_of_reach.insert(bead);
                            continue;
                        }
                        Named::Gone => continue,
                        Named::InSeveralSessions(sessions) => {
                            let ambiguous = Conflict::PaneIdInSeveralSessions {
                                bead: bead.clone(),
                                pane_id: named.clone(),
                                sessions,
                            };
                            conflicts.push(ambiguous.clone());
                            refused.insert(bead, ambiguous);
                            continue;
                        }
                    };
                    // A bead naming a session herdr no longer reports in its
                    // pane names an agent that is not there: one that has left
                    // the pane, or been replaced in it. Only a pane herdr
                    // reports no session for at all is still taken at its
                    // word.
                    if session.is_some() && pane.agent_session_id().is_some() {
                        continue;
                    }
                    (pane, JoinSource::AgentPane)
                }
            };
            let key = pane.key();
            let holds = pane_project[&key];
            if holds != Some(tree.project) {
                let elsewhere = Conflict::PaneInAnotherProject {
                    bead: bead.clone(),
                    pane: key,
                    pane_project: holds.map(str::to_string),
                };
                conflicts.push(elsewhere.clone());
                refused.insert(bead, elsewhere);
                continue;
            }
            claimed_pane.insert(bead.clone(), key.clone());
            claims.entry(key).or_default().insert(bead, source);
        }
    }

    let mut agents: BTreeMap<BeadKey, AgentRef> = BTreeMap::new();
    for (key, beads) in claims {
        let pane = live[&key];
        let mut named: Vec<(BeadKey, JoinSource)> = beads.into_iter().collect();
        if named.len() == 1 {
            let (bead, source) = named.remove(0);
            agents.insert(bead, agent_ref(pane, source));
        } else {
            // Beads that all reached the pane through its agent's session
            // contest the agent; any bead that named the pane itself makes it
            // the pane they contest.
            let by_session = named
                .iter()
                .all(|(_, source)| *source == JoinSource::AgentSession);
            let named: Vec<BeadKey> = named.into_iter().map(|(bead, _)| bead).collect();
            let caption = pane.caption().map(str::to_string);
            let shared = match pane.live_agent_session().filter(|_| by_session) {
                Some(agent_session) => Conflict::SeveralBeadsNameOneAgentSession {
                    agent_session: agent_session.to_string(),
                    pane: key,
                    caption,
                    beads: named.clone(),
                },
                None => Conflict::SeveralBeadsNameOnePane {
                    pane: key,
                    caption,
                    beads: named.clone(),
                },
            };
            conflicts.push(shared.clone());
            for bead in named {
                refused.insert(bead, shared.clone());
            }
        }
    }

    // The inferred direction: the pane names its bead.
    let mut named_by: BTreeMap<BeadKey, BTreeSet<PaneKey>> = BTreeMap::new();
    for pane in panes {
        let Some(id) = &pane.display_agent else {
            continue;
        };
        let Some(project) = pane_project[&pane.key()] else {
            continue;
        };
        // Most `display_agent` values are free text rather than a bead id.
        let Some(holders) = projects_holding.get(id.as_str()) else {
            continue;
        };
        // A pane in a project this run left out is on that project's work,
        // which no tree drew: an id it names that no drawn bead of its own
        // project carries says nothing about another project's bead.
        if holders.contains(project) {
            named_by
                .entry(BeadKey {
                    project: project.to_string(),
                    id: id.clone(),
                })
                .or_default()
                .insert(pane.key());
        } else if cfg.reads(project) {
            for holder in holders {
                conflicts.push(Conflict::PaneInAnotherProject {
                    bead: BeadKey {
                        project: holder.to_string(),
                        id: id.clone(),
                    },
                    pane: pane.key(),
                    pane_project: Some(project.to_string()),
                });
            }
        }
    }

    for (bead, from_panes) in named_by {
        let mut inferred: Vec<PaneKey> = from_panes.into_iter().collect();

        if inferred.len() > 1 {
            conflicts.push(Conflict::SeveralPanesNameOneBead {
                bead,
                panes: inferred,
            });
            continue;
        }

        let only = inferred.remove(0);
        // The disagreement stands on what the bead's key named, whether or not
        // that claim survived to be awarded.
        if let Some(claimed) = claimed_pane.get(&bead).filter(|p| **p != only) {
            conflicts.push(Conflict::BeadAndPaneDisagree {
                bead: bead.clone(),
                named_by_bead: claimed.clone(),
                named_by_pane: only.clone(),
            });
        }
        // A bead naming its agent's session is not won by a pane holding
        // another agent, nor by one of several panes holding its own.
        if let Some(session) = named_session.get(&bead) {
            let pane = live[&only];
            let holders = live_agents.get(session).map_or(0, Vec::len);
            if holders > 1
                || (pane.agent_session_id().is_some()
                    && pane.live_agent_session() != Some(*session))
            {
                continue;
            }
        }
        // The exact direction wins where it resolved uncontested; the inferred
        // one only fills the gap it left.
        agents
            .entry(bead)
            .or_insert_with(|| agent_ref(live[&only], JoinSource::DisplayAgent));
    }

    conflicts.sort();
    conflicts.dedup();

    Joined {
        agents,
        refused,
        out_of_reach,
        under_a_seat: BTreeSet::new(),
        conflicts,
    }
}

/// What a pane id a bead's key names comes to in this run.
enum Named<'a> {
    /// The one live pane carrying the id.
    Live(&'a Pane),
    /// A session this run could not ask was holding the id when it last
    /// answered.
    OutOfReach,
    /// Nothing live carries the id.
    Gone,
    /// Each of these sessions holds a live pane under the id.
    InSeveralSessions(Vec<String>),
}

fn pane_named<'a>(
    named: &str,
    live_under: &BTreeMap<&str, Vec<&'a Pane>>,
    out_of_reach: &BTreeSet<String>,
) -> Named<'a> {
    // An id this run is missing is asked about before the live panes are,
    // because a live pane carrying it need not be the one the bead named: a
    // session mints its ids from `w1` up, so the same id in two sessions is
    // two panes. That is the ambiguity `PaneIdInSeveralSessions` awards to
    // nobody where both panes answered, and here one of them cannot be asked
    // at all — so it is not reported as a disagreement, and the claim
    // resolves to nothing whether or not something live carries its id.
    if out_of_reach.contains(named) {
        return Named::OutOfReach;
    }
    // A named pane that is not live resolves to nothing; that absence is
    // `orphan-claim`'s to report, not a disagreement.
    match live_under.get(named).map(Vec::as_slice) {
        None | Some([]) => Named::Gone,
        Some([pane]) => Named::Live(pane),
        Some(several) => {
            Named::InSeveralSessions(several.iter().map(|p| p.session.clone()).collect())
        }
    }
}

fn agent_ref(pane: &Pane, source: JoinSource) -> AgentRef {
    AgentRef {
        pane: pane.key(),
        pane_status: pane.agent_status.clone(),
        title: pane.caption().map(str::to_string),
        source,
        agent_session: pane.live_agent_session().map(str::to_string),
    }
}

/// Live panes that resolved to no bead in any tree.
pub fn unattributed<'a>(panes: &'a [Pane], joined: &Joined) -> Vec<&'a Pane> {
    let taken: HashSet<&PaneKey> = joined.agents.values().map(|a| &a.pane).collect();
    panes.iter().filter(|p| !taken.contains(&p.key())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::bd::parse_shared_beads;
    use crate::collect::herdr::parse_agent_list;
    use crate::config::Join;
    use crate::model::tree::Nesting;
    use crate::model::types::testing::{key as pane_key, A_SESSION};
    use crate::model::types::Bead;
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;

    const BEADS: &str = include_str!("../../tests/fixtures/display_agent_bd_list.json");
    const PANES: &str = include_str!("../../tests/fixtures/herdr_agent_list.json");

    /// A tracker and a herdr session captured from the same live moment, so
    /// the pane a bead names is a pane the list reports. The pair the other
    /// fixtures cannot make: nothing hand-written can show that the ids on
    /// both sides agree, because whoever writes it makes them agree.
    const JOINED_BEADS: &str = include_str!("../../tests/fixtures/joined_bd_list.json");
    const JOINED_PANES: &str = include_str!("../../tests/fixtures/joined_herdr_agent_list.json");

    const FIXTURE_PROJECT_PATH: &str = "/tmp/bdi-ground/dunwich";

    fn project(name: &str, path: &str) -> Project {
        Project {
            name: name.to_string(),
            path: path.into(),
            database: None,
            user: None,
            server: None,
            environment_command: None,
            credential_command: None,
            prefix: None,
            poll: Default::default(),
            events_journal: false,
            serve: None,
            badges: Vec::new(),
            worktrees: Vec::new(),
        }
    }

    fn project_working_in(name: &str, path: &str, worktrees: &[&str]) -> Project {
        Project {
            worktrees: worktrees.iter().map(Into::into).collect(),
            ..project(name, path)
        }
    }

    /// The root of a hand-written tree: the one row that depends on nothing.
    fn root_row(beads: &[Arc<Bead>]) -> String {
        beads
            .iter()
            .find(|b| b.dependencies.is_empty())
            .expect("a root row")
            .id
            .clone()
    }

    fn rows(json: &str) -> Vec<Arc<Bead>> {
        let beads = parse_shared_beads(json).expect("the rows parse");
        let root = root_row(&beads);
        Nesting::of(&beads)
            .assemble(&root)
            .expect("the rows assemble")
            .beads
    }

    /// The bodies of `herdr agent list`'s `agents` array, wrapped in its
    /// envelope so the tests exercise the real parser.
    fn panes(agents: &str) -> Vec<Pane> {
        parse_agent_list(
            A_SESSION,
            &format!(r#"{{"id":"cli:agent:list","result":{{"agents":[{agents}]}}}}"#),
        )
        .expect("the panes parse")
    }

    /// The same bodies, listed by `session` rather than by the one a test's
    /// panes are in unless it says otherwise.
    fn panes_in(session: &str, agents: &str) -> Vec<Pane> {
        parse_agent_list(
            session,
            &format!(r#"{{"id":"cli:agent:list","result":{{"agents":[{agents}]}}}}"#),
        )
        .expect("the panes parse")
    }

    fn key(project: &str, id: &str) -> BeadKey {
        BeadKey {
            project: project.to_string(),
            id: id.to_string(),
        }
    }

    /// The five disagreements whose panes are the ones they already carry.
    #[test]
    fn a_disagreement_names_the_panes_it_left_holding_nothing() {
        assert_eq!(
            Conflict::SeveralBeadsNameOneAgentSession {
                agent_session: "sess-mine".to_string(),
                pane: pane_key("w:p1"),
                caption: None,
                beads: vec![key("proj", "p-1"), key("proj", "p-2")],
            }
            .refused_panes(),
            vec![pane_key("w:p1")]
        );
        assert_eq!(
            Conflict::AgentSessionInSeveralPanes {
                bead: key("proj", "p-1"),
                agent_session: "sess-mine".to_string(),
                panes: vec![pane_key("w:p3"), pane_key("w:p4")],
            }
            .refused_panes(),
            vec![pane_key("w:p3"), pane_key("w:p4")]
        );
        assert_eq!(
            Conflict::SeveralBeadsNameOnePane {
                pane: pane_key("w:p1"),
                caption: None,
                beads: vec![key("proj", "p-1"), key("proj", "p-2")],
            }
            .refused_panes(),
            vec![pane_key("w:p1")]
        );
        assert_eq!(
            Conflict::PaneInAnotherProject {
                bead: key("proj", "p-1"),
                pane: pane_key("w:p2"),
                pane_project: Some("ferry".to_string()),
            }
            .refused_panes(),
            vec![pane_key("w:p2")]
        );
        assert_eq!(
            Conflict::SeveralPanesNameOneBead {
                bead: key("proj", "p-1"),
                panes: vec![pane_key("w:p3"), pane_key("w:p4")],
            }
            .refused_panes(),
            vec![pane_key("w:p3"), pane_key("w:p4")]
        );
    }

    /// The one disagreement the join settles rather than leaves open: the
    /// bead's own key wins, so the pane it named holds the bead and only the
    /// pane that named the bead back comes away empty.
    #[test]
    fn the_pane_a_bead_named_keeps_its_bead_when_another_pane_names_it_too() {
        assert_eq!(
            Conflict::BeadAndPaneDisagree {
                bead: key("proj", "p-1"),
                named_by_bead: pane_key("w:p1"),
                named_by_pane: pane_key("w:p2"),
            }
            .refused_panes(),
            vec![pane_key("w:p2")]
        );
    }

    /// A pane id alone names a pane in every session that holds one, and the
    /// claim was refused at each of them.
    #[test]
    fn an_id_several_sessions_hold_names_a_refused_pane_in_each_of_them() {
        assert_eq!(
            Conflict::PaneIdInSeveralSessions {
                bead: key("proj", "p-1"),
                pane_id: "w:p1".to_string(),
                sessions: vec![A_SESSION.to_string(), "kadath".to_string()],
            }
            .refused_panes(),
            vec![
                pane_key("w:p1"),
                PaneKey {
                    session: "kadath".to_string(),
                    id: "w:p1".to_string(),
                },
            ]
        );
    }

    /// One bead naming `w:p1`, in a tracker at `/srv/proj`.
    const A_BEAD_NAMING_W_P1: &str = r#"[
      {"id":"p-1","title":"one","status":"in_progress",
       "metadata":{"agent_pane":"w:p1"}}
    ]"#;

    /// A pane id is minted per session, and a bead names one by id alone. Two
    /// sessions each holding that id is a claim nothing the bead wrote can
    /// settle, so nobody is awarded and the disagreement names the sessions.
    #[test]
    fn a_pane_id_held_by_two_sessions_is_refused_and_names_the_sessions() {
        let beads = rows(A_BEAD_NAMING_W_P1);
        let mut live = panes(r#"{"pane_id":"w:p1","cwd":"/srv/proj","agent_status":"idle"}"#);
        live.extend(panes_in(
            "kadath",
            r#"{"pane_id":"w:p1","cwd":"/srv/proj","agent_status":"working"}"#,
        ));
        let cfg = vec![project("proj", "/srv/proj")];

        let joined = resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            Listed::all(&live),
            &Config::naming(cfg),
        );

        let ambiguous = Conflict::PaneIdInSeveralSessions {
            bead: key("proj", "p-1"),
            pane_id: "w:p1".to_string(),
            sessions: vec![A_SESSION.to_string(), "kadath".to_string()],
        };
        assert_eq!(joined.agents, BTreeMap::new());
        assert_eq!(joined.conflicts, vec![ambiguous.clone()]);
        assert_eq!(
            joined.refused,
            BTreeMap::from([(key("proj", "p-1"), ambiguous)])
        );
        assert_eq!(
            unattributed(&live, &joined).len(),
            2,
            "both panes are still live, and neither is anybody's"
        );
    }

    /// The refusal is of the bead's own key, and only of that. A pane that
    /// names the bead itself has said which of the two it is, so the
    /// inferred direction fills the gap the refused claim left — as it does
    /// after every other refusal — and the disagreement is still reported,
    /// because the key the seat wrote still names two panes.
    #[test]
    fn a_pane_naming_the_bead_resolves_it_where_its_id_alone_could_not() {
        let beads = rows(A_BEAD_NAMING_W_P1);
        let mut live = panes(r#"{"pane_id":"w:p1","cwd":"/srv/proj","agent_status":"idle"}"#);
        live.extend(panes_in(
            "kadath",
            r#"{"pane_id":"w:p1","cwd":"/srv/proj","agent_status":"working","display_agent":"p-1"}"#,
        ));
        let cfg = vec![project("proj", "/srv/proj")];

        let joined = resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            Listed::all(&live),
            &Config::naming(cfg),
        );

        let a = pane_of(&joined, "proj", "p-1");
        assert_eq!(
            a.pane,
            PaneKey {
                session: "kadath".to_string(),
                id: "w:p1".to_string(),
            }
        );
        assert_eq!(a.source, JoinSource::DisplayAgent);
        assert_eq!(
            joined.conflicts.len(),
            1,
            "the key's ambiguity still stands"
        );
        assert!(matches!(
            joined.conflicts[0],
            Conflict::PaneIdInSeveralSessions { .. }
        ));
    }

    fn pane_of<'a>(joined: &'a Joined, project: &str, id: &str) -> &'a AgentRef {
        joined
            .agents
            .get(&key(project, id))
            .unwrap_or_else(|| panic!("{project}/{id} resolved to a pane"))
    }

    fn loose(panes: &[Pane], joined: &Joined) -> Vec<String> {
        let mut ids: Vec<String> = unattributed(panes, joined)
            .into_iter()
            .map(|p| p.pane_id.clone())
            .collect();
        ids.sort();
        ids
    }

    // ---- the two directions -------------------------------------------

    /// The whole point of the tool, against the shapes a real tracker and a
    /// real herdr session actually emit rather than the ones a test writes.
    #[test]
    fn a_captured_bead_takes_the_captured_pane_it_names() {
        let beads = rows(JOINED_BEADS);
        let live = parse_agent_list(A_SESSION, JOINED_PANES).expect("the fixture parses");
        let cfg = vec![project("dunwich", FIXTURE_PROJECT_PATH)];

        let joined = resolve(
            &[ProjectRows {
                project: "dunwich",
                rows: &beads,
            }],
            Listed::all(&live),
            &Config::naming(cfg),
        );

        let a = pane_of(&joined, "dunwich", "dun-9fw.22");
        assert_eq!(a.pane, pane_key("wM:pK"));
        assert_eq!(a.source, JoinSource::AgentPane);
        assert_eq!(joined.conflicts, vec![]);
        assert_eq!(joined.refused, BTreeMap::new());
    }

    #[test]
    fn the_bead_naming_its_pane_resolves_exactly() {
        let beads = rows(
            r#"[
              {"id":"p-1","title":"root","status":"open"},
              {"id":"p-1.1","title":"work","status":"in_progress",
               "dependencies":[{"depends_on_id":"p-1","type":"parent-child"}],
               "metadata":{"agent_pane":"w:p1"}}
            ]"#,
        );
        let live = panes(
            r#"{"pane_id":"w:p1","cwd":"/home/user/proj","agent_status":"working",
                "title":"doing the work"}"#,
        );
        let cfg = vec![project("proj", "/home/user/proj")];

        let joined = resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            Listed::all(&live),
            &Config::naming(cfg),
        );

        let a = pane_of(&joined, "proj", "p-1.1");
        assert_eq!(a.pane, pane_key("w:p1"));
        assert_eq!(a.source, JoinSource::AgentPane);
        assert_eq!(a.pane_status, PaneStatus::Working);
        assert_eq!(a.title.as_deref(), Some("doing the work"));
        assert_eq!(joined.conflicts, vec![]);
    }

    /// The line an agent gets is the pane's caption — its label for the state
    /// it is actually in — rather than its title.
    #[test]
    fn an_agent_carries_the_panes_caption() {
        let beads = rows(
            r#"[{"id":"p-1","title":"root","status":"in_progress",
                 "metadata":{"agent_pane":"w:p1"}}]"#,
        );
        let live = panes(
            r#"{"pane_id":"w:p1","cwd":"/home/user/proj","agent_status":"working",
                "title":"the title",
                "state_labels":{"idle":"the idle line","working":"the working line"}}"#,
        );
        let cfg = vec![project("proj", "/home/user/proj")];

        let joined = resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            Listed::all(&live),
            &Config::naming(cfg),
        );

        assert_eq!(
            pane_of(&joined, "proj", "p-1").title.as_deref(),
            Some("the working line")
        );
    }

    /// The key is configurable, so a hardcoded `agent_pane` cannot pass.
    #[test]
    fn the_exact_direction_reads_the_configured_key() {
        let beads = rows(
            r#"[
              {"id":"p-1","title":"root","status":"open",
               "metadata":{"herdr_pane":"w:p1","agent_pane":"w:p2"}}
            ]"#,
        );
        let live = panes(
            r#"{"pane_id":"w:p1","cwd":"/home/user/proj","agent_status":"idle"},
               {"pane_id":"w:p2","cwd":"/home/user/proj","agent_status":"idle"}"#,
        );
        let cfg = vec![project("proj", "/home/user/proj")];
        let join = Join {
            pane_key: "herdr_pane".to_string(),
            ..Join::default()
        };

        let joined = resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            Listed::all(&live),
            &Config {
                join,
                ..Config::naming(cfg)
            },
        );

        assert_eq!(pane_of(&joined, "proj", "p-1").pane, pane_key("w:p1"));
    }

    #[test]
    fn a_pane_naming_its_bead_resolves_as_inferred() {
        let beads = rows(BEADS);
        let live = parse_agent_list(A_SESSION, PANES).expect("the fixture parses");
        let cfg = vec![project("dunwich", FIXTURE_PROJECT_PATH)];

        let joined = resolve(
            &[ProjectRows {
                project: "dunwich",
                rows: &beads,
            }],
            Listed::all(&live),
            &Config::naming(cfg),
        );

        // wG:p5 carries display_agent=dun-2kd.3; the one bead in the tracker
        // that names a pane names one this capture does not hold, so the
        // inferred direction is the only one that fires.
        let a = pane_of(&joined, "dunwich", "dun-2kd.3");
        assert_eq!(a.pane, pane_key("wG:p5"));
        assert_eq!(a.source, JoinSource::DisplayAgent);
        assert_eq!(joined.agents.len(), 1);
        assert_eq!(joined.conflicts, vec![]);
    }

    /// A pane herdr reports a live agent in, with that agent's session.
    fn an_agent(pane: &str, session: &str) -> String {
        format!(
            r#"{{"pane_id":"{pane}","cwd":"/home/user/proj","agent_status":"working",
                 "agent":"claude",
                 "agent_session":{{"agent":"claude","kind":"id","source":"herdr:claude",
                                   "value":"{session}"}}}}"#
        )
    }

    /// One project's single bead, carrying `metadata`, joined against `live`.
    fn join_one(metadata: &str, live: &[Pane]) -> Joined {
        let beads = rows(&format!(
            r#"[{{"id":"p-1","title":"root","status":"in_progress","metadata":{metadata}}}]"#
        ));
        resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            Listed::all(live),
            &Config::naming(vec![project("proj", "/home/user/proj")]),
        )
    }

    #[test]
    fn a_bead_naming_an_agent_session_is_joined_to_the_pane_herdr_reports_it_in() {
        let live = panes(&format!(
            "{},{}",
            an_agent("w:p1", "sess-other"),
            an_agent("w:p2", "sess-mine")
        ));

        let joined = join_one(r#"{"agent_session":"sess-mine"}"#, &live);

        let a = pane_of(&joined, "proj", "p-1");
        assert_eq!(a.pane, pane_key("w:p2"));
        assert_eq!(a.source, JoinSource::AgentSession);
        assert_eq!(a.agent_session.as_deref(), Some("sess-mine"));
        assert_eq!(loose(&live, &joined), vec!["w:p1"]);
    }

    /// The key is configurable, so a hardcoded `agent_session` cannot pass.
    #[test]
    fn the_agent_session_is_read_from_the_configured_key() {
        let beads = rows(
            r#"[{"id":"p-1","title":"root","status":"in_progress",
                 "metadata":{"seat_session":"sess-mine","agent_session":"sess-other"}}]"#,
        );
        let live = panes(&format!(
            "{},{}",
            an_agent("w:p1", "sess-other"),
            an_agent("w:p2", "sess-mine")
        ));
        let join = Join {
            agent_session_key: "seat_session".to_string(),
            ..Join::default()
        };

        let joined = resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            Listed::all(&live),
            &Config {
                join,
                ..Config::naming(vec![project("proj", "/home/user/proj")])
            },
        );

        assert_eq!(pane_of(&joined, "proj", "p-1").pane, pane_key("w:p2"));
    }

    /// An agent resumed in another pane keeps its bead, and the pane it left
    /// is no longer its.
    #[test]
    fn an_agent_session_that_moved_pane_is_joined_in_its_new_pane_with_no_conflict() {
        let live = panes(&format!(
            "{},{}",
            an_agent("w:p1", "sess-later"),
            an_agent("w:p2", "sess-mine")
        ));

        let joined = join_one(
            r#"{"agent_pane":"w:p1","agent_session":"sess-mine"}"#,
            &live,
        );

        assert_eq!(pane_of(&joined, "proj", "p-1").pane, pane_key("w:p2"));
        assert_eq!(joined.conflicts, vec![]);
        assert_eq!(joined.refused, BTreeMap::new());
        assert_eq!(loose(&live, &joined), vec!["w:p1"]);
    }

    /// A reused pane, or a `/clear` in it, puts another agent where the
    /// bead's agent was.
    #[test]
    fn a_pane_now_holding_another_agent_session_is_not_joined_to_the_bead() {
        let live = panes(&an_agent("w:p1", "sess-later"));

        let joined = join_one(
            r#"{"agent_pane":"w:p1","agent_session":"sess-mine"}"#,
            &live,
        );

        assert_eq!(joined.agents, BTreeMap::new());
        assert_eq!(joined.conflicts, vec![]);
        assert_eq!(loose(&live, &joined), vec!["w:p1"]);
    }

    /// A `/clear` leaves the pane's `display_agent` naming the bead, and the
    /// new agent in the pane is still not the one the bead names.
    #[test]
    fn a_pane_naming_the_bead_does_not_win_it_while_holding_another_agent_session() {
        let live = panes(
            r#"{"pane_id":"w:p1","cwd":"/home/user/proj","agent_status":"working",
                "agent":"claude","display_agent":"p-1",
                "agent_session":{"agent":"claude","kind":"id","source":"herdr:claude",
                                 "value":"sess-later"}}"#,
        );

        let joined = join_one(
            r#"{"agent_pane":"w:p1","agent_session":"sess-mine"}"#,
            &live,
        );

        assert_eq!(joined.agents, BTreeMap::new());
    }

    /// Nothing says which of two panes holding one agent session is the
    /// agent, and a `display_agent` on one of them does not either.
    #[test]
    fn a_pane_naming_the_bead_does_not_win_a_session_several_panes_hold() {
        let live = panes(&format!(
            "{},{}",
            an_agent("w:p1", "sess-mine"),
            an_agent("w:p2", "sess-mine").replacen(
                r#""agent":"claude","#,
                r#""agent":"claude","display_agent":"p-1","#,
                1
            )
        ));

        let joined = join_one(r#"{"agent_session":"sess-mine"}"#, &live);

        assert_eq!(joined.agents, BTreeMap::new());
    }

    /// herdr keeps a pane's agent session after its agent has left, so the
    /// session it still carries names nobody in it.
    #[test]
    fn a_pane_whose_agent_has_left_is_not_joined_to_the_bead_naming_its_old_session() {
        let live = panes(
            r#"{"pane_id":"w:p1","cwd":"/home/user/proj","agent_status":"idle",
                "agent_session":{"agent":"claude","kind":"id","source":"herdr:claude",
                                 "value":"sess-mine"}}"#,
        );

        let joined = join_one(
            r#"{"agent_pane":"w:p1","agent_session":"sess-mine"}"#,
            &live,
        );

        assert_eq!(joined.agents, BTreeMap::new());
        assert_eq!(joined.conflicts, vec![]);
    }

    /// Where herdr reports no session for the pane the bead names, the bead
    /// joins by its pane as though it named no session. An agent herdr reads
    /// no session for keeps its bead, at the cost of a reused pane still
    /// inheriting a claim for it.
    #[test]
    fn a_bead_naming_a_session_joins_by_its_pane_where_herdr_reports_none_there() {
        let live = panes(
            r#"{"pane_id":"w:p1","cwd":"/home/user/proj","agent_status":"working",
                "agent":"claude"}"#,
        );

        let joined = join_one(
            r#"{"agent_pane":"w:p1","agent_session":"sess-mine"}"#,
            &live,
        );

        let a = pane_of(&joined, "proj", "p-1");
        assert_eq!(a.pane, pane_key("w:p1"));
        assert_eq!(a.source, JoinSource::AgentPane);
        assert_eq!(a.agent_session, None);
    }

    /// A bead that names no session joins by its pane, whatever session herdr
    /// reports there.
    #[test]
    fn a_bead_naming_only_a_pane_joins_it_whatever_session_it_holds() {
        let live = panes(&an_agent("w:p1", "sess-any"));

        let joined = join_one(r#"{"agent_pane":"w:p1"}"#, &live);

        let a = pane_of(&joined, "proj", "p-1");
        assert_eq!(a.pane, pane_key("w:p1"));
        assert_eq!(a.source, JoinSource::AgentPane);
        assert_eq!(a.agent_session.as_deref(), Some("sess-any"));
    }

    /// A session of another kind is not the agent's id, so it never matches
    /// the one a bead names, even where the two read the same.
    #[test]
    fn a_session_of_another_kind_is_not_the_agent_session_a_bead_names() {
        let live = panes(
            r#"{"pane_id":"w:p1","cwd":"/home/user/proj","agent_status":"working",
                "agent":"claude",
                "agent_session":{"agent":"claude","kind":"path","source":"herdr:claude",
                                 "value":"sess-mine"}}"#,
        );

        let joined = join_one(r#"{"agent_session":"sess-mine"}"#, &live);

        assert_eq!(joined.agents, BTreeMap::new());
    }

    /// A session herdr reports in no live pane is an agent that is not there,
    /// which `orphan-claim` reports; the join awards nothing.
    #[test]
    fn a_bead_naming_an_agent_session_no_pane_holds_is_joined_to_nothing() {
        let live = panes(&an_agent("w:p1", "sess-other"));

        let joined = join_one(r#"{"agent_session":"sess-mine"}"#, &live);

        assert_eq!(joined.agents, BTreeMap::new());
        assert_eq!(joined.out_of_reach, BTreeSet::new());
        assert_eq!(joined.conflicts, vec![]);
    }

    /// A pane id a quiet session held cannot say the agent left it, so a
    /// session found nowhere live is no orphan while the pane is out of reach.
    #[test]
    fn a_bead_whose_session_is_found_nowhere_is_out_of_reach_with_its_pane() {
        let beads = rows(
            r#"[{"id":"p-1","title":"root","status":"in_progress",
                 "metadata":{"agent_pane":"w:p1","agent_session":"sess-mine"}}]"#,
        );
        let quiet = OutOfReach {
            pane_ids: BTreeSet::from(["w:p1".to_string()]),
            ..OutOfReach::default()
        };

        let joined = resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            Listed {
                panes: &[],
                out_of_reach: &quiet,
            },
            &Config::naming(vec![project("proj", "/home/user/proj")]),
        );

        assert_eq!(joined.out_of_reach, BTreeSet::from([key("proj", "p-1")]));
    }

    /// A bead naming only its agent session has no pane id to place, so the
    /// session itself is placed: one a quiet session's pane last held is out
    /// of reach, and one nothing held is no agent at all.
    #[test]
    fn a_bead_naming_only_a_session_a_quiet_session_last_held_is_out_of_reach() {
        let beads = rows(
            r#"[{"id":"p-1","title":"the seat that went quiet","status":"in_progress",
                 "metadata":{"agent_session":"sess-quiet"}},
                {"id":"p-2","title":"the seat that died","status":"in_progress",
                 "metadata":{"agent_session":"sess-gone"}}]"#,
        );
        let mut missing = OutOfReach::default();
        missing.extend(&panes_in("kadath", &an_agent("w:p1", "sess-quiet")));

        let joined = resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            Listed {
                panes: &[],
                out_of_reach: &missing,
            },
            &Config::naming(vec![project("proj", "/home/user/proj")]),
        );

        assert_eq!(joined.out_of_reach, BTreeSet::from([key("proj", "p-1")]));
        assert_eq!(joined.conflicts, vec![]);
    }

    /// herdr keeps a pane's agent session after its agent has left, so what a
    /// quiet session last held places only the agents live in its panes.
    #[test]
    fn what_a_quiet_session_held_is_its_pane_ids_and_the_agents_live_in_them() {
        let mut missing = OutOfReach::default();
        missing.extend(&panes_in(
            "kadath",
            &format!(
                r#"{},{{"pane_id":"w:p2","cwd":"/home/user/proj","agent_status":"idle",
                     "agent_session":{{"agent":"claude","kind":"id","source":"herdr:claude",
                                       "value":"sess-left"}}}}"#,
                an_agent("w:p1", "sess-quiet")
            ),
        ));

        assert_eq!(
            missing,
            OutOfReach {
                pane_ids: BTreeSet::from(["w:p1".to_string(), "w:p2".to_string()]),
                agent_sessions: BTreeSet::from(["sess-quiet".to_string()]),
            }
        );
    }

    /// Nothing says which of two live panes holding one agent session is the
    /// agent, so the bead gets neither and says which panes hold it.
    #[test]
    fn an_agent_session_two_live_panes_hold_is_refused_and_names_the_panes() {
        let live = panes(&format!(
            "{},{}",
            an_agent("w:p1", "sess-mine"),
            an_agent("w:p2", "sess-mine")
        ));

        let joined = join_one(r#"{"agent_session":"sess-mine"}"#, &live);

        let held_twice = Conflict::AgentSessionInSeveralPanes {
            bead: key("proj", "p-1"),
            agent_session: "sess-mine".to_string(),
            panes: vec![pane_key("w:p1"), pane_key("w:p2")],
        };
        assert_eq!(joined.agents, BTreeMap::new());
        assert_eq!(joined.conflicts, vec![held_twice.clone()]);
        assert_eq!(
            joined.refused,
            BTreeMap::from([(key("proj", "p-1"), held_twice)])
        );
    }

    /// An agent resumed in another pane leaves its old pane carrying its
    /// session with no agent in it. That pane is not a second holder.
    #[test]
    fn an_agent_resumed_in_another_pane_raises_no_conflict() {
        let live = panes(&format!(
            r#"{{"pane_id":"w:p1","cwd":"/home/user/proj","agent_status":"idle",
                 "agent_session":{{"agent":"claude","kind":"id","source":"herdr:claude",
                                   "value":"sess-mine"}}}},{}"#,
            an_agent("w:p2", "sess-mine")
        ));

        let joined = join_one(
            r#"{"agent_pane":"w:p1","agent_session":"sess-mine"}"#,
            &live,
        );

        assert_eq!(pane_of(&joined, "proj", "p-1").pane, pane_key("w:p2"));
        assert_eq!(joined.conflicts, vec![]);
        assert_eq!(joined.refused, BTreeMap::new());
    }

    /// Two beads naming the agent in one pane are a disagreement about the
    /// agent, so it is reported by its session as well as its pane.
    #[test]
    fn several_beads_naming_one_agent_session_leaves_them_all_unclaimed_and_reported() {
        let beads = rows(
            r#"[
              {"id":"p-1","title":"root","status":"open"},
              {"id":"p-1.1","title":"one","status":"in_progress",
               "dependencies":[{"depends_on_id":"p-1","type":"parent-child"}],
               "metadata":{"agent_session":"sess-mine"}},
              {"id":"p-1.2","title":"two","status":"in_progress",
               "dependencies":[{"depends_on_id":"p-1","type":"parent-child"}],
               "metadata":{"agent_session":"sess-mine"}}
            ]"#,
        );
        let live = panes(&an_agent("w:p1", "sess-mine"));

        let joined = resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            Listed::all(&live),
            &Config::naming(vec![project("proj", "/home/user/proj")]),
        );

        let contested = Conflict::SeveralBeadsNameOneAgentSession {
            agent_session: "sess-mine".to_string(),
            pane: pane_key("w:p1"),
            caption: None,
            beads: vec![key("proj", "p-1.1"), key("proj", "p-1.2")],
        };
        assert_eq!(
            joined.agents,
            BTreeMap::new(),
            "neither bead gets the agent"
        );
        assert_eq!(joined.conflicts, vec![contested.clone()]);
        assert_eq!(
            joined.refused,
            BTreeMap::from([
                (key("proj", "p-1.1"), contested.clone()),
                (key("proj", "p-1.2"), contested),
            ])
        );
    }

    /// Where one bead names the agent and another only its pane, they agree
    /// on nothing but the pane, so the pane is what is contested.
    #[test]
    fn a_bead_naming_the_agent_and_one_naming_only_its_pane_contest_the_pane() {
        let beads = rows(
            r#"[
              {"id":"p-1","title":"root","status":"open"},
              {"id":"p-1.1","title":"one","status":"in_progress",
               "dependencies":[{"depends_on_id":"p-1","type":"parent-child"}],
               "metadata":{"agent_session":"sess-mine"}},
              {"id":"p-1.2","title":"two","status":"in_progress",
               "dependencies":[{"depends_on_id":"p-1","type":"parent-child"}],
               "metadata":{"agent_pane":"w:p1"}}
            ]"#,
        );
        let live = panes(&an_agent("w:p1", "sess-mine"));

        let joined = resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            Listed::all(&live),
            &Config::naming(vec![project("proj", "/home/user/proj")]),
        );

        assert_eq!(
            joined.conflicts,
            vec![Conflict::SeveralBeadsNameOnePane {
                pane: pane_key("w:p1"),
                caption: None,
                beads: vec![key("proj", "p-1.1"), key("proj", "p-1.2")],
            }]
        );
    }

    /// The pane exists and its bead does not — that absence is `orphan-claim`'s
    /// to report, not a conflict.
    #[test]
    fn a_bead_naming_a_dead_pane_resolves_to_nothing() {
        let beads = rows(
            r#"[{"id":"p-1","title":"root","status":"in_progress",
                 "metadata":{"agent_pane":"w:pGONE"}}]"#,
        );
        let live = panes(r#"{"pane_id":"w:p1","cwd":"/home/user/proj","agent_status":"idle"}"#);
        let cfg = vec![project("proj", "/home/user/proj")];

        let joined = resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            Listed::all(&live),
            &Config::naming(cfg),
        );

        assert_eq!(joined.agents, BTreeMap::new());
        assert_eq!(joined.conflicts, vec![]);
    }

    /// The same absence, over a run that is missing the pane rather than
    /// holding no such pane. It resolves to nothing either way — a pane this
    /// run cannot see is not a pane it can award — and the difference is
    /// carried out for the rule that would otherwise read the absence.
    ///
    /// Both beads are asked at once, because a set that took every claim
    /// that resolved to nothing would say the same thing and mean nothing.
    #[test]
    fn a_bead_naming_a_pane_this_run_is_missing_is_told_from_one_naming_a_dead_pane() {
        let beads = rows(
            r#"[{"id":"p-1","title":"the seat that went quiet","status":"in_progress",
                 "metadata":{"agent_pane":"w:pQUIET"}},
                {"id":"p-2","title":"the seat that died","status":"in_progress",
                 "metadata":{"agent_pane":"w:pGONE"}}]"#,
        );
        let live = panes(r#"{"pane_id":"w:p1","cwd":"/home/user/proj","agent_status":"idle"}"#);
        let cfg = vec![project("proj", "/home/user/proj")];
        let missing = OutOfReach {
            pane_ids: BTreeSet::from(["w:pQUIET".to_string()]),
            ..OutOfReach::default()
        };

        let joined = resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            Listed {
                panes: &live,
                out_of_reach: &missing,
            },
            &Config::naming(cfg),
        );

        assert_eq!(joined.agents, BTreeMap::new(), "neither pane is live");
        assert_eq!(
            joined.out_of_reach,
            BTreeSet::from([key("proj", "p-1")]),
            "only the claim naming the pane this run is missing"
        );
        assert_eq!(joined.conflicts, vec![]);
    }

    /// A pane id names a pane only within its session, so a live pane
    /// carrying the id a bead named is not necessarily that bead's — the
    /// session that has gone quiet was holding one of the same name. The
    /// join awards neither, exactly as it awards neither when two sessions
    /// that both answered hold the id; it is not reported as a disagreement,
    /// because this run cannot establish that there are two panes, only that
    /// it cannot rule it out.
    #[test]
    fn a_live_pane_carrying_an_id_a_quiet_session_also_held_is_awarded_to_nobody() {
        let beads = rows(
            r#"[{"id":"p-1","title":"root","status":"in_progress",
                 "metadata":{"agent_pane":"w:p1"}}]"#,
        );
        let live = panes(r#"{"pane_id":"w:p1","cwd":"/home/user/proj","agent_status":"working"}"#);
        let cfg = vec![project("proj", "/home/user/proj")];
        let missing = OutOfReach {
            pane_ids: BTreeSet::from(["w:p1".to_string()]),
            ..OutOfReach::default()
        };

        let joined = resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            Listed {
                panes: &live,
                out_of_reach: &missing,
            },
            &Config::naming(cfg),
        );

        assert_eq!(
            joined.agents,
            BTreeMap::new(),
            "the live pane of that id may be the other session's"
        );
        assert_eq!(joined.out_of_reach, BTreeSet::from([key("proj", "p-1")]));
        assert_eq!(joined.conflicts, vec![]);
    }

    /// `display_agent` is free text. Most of it is not a bead id.
    #[test]
    fn a_pane_whose_display_agent_names_no_bead_joins_nothing() {
        let beads = rows(r#"[{"id":"p-1","title":"root","status":"open"}]"#);
        let live = panes(
            r#"{"pane_id":"w:p1","cwd":"/home/user/proj","agent_status":"working",
                "display_agent":"orch: some-effort"}"#,
        );
        let cfg = vec![project("proj", "/home/user/proj")];

        let joined = resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            Listed::all(&live),
            &Config::naming(cfg),
        );

        assert_eq!(joined.agents, BTreeMap::new());
        assert_eq!(joined.conflicts, vec![]);
        assert_eq!(loose(&live, &joined), vec!["w:p1"]);
    }

    // ---- project scope -------------------------------------------------

    #[test]
    fn the_longest_configured_path_containing_a_pane_wins() {
        let cfg = vec![
            project("outer", "/home/user/dev"),
            project("inner", "/home/user/dev/inner"),
        ];

        assert_eq!(
            project_holding(Path::new("/home/user/dev/inner/src"), &cfg).map(|p| p.name.as_str()),
            Some("inner")
        );
        assert_eq!(
            project_holding(Path::new("/home/user/dev/other"), &cfg).map(|p| p.name.as_str()),
            Some("outer")
        );
        assert_eq!(project_holding(Path::new("/home/user"), &cfg), None);
    }

    /// One worktree per seat puts the panes in sibling worktrees, under
    /// neither each other nor the checkout bdi was run from.
    #[test]
    fn a_pane_in_another_worktree_of_the_repository_is_in_the_project() {
        let cfg = vec![project_working_in(
            "proj",
            "/home/user/proj",
            &["/home/user/proj", "/tmp/seat-a/wt"],
        )];

        assert_eq!(
            project_holding(Path::new("/tmp/seat-a/wt/src"), &cfg).map(|p| p.name.as_str()),
            Some("proj")
        );
        assert_eq!(project_holding(Path::new("/tmp/seat-b/wt"), &cfg), None);
    }

    /// A worktree is territory like any other, so the deepest directory
    /// containing the pane still wins the paths two projects share.
    #[test]
    fn a_worktree_deeper_than_another_projects_path_wins_the_pane() {
        let cfg = vec![
            project("outer", "/home/user/dev"),
            project_working_in("inner", "/srv/inner", &["/home/user/dev/wt"]),
        ];

        assert_eq!(
            project_holding(Path::new("/home/user/dev/wt/src"), &cfg).map(|p| p.name.as_str()),
            Some("inner")
        );
    }

    /// A pane in a linked worktree of a project that was never asked where
    /// it is worked — one the scope left out — is held by nothing as it
    /// stands, and is placed by where its directory sits in the main working
    /// tree instead.
    ///
    /// Only a pane built by hand can say so. Every pane read off the wire,
    /// which is every fixture under `tests/fixtures/`, has no main-tree
    /// place and is placed by its `cwd` alone, so a green run of those says
    /// nothing about this rule.
    #[test]
    fn a_pane_nothing_holds_is_placed_by_where_it_sits_in_the_main_working_tree() {
        let cfg = vec![project("proj", "/home/user/proj")];
        let mut live =
            panes(r#"{"pane_id":"w:p1","cwd":"/tmp/seat-a/wt/src","agent_status":"working"}"#);
        let pane = live
            .remove(0)
            .with_cwd_in_the_main_working_tree(Some(PathBuf::from("/home/user/proj/src")));

        assert_eq!(
            project_of(&pane, &cfg).map(|p| p.name.as_str()),
            Some("proj")
        );
    }

    /// The directory the pane is in wins over the one it corresponds to: a
    /// project configured at its place in a linked worktree holds a pane
    /// there, whatever the main working tree is under.
    #[test]
    fn a_pane_something_holds_is_placed_by_its_own_directory_first() {
        let cfg = vec![
            project("seat", "/tmp/seat-a/wt"),
            project("main", "/home/user/proj"),
        ];
        let mut live =
            panes(r#"{"pane_id":"w:p1","cwd":"/tmp/seat-a/wt/src","agent_status":"working"}"#);
        let pane = live
            .remove(0)
            .with_cwd_in_the_main_working_tree(Some(PathBuf::from("/home/user/proj/src")));

        assert_eq!(
            project_of(&pane, &cfg).map(|p| p.name.as_str()),
            Some("seat")
        );
    }

    #[test]
    fn a_pane_read_off_the_wire_is_placed_by_its_directory_alone() {
        let cfg = vec![project("proj", "/home/user/proj")];
        let live =
            panes(r#"{"pane_id":"w:p1","cwd":"/tmp/seat-a/wt/src","agent_status":"working"}"#);

        assert_eq!(live[0].cwd_in_the_main_working_tree(), None);
        assert_eq!(project_of(&live[0], &cfg), None);
    }

    /// The conflict a pane raises names the project it sits in, so the
    /// placement reaches it through the same rule.
    #[test]
    fn a_pane_placed_by_the_main_working_tree_is_reported_in_that_project() {
        let two = rows(
            r#"[{"id":"x-1","title":"in project two","status":"in_progress",
                 "metadata":{"agent_pane":"w:p1"}}]"#,
        );
        let mut live =
            panes(r#"{"pane_id":"w:p1","cwd":"/tmp/seat-a/wt/src","agent_status":"working"}"#);
        let pane = live
            .remove(0)
            .with_cwd_in_the_main_working_tree(Some(PathBuf::from("/home/user/one/src")));
        let cfg = vec![
            project("one", "/home/user/one"),
            project("two", "/home/user/two"),
        ];

        let joined = resolve(
            &[ProjectRows {
                project: "two",
                rows: &two,
            }],
            Listed::all(&[pane]),
            &Config::naming(cfg),
        );

        assert_eq!(
            joined.conflicts,
            vec![Conflict::PaneInAnotherProject {
                bead: key("two", "x-1"),
                pane: pane_key("w:p1"),
                pane_project: Some("one".to_string()),
            }]
        );
    }

    /// A sibling directory sharing a textual prefix is a different project.
    #[test]
    fn a_path_is_matched_by_whole_directories_rather_than_by_text() {
        let cfg = vec![project("bead", "/home/user/dev/bead")];

        assert_eq!(
            project_holding(Path::new("/home/user/dev/beady"), &cfg),
            None
        );
    }

    #[test]
    fn a_pane_in_no_configured_project_joins_nothing_and_is_unattributed() {
        let beads = rows(BEADS);
        let live = parse_agent_list(A_SESSION, PANES).expect("the fixture parses");
        let cfg = vec![project("dunwich", FIXTURE_PROJECT_PATH)];

        let joined = resolve(
            &[ProjectRows {
                project: "dunwich",
                rows: &beads,
            }],
            Listed::all(&live),
            &Config::naming(cfg),
        );

        // wF:pD names bead lnd-4t8.5 from an unconfigured lander, and is as
        // unattributed as the panes that named nothing at all.
        assert_eq!(
            loose(&live, &joined),
            vec!["wE:p1", "wE:p2", "wF:p1", "wF:pD", "wG:p1", "wG:p2", "wG:p4", "wG:p6", "wH:p1",]
        );
        assert_eq!(joined.conflicts, vec![]);
    }

    /// The reason the key is `(project, id)`: prefixes are per-tracker and
    /// nothing coordinates them.
    #[test]
    fn colliding_prefixes_do_not_cross_attach_an_inferred_agent() {
        let one = rows(r#"[{"id":"x-1","title":"in project one","status":"in_progress"}]"#);
        let two = rows(r#"[{"id":"x-1","title":"in project two","status":"in_progress"}]"#);
        let live = panes(
            r#"{"pane_id":"w:p1","cwd":"/home/user/one/src","agent_status":"working",
                "display_agent":"x-1"}"#,
        );
        let cfg = vec![
            project("one", "/home/user/one"),
            project("two", "/home/user/two"),
        ];

        let joined = resolve(
            &[
                ProjectRows {
                    project: "one",
                    rows: &one,
                },
                ProjectRows {
                    project: "two",
                    rows: &two,
                },
            ],
            Listed::all(&live),
            &Config::naming(cfg),
        );

        assert_eq!(pane_of(&joined, "one", "x-1").pane, pane_key("w:p1"));
        assert_eq!(
            joined.agents.len(),
            1,
            "project two's x-1 is a different bead"
        );
    }

    #[test]
    fn colliding_prefixes_do_not_cross_attach_an_exact_agent() {
        let one = rows(r#"[{"id":"x-1","title":"in project one","status":"open"}]"#);
        let two = rows(
            r#"[{"id":"x-1","title":"in project two","status":"in_progress",
                 "metadata":{"agent_pane":"w:p1"}}]"#,
        );
        let live =
            panes(r#"{"pane_id":"w:p1","cwd":"/home/user/one/src","agent_status":"working"}"#);
        let cfg = vec![
            project("one", "/home/user/one"),
            project("two", "/home/user/two"),
        ];

        let joined = resolve(
            &[
                ProjectRows {
                    project: "one",
                    rows: &one,
                },
                ProjectRows {
                    project: "two",
                    rows: &two,
                },
            ],
            Listed::all(&live),
            &Config::naming(cfg),
        );

        assert_eq!(
            joined.agents,
            BTreeMap::new(),
            "the pane is not project two's to claim"
        );
        assert_eq!(
            joined.conflicts,
            vec![Conflict::PaneInAnotherProject {
                bead: key("two", "x-1"),
                pane: pane_key("w:p1"),
                pane_project: Some("one".to_string()),
            }]
        );
        assert_eq!(
            joined.refused,
            BTreeMap::from([(
                key("two", "x-1"),
                Conflict::PaneInAnotherProject {
                    bead: key("two", "x-1"),
                    pane: pane_key("w:p1"),
                    pane_project: Some("one".to_string()),
                }
            )])
        );
    }

    /// A pane naming a bead that exists only in another project is the same
    /// finding from the other direction.
    #[test]
    fn a_pane_naming_another_projects_bead_does_not_join_and_is_reported() {
        let one = rows(r#"[{"id":"a-1","title":"only in one","status":"open"}]"#);
        let two = rows(r#"[{"id":"b-1","title":"only in two","status":"open"}]"#);
        let live = panes(
            r#"{"pane_id":"w:p1","cwd":"/home/user/two","agent_status":"working",
                "display_agent":"a-1"}"#,
        );
        let cfg = vec![
            project("one", "/home/user/one"),
            project("two", "/home/user/two"),
        ];

        let joined = resolve(
            &[
                ProjectRows {
                    project: "one",
                    rows: &one,
                },
                ProjectRows {
                    project: "two",
                    rows: &two,
                },
            ],
            Listed::all(&live),
            &Config::naming(cfg),
        );

        assert_eq!(joined.agents, BTreeMap::new());
        assert_eq!(
            joined.conflicts,
            vec![Conflict::PaneInAnotherProject {
                bead: key("one", "a-1"),
                pane: pane_key("w:p1"),
                pane_project: Some("two".to_string()),
            }]
        );
        assert_eq!(
            joined.refused,
            BTreeMap::new(),
            "project one's bead named no pane, so it is owed no reason it has none: \
             what a pane in another project calls itself is not its claim"
        );
    }

    /// A pane under a project the scope left out is neither drawn nor
    /// reported. Its own tracker was never read, so an id it names that a
    /// read project happens to hold says nothing about that project's bead —
    /// the pane is most likely on its own project's bead of that id.
    #[test]
    fn a_pane_in_a_project_the_scope_left_out_names_nothing_and_is_not_reported() {
        let one = rows(r#"[{"id":"x-1","title":"in project one","status":"in_progress"}]"#);
        let live = panes(
            r#"{"pane_id":"w:p1","cwd":"/home/user/two/src","agent_status":"working",
                "display_agent":"x-1"}"#,
        );
        let cfg = Config::naming(vec![
            project("one", "/home/user/one"),
            project("two", "/home/user/two"),
        ])
        .scoped_to(&["one".to_string()])
        .expect("one is configured");

        let joined = resolve(
            &[ProjectRows {
                project: "one",
                rows: &one,
            }],
            Listed::all(&live),
            &cfg,
        );

        assert_eq!(joined.agents, BTreeMap::new());
        assert_eq!(joined.conflicts, vec![]);
    }

    /// A read bead naming a pane that sits in a project the scope left out
    /// is reported as a pane in that project — which the config still names
    /// — rather than as one in a directory no project covers.
    #[test]
    fn a_bead_naming_a_pane_in_a_project_the_scope_left_out_is_told_which_project() {
        let beads = rows(
            r#"[{"id":"p-1","title":"root","status":"in_progress",
                 "metadata":{"agent_pane":"w:p1"}}]"#,
        );
        let live =
            panes(r#"{"pane_id":"w:p1","cwd":"/home/user/two/src","agent_status":"working"}"#);
        let cfg = Config::naming(vec![
            project("one", "/home/user/one"),
            project("two", "/home/user/two"),
        ])
        .scoped_to(&["one".to_string()])
        .expect("one is configured");

        let joined = resolve(
            &[ProjectRows {
                project: "one",
                rows: &beads,
            }],
            Listed::all(&live),
            &cfg,
        );

        assert_eq!(
            joined.conflicts,
            vec![Conflict::PaneInAnotherProject {
                bead: key("one", "p-1"),
                pane: pane_key("w:p1"),
                pane_project: Some("two".to_string()),
            }]
        );
    }

    #[test]
    fn a_bead_naming_a_pane_in_no_configured_project_does_not_join_and_is_reported() {
        let beads = rows(
            r#"[{"id":"p-1","title":"root","status":"in_progress",
                 "metadata":{"agent_pane":"w:p1"}}]"#,
        );
        let live = panes(r#"{"pane_id":"w:p1","cwd":"/tmp","agent_status":"working"}"#);
        let cfg = vec![project("proj", "/home/user/proj")];

        let joined = resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            Listed::all(&live),
            &Config::naming(cfg),
        );

        assert_eq!(joined.agents, BTreeMap::new());
        let outside = Conflict::PaneInAnotherProject {
            bead: key("proj", "p-1"),
            pane: pane_key("w:p1"),
            pane_project: None,
        };
        assert_eq!(joined.conflicts, vec![outside.clone()]);
        assert_eq!(
            joined.refused,
            BTreeMap::from([(key("proj", "p-1"), outside)])
        );
    }

    // ---- the disagreements ---------------------------------------------

    #[test]
    fn the_two_directions_naming_different_panes_is_reported_and_the_beads_key_wins() {
        let beads = rows(
            r#"[{"id":"p-1","title":"root","status":"in_progress",
                 "metadata":{"agent_pane":"w:p1"}}]"#,
        );
        let live = panes(
            r#"{"pane_id":"w:p1","cwd":"/home/user/proj","agent_status":"working"},
               {"pane_id":"w:p2","cwd":"/home/user/proj","agent_status":"idle",
                "display_agent":"p-1"}"#,
        );
        let cfg = vec![project("proj", "/home/user/proj")];

        let joined = resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            Listed::all(&live),
            &Config::naming(cfg),
        );

        let a = pane_of(&joined, "proj", "p-1");
        assert_eq!(
            a.pane,
            pane_key("w:p1"),
            "the bead's own key is exact and wins"
        );
        assert_eq!(a.source, JoinSource::AgentPane);
        assert_eq!(
            joined.conflicts,
            vec![Conflict::BeadAndPaneDisagree {
                bead: key("proj", "p-1"),
                named_by_bead: pane_key("w:p1"),
                named_by_pane: pane_key("w:p2"),
            }],
            "winning is not the same as agreeing"
        );
    }

    /// Discrimination for the test above: both directions pointing at one pane
    /// is agreement, and agreement is not a finding.
    #[test]
    fn the_two_directions_naming_one_pane_is_not_a_conflict() {
        let beads = rows(
            r#"[{"id":"p-1","title":"root","status":"in_progress",
                 "metadata":{"agent_pane":"w:p1"}}]"#,
        );
        let live = panes(
            r#"{"pane_id":"w:p1","cwd":"/home/user/proj","agent_status":"working",
                "display_agent":"p-1"}"#,
        );
        let cfg = vec![project("proj", "/home/user/proj")];

        let joined = resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            Listed::all(&live),
            &Config::naming(cfg),
        );

        assert_eq!(
            pane_of(&joined, "proj", "p-1").source,
            JoinSource::AgentPane
        );
        assert_eq!(joined.conflicts, vec![]);
    }

    #[test]
    fn several_panes_naming_one_bead_leaves_it_unclaimed_and_reported() {
        let beads = rows(r#"[{"id":"p-1","title":"root","status":"in_progress"}]"#);
        let live = panes(
            r#"{"pane_id":"w:p2","cwd":"/home/user/proj","agent_status":"working",
                "display_agent":"p-1"},
               {"pane_id":"w:p1","cwd":"/home/user/proj","agent_status":"idle",
                "display_agent":"p-1"}"#,
        );
        let cfg = vec![project("proj", "/home/user/proj")];

        let joined = resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            Listed::all(&live),
            &Config::naming(cfg),
        );

        assert_eq!(joined.agents, BTreeMap::new(), "neither pane wins");
        assert_eq!(
            joined.conflicts,
            vec![Conflict::SeveralPanesNameOneBead {
                bead: key("proj", "p-1"),
                panes: vec![pane_key("w:p1"), pane_key("w:p2")],
            }]
        );
        assert_eq!(loose(&live, &joined), vec!["w:p1", "w:p2"]);
    }

    #[test]
    fn several_beads_naming_one_pane_leaves_them_all_unclaimed_and_reported() {
        let beads = rows(
            r#"[
              {"id":"p-1","title":"root","status":"open"},
              {"id":"p-1.1","title":"one","status":"in_progress",
               "dependencies":[{"depends_on_id":"p-1","type":"parent-child"}],
               "metadata":{"agent_pane":"w:p1"}},
              {"id":"p-1.2","title":"two","status":"in_progress",
               "dependencies":[{"depends_on_id":"p-1","type":"parent-child"}],
               "metadata":{"agent_pane":"w:p1"}}
            ]"#,
        );
        let live = panes(r#"{"pane_id":"w:p1","cwd":"/home/user/proj","agent_status":"working"}"#);
        let cfg = vec![project("proj", "/home/user/proj")];

        let joined = resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            Listed::all(&live),
            &Config::naming(cfg),
        );

        assert_eq!(joined.agents, BTreeMap::new(), "neither bead gets the pane");
        let contested = Conflict::SeveralBeadsNameOnePane {
            pane: pane_key("w:p1"),
            caption: None,
            beads: vec![key("proj", "p-1.1"), key("proj", "p-1.2")],
        };
        assert_eq!(joined.conflicts, vec![contested.clone()]);
        assert_eq!(
            joined.refused,
            BTreeMap::from([
                (key("proj", "p-1.1"), contested.clone()),
                (key("proj", "p-1.2"), contested),
            ]),
            "each of them claimed the pane, so each is owed the reason it has none"
        );
    }

    /// The reading this was measured from: one seat claimed three beads in
    /// turn and cleared its key on none of them, so three claims stood on the
    /// pane it was still sitting in. `bdi` awards it to none of them, and the
    /// only thing on the screen that tells the live claim from the two stale
    /// ones is the pane's own account of what it is working on — which the
    /// disagreement therefore carries. The session here is the one captured
    /// while it was happening.
    #[test]
    fn a_contested_pane_carries_its_own_account_of_what_it_is_working_on() {
        let beads = rows(
            r#"[
              {"id":"dun-9fw","title":"dunwich v1","status":"open"},
              {"id":"dun-2bb.16","title":"a claim its seat moved on from",
               "status":"in_progress",
               "dependencies":[{"depends_on_id":"dun-9fw","type":"parent-child"}],
               "metadata":{"agent_pane":"wM:pG"}},
              {"id":"dun-9fw.12","title":"retiring the dep-tree row shape",
               "status":"in_progress",
               "dependencies":[{"depends_on_id":"dun-9fw","type":"parent-child"}],
               "metadata":{"agent_pane":"wM:pG"}},
              {"id":"dun-2bb.19","title":"the other claim it moved on from",
               "status":"in_progress",
               "dependencies":[{"depends_on_id":"dun-9fw","type":"parent-child"}],
               "metadata":{"agent_pane":"wM:pG"}}
            ]"#,
        );
        let live = parse_agent_list(A_SESSION, JOINED_PANES).expect("the fixture parses");
        let cfg = vec![project("dunwich", FIXTURE_PROJECT_PATH)];

        let joined = resolve(
            &[ProjectRows {
                project: "dunwich",
                rows: &beads,
            }],
            Listed::all(&live),
            &Config::naming(cfg),
        );

        assert_eq!(joined.agents, BTreeMap::new(), "none of the three gets it");
        assert_eq!(
            joined.conflicts,
            vec![Conflict::SeveralBeadsNameOnePane {
                pane: pane_key("wM:pG"),
                caption: Some(
                    "dun-9fw.12: retiring the dep-tree row shape from fixtures".to_string()
                ),
                beads: vec![
                    key("dunwich", "dun-2bb.16"),
                    key("dunwich", "dun-2bb.19"),
                    key("dunwich", "dun-9fw.12"),
                ],
            }]
        );
    }

    /// A pane with nothing to say about itself still contests, and the
    /// disagreement says what it can rather than inventing the rest.
    #[test]
    fn a_contested_pane_that_says_nothing_about_itself_carries_nothing() {
        let beads = rows(
            r#"[
              {"id":"p-1","title":"root","status":"open"},
              {"id":"p-1.1","title":"one","status":"in_progress",
               "dependencies":[{"depends_on_id":"p-1","type":"parent-child"}],
               "metadata":{"agent_pane":"w:p1"}},
              {"id":"p-1.2","title":"two","status":"in_progress",
               "dependencies":[{"depends_on_id":"p-1","type":"parent-child"}],
               "metadata":{"agent_pane":"w:p1"}}
            ]"#,
        );
        let live = panes(r#"{"pane_id":"w:p1","cwd":"/home/user/proj","agent_status":"working"}"#);
        let cfg = vec![project("proj", "/home/user/proj")];

        let joined = resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            Listed::all(&live),
            &Config::naming(cfg),
        );

        assert_eq!(
            joined.conflicts,
            vec![Conflict::SeveralBeadsNameOnePane {
                pane: pane_key("w:p1"),
                caption: None,
                beads: vec![key("proj", "p-1.1"), key("proj", "p-1.2")],
            }]
        );
    }

    /// A pane contested by two beads is still a pane one of them might be
    /// working. The inferred direction fills the gap the void left; both
    /// findings stand.
    #[test]
    fn a_bead_whose_exact_claim_was_voided_still_takes_an_uncontested_inferred_pane() {
        let beads = rows(
            r#"[
              {"id":"p-1","title":"root","status":"open"},
              {"id":"p-1.1","title":"one","status":"in_progress",
               "dependencies":[{"depends_on_id":"p-1","type":"parent-child"}],
               "metadata":{"agent_pane":"w:p1"}},
              {"id":"p-1.2","title":"two","status":"in_progress",
               "dependencies":[{"depends_on_id":"p-1","type":"parent-child"}],
               "metadata":{"agent_pane":"w:p1"}}
            ]"#,
        );
        let live = panes(
            r#"{"pane_id":"w:p1","cwd":"/home/user/proj","agent_status":"working"},
               {"pane_id":"w:p9","cwd":"/home/user/proj","agent_status":"working",
                "display_agent":"p-1.1"}"#,
        );
        let cfg = vec![project("proj", "/home/user/proj")];

        let joined = resolve(
            &[ProjectRows {
                project: "proj",
                rows: &beads,
            }],
            Listed::all(&live),
            &Config::naming(cfg),
        );

        let a = pane_of(&joined, "proj", "p-1.1");
        assert_eq!(a.pane, pane_key("w:p9"));
        assert_eq!(a.source, JoinSource::DisplayAgent);
        assert_eq!(
            joined.conflicts,
            vec![
                Conflict::BeadAndPaneDisagree {
                    bead: key("proj", "p-1.1"),
                    named_by_bead: pane_key("w:p1"),
                    named_by_pane: pane_key("w:p9"),
                },
                Conflict::SeveralBeadsNameOnePane {
                    pane: pane_key("w:p1"),
                    caption: None,
                    beads: vec![key("proj", "p-1.1"), key("proj", "p-1.2")],
                },
            ]
        );
    }

    // ---- the contract ----------------------------------------------------

    #[test]
    fn a_conflict_serialises_under_its_own_name() {
        let out = serde_json::to_string(&Conflict::SeveralPanesNameOneBead {
            bead: key("proj", "p-1"),
            panes: vec![pane_key("w:p1")],
        })
        .unwrap();

        assert_eq!(
            out,
            r#"{"conflict":"several-panes-name-one-bead","bead":{"project":"proj","id":"p-1"},"panes":[{"session":"default","id":"w:p1"}]}"#
        );
    }
}
