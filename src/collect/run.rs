//! The boundary every subprocess crosses: what a child is told, what it
//! answers with, and why a run that gave nothing usable did not.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::process::Command;

use crate::collect::environment::NEVER_INHERITED;

/// The variables a child process is given on top of the environment `bdi`
/// itself runs in; a set value replaces whatever the parent holds.
///
/// A working directory does not carry a credential. A child inherits the
/// parent's environment whatever its cwd, so reading a second tracker means
/// changing this, not only the directory.
///
/// `environment::NEVER_INHERITED` is the exception to the inheritance: a
/// subprocess holds one of those only if this names it.
pub type Env = BTreeMap<String, String>;

/// Why a command did not yield usable output. Each kind wants a different
/// response from the caller, so they stay apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// The tracker refused the credential it was given.
    Auth,
    /// The tracker did not answer, or answered in a way we cannot place.
    Unavailable,
    /// What the command was asked about is not there.
    Gone,
    /// It is there, but too busy to answer; the same command may work later.
    Busy,
    /// Nothing is installed under that name for the command to run.
    NotInstalled,
    /// The program is there and the run could not be started: no execute
    /// bit, or a directory it was to be run in that is not there.
    Unstartable,
    /// The command ran and returned something we cannot read.
    Parse,
    /// The tracker cannot run what it was asked at all, so asking it again
    /// answers the same.
    Unsupported,
    /// The program does not know a flag or subcommand on its command line
    /// and refused the whole of it before running: a bd older than the flag,
    /// or newer than it and without it.
    UnknownFlag,
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

/// What bd says to a statement its store cannot run: `bd sql` against its
/// embedded Dolt, which has no server for the statement to reach. Measured
/// 2026-09-02 on bd 1.2.2: `Error: 'bd sql' is not yet supported in embedded
/// mode`, exit 1, nothing on stdout; the same words from 1.0.4, 1.1.0 and
/// 1.1.2.
const CANNOT_RUN: &str = "not yet supported";

/// What cobra, bd's command-line parser, says to a flag or subcommand it does
/// not have, before bd itself runs. Measured 2026-09-02 on bd 0.42.0 through
/// 1.0.3, none of which has `-C`: `Error: unknown shorthand flag: 'C' in -C`
/// then the usage text, exit 1, plain text with `--json` given. A long flag
/// gets `unknown flag: --readonly` and a subcommand `unknown command "sql"
/// for "bd"`. The prefix is cobra's own, and keeps the same words quoted
/// inside another failure from reading as this one.
const NOT_KNOWN: [&str; 3] = [
    "error: unknown flag",
    "error: unknown shorthand flag",
    "error: unknown command",
];

/// The one program whose refusal of its command line is a bd to replace. A
/// credential command or direnv says the same words to a flag it lacks, and
/// neither is answered by a newer bd.
const BD: &str = "bd";

/// Whether anything is installed under that name for the child to have run,
/// asked the way the kernel asked: a name holding no separator is looked for
/// on `PATH`, and anything else is a path, resolved where the child would
/// have resolved it.
///
/// The `PATH` is the child's rather than this process's. `environment`
/// captures whatever entering a project's directory produces, `PATH`
/// included, and that overlay is what the child was given — so a `bd` direnv
/// supplies and `bdi`'s own shell does not is installed as far as this
/// question goes.
///
/// A relative name is resolved against the child's working directory for the
/// same reason, whether it is the program's or a `PATH` entry's. The child
/// searched its `PATH` after entering that directory, so a relative entry
/// there names somewhere `bdi`'s own directory says nothing about.
fn installed(program: &str, cwd: Option<&Path>, env: &Env) -> bool {
    let named = Path::new(program);
    let where_the_child_looked =
        |at: &Path| cwd.map_or_else(|| at.to_path_buf(), |directory| directory.join(at));
    if program.contains(std::path::MAIN_SEPARATOR) {
        return under_that_name(&where_the_child_looked(named));
    }
    env.get(PATH)
        .map(std::ffi::OsString::from)
        .or_else(|| std::env::var_os(PATH))
        .is_some_and(|path| {
            std::env::split_paths(&path)
                .any(|at| under_that_name(&where_the_child_looked(&at).join(named)))
        })
}

/// Whether the filesystem holds an entry there, which is a different
/// question from whether it resolves.
///
/// A symlink whose target has gone — a profile collected out from under it,
/// a build deleted — is something installed and broken, and it is the state
/// this whole distinction is drawn for. `Path::exists` follows the link and
/// so answers `false` for one, which is the answer reserved for a name
/// nothing holds. `symlink_metadata` is the `lstat` that stops at the entry.
fn under_that_name(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// Where a child looks for a program it was named without a path.
const PATH: &str = "PATH";

/// What herdr says when the pane a command names is not there, and when a
/// pane is in the alternate screen and working so its history cannot be
/// scrolled. Measured against herdr 0.8.2 on 2026-08-30. One phrase each,
/// where bd takes several: herdr answers with a machine-readable code and bd
/// with prose.
const NO_SUCH_PANE: &str = "agent_not_found";
const PANE_BUSY: &str = "agent_not_idle";

impl RunFailure {
    pub fn not_installed(program: &str, cause: impl fmt::Display) -> Self {
        Self {
            kind: FailureKind::NotInstalled,
            program: program.to_string(),
            detail: format!("{program} is not installed: {cause}"),
        }
    }

    pub fn unstartable(program: &str, cause: impl fmt::Display) -> Self {
        Self {
            kind: FailureKind::Unstartable,
            program: program.to_string(),
            detail: format!("{program} could not be started: {cause}"),
        }
    }

    /// Which of the two a refused spawn was.
    ///
    /// `NotInstalled` is the answer that costs the reader their notice, so
    /// it is the one that has to be earned rather than defaulted to, and
    /// the error alone cannot earn it. `ENOENT` is what the kernel answers
    /// to a working directory that is not there, and to a program whose own
    /// interpreter or loader is missing, as readily as to a name nothing
    /// holds — it reports the interpreter's absence as the program's. So
    /// both are asked after instead: anything found under that name, or a
    /// directory that was never there to look in, means this is a machine
    /// that has the thing and could not start it.
    fn could_not_start(
        program: &str,
        cwd: Option<&Path>,
        env: &Env,
        cause: &std::io::Error,
    ) -> Self {
        let nothing_was_installed = cause.kind() == std::io::ErrorKind::NotFound
            && !installed(program, cwd, env)
            && cwd.is_none_or(Path::is_dir);
        if nothing_was_installed {
            Self::not_installed(program, cause)
        } else {
            Self::unstartable(program, cause)
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
        } else if said.contains(CANNOT_RUN) {
            (
                FailureKind::Unsupported,
                format!("{program} cannot run that against this tracker"),
            )
        } else if program == BD && NOT_KNOWN.iter().any(|phrase| said.contains(phrase)) {
            (
                FailureKind::UnknownFlag,
                format!("{program} does not know a flag bdi uses"),
            )
        } else if said.contains(NO_SUCH_PANE) {
            (
                FailureKind::Gone,
                format!("{program} no longer has that pane"),
            )
        } else if said.contains(PANE_BUSY) {
            (
                FailureKind::Busy,
                format!("{program} cannot read that pane while it is busy"),
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
///
/// `Sync` because a collection reads its projects' trackers together, each
/// on a thread of its own, through the one runner it was given.
pub trait Runner: Sync {
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
        for inherited in NEVER_INHERITED {
            cmd.env_remove(inherited);
        }
        cmd.envs(env);

        let out = cmd
            .output()
            .map_err(|e| RunFailure::could_not_start(program, cwd, env, &e))?;
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

        /// Add rows to an answer already staged for `argv`, which is how a
        /// test stages a tracker holding more than one root: bd answers for
        /// the whole tracker in one array, not one array per root.
        pub fn merging(mut self, argv: &str, rows: &str) -> Self {
            let standing = match self.responses.get(argv) {
                Some(Ok(out)) => out.clone(),
                _ => return self.with(argv, rows),
            };
            let joined = format!(
                "[{},{}]",
                standing
                    .trim()
                    .trim_start_matches('[')
                    .trim_end_matches(']'),
                rows.trim().trim_start_matches('[').trim_end_matches(']')
            );
            self.responses.insert(argv.to_string(), Ok(joined));
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
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    /// The two shapes bd writes when it cannot open a tracker: a credential
    /// the server refuses, and a server that does not answer. Both name a
    /// database, a host and a user the output must never carry.
    const REFUSED: &str = r#"Error: failed to open database: failed to check if database "atlas" exists on server db.example.invalid:3306: Error 1045 (28000): Access denied for user 'atlas'"#;
    const UNREACHABLE: &str = "Error: failed to open database: Dolt server unreachable at nosuchhost.invalid:3306: dial tcp: lookup nosuchhost.invalid: no such host";

    /// bd's refusal of `bd sql` on its embedded Dolt, as `CANNOT_RUN` was
    /// measured from.
    const EMBEDDED: &str = "Error: 'bd sql' is not yet supported in embedded mode";

    /// What a bd with no `-C` says to bdi's first tracker call, measured
    /// 2026-09-02 on bd 0.42.0 through 1.0.3 in an empty directory: cobra
    /// refuses the command line before bd runs, so it is plain text with
    /// `--json` given, exit 1. The usage text after these lines was not kept
    /// by the measurement.
    const NO_SUCH_FLAG: &str =
        "Error: unknown shorthand flag: 'C' in -C\nUsage:\n  bd list [flags]\n";
    /// The same refusal of a long flag and of a subcommand, in cobra's words
    /// for each, measured the same day.
    const NO_SUCH_LONG_FLAG: &str = "Error: unknown flag: --readonly";
    const NO_SUCH_COMMAND: &str = r#"Error: unknown command "sql" for "bd""#;

    /// A stderr no phrase places.
    const UNPLACED: &str = "Error: something neither bd nor herdr has been measured saying";

    /// The two shapes herdr writes when it cannot read a pane, measured
    /// against herdr 0.8.2 on 2026-08-30. Both name the pane and the command,
    /// and `focus` answers the first of them the same way bar its `id`.
    const NO_SUCH_PANE: &str = r#"{"error":{"code":"agent_not_found","message":"agent target wCW:nosuchpane not found"},"id":"cli:agent:read"}"#;
    const PANE_BUSY: &str = r#"{"error":{"code":"agent_not_idle","message":"cannot read 8 lines while wCW:pM is working: its alternate-screen history can only be captured by scrolling while idle. Wait and retry, or use --source visible"},"id":"cli:agent:read"}"#;

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
    /// inherited, bar the credential itself; `tests/credential.rs` holds that.
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

    /// A statement the tracker cannot run at all is told apart from a
    /// tracker that did not answer it: asking the first again answers the
    /// same, where the second is worth asking again on the next refresh.
    #[test]
    fn a_statement_the_tracker_cannot_run_is_told_apart_from_an_unanswered_one() {
        assert_eq!(failing_command(EMBEDDED).kind, FailureKind::Unsupported);
        assert_eq!(failing_command(UNREACHABLE).kind, FailureKind::Unavailable);
    }

    /// A bd that does not know a flag or subcommand bdi uses refuses the
    /// command line before it runs, which is not a tracker that did not
    /// answer: the reader wants a newer bd, not a look at the network.
    #[test]
    fn a_bd_that_does_not_know_a_flag_bdi_uses_is_told_apart_from_an_unanswered_tracker() {
        for said in [NO_SUCH_FLAG, NO_SUCH_LONG_FLAG, NO_SUCH_COMMAND] {
            assert_eq!(
                RunFailure::from_exit("bd", Some(1), said).kind,
                FailureKind::UnknownFlag,
                "on {said:?}"
            );
        }
        assert_eq!(
            RunFailure::from_exit("bd", Some(1), UNREACHABLE).kind,
            FailureKind::Unavailable
        );
    }

    /// The bd this build runs against still refuses a flag it does not have
    /// in the words the classifier reads, which is the whole of what the
    /// classification rests on. cobra refuses before bd runs, so no tracker
    /// is touched.
    #[test]
    fn the_bd_on_path_refuses_a_flag_it_lacks_in_the_words_the_classifier_reads() {
        let failure = RealRunner
            .run(
                "bd",
                &[
                    "--readonly",
                    "list",
                    "--json",
                    "--no-such-flag-bdi-never-uses",
                ],
                None,
                &Env::new(),
            )
            .expect_err("cobra refuses the command line");

        assert_eq!(failure.kind, FailureKind::UnknownFlag);
    }

    /// cobra's refusal is an error line of its own, `Error: unknown …`. The
    /// same words inside another failure — a path or a statement bd was
    /// quoting back — are not bd refusing its command line.
    #[test]
    fn cobras_words_inside_another_failure_do_not_make_it_a_bd_to_replace() {
        let quoting =
            r#"Error: failed to open database: no such directory "/srv/unknown command/tracker""#;

        assert_eq!(
            RunFailure::from_exit("bd", Some(1), quoting).kind,
            FailureKind::Unavailable
        );
    }

    /// A credential command or direnv refusing its own command line is a
    /// configured command that failed, which is what it always was: only bd
    /// refusing a flag is a bd to replace, and the screen must not send the
    /// reader to upgrade the wrong program.
    #[test]
    fn only_bds_refusal_of_its_command_line_is_a_bd_to_replace() {
        for program in ["sh", "direnv"] {
            assert_eq!(
                RunFailure::from_exit(program, Some(1), NO_SUCH_COMMAND).kind,
                FailureKind::Unavailable,
                "from {program}"
            );
        }
    }

    /// Placing cobra's refusal moves no other failure: each stderr bd or
    /// herdr is known to write still lands where it did.
    #[test]
    fn placing_cobras_refusal_moves_no_other_failure() {
        let placed = [
            (REFUSED, FailureKind::Auth),
            (UNREACHABLE, FailureKind::Unavailable),
            (EMBEDDED, FailureKind::Unsupported),
            (NO_SUCH_PANE, FailureKind::Gone),
            (PANE_BUSY, FailureKind::Busy),
            (UNPLACED, FailureKind::Unavailable),
        ];

        for (said, expected) in placed {
            assert_eq!(failing_command(said).kind, expected, "on {said:?}");
        }
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
        let failure = failing_command(UNPLACED);

        assert_eq!(failure.kind, FailureKind::Unavailable);
    }

    /// The tail's two everyday failures against the one that ends the join.
    /// A closed pane is one row to stop tailing and a busy pane is a retry,
    /// where an unreachable herdr drops the whole live tier.
    #[test]
    fn herdrs_own_failures_are_told_apart_from_an_unreachable_herdr() {
        assert_eq!(failing_command(NO_SUCH_PANE).kind, FailureKind::Gone);
        assert_eq!(failing_command(PANE_BUSY).kind, FailureKind::Busy);
        assert_eq!(failing_command(UNREACHABLE).kind, FailureKind::Unavailable);
    }

    /// herdr's failures name the pane, its workspace and the command asked of
    /// it, so its JSON is classified and then dropped like bd's text.
    #[test]
    fn herdrs_error_json_never_survives_into_the_failure() {
        for said in [NO_SUCH_PANE, PANE_BUSY] {
            let failure = failing_command(said);
            let shown = format!("{failure} {failure:?}");

            for leaked in ["agent_not", "wCW", "cli:agent", "\"error\""] {
                assert!(!shown.contains(leaked), "{leaked:?} survived into: {shown}");
            }
        }
    }

    #[test]
    fn a_program_that_is_not_installed_says_nothing_is_installed() {
        let failure = RealRunner
            .run("bdi-no-such-program", &[], None, &Env::new())
            .expect_err("nothing by that name is on PATH");

        assert_eq!(failure.kind, FailureKind::NotInstalled);
        assert_eq!(failure.program, "bdi-no-such-program");
    }

    /// A program on `PATH` that the kernel will not start is not a program
    /// nobody installed, and the reader's answer to the two is different:
    /// one is a machine that never had the thing, the other a machine that
    /// had it and lost it.
    #[test]
    fn a_program_that_is_there_and_will_not_start_is_told_apart_from_a_missing_one() {
        let program =
            std::env::temp_dir().join(format!("bdi-not-executable-{}", std::process::id()));
        std::fs::write(&program, "#!/bin/sh\nexit 0\n").expect("the file is ours to write");
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o644))
            .expect("the mode is ours to set");

        let failure = RealRunner
            .run(&program.to_string_lossy(), &[], None, &Env::new())
            .expect_err("the file has no execute bit");
        std::fs::remove_file(&program).expect("the file is ours to remove");

        assert_eq!(failure.kind, FailureKind::Unstartable);
    }

    /// A working directory that is not there raises the same `ENOENT` as a
    /// program that is not there, so the directory is asked after and it
    /// decides: a project whose path has gone is something the reader had
    /// and lost, and the silence `Absent` earns belongs to neither.
    #[test]
    fn a_directory_that_is_not_there_is_not_a_program_that_was_never_installed() {
        let failure = RealRunner
            .run(
                "sh",
                &["-c", "true"],
                Some(Path::new("/bdi-no-such-directory")),
                &Env::new(),
            )
            .expect_err("the directory is not there");

        assert_eq!(failure.kind, FailureKind::Unstartable);
    }

    /// A program is installed and its interpreter is not. The kernel reports
    /// the interpreter's absence as the program's — `ENOENT`, the same as a
    /// name nothing holds — so the error alone would have called an
    /// installed provider one nobody installed, and given the reader the
    /// silence instead of the notice.
    #[test]
    fn a_program_whose_interpreter_is_missing_is_not_a_program_nobody_installed() {
        let (dir, program) = a_script_whose_interpreter_is_missing("by-its-path");

        let failure = RealRunner
            .run(&program.to_string_lossy(), &[], None, &Env::new())
            .expect_err("the interpreter is not there");
        std::fs::remove_dir_all(&dir).expect("the directory is ours to remove");

        assert_eq!(failure.kind, FailureKind::Unstartable);
    }

    /// The `PATH` searched is the child's own. `environment` hands over
    /// whatever entering a project's directory produced, `PATH` included, so
    /// a program installed only where direnv puts it is installed — and
    /// asking the `PATH` `bdi` itself runs under would answer about a
    /// different set of binaries entirely.
    #[test]
    fn a_program_installed_only_on_the_childs_own_path_is_found_there() {
        let (dir, _) = a_script_whose_interpreter_is_missing("by-its-name");
        let mut env = Env::new();
        env.insert("PATH".to_string(), dir.to_string_lossy().to_string());

        let failure = RealRunner
            .run("bdi-broken-interpreter", &[], None, &env)
            .expect_err("the interpreter is not there");
        std::fs::remove_dir_all(&dir).expect("the directory is ours to remove");

        assert_eq!(failure.kind, FailureKind::Unstartable);
    }

    /// A `PATH` entry that is relative names a directory under the child's
    /// own, because the child searched its `PATH` after entering that
    /// directory. Resolving one against `bdi`'s directory instead asks about
    /// somewhere nothing was ever installed, and answers `NotInstalled` for
    /// a program the child found and could not start.
    #[test]
    fn a_relative_path_entry_is_read_from_the_directory_the_child_entered() {
        let (dir, _) = a_script_whose_interpreter_is_missing("by-a-relative-entry");
        let under = dir.join("bin");
        std::fs::create_dir_all(&under).expect("the directory is ours to make");
        std::fs::rename(
            dir.join("bdi-broken-interpreter"),
            under.join("bdi-broken-interpreter"),
        )
        .expect("the file is ours to move");
        let mut env = Env::new();
        env.insert("PATH".to_string(), "bin".to_string());

        let failure = RealRunner
            .run("bdi-broken-interpreter", &[], Some(&dir), &env)
            .expect_err("the interpreter is not there");
        std::fs::remove_dir_all(&dir).expect("the directory is ours to remove");

        assert_eq!(failure.kind, FailureKind::Unstartable);
    }

    /// A symlink whose target has gone is the plainest case of a program
    /// that is installed and broken: something put it there and something
    /// else took away what it points at. The spawn fails `ENOENT` like a
    /// name nothing holds, and asking whether the path resolves agrees with
    /// the error rather than correcting it.
    #[test]
    fn a_program_whose_symlink_dangles_is_installed_and_broken() {
        let program = std::env::temp_dir().join(format!("bdi-dangling-{}", std::process::id()));
        let _ = std::fs::remove_file(&program);
        std::os::unix::fs::symlink("/bdi-no-such-target", &program)
            .expect("the link is ours to make");

        let failure = RealRunner
            .run(&program.to_string_lossy(), &[], None, &Env::new())
            .expect_err("the target is not there");
        std::fs::remove_file(&program).expect("the link is ours to remove");

        assert_eq!(failure.kind, FailureKind::Unstartable);
    }

    /// A directory holding one executable script naming an interpreter that
    /// is not there, and the path to it.
    fn a_script_whose_interpreter_is_missing(named: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "bdi-broken-interpreter-{named}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("the directory is ours to make");
        let program = dir.join("bdi-broken-interpreter");
        std::fs::write(&program, "#!/bdi-no-such-interpreter\nexit 0\n")
            .expect("the file is ours to write");
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755))
            .expect("the mode is ours to set");
        (dir, program)
    }

    /// Both missing at once. A directory that was never there to look in is
    /// no evidence that nothing is installed, so the answer is the one that
    /// does not cost the reader a notice.
    #[test]
    fn a_missing_program_in_a_missing_directory_is_reported_as_the_directory() {
        let failure = RealRunner
            .run(
                "bdi-no-such-program",
                &[],
                Some(Path::new("/bdi-no-such-directory")),
                &Env::new(),
            )
            .expect_err("neither the directory nor the program is there");

        assert_eq!(failure.kind, FailureKind::Unstartable);
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
