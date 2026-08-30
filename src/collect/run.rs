use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::process::Command;

/// The variables a child process is given on top of the environment `bdi`
/// itself runs in; a set value replaces whatever the parent holds.
///
/// A working directory does not carry a credential. A child inherits the
/// parent's environment whatever its cwd, so reading a second tracker means
/// changing this, not only the directory.
pub type Env = BTreeMap<String, String>;

/// Why a command did not yield usable output. Each kind wants a different
/// response from the caller, so they stay apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// The tracker refused the credential it was given.
    Auth,
    /// The tracker did not answer, or answered in a way we cannot place.
    Unavailable,
    /// The command never ran: not installed, not executable, no such directory.
    Exec,
    /// The command ran and returned something we cannot read.
    Parse,
}

/// A command that did not yield usable output, classified.
///
/// `detail` is written here and never copied from the command's own stderr:
/// bd names the database and the user it authenticated as when it fails, and
/// text that is never kept cannot leak into the output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunFailure {
    pub kind: FailureKind,
    pub program: String,
    pub detail: String,
}

/// What bd's client says when the server refuses the credential. Measured
/// against this repo's own tracker on 2026-08-30:
/// `Error 1045 (28000): Access denied for user '<user>'`.
const REFUSAL: [&str; 2] = ["access denied", "error 1045"];

/// What it says when the server is not there. Measured the same day:
/// `Dolt server unreachable at <host>: dial tcp: lookup <host>: no such host`.
const NO_ANSWER: [&str; 4] = [
    "unreachable",
    "dial tcp",
    "connection refused",
    "i/o timeout",
];

impl RunFailure {
    pub fn exec(program: &str, cause: impl fmt::Display) -> Self {
        Self {
            kind: FailureKind::Exec,
            program: program.to_string(),
            detail: format!("{program} could not be run: {cause}"),
        }
    }

    pub fn parse(program: &str, cause: impl fmt::Display) -> Self {
        Self {
            kind: FailureKind::Parse,
            program: program.to_string(),
            detail: format!("{program} returned output bdi cannot read: {cause}"),
        }
    }

    /// Classify a non-zero exit from what the command wrote to stderr, then
    /// drop that text.
    fn from_exit(program: &str, code: Option<i32>, stderr: &str) -> Self {
        let said = stderr.to_lowercase();
        let (kind, detail) = if REFUSAL.iter().any(|phrase| said.contains(phrase)) {
            (
                FailureKind::Auth,
                format!("{program} was refused the tracker's credential"),
            )
        } else if NO_ANSWER.iter().any(|phrase| said.contains(phrase)) {
            (
                FailureKind::Unavailable,
                format!("{program} could not reach the tracker"),
            )
        } else {
            let detail = match code {
                Some(code) => format!("{program} exited {code} for a reason bdi cannot place"),
                None => format!("{program} was killed by a signal"),
            };
            (FailureKind::Unavailable, detail)
        };

        Self {
            kind,
            program: program.to_string(),
            detail,
        }
    }
}

impl fmt::Display for RunFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.detail)
    }
}

impl std::error::Error for RunFailure {}

/// Runs one command and hands back its stdout. A trait so every call site is
/// testable without spawning anything.
pub trait Runner {
    fn run(
        &self,
        program: &str,
        args: &[&str],
        cwd: Option<&Path>,
        env: &Env,
    ) -> Result<String, RunFailure>;
}

pub struct RealRunner;

impl Runner for RealRunner {
    fn run(
        &self,
        program: &str,
        args: &[&str],
        cwd: Option<&Path>,
        env: &Env,
    ) -> Result<String, RunFailure> {
        let mut cmd = Command::new(program);
        cmd.args(args);
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        cmd.envs(env);

        let out = cmd.output().map_err(|e| RunFailure::exec(program, e))?;
        if !out.status.success() {
            return Err(RunFailure::from_exit(
                program,
                out.status.code(),
                &String::from_utf8_lossy(&out.stderr),
            ));
        }
        String::from_utf8(out.stdout).map_err(|e| RunFailure::parse(program, e))
    }
}

#[cfg(test)]
pub mod testing {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Mutex;

    /// One invocation as the fake saw it, including the directory and the
    /// environment — the two a fake that ignored them would let a wrong
    /// implementation pass.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Call {
        pub argv: String,
        pub cwd: Option<PathBuf>,
        pub env: Env,
    }

    /// A Runner that replays canned output keyed by the joined argv.
    #[derive(Default)]
    pub struct FakeRunner {
        responses: HashMap<String, Result<String, RunFailure>>,
        calls: Mutex<Vec<Call>>,
    }

    impl FakeRunner {
        pub fn with(mut self, argv: &str, out: &str) -> Self {
            self.responses.insert(argv.to_string(), Ok(out.to_string()));
            self
        }

        pub fn failing(mut self, argv: &str, failure: RunFailure) -> Self {
            self.responses.insert(argv.to_string(), Err(failure));
            self
        }

        pub fn calls(&self) -> Vec<Call> {
            self.calls.lock().unwrap().clone()
        }

        /// The single call matching `argv`, or a panic naming what did happen.
        pub fn call(&self, argv: &str) -> Call {
            let calls = self.calls();
            let mut matching = calls.iter().filter(|c| c.argv == argv);
            let found = matching.next().unwrap_or_else(|| {
                let seen: Vec<&str> = calls.iter().map(|c| c.argv.as_str()).collect();
                panic!("no call to `{argv}`; saw {seen:?}")
            });
            assert!(
                matching.next().is_none(),
                "`{argv}` was called more than once"
            );
            found.clone()
        }
    }

    impl Runner for FakeRunner {
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
            match self.responses.get(&argv) {
                Some(Ok(out)) => Ok(out.clone()),
                Some(Err(failure)) => Err(failure.clone()),
                None => panic!("FakeRunner has no response for: {argv}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::FakeRunner;
    use super::*;

    /// The two shapes bd writes when it cannot open a tracker: a credential
    /// the server refuses, and a server that does not answer. Both name a
    /// database, a host and a user the output must never carry.
    const REFUSED: &str = r#"Error: failed to open database: failed to check if database "atlas" exists on server db.example.invalid:3306: Error 1045 (28000): Access denied for user 'atlas'"#;
    const UNREACHABLE: &str = "Error: failed to open database: Dolt server unreachable at nosuchhost.invalid:3306: dial tcp: lookup nosuchhost.invalid: no such host";

    /// A real subprocess writing `stderr` and exiting non-zero.
    fn failing_command(stderr: &str) -> RunFailure {
        RealRunner
            .run(
                "sh",
                &["-c", "printf '%s' \"$1\" >&2; exit 1", "sh", stderr],
                None,
                &Env::new(),
            )
            .expect_err("the command exits non-zero")
    }

    #[test]
    fn stdout_comes_back_from_a_command_that_succeeds() {
        let out = RealRunner
            .run("sh", &["-c", "printf 'hello'"], None, &Env::new())
            .expect("sh runs");

        assert_eq!(out, "hello");
    }

    /// The leading risk in the design: a child inherits the parent's
    /// environment whatever its directory, so a per-tracker credential has to
    /// travel in the overlay. What the overlay does not name is still
    /// inherited, which is what lets a project on the ambient credential work.
    #[test]
    fn an_overlaid_variable_replaces_the_parents_and_the_rest_is_inherited() {
        let mut env = Env::new();
        env.insert("HOME".to_string(), "/nowhere-in-particular".to_string());

        let out = RealRunner
            .run(
                "sh",
                &["-c", "printf '%s|%s' \"$HOME\" \"${PATH:+set}\""],
                None,
                &env,
            )
            .expect("sh runs");

        assert_eq!(out, "/nowhere-in-particular|set");
    }

    #[test]
    fn a_command_runs_in_the_directory_it_is_given() {
        let out = RealRunner
            .run("sh", &["-c", "pwd"], Some(Path::new("/")), &Env::new())
            .expect("sh runs");

        assert_eq!(out.trim(), "/");
    }

    #[test]
    fn a_refused_credential_and_an_unreachable_server_are_told_apart() {
        assert_eq!(failing_command(REFUSED).kind, FailureKind::Auth);
        assert_eq!(failing_command(UNREACHABLE).kind, FailureKind::Unavailable);
    }

    /// bd's failures name the database and the user it authenticated as.
    #[test]
    fn bds_error_text_never_survives_into_the_failure() {
        let failure = failing_command(REFUSED);
        let shown = format!("{failure} {failure:?}");

        for secret in ["atlas", "db.example.invalid", "Access denied", "1045"] {
            assert!(!shown.contains(secret), "{secret:?} survived into: {shown}");
        }
    }

    /// A failure we cannot place is reported as an unanswered tracker, never
    /// guessed at as a credential problem.
    #[test]
    fn an_unrecognised_failure_is_not_reported_as_a_refused_credential() {
        let failure = failing_command("Error: unknown flag: --whatever");

        assert_eq!(failure.kind, FailureKind::Unavailable);
    }

    #[test]
    fn a_program_that_is_not_installed_is_an_exec_failure() {
        let failure = RealRunner
            .run("bdi-no-such-program", &[], None, &Env::new())
            .expect_err("nothing by that name is on PATH");

        assert_eq!(failure.kind, FailureKind::Exec);
        assert_eq!(failure.program, "bdi-no-such-program");
    }

    #[test]
    fn the_fake_records_the_directory_and_environment_it_was_called_with() {
        let runner = FakeRunner::default().with("bd whoami", "someone");
        let mut env = Env::new();
        env.insert("K".to_string(), "v".to_string());

        runner
            .run("bd", &["whoami"], Some(Path::new("/tmp/proj")), &env)
            .unwrap();

        let call = runner.call("bd whoami");
        assert_eq!(call.cwd.as_deref(), Some(Path::new("/tmp/proj")));
        assert_eq!(call.env, env);
    }
}
