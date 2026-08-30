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

/// A runner that replays one canned answer per command line, either wherever
/// that line is run or only in one project's directory, and records every
/// call it was asked to make. Two trackers answer the same argv with beads of
/// their own, so the directory a call carries is part of what identifies it.
#[derive(Default)]
pub struct Canned {
    responses: HashMap<(Option<PathBuf>, String), Result<String, RunFailure>>,
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
            .get(&(cwd.map(Path::to_path_buf), argv.clone()))
            .or_else(|| self.responses.get(&(None, argv.clone())))
            .cloned()
            .unwrap_or_else(|| panic!("no canned response for `{argv}` in {cwd:?}"))
    }
}

impl Canned {
    pub fn answering(mut self, argv: &str, out: &str) -> Self {
        self.responses
            .insert((None, argv.to_string()), Ok(out.to_string()));
        self
    }

    pub fn answering_in(mut self, cwd: &str, argv: &str, out: &str) -> Self {
        self.responses
            .insert((Some(cwd.into()), argv.to_string()), Ok(out.to_string()));
        self
    }

    pub fn failing(mut self, argv: &str, kind: FailureKind) -> Self {
        self.responses
            .insert((None, argv.to_string()), Err(refused(kind)));
        self
    }

    pub fn failing_in(mut self, cwd: &str, argv: &str, kind: FailureKind) -> Self {
        self.responses
            .insert((Some(cwd.into()), argv.to_string()), Err(refused(kind)));
        self
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }
}

/// bd names the database and the SQL user when it turns a call away.
fn refused(kind: FailureKind) -> RunFailure {
    RunFailure {
        kind,
        program: "bd".to_string(),
        detail: "Access denied for user 'orbital' at db.example.invalid:3306".to_string(),
    }
}
