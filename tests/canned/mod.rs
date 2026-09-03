//! The runner double the integration tests share.
//!
//! Each test binary uses the part of it that binary needs, so an unused
//! method here is not a dead one.
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use beady_eye::collect::run::{Env, FailureKind, RunFailure, Runner};

/// One invocation as the double was asked to make it. The directory and the
/// environment are what say which tracker a call was for and which
/// credential it went out with, so both are kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub argv: String,
    pub cwd: Option<PathBuf>,
    pub env: Env,
}

/// A runner that replays one canned answer per command line, or one answer
/// for every call to a program, and records every call it was asked to make.
#[derive(Default)]
pub struct Canned {
    responses: HashMap<String, Result<String, RunFailure>>,
    whatever_it_asks: HashMap<String, String>,
    calls: Mutex<Vec<Call>>,
}

impl Runner for Canned {
    fn run(
        &self,
        program: &str,
        args: &[&str],
        cwd: Option<&Path>,
        env: &Env,
    ) -> Result<String, RunFailure> {
        let argv = format!("{program} {}", args.join(" "));
        self.calls.lock().unwrap().push(Call {
            argv: argv.clone(),
            cwd: cwd.map(Path::to_path_buf),
            env: env.clone(),
        });
        self.responses
            .get(&argv)
            .cloned()
            .or_else(|| self.whatever_it_asks.get(program).cloned().map(Ok))
            .unwrap_or_else(|| panic!("no canned response for `{argv}` in {cwd:?}"))
    }
}

impl Canned {
    pub fn answering(mut self, argv: &str, out: &str) -> Self {
        self.responses.insert(argv.to_string(), Ok(out.to_string()));
        self
    }

    /// Every call to `program`, whatever it asks and wherever it is run,
    /// answers `out`.
    pub fn answering_every(mut self, program: &str, out: &str) -> Self {
        self.whatever_it_asks
            .insert(program.to_string(), out.to_string());
        self
    }

    pub fn failing(mut self, argv: &str, kind: FailureKind) -> Self {
        self.responses.insert(argv.to_string(), Err(refused(kind)));
        self
    }

    /// A herdr running one session, the default, whose `agent list` answers
    /// `agents` — the two calls a collection makes of a herdr with one
    /// session, staged together because neither is any use alone.
    pub fn herdr_holding(self, agents: &str) -> Self {
        self.answering("herdr session list --json", ONE_SESSION)
            .answering("herdr --session default agent list", agents)
    }

    /// A herdr that fails at the first thing it is asked, which is for its
    /// sessions, so nothing after it is asked at all.
    pub fn herdr_failing(self, kind: FailureKind) -> Self {
        self.failing("herdr session list --json", kind)
    }

    /// A herdr running the default session and `others` beside it, each
    /// answering `agent list` as staged — with `agents`, or, given none,
    /// with a failure to answer at all.
    pub fn herdr_running(mut self, others: &[(&str, Option<&str>)]) -> Self {
        let mut listed = vec![ONE_SESSION_ROW.to_string()];
        for (session, agents) in others {
            listed.push(format!(
                r#"{{"default":false,"name":"{session}","running":true,"session_dir":"/h/sessions/{session}","socket_path":"/h/sessions/{session}/herdr.sock"}}"#
            ));
            let asked = format!("herdr --session {session} agent list");
            self = match agents {
                Some(agents) => self.answering(&asked, agents),
                None => self.failing(&asked, FailureKind::Unavailable),
            };
        }
        self.answering(
            "herdr session list --json",
            &format!(r#"{{"sessions":[{}]}}"#, listed.join(",")),
        )
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }
}

/// The default session as `herdr session list --json` writes it, and what
/// it says on a box running that session and nothing else.
const ONE_SESSION_ROW: &str = r#"{"default":true,"name":"default","running":true,"session_dir":"/h","socket_path":"/h/herdr.sock"}"#;
const ONE_SESSION: &str = r#"{"sessions":[{"default":true,"name":"default","running":true,"session_dir":"/h","socket_path":"/h/herdr.sock"}]}"#;

/// A failure as a tracker writes one: naming the database and the SQL user it
/// turned away, which is what must never reach the output.
pub fn refused(kind: FailureKind) -> RunFailure {
    RunFailure {
        kind,
        program: "bd".to_string(),
        detail: "Access denied for user 'orbital' at db.example.invalid:3306".to_string(),
    }
}
