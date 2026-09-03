//! herdr's command line as the way to the panes on this machine.
//!
//! The one module that spells `herdr session …` or `herdr agent …`, or reads
//! their envelopes. Each question the seam asks is one herdr invocation,
//! answered in herdr's own JSON and parsed here and nowhere else.
//!
//! A box runs several sessions at once, each its own server with its own
//! socket, and `herdr agent list` answers for one of them: the one named on
//! the command line, else the one `bdi`'s environment names, else the
//! default. `bdi` may be run outside herdr, so it takes the sessions from
//! `herdr session list` and names each on the command line, and nothing here
//! treats the session `bdi` happens to sit in as special.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;

use crate::collect::agents::Agents;
use crate::collect::run::{Env, RunFailure, Runner};
use crate::model::types::{Pane, PaneKey, PaneStatus};

#[derive(Deserialize)]
struct Envelope {
    result: AgentList,
}

#[derive(Deserialize)]
struct AgentList {
    agents: Vec<Agent>,
}

/// What `herdr session list --json` answers, which has no envelope.
#[derive(Deserialize)]
struct SessionList {
    sessions: Vec<Session>,
}

/// One session as herdr writes it. It says where the session's socket is as
/// well, which `bdi` never reads: herdr is asked by the session's name.
#[derive(Deserialize)]
struct Session {
    name: String,
    running: bool,
}

/// Parse the output of `herdr session list --json` into the names of the
/// sessions that are running. A session that is not running has no server
/// to answer for it and holds no pane.
pub fn parse_session_list(s: &str) -> anyhow::Result<Vec<String>> {
    let listed: SessionList = serde_json::from_str(s)?;
    Ok(listed
        .sessions
        .into_iter()
        .filter(|session| session.running)
        .map(|session| session.name)
        .collect())
}

/// One pane as herdr writes it.
///
/// The field names are herdr's and stay so on `Pane`, under the terminology
/// rule — another provider maps into them. What is herdr's alone is the shape
/// on the wire: which of them it may leave out, which is what the defaults
/// here say.
#[derive(Deserialize)]
struct Agent {
    pane_id: String,
    cwd: PathBuf,
    #[serde(default)]
    display_agent: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    state_labels: BTreeMap<String, String>,
    agent_status: PaneStatus,
}

impl Agent {
    /// This pane as `bdi` holds it, in the session that listed it. The
    /// listing does not name the session: a session answers for its own
    /// panes, and which one was asked is the caller's to remember.
    fn in_session(self, session: &str) -> Pane {
        let mut pane = Pane::answered(
            session.to_string(),
            self.pane_id,
            self.cwd,
            self.agent_status,
        );
        pane.display_agent = self.display_agent;
        pane.title = self.title;
        pane.state_labels = self.state_labels;
        pane
    }
}

/// Parse the output of `herdr agent list`, asked of `session`.
pub fn parse_agent_list(session: &str, s: &str) -> anyhow::Result<Vec<Pane>> {
    let envelope: Envelope = serde_json::from_str(s)?;
    Ok(envelope
        .result
        .agents
        .into_iter()
        .map(|agent| agent.in_session(session))
        .collect())
}

/// herdr as the provider of a run's panes, reached through one runner.
pub struct Herdr<'r> {
    runner: &'r dyn Runner,
}

impl<'r> Herdr<'r> {
    pub fn new(runner: &'r dyn Runner) -> Self {
        Self { runner }
    }
}

impl Agents for Herdr<'_> {
    fn name(&self) -> &'static str {
        "herdr"
    }

    fn sessions(&self) -> Result<Vec<String>, RunFailure> {
        session_list(self.runner)
    }

    fn list(&self, session: &str) -> Result<Vec<Pane>, RunFailure> {
        agent_list(self.runner, session)
    }

    fn read(&self, pane: &PaneKey, lines: u16) -> Result<Vec<String>, RunFailure> {
        agent_read(self.runner, pane, lines)
    }

    fn focus(&self, pane: &PaneKey) -> Result<(), RunFailure> {
        agent_focus(self.runner, pane)
    }
}

/// `herdr session list --json`: every session on the box, whichever one
/// `bdi` is in, if any. Asked once per collection and takes no project's
/// directory or credential. A failure here is not fatal: the caller degrades
/// to a tier with no panes in it.
fn session_list(runner: &dyn Runner) -> Result<Vec<String>, RunFailure> {
    let out = runner.run("herdr", &["session", "list", "--json"], None, &Env::new())?;
    parse_session_list(&out).map_err(|e| RunFailure::parse("herdr", e))
}

/// `herdr --session <name> agent list`, which answers with JSON and needs no
/// flag to.
///
/// The session is named on the command line every time, because `agent
/// list` otherwise answers for whichever session `bdi`'s own environment
/// names — the one the pane `bdi` sits in, or the default outside herdr —
/// and nothing from any other (`bdi-dd5`). A failure here is one session's:
/// the caller reports it and draws the rest.
fn agent_list(runner: &dyn Runner, session: &str) -> Result<Vec<Pane>, RunFailure> {
    let out = runner.run(
        "herdr",
        &["--session", session, "agent", "list"],
        None,
        &Env::new(),
    )?;
    parse_agent_list(session, &out).map_err(|e| RunFailure::parse("herdr", e))
}

/// `herdr --session <session> agent read <pane>`, as the lines it drew.
///
/// The session is named for the reason `agent_list` names it, and here the
/// cost of leaving it off is quieter: a pane id is minted per session, so a
/// read that named none would read whichever session `bdi` sits in and draw
/// that session's pane of the same id under this one's name.
///
/// Always the visible screen. herdr's other snapshots are not `bdi`'s to ask
/// for: `recent` and `recent-unwrapped` scroll a pane's history, which herdr
/// will only do while the pane is idle — asked for either while an
/// alternate-screen pane is working, which is every Claude Code agent most of
/// the time, it refuses with `agent_not_idle` and names `visible` as the way
/// through — and `detection` is the snapshot herdr takes to decide a pane's
/// state rather than a view it offers a reader.
///
/// `lines` counts back from the newest and clamps to what the snapshot holds.
/// With its styling, as SGR sequences in the rows: what the pane drew is the
/// colour it drew it in, and the tail reads that off the text where it draws
/// it. Every row is wrapped at the pane's own width, and ends `\r\n`.
fn agent_read(runner: &dyn Runner, pane: &PaneKey, lines: u16) -> Result<Vec<String>, RunFailure> {
    let lines = lines.to_string();
    let out = runner.run(
        "herdr",
        &[
            "--session",
            &pane.session,
            "agent",
            "read",
            &pane.id,
            "--source",
            "visible",
            "--lines",
            &lines,
            "--format",
            "ansi",
        ],
        None,
        &Env::new(),
    )?;
    Ok(out.lines().map(str::to_string).collect())
}

/// `herdr --session <session> agent focus <pane>` — the only write `bdi`
/// performs.
fn agent_focus(runner: &dyn Runner, pane: &PaneKey) -> Result<(), RunFailure> {
    runner.run(
        "herdr",
        &["--session", &pane.session, "agent", "focus", &pane.id],
        None,
        &Env::new(),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::types::testing::{key, A_SESSION};
    use crate::model::types::PaneStatus;
    use pretty_assertions::assert_eq;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    const FIXTURE: &str = include_str!("../../tests/fixtures/herdr_agent_list.json");

    fn pane<'a>(panes: &'a [Pane], id: &str) -> &'a Pane {
        panes
            .iter()
            .find(|p| p.pane_id == id)
            .expect("pane is in the fixture")
    }

    #[test]
    fn unwraps_the_envelope() {
        let panes = parse_agent_list(A_SESSION, FIXTURE).expect("parses");

        assert_eq!(panes.len(), 10);
    }

    #[test]
    fn a_pane_that_has_not_identified_itself_is_kept() {
        let panes = parse_agent_list(A_SESSION, FIXTURE).unwrap();
        let p = pane(&panes, "wCW:p1");

        assert_eq!(p.display_agent, None);
        assert_eq!(p.title, None);
        assert_eq!(p.state_labels, BTreeMap::new());
        assert_eq!(p.caption(), None);
    }

    #[test]
    fn reads_every_field_of_an_identified_pane() {
        let panes = parse_agent_list(A_SESSION, FIXTURE).unwrap();
        let p = pane(&panes, "wCW:p6");

        assert_eq!(p.cwd, PathBuf::from("/tmp/bdi-ground/beady-eye"));
        assert_eq!(p.display_agent.as_deref(), Some("bdi-3um.5"));
        assert_eq!(
            p.title.as_deref(),
            Some("parse herdr agent list into typed panes")
        );
        assert_eq!(p.agent_status, PaneStatus::Working);
        assert_eq!(
            p.state_labels.get("idle").map(String::as_str),
            Some("asleep: fixture captured, awaiting review")
        );
    }

    #[test]
    fn caption_prefers_the_label_for_the_current_state() {
        let panes = parse_agent_list(A_SESSION, FIXTURE).unwrap();
        let p = pane(&panes, "wCW:p6");

        assert_eq!(p.agent_status, PaneStatus::Working);
        assert_eq!(p.caption(), Some("writing the parser and its tests"));
    }

    #[test]
    fn caption_falls_back_to_title_when_a_pane_has_no_labels() {
        let panes = parse_agent_list(A_SESSION, FIXTURE).unwrap();
        let p = pane(&panes, "wCW:p5");

        assert_eq!(p.state_labels, BTreeMap::new());
        assert_eq!(p.caption(), Some("parse bd dep-tree JSON into typed rows"));
    }

    /// The same labels under two states, so the lookup cannot be a fixed key.
    #[test]
    fn caption_follows_the_state_the_pane_is_in() {
        let list = r#"{"id":"cli:agent:list","result":{"type":"agent_list","agents":[
            {"pane_id":"w:p1","cwd":"/tmp","agent_status":"idle","title":"a title",
             "state_labels":{"idle":"the idle line","working":"the working line"}},
            {"pane_id":"w:p2","cwd":"/tmp","agent_status":"working","title":"a title",
             "state_labels":{"idle":"the idle line","working":"the working line"}}
        ]}}"#;

        let panes = parse_agent_list(A_SESSION, list).unwrap();

        assert_eq!(panes[0].caption(), Some("the idle line"));
        assert_eq!(panes[1].caption(), Some("the working line"));
    }

    /// Labels present, but none for this state — the title still wins over
    /// whichever label happens to be there.
    #[test]
    fn caption_falls_back_to_title_when_no_label_covers_this_state() {
        let list = r#"{"id":"cli:agent:list","result":{"type":"agent_list","agents":[
            {"pane_id":"w:p1","cwd":"/tmp","agent_status":"blocked","title":"the title",
             "state_labels":{"idle":"the idle line","working":"the working line"}}
        ]}}"#;

        let panes = parse_agent_list(A_SESSION, list).unwrap();

        assert_eq!(panes[0].caption(), Some("the title"));
    }

    /// herdr's four states, so a rename in the enum cannot pass unnoticed. The
    /// fixture is one capture and holds whichever states this machine was in.
    #[test]
    fn reads_each_state_herdr_reports() {
        let list = r#"{"id":"cli:agent:list","result":{"type":"agent_list","agents":[
            {"pane_id":"w:p1","cwd":"/tmp","agent_status":"idle"},
            {"pane_id":"w:p2","cwd":"/tmp","agent_status":"working"},
            {"pane_id":"w:p3","cwd":"/tmp","agent_status":"blocked"},
            {"pane_id":"w:p4","cwd":"/tmp","agent_status":"done"}
        ]}}"#;

        let got: Vec<PaneStatus> = parse_agent_list(A_SESSION, list)
            .expect("parses")
            .into_iter()
            .map(|p| p.agent_status)
            .collect();

        assert_eq!(
            got,
            vec![
                PaneStatus::Idle,
                PaneStatus::Working,
                PaneStatus::Blocked,
                PaneStatus::Done,
            ]
        );
    }

    /// A state a future herdr reports must not break the parse.
    #[test]
    fn an_unrecognised_state_is_kept_verbatim() {
        let list = r#"{"id":"cli:agent:list","result":{"type":"agent_list","agents":[
            {"pane_id":"w:p1","cwd":"/tmp","agent_status":"hibernating","title":"a state we do not know"}
        ]}}"#;

        let panes = parse_agent_list(A_SESSION, list).expect("an unknown state still parses");

        assert_eq!(
            panes[0].agent_status,
            PaneStatus::Other("hibernating".into())
        );
        assert_eq!(panes[0].caption(), Some("a state we do not know"));

        let out = serde_json::to_string(&panes[0].agent_status).unwrap();
        assert_eq!(out, r#""hibernating""#);
    }

    /// The contract spells the known states the way herdr does, not the way
    /// Rust does.
    #[test]
    fn a_known_state_serialises_back_to_herdrs_spelling() {
        let out = serde_json::to_string(&PaneStatus::Blocked).unwrap();

        assert_eq!(out, r#""blocked""#);
    }

    use crate::collect::run::testing::FakeRunner;
    use crate::collect::run::FailureKind;

    const SESSIONS: &str = include_str!("../../tests/fixtures/herdr_session_list.json");

    /// The capture: three sessions running on this machine, and `bdi` in
    /// none of them in particular.
    #[test]
    fn session_list_names_every_running_session() {
        let sessions = parse_session_list(SESSIONS).expect("parses");

        assert_eq!(sessions, ["default", "beacon", "persistent-agents"]);
    }

    /// A session that is not running has no server to answer for it, so it
    /// is not a session to ask.
    #[test]
    fn a_session_that_is_not_running_is_left_out() {
        let list = r#"{"sessions":[
            {"default":true,"name":"default","running":true,"session_dir":"/h","socket_path":"/h/herdr.sock"},
            {"default":false,"name":"stopped","running":false,"session_dir":"/h/sessions/stopped","socket_path":"/h/sessions/stopped/herdr.sock"}
        ]}"#;

        let sessions = parse_session_list(list).expect("parses");

        assert_eq!(sessions, ["default"]);
    }

    /// The one call that finds the sessions, and it names no session itself:
    /// there is no session to name before this has answered.
    #[test]
    fn session_list_asks_herdr_once_with_no_session_named() {
        let runner = FakeRunner::default().with("herdr session list --json", SESSIONS);

        assert_eq!(session_list(&runner).unwrap().len(), 3);

        let call = runner.call("herdr session list --json");
        assert_eq!(call.cwd, None);
        assert!(call.env.is_empty());
    }

    /// One session's panes, asked of that session by name on the command
    /// line — never left to whatever session `bdi`'s own environment names —
    /// and every pane that comes back is held as that session's.
    #[test]
    fn agent_list_asks_the_session_it_is_given_by_name() {
        let runner = FakeRunner::default().with("herdr --session beacon agent list", FIXTURE);

        let panes = agent_list(&runner, "beacon").unwrap();

        assert_eq!(panes.len(), 10);
        assert!(panes.iter().all(|pane| pane.session == "beacon"));
        let call = runner.call("herdr --session beacon agent list");
        assert_eq!(call.cwd, None);
        assert!(call.env.is_empty());
    }

    #[test]
    fn a_missing_herdr_is_a_failure_the_caller_can_degrade_on() {
        let runner = FakeRunner::default().failing(
            "herdr session list --json",
            RunFailure::not_installed("herdr", "No such file or directory (os error 2)"),
        );

        assert_eq!(
            session_list(&runner).unwrap_err().kind,
            FailureKind::NotInstalled
        );
    }

    #[test]
    fn output_herdr_could_not_have_written_is_a_parse_failure() {
        let sessions = FakeRunner::default().with("herdr session list --json", "not json at all");
        let agents =
            FakeRunner::default().with("herdr --session default agent list", "not json at all");

        assert_eq!(
            session_list(&sessions).unwrap_err().kind,
            FailureKind::Parse
        );
        assert_eq!(
            agent_list(&agents, "default").unwrap_err().kind,
            FailureKind::Parse
        );
    }

    /// The read a tail makes: one pane, the visible screen — the caller is
    /// given no way to ask for another snapshot — with the styling the pane
    /// drew it in.
    #[test]
    fn agent_read_asks_for_one_panes_visible_screen_with_its_styling() {
        const ARGV: &str =
            "herdr --session default agent read wCW:p6 --source visible --lines 40 --format ansi";
        let runner = FakeRunner::default().with(ARGV, "");

        agent_read(&runner, &key("wCW:p6"), 40).expect("herdr answers");

        let call = runner.call(ARGV);
        assert_eq!(call.cwd, None);
        assert!(call.env.is_empty());
    }

    /// herdr ends its output with a newline, which a split would read as one
    /// more, empty, line.
    #[test]
    fn the_newline_herdr_ends_on_is_not_a_line() {
        let runner = FakeRunner::default().with(
            "herdr --session default agent read w:p1 --source visible --lines 2 --format ansi",
            "one\ntwo\n",
        );

        let lines = agent_read(&runner, &key("w:p1"), 2).unwrap();

        assert_eq!(lines, ["one", "two"]);
    }

    /// The styled form ends every row with a carriage return before the
    /// newline, and the return is a row ending rather than a character on
    /// the row.
    #[test]
    fn a_carriage_return_before_the_newline_is_part_of_the_row_ending() {
        let runner = FakeRunner::default().with(
            "herdr --session default agent read w:p1 --source visible --lines 2 --format ansi",
            "\x1b[1mone\x1b[0m\r\ntwo\r\n",
        );

        let lines = agent_read(&runner, &key("w:p1"), 2).unwrap();

        assert_eq!(lines, ["\x1b[1mone\x1b[0m", "two"]);
    }

    /// A pane's own blank lines are its shape and are drawn as it drew them.
    #[test]
    fn a_blank_line_within_the_snapshot_is_kept() {
        let runner = FakeRunner::default().with(
            "herdr --session default agent read w:p1 --source visible --lines 4 --format ansi",
            "\none\n\nthree\n",
        );

        let lines = agent_read(&runner, &key("w:p1"), 4).unwrap();

        assert_eq!(lines, ["", "one", "", "three"]);
    }

    #[test]
    fn a_pane_that_has_drawn_nothing_reads_as_no_lines() {
        let runner = FakeRunner::default().with(
            "herdr --session default agent read w:p1 --source visible --lines 4 --format ansi",
            "",
        );

        let lines = agent_read(&runner, &key("w:p1"), 4).unwrap();

        assert_eq!(lines, Vec::<String>::new());
    }

    /// Asked through the seam, because that is the only way anything reaches
    /// this: `focus` is the one question no other test puts to the adapter,
    /// and a `focus` that never ran herdr answers `Ok(())` exactly as this
    /// one does. The call it left on the runner is what tells them apart.
    #[test]
    fn a_focus_through_the_seam_names_the_pane_and_nothing_else() {
        let runner = FakeRunner::default().with("herdr --session default agent focus wCW:p6", "");

        Herdr::new(&runner)
            .focus(&key("wCW:p6"))
            .expect("herdr answers");

        let call = runner.call("herdr --session default agent focus wCW:p6");
        assert_eq!(call.cwd, None);
        assert!(call.env.is_empty());
    }

    fn vanished() -> RunFailure {
        RunFailure {
            kind: FailureKind::Gone,
            program: "herdr".to_string(),
            detail: "herdr no longer has that pane".to_string(),
        }
    }

    /// The tail's commonest real failure. Both reach the caller as the kind
    /// run.rs gave them, so a closed pane is never drawn as a dead herdr.
    #[test]
    fn a_closed_pane_reaches_the_caller_as_its_own_kind() {
        let read = FakeRunner::default().failing(
            "herdr --session default agent read w:gone --source visible --lines 4 --format ansi",
            vanished(),
        );
        let focus =
            FakeRunner::default().failing("herdr --session default agent focus w:gone", vanished());

        assert_eq!(
            agent_read(&read, &key("w:gone"), 4).unwrap_err().kind,
            FailureKind::Gone
        );
        assert_eq!(
            agent_focus(&focus, &key("w:gone")).unwrap_err().kind,
            FailureKind::Gone
        );
    }
}
