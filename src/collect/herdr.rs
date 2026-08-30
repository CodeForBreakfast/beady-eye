use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::collect::run::{Env, RunFailure, Runner};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaneStatus {
    Idle,
    Working,
    /// A TTY prompt is waiting — a permission gate, or a pane at a startup
    /// confirmation. A property of the terminal, never of the work.
    Blocked,
    Done,
    #[serde(untagged)]
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Pane {
    pub pane_id: String,
    pub cwd: PathBuf,
    #[serde(default)]
    pub display_agent: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub state_labels: BTreeMap<String, String>,
    pub agent_status: PaneStatus,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub tab_id: Option<String>,
}

impl Pane {
    /// The line to show for this pane: its state label for the state it is
    /// actually in, falling back to its title.
    pub fn caption(&self) -> Option<&str> {
        let state = match &self.agent_status {
            PaneStatus::Idle => "idle",
            PaneStatus::Working => "working",
            PaneStatus::Blocked => "blocked",
            PaneStatus::Done => "done",
            PaneStatus::Other(s) => s.as_str(),
        };
        self.state_labels
            .get(state)
            .map(String::as_str)
            .or(self.title.as_deref())
    }
}

#[derive(Deserialize)]
struct Envelope {
    result: AgentList,
}

#[derive(Deserialize)]
struct AgentList {
    agents: Vec<Pane>,
}

/// Parse the output of `herdr agent list`.
pub fn parse_agent_list(s: &str) -> anyhow::Result<Vec<Pane>> {
    let envelope: Envelope = serde_json::from_str(s)?;
    Ok(envelope.result.agents)
}

/// `herdr agent list`, which answers with JSON and needs no flag to.
///
/// It reports on the whole machine, so it is asked once and takes no
/// project's directory or credential. A failure here is not fatal: the caller
/// degrades to the beads-only tier.
pub fn agent_list(runner: &dyn Runner) -> Result<Vec<Pane>, RunFailure> {
    let out = runner.run("herdr", &["agent", "list"], None, &Env::new())?;
    parse_agent_list(&out).map_err(|e| RunFailure::parse("herdr", e))
}

/// `herdr agent read <pane>`, as the lines it drew.
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
/// Plain text, because the alternative carries terminal escapes no part of
/// `bdi` reads.
pub fn agent_read(runner: &dyn Runner, pane: &str, lines: u16) -> Result<Vec<String>, RunFailure> {
    let lines = lines.to_string();
    let out = runner.run(
        "herdr",
        &[
            "agent", "read", pane, "--source", "visible", "--lines", &lines, "--format", "text",
        ],
        None,
        &Env::new(),
    )?;
    Ok(out.lines().map(str::to_string).collect())
}

/// `herdr agent focus <pane>` — the only write `bdi` performs.
pub fn agent_focus(runner: &dyn Runner, pane: &str) -> Result<(), RunFailure> {
    runner.run("herdr", &["agent", "focus", pane], None, &Env::new())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    const FIXTURE: &str = include_str!("../../tests/fixtures/herdr_agent_list.json");

    fn pane<'a>(panes: &'a [Pane], id: &str) -> &'a Pane {
        panes
            .iter()
            .find(|p| p.pane_id == id)
            .expect("pane is in the fixture")
    }

    #[test]
    fn unwraps_the_envelope() {
        let panes = parse_agent_list(FIXTURE).expect("parses");

        assert_eq!(panes.len(), 10);
    }

    #[test]
    fn a_pane_that_has_not_identified_itself_is_kept() {
        let panes = parse_agent_list(FIXTURE).unwrap();
        let p = pane(&panes, "wCW:p1");

        assert_eq!(p.display_agent, None);
        assert_eq!(p.title, None);
        assert_eq!(p.state_labels, BTreeMap::new());
        assert_eq!(p.caption(), None);
    }

    #[test]
    fn reads_every_field_of_an_identified_pane() {
        let panes = parse_agent_list(FIXTURE).unwrap();
        let p = pane(&panes, "wCW:p6");

        assert_eq!(p.cwd, PathBuf::from("/tmp/bdi-ground/beady-eye"));
        assert_eq!(p.display_agent.as_deref(), Some("bdi-3um.5"));
        assert_eq!(
            p.title.as_deref(),
            Some("parse herdr agent list into typed panes")
        );
        assert_eq!(p.agent_status, PaneStatus::Working);
        assert_eq!(p.workspace_id.as_deref(), Some("wCW"));
        assert_eq!(p.tab_id.as_deref(), Some("wCW:t1"));
        assert_eq!(
            p.state_labels.get("idle").map(String::as_str),
            Some("asleep: fixture captured, awaiting review")
        );
    }

    #[test]
    fn caption_prefers_the_label_for_the_current_state() {
        let panes = parse_agent_list(FIXTURE).unwrap();
        let p = pane(&panes, "wCW:p6");

        assert_eq!(p.agent_status, PaneStatus::Working);
        assert_eq!(p.caption(), Some("writing the parser and its tests"));
    }

    #[test]
    fn caption_falls_back_to_title_when_a_pane_has_no_labels() {
        let panes = parse_agent_list(FIXTURE).unwrap();
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

        let panes = parse_agent_list(list).unwrap();

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

        let panes = parse_agent_list(list).unwrap();

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

        let got: Vec<PaneStatus> = parse_agent_list(list)
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

        let panes = parse_agent_list(list).expect("an unknown state still parses");

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

    #[test]
    fn agent_list_asks_herdr_once_for_the_whole_machine() {
        let runner = FakeRunner::default().with("herdr agent list", FIXTURE);

        assert_eq!(agent_list(&runner).unwrap().len(), 10);

        let call = runner.call("herdr agent list");
        assert_eq!(call.cwd, None);
        assert!(call.env.is_empty());
    }

    #[test]
    fn a_missing_herdr_is_a_failure_the_caller_can_degrade_on() {
        let runner = FakeRunner::default().failing(
            "herdr agent list",
            RunFailure::exec("herdr", "No such file or directory (os error 2)"),
        );

        assert_eq!(agent_list(&runner).unwrap_err().kind, FailureKind::Exec);
    }

    #[test]
    fn output_herdr_could_not_have_written_is_a_parse_failure() {
        let runner = FakeRunner::default().with("herdr agent list", "not json at all");

        assert_eq!(agent_list(&runner).unwrap_err().kind, FailureKind::Parse);
    }

    /// The read a tail makes: one pane, the visible screen — the caller is
    /// given no way to ask for another snapshot — and text rather than escapes.
    #[test]
    fn agent_read_asks_for_one_panes_visible_screen_as_plain_text() {
        const ARGV: &str = "herdr agent read wCW:p6 --source visible --lines 40 --format text";
        let runner = FakeRunner::default().with(ARGV, "");

        agent_read(&runner, "wCW:p6", 40).expect("herdr answers");

        let call = runner.call(ARGV);
        assert_eq!(call.cwd, None);
        assert!(call.env.is_empty());
    }

    /// herdr ends its output with a newline, which a split would read as one
    /// more, empty, line.
    #[test]
    fn the_newline_herdr_ends_on_is_not_a_line() {
        let runner = FakeRunner::default().with(
            "herdr agent read w:p1 --source visible --lines 2 --format text",
            "one\ntwo\n",
        );

        let lines = agent_read(&runner, "w:p1", 2).unwrap();

        assert_eq!(lines, ["one", "two"]);
    }

    /// A pane's own blank lines are its shape and are drawn as it drew them.
    #[test]
    fn a_blank_line_within_the_snapshot_is_kept() {
        let runner = FakeRunner::default().with(
            "herdr agent read w:p1 --source visible --lines 4 --format text",
            "\none\n\nthree\n",
        );

        let lines = agent_read(&runner, "w:p1", 4).unwrap();

        assert_eq!(lines, ["", "one", "", "three"]);
    }

    #[test]
    fn a_pane_that_has_drawn_nothing_reads_as_no_lines() {
        let runner = FakeRunner::default().with(
            "herdr agent read w:p1 --source visible --lines 4 --format text",
            "",
        );

        let lines = agent_read(&runner, "w:p1", 4).unwrap();

        assert_eq!(lines, Vec::<String>::new());
    }

    #[test]
    fn agent_focus_names_the_pane_and_nothing_else() {
        let runner = FakeRunner::default().with("herdr agent focus wCW:p6", "");

        agent_focus(&runner, "wCW:p6").expect("herdr answers");

        let call = runner.call("herdr agent focus wCW:p6");
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
            "herdr agent read w:gone --source visible --lines 4 --format text",
            vanished(),
        );
        let focus = FakeRunner::default().failing("herdr agent focus w:gone", vanished());

        assert_eq!(
            agent_read(&read, "w:gone", 4).unwrap_err().kind,
            FailureKind::Gone
        );
        assert_eq!(
            agent_focus(&focus, "w:gone").unwrap_err().kind,
            FailureKind::Gone
        );
    }
}
