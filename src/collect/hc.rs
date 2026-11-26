//! The herdr app's terminals, through its `hc` command line.
//!
//! The one module that spells an `hc` command or reads what it answers. The
//! app runs one core per machine, and the core lists the terminals of every
//! machine it has added beside its own, each by its address: `t7` here,
//! `vps/t7` on a machine called `vps`. A seat writes that address as its pane.
//!
//! `hc ls` leaves out a terminal's labels and where its program is working,
//! so the list is asked of the core's API instead, as one request on the
//! stdin of `hc connect`.
//!
//! Every `hc` command starts the core where none is running, this one
//! included. So `bdi` asks `hc` nothing unless the reader's config asks it to.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;

use crate::collect::agents::Agents;
use crate::collect::run::{Env, FailureKind, RunFailure, Runner};
use crate::model::types::{AgentSession, Pane, PaneKey, PaneStatus};

/// The session `bdi` holds the app's terminals in. The app has sessions of
/// its own, and its default is called `default` as herdr's is, so `bdi`
/// names the one it reads after the app. herdr refuses a session name with a
/// space in it, so no herdr session is ever routed to the app.
pub const SESSION: &str = "herdr app";

/// The one request `bdi` makes of the core's API.
const TERMINAL_LIST: &str = r#"{"jsonrpc":"2.0","id":1,"method":"terminal.list","params":{}}"#;

/// The core's answer to a request, which carries an error instead of a
/// result where it refused, and exits 0 either way.
#[derive(Deserialize)]
struct Answer {
    result: Option<TerminalList>,
}

#[derive(Deserialize)]
struct TerminalList {
    terminals: Vec<Terminal>,
}

/// One terminal as the core writes it: only the fields `bdi` reads.
#[derive(Deserialize)]
struct Terminal {
    address: String,
    /// Where it was started, which its program may since have left.
    cwd: PathBuf,
    #[serde(default)]
    foreground: Option<Foreground>,
    #[serde(default)]
    labels: BTreeMap<String, String>,
    /// What the program last reported through OSC 7501. None where it never
    /// reported, which is a terminal with no agent in it.
    #[serde(default)]
    status: Option<Status>,
}

#[derive(Deserialize)]
struct Foreground {
    name: String,
    cwd: PathBuf,
}

/// One OSC 7501 report, in the program's own words.
#[derive(Deserialize)]
struct Status {
    state: PaneStatus,
    #[serde(default)]
    app: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    msg: Option<String>,
}

impl Terminal {
    /// This terminal as a pane, where its program has reported a status.
    ///
    /// The agent is the program's own name for itself, else the name of the
    /// process in front, as `hc ls` names it. The message the program gave
    /// is the label for the state it gave it with.
    fn into_pane(self, session_label: Option<&str>) -> Option<Pane> {
        let status = self.status?;
        let (agent, cwd) = match self.foreground {
            Some(front) => (status.app.or(Some(front.name)), front.cwd),
            None => (status.app, self.cwd),
        };
        let agent_session = session_label.and_then(|key| {
            Some(AgentSession {
                agent: agent.clone().unwrap_or_default(),
                kind: "id".to_string(),
                source: format!("hc:{key}"),
                value: self.labels.get(key)?.clone(),
            })
        });
        let mut pane = Pane::answered(SESSION.to_string(), self.address, cwd, status.state);
        if let Some(msg) = status.msg {
            pane.state_labels
                .insert(state_word(&pane.agent_status), msg);
        }
        pane.agent = agent;
        pane.agent_session = agent_session;
        pane.title = status.title.filter(|title| !title.is_empty());
        Some(pane)
    }
}

/// A state as the program spelled it, which is the key a label is kept under.
fn state_word(state: &PaneStatus) -> String {
    serde_json::to_value(state)
        .ok()
        .and_then(|word| word.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Parse the core's answer to `terminal.list` into the panes of the
/// terminals whose programs have reported a status.
pub fn parse_terminal_list(s: &str, session_label: Option<&str>) -> anyhow::Result<Vec<Pane>> {
    let answer: Answer = serde_json::from_str(s)?;
    let list = answer
        .result
        .ok_or_else(|| anyhow::anyhow!("the core answered with an error"))?;
    Ok(list
        .terminals
        .into_iter()
        .filter_map(|terminal| terminal.into_pane(session_label))
        .collect())
}

/// The herdr app as a provider of panes, reached through one runner.
pub struct Hc<'r> {
    runner: &'r dyn Runner,
    session_label: Option<String>,
}

impl<'r> Hc<'r> {
    /// `session_label` names the label a terminal holds its agent session in.
    pub fn new(runner: &'r dyn Runner, session_label: Option<String>) -> Self {
        Self {
            runner,
            session_label,
        }
    }
}

impl Agents for Hc<'_> {
    fn name(&self) -> &'static str {
        "hc"
    }

    /// The one session `bdi` reads, which takes no command to name.
    fn sessions(&self) -> Result<Vec<String>, RunFailure> {
        Ok(vec![SESSION.to_string()])
    }

    fn list(&self, _session: &str) -> Result<Vec<Pane>, RunFailure> {
        let out = self
            .runner
            .run_feeding("hc", &["connect"], &format!("{TERMINAL_LIST}\n"))?;
        parse_terminal_list(&out, self.session_label.as_deref())
            .map_err(|e| RunFailure::parse("hc", e).reading("terminal list"))
    }

    /// `hc read <address>`: the screen as text, without its styling, of which
    /// the last `lines` rows are kept.
    fn read(&self, pane: &PaneKey, lines: u16) -> Result<Vec<String>, RunFailure> {
        let out = self
            .runner
            .run("hc", &["read", &pane.id], None, &Env::new())?;
        let rows: Vec<&str> = out.lines().collect();
        let from = rows.len().saturating_sub(usize::from(lines));
        Ok(rows[from..].iter().map(|row| row.to_string()).collect())
    }

    /// The app has no way to be asked to bring a terminal to the front.
    fn focus(&self, _pane: &PaneKey) -> Result<(), RunFailure> {
        Err(RunFailure {
            kind: FailureKind::Unsupported,
            program: "hc".to_string(),
            detail: "the herdr app cannot be asked to bring a terminal to the front".to_string(),
            unreadable: None,
        })
    }
}

/// herdr's panes and the app's terminals, read together.
///
/// The app's terminals are held in their one session, and herdr's in theirs.
/// A run asks each question of whichever provider holds the session it names.
pub struct WithHc<'r, A> {
    others: A,
    hc: Hc<'r>,
}

impl<'r, A: Agents> WithHc<'r, A> {
    pub fn new(others: A, hc: Hc<'r>) -> Self {
        Self { others, hc }
    }

    fn holding(&self, session: &str) -> &dyn Agents {
        if session == SESSION {
            &self.hc
        } else {
            &self.others
        }
    }
}

impl<A: Agents> Agents for WithHc<'_, A> {
    fn name(&self) -> &'static str {
        "herdr and hc"
    }

    /// The other provider's sessions, then the app's. A machine without the
    /// other provider installed is one where the app is read alone.
    fn sessions(&self) -> Result<Vec<String>, RunFailure> {
        let mut sessions = match self.others.sessions() {
            Ok(sessions) => sessions,
            Err(failure) if failure.kind == FailureKind::NotInstalled => Vec::new(),
            Err(failure) => return Err(failure),
        };
        sessions.extend(self.hc.sessions()?);
        Ok(sessions)
    }

    fn list(&self, session: &str) -> Result<Vec<Pane>, RunFailure> {
        self.holding(session).list(session)
    }

    fn read(&self, pane: &PaneKey, lines: u16) -> Result<Vec<String>, RunFailure> {
        self.holding(&pane.session).read(pane, lines)
    }

    fn focus(&self, pane: &PaneKey) -> Result<(), RunFailure> {
        self.holding(&pane.session).focus(pane)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::agents::testing::{pane, Asked, Fake};
    use crate::collect::run::testing::FakeRunner;
    use crate::model::types::testing::A_SESSION;
    use pretty_assertions::assert_eq;

    const FIXTURE: &str = include_str!("../../tests/fixtures/hc_0.1.21_terminal_list.json");
    const LABEL: Option<&str> = Some("claude.session");
    const CONNECT: &str = "hc connect";

    fn terminal<'a>(panes: &'a [Pane], address: &str) -> &'a Pane {
        panes
            .iter()
            .find(|p| p.pane_id == address)
            .expect("the terminal is in the fixture")
    }

    fn in_hc(address: &str) -> PaneKey {
        PaneKey {
            session: SESSION.to_string(),
            id: address.to_string(),
        }
    }

    fn in_herdr(id: &str) -> PaneKey {
        PaneKey {
            session: A_SESSION.to_string(),
            id: id.to_string(),
        }
    }

    /// A shell that never reported a status has no agent in it, and herdr
    /// lists only the panes with one.
    #[test]
    fn a_terminal_that_never_reported_a_status_is_left_out() {
        let panes = parse_terminal_list(FIXTURE, LABEL).expect("parses");

        let addresses: Vec<&str> = panes.iter().map(|p| p.pane_id.as_str()).collect();
        assert_eq!(addresses, ["t4", "t7", "kadath/t3"]);
    }

    #[test]
    fn reads_every_field_of_a_terminal_that_reported() {
        let panes = parse_terminal_list(FIXTURE, LABEL).unwrap();
        let p = terminal(&panes, "t4");

        assert_eq!(p.session, SESSION);
        assert_eq!(p.cwd, PathBuf::from("/tmp/bdi-ground/dunwich"));
        assert_eq!(p.agent.as_deref(), Some("claude"));
        assert_eq!(p.agent_status, PaneStatus::Working);
        assert_eq!(p.title.as_deref(), Some("reading the trailers"));
        assert_eq!(p.caption(), Some("parsing the log"));
        assert_eq!(p.display_agent, None);
    }

    /// Where the program is working now, rather than where the terminal
    /// was started, is what places the pane in a project.
    #[test]
    fn a_terminal_is_where_its_program_is_working() {
        let panes = parse_terminal_list(FIXTURE, LABEL).unwrap();

        assert_eq!(
            terminal(&panes, "t7").cwd,
            PathBuf::from("/tmp/bdi-ground/lander")
        );
    }

    #[test]
    fn the_session_label_the_config_names_is_the_agent_session() {
        let panes = parse_terminal_list(FIXTURE, LABEL).unwrap();

        assert_eq!(
            terminal(&panes, "t4").live_agent_session(),
            Some("4c1f0a2e-7d3b-4e8a-9f61-2b5c8d0e7a13")
        );
    }

    #[test]
    fn a_terminal_without_the_label_has_no_agent_session() {
        let panes = parse_terminal_list(FIXTURE, LABEL).unwrap();

        assert_eq!(terminal(&panes, "t7").agent_session, None);
    }

    /// The label is the reader's to name, so with none named no label is an
    /// agent session, whatever it is called.
    #[test]
    fn with_no_label_named_no_terminal_has_an_agent_session() {
        let panes = parse_terminal_list(FIXTURE, None).unwrap();

        assert!(panes.iter().all(|p| p.agent_session.is_none()));
    }

    /// A seat writes its terminal's address as its pane, machine and all.
    #[test]
    fn a_terminal_on_another_machine_keeps_its_whole_address() {
        let panes = parse_terminal_list(FIXTURE, LABEL).unwrap();
        let p = terminal(&panes, "kadath/t3");

        assert_eq!(p.agent_status, PaneStatus::Idle);
        assert_eq!(
            p.live_agent_session(),
            Some("9e2d7c41-0b6a-4f3e-8d15-7a4c2e9b1f08")
        );
    }

    /// The app's five states, `error` among them, which herdr has no word for.
    #[test]
    fn reads_each_state_the_app_reports() {
        let states = ["idle", "working", "done", "blocked", "error"];
        let terminals: Vec<String> = states
            .iter()
            .enumerate()
            .map(|(n, state)| {
                format!(
                    r#"{{"address":"t{n}","cwd":"/tmp","labels":{{}},"status":{{"state":"{state}"}}}}"#
                )
            })
            .collect();
        let list = format!(r#"{{"result":{{"terminals":[{}]}}}}"#, terminals.join(","));

        let got: Vec<PaneStatus> = parse_terminal_list(&list, None)
            .expect("parses")
            .into_iter()
            .map(|p| p.agent_status)
            .collect();

        assert_eq!(
            got,
            [
                PaneStatus::Idle,
                PaneStatus::Working,
                PaneStatus::Done,
                PaneStatus::Blocked,
                PaneStatus::Other("error".to_string()),
            ]
        );
    }

    /// A program that reported without naming itself is named by the process
    /// in front, as `hc ls` names it.
    #[test]
    fn a_program_that_gave_no_name_is_named_by_its_process() {
        let list = r#"{"result":{"terminals":[{"address":"t1","cwd":"/tmp","labels":{},
            "foreground":{"name":"node","cwd":"/tmp","pid":1,"argv":["node"]},
            "status":{"state":"working"}}]}}"#;

        let panes = parse_terminal_list(list, None).unwrap();

        assert_eq!(panes[0].agent.as_deref(), Some("node"));
    }

    /// The core answers a request it refused with an error in place of a
    /// result, and exits 0.
    #[test]
    fn an_answer_holding_an_error_is_a_parse_failure() {
        let refused = r#"{"error":{"code":-32700,"message":"no"},"id":null,"jsonrpc":"2.0"}"#;
        let runner = FakeRunner::default().with(CONNECT, refused);

        assert_eq!(
            Hc::new(&runner, None).list(SESSION).unwrap_err().kind,
            FailureKind::Parse
        );
    }

    /// One request on the API's stdin, in no directory and with nothing laid
    /// over the environment.
    #[test]
    fn the_list_is_one_request_to_the_api() {
        let runner = FakeRunner::default().with(CONNECT, FIXTURE);

        let panes = Hc::new(&runner, LABEL.map(str::to_string))
            .list(SESSION)
            .unwrap();

        assert_eq!(panes.len(), 3);
        let call = runner.call(CONNECT);
        assert_eq!(call.cwd, None);
        assert!(call.env.is_empty());
        assert_eq!(
            call.input.as_deref(),
            Some(format!("{TERMINAL_LIST}\n").as_str())
        );
    }

    /// Asking which sessions there are is free: every `hc` command starts the
    /// core, so a question that ran one would start it for nothing.
    #[test]
    fn the_session_is_named_without_running_anything() {
        let runner = FakeRunner::default();

        assert_eq!(Hc::new(&runner, None).sessions().unwrap(), [SESSION]);
        assert!(runner.calls().is_empty());
    }

    #[test]
    fn a_read_keeps_the_last_rows_of_the_screen() {
        let runner = FakeRunner::default().with("hc read kadath/t3", "one\ntwo\nthree\n");

        let lines = Hc::new(&runner, None).read(&in_hc("kadath/t3"), 2).unwrap();

        assert_eq!(lines, ["two", "three"]);
    }

    #[test]
    fn a_read_of_more_rows_than_the_screen_holds_is_the_screen() {
        let runner = FakeRunner::default().with("hc read t4", "one\n");

        let lines = Hc::new(&runner, None).read(&in_hc("t4"), 40).unwrap();

        assert_eq!(lines, ["one"]);
    }

    #[test]
    fn a_focus_is_refused_with_its_reason_and_runs_nothing() {
        let runner = FakeRunner::default();

        let refused = Hc::new(&runner, None).focus(&in_hc("t4")).unwrap_err();

        assert_eq!(refused.kind, FailureKind::Unsupported);
        assert!(refused.detail.contains("front"));
        assert!(runner.calls().is_empty());
    }

    #[test]
    fn both_are_read_and_the_app_after_the_other() {
        let runner = FakeRunner::default();
        let both = WithHc::new(Fake::holding(Vec::new()), Hc::new(&runner, None));

        assert_eq!(both.sessions().unwrap(), [A_SESSION, SESSION]);
    }

    /// herdr names a session with ASCII letters, digits, `.`, `_` and `-`
    /// alone, so a herdr session called `hc` stays herdr's.
    #[test]
    fn the_apps_session_is_one_herdr_cannot_name() {
        assert!(!SESSION
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')));
    }

    /// A machine running the app and not herdr is one where the app is read
    /// alone, rather than one where nothing is.
    #[test]
    fn the_app_is_read_alone_where_the_other_is_not_installed() {
        let runner = FakeRunner::default();
        let missing = Fake::unlistable(RunFailure::not_installed("herdr", "not there"));
        let both = WithHc::new(missing, Hc::new(&runner, None));

        assert_eq!(both.sessions().unwrap(), [SESSION]);
    }

    #[test]
    fn the_other_failing_any_other_way_is_the_failure() {
        let runner = FakeRunner::default();
        let failing = Fake::unlistable(RunFailure::parse("herdr", "garbled"));
        let both = WithHc::new(failing, Hc::new(&runner, None));

        assert_eq!(both.sessions().unwrap_err().kind, FailureKind::Parse);
    }

    /// Each question goes to the provider holding the session it names, and
    /// the app's are never put to the other.
    #[test]
    fn each_question_goes_to_the_provider_holding_its_session() {
        let runner = FakeRunner::default()
            .with(CONNECT, FIXTURE)
            .with("hc read t4", "the app's screen\n");
        let other = Fake::holding(vec![pane("w:p1", "/tmp", PaneStatus::Idle)])
            .showing("w:p1", ["herdr's screen"]);
        let both = WithHc::new(other, Hc::new(&runner, None));

        assert_eq!(both.list(A_SESSION).unwrap()[0].pane_id, "w:p1");
        assert_eq!(both.list(SESSION).unwrap().len(), 3);
        assert_eq!(both.read(&in_herdr("w:p1"), 1).unwrap(), ["herdr's screen"]);
        assert_eq!(both.read(&in_hc("t4"), 1).unwrap(), ["the app's screen"]);
        assert_eq!(both.focus(&in_herdr("w:p1")), Ok(()));
        assert_eq!(
            both.focus(&in_hc("t4")).unwrap_err().kind,
            FailureKind::Unsupported
        );
        assert_eq!(
            both.others.asked(),
            [
                Asked::List {
                    session: A_SESSION.to_string()
                },
                Asked::Read {
                    pane: in_herdr("w:p1"),
                    lines: 1
                },
                Asked::Focus {
                    pane: in_herdr("w:p1")
                },
            ]
        );
    }
}
