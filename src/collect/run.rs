//! The boundary every subprocess crosses: what a child is told, what it
//! answers with, and why a run that gave nothing usable did not.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::process::Command;

use crate::collect::environment::NEVER_INHERITED;
use crate::model::types::Unreadable;

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
    /// The run could not be started, and whether anything is installed under
    /// that name was not established: an unsearchable directory on `PATH`
    /// refuses the search and the spawn on the same permission, and a
    /// working directory that was never there explains the error on its own.
    /// It is the weaker of the two, and the one a caller that cannot tell
    /// falls to, so what it says of the machine stays true either way.
    Unstartable,
    /// Something was found under that name and the run could not be started:
    /// a symlink whose target has gone, no execute bit, a directory under
    /// the name. This is the one that earns the word installed.
    InstalledUnstartable,
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
///
/// `unreadable` is the one thing a failure carries past its classification,
/// and only a `Parse` has it. Nothing was on stderr for it to have come from:
/// a command whose output would not parse is a command that succeeded, so
/// what is described is its stdout and `bdi`'s reading of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunFailure {
    pub kind: FailureKind,
    pub program: String,
    pub detail: String,
    pub unreadable: Option<Unreadable>,
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

/// What a search for the program found: something under that name, nothing
/// under it, or a directory that refused the search, which settles neither.
///
/// The third answer is the whole of what keeps this the same on both libcs.
/// A `PATH` entry nothing may search refuses the search and the `execve`
/// alike, on the one missing permission, and the libc chooses which of them
/// to report: glibc carries the `EACCES` across the whole search and answers
/// with it, where Darwin's answers `ENOENT` — the error a name nothing holds
/// gets — for the identical search. Both of its mechanisms do, measured
/// 2026-09-04 on Darwin 25.6.0 against glibc 2.42 with one C program:
/// `posix_spawnp` and `execvp` agree within each platform and disagree
/// across them, so this is the libc's answer rather than the call shape's.
/// What they agree on is the `EACCES` from the `lstat`, so the search says
/// what the spawn's error cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnderThatName {
    Something,
    Nothing,
    /// A directory the search had to look inside would not be searched, so
    /// nothing about an installation was established.
    Unestablished,
}

impl UnderThatName {
    /// One `PATH` entry's answer against everything the entries before it
    /// gave. Something found anywhere is the search's answer; short of that,
    /// one directory that refused it leaves the whole search unestablished,
    /// wherever on `PATH` that directory sits.
    fn or(self, next: Self) -> Self {
        match (self, next) {
            (Self::Something, _) | (_, Self::Something) => Self::Something,
            (Self::Unestablished, _) | (_, Self::Unestablished) => Self::Unestablished,
            (Self::Nothing, Self::Nothing) => Self::Nothing,
        }
    }
}

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
fn installed(program: &str, cwd: Option<&Path>, env: &Env) -> UnderThatName {
    let named = Path::new(program);
    let where_the_child_looked =
        |at: &Path| cwd.map_or_else(|| at.to_path_buf(), |directory| directory.join(at));
    if program.contains(std::path::MAIN_SEPARATOR) {
        return under_that_name(&where_the_child_looked(named));
    }
    env.get(PATH)
        .map(std::ffi::OsString::from)
        .or_else(|| std::env::var_os(PATH))
        .map_or(UnderThatName::Nothing, |path| {
            std::env::split_paths(&path)
                .map(|at| under_that_name(&where_the_child_looked(&at).join(named)))
                .fold(UnderThatName::Nothing, UnderThatName::or)
        })
}

/// Whether the machine holds a program of that name, asked before anything is
/// run, from where the child that would run it is going to look.
///
/// The two callers of the search ask it for opposite reasons and want
/// opposite answers to its third one. `RunFailure` asks after a spawn already
/// failed, so it wants the weakest claim that stays true — a directory that
/// refused the search leaves the machine unestablished and the failure says
/// so. This asks *before*, to decide whether to run anything at all, and a
/// search that established nothing is no ground to act on: only something
/// actually found is a yes.
///
/// `cwd` is the working directory that spawn would be given, and passing it
/// is what keeps the two agreeing: POSIX makes an empty `PATH` entry the
/// working directory, so a search made from somewhere else answers about a
/// program the child would not have found, or misses one it would. A decision
/// to run something has to be taken about the run that would actually happen.
///
/// The `PATH` is `bdi`'s own, because the question is asked to decide what a
/// child would be and a project's environment is the thing not captured yet.
pub fn found_on_path(program: &str, cwd: Option<&Path>) -> bool {
    installed(program, cwd, &Env::new()) == UnderThatName::Something
}

/// What the filesystem holds there, which is a different question from
/// whether it resolves, and a different one again from whether we were
/// allowed to ask.
///
/// A symlink whose target has gone — a profile collected out from under it,
/// a build deleted — is something installed and broken, and it is the state
/// this whole distinction is drawn for. `Path::exists` follows the link and
/// so answers `false` for one, which is the answer reserved for a name
/// nothing holds. `symlink_metadata` is the `lstat` that stops at the entry.
///
/// That `lstat` wants the same search permission on the directory `execve`
/// was refused, so a refusal is not an absence and is kept apart from one.
/// Nor is any other error it can fail with: a name too long to exist, a link
/// that loops, a process out of file descriptors. Only the two the kernel
/// gives for *no entry here* answer the question — nothing under that name,
/// and nothing under a `PATH` entry that is not a directory to hold it. The
/// rest are the search failing rather than succeeding at nothing, and they
/// are the same two the child kept searching past.
fn under_that_name(path: &Path) -> UnderThatName {
    match std::fs::symlink_metadata(path) {
        Ok(_) => UnderThatName::Something,
        Err(absent)
            if absent.kind() == std::io::ErrorKind::NotFound
                || absent.kind() == std::io::ErrorKind::NotADirectory =>
        {
            UnderThatName::Nothing
        }
        Err(_) => UnderThatName::Unestablished,
    }
}

/// Whether the child could have entered the working directory it was given.
///
/// A directory that is not there, and one that will not be searched, each
/// refused the child on its own account, and a `PATH` it never searched from
/// there is no evidence about the machine. `lstat` through the directory
/// wants the search permission `chdir` wanted, and fails where the directory
/// is missing or is not one, so the single call settles all three. Where no
/// directory was given the child stayed where `bdi` is, which it plainly
/// entered.
///
/// An empty path is not that case and is asked separately, because joining
/// `.` onto it gives `.` — bdi's own directory, which answers that the child
/// entered somewhere it was never sent. `chdir` refuses an empty path
/// outright: measured through `Command` on 2026-09-04, `current_dir("")`
/// answers `ENOENT`. It is the opposite of the rule for a `PATH` entry,
/// where POSIX makes the empty one mean the working directory.
fn the_child_entered(cwd: Option<&Path>) -> bool {
    cwd.is_none_or(|directory| {
        !directory.as_os_str().is_empty()
            && under_that_name(&directory.join(std::path::Component::CurDir))
                == UnderThatName::Something
    })
}

/// Whether bdi's own call carries a fault of its own to report, which is a
/// different question from anything the machine answered.
///
/// Both members are about the call bdi built, which bdi controls. Neither is
/// a list of the errors a `PATH` search can come back with: that list belongs
/// to the platform, it is unspecified, and it was measured answering three
/// different ways across two libcs.
///
/// `std` refuses a `Command` carrying a NUL byte — in the program, an
/// argument, an environment key or value, or the working directory — before
/// it reaches the kernel, so the refusal has no errno at all. That is
/// structural rather than enumerated, and holds for members of the family
/// nobody has thought of yet. Measured through `Command` on glibc 2.42 on
/// 2026-09-04: every NUL case answers `raw_os_error() == None`, and every
/// other refusal measured carries a number.
///
/// A working directory the child could not enter refused it after the fork
/// and before the `execve`, and no errno separates that from a search. A
/// directory that is gone answers `ENOENT`, as an absent program does; one
/// that is a file answers `ENOTDIR`, as a file-shaped `PATH` entry does on
/// glibc; one nothing may enter answers `EACCES`, as a program without its
/// execute bit does. Measured in the same run. `lstat` tells those apart and
/// the error cannot.
fn ours_to_report(cause: &std::io::Error, cwd: Option<&Path>) -> bool {
    cause.raw_os_error().is_none() || !the_child_entered(cwd)
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
            unreadable: None,
        }
    }

    pub fn unstartable(program: &str, cause: impl fmt::Display) -> Self {
        Self::could_not_be_started(FailureKind::Unstartable, program, cause)
    }

    /// The detail is the same sentence for both: it reports the kernel's
    /// refusal, which is all either kind knows, and the claim the two are
    /// told apart by is made in the phrase rather than here.
    fn could_not_be_started(kind: FailureKind, program: &str, cause: impl fmt::Display) -> Self {
        Self {
            kind,
            program: program.to_string(),
            detail: format!("{program} could not be started: {cause}"),
            unreadable: None,
        }
    }

    /// Which of the three a refused spawn was.
    ///
    /// Both of the answers that say something about the machine have to be
    /// earned, and the same probe earns them. `NotInstalled` is the one that
    /// costs the reader their notice; `InstalledUnstartable` is the one that
    /// sends them looking for a program to repair. Neither may be defaulted
    /// to, and what is left when neither is earned is `Unstartable`, which
    /// claims nothing beyond the refusal itself.
    ///
    /// The spawn's error earns none of them, and which failure it was is not
    /// read here at all.
    /// `ENOENT` is what the kernel answers to a working directory that is
    /// not there, and to a program whose own interpreter or loader is
    /// missing, as readily as to a name nothing holds — it reports the
    /// interpreter's absence as the program's. `EACCES` is what it answers
    /// for a whole family: an unsearchable directory on `PATH`, which one
    /// entry produces on a machine that has no such program anywhere,
    /// alongside a program that is plainly there without its execute bit.
    /// And which of those two a refused search comes back as is the libc's
    /// to choose — glibc `EACCES`, Darwin `ENOENT` — so an answer read off
    /// the error is an answer that inverts between platforms.
    ///
    /// The search is asked instead, on every refusal. It tells apart what
    /// the error conflates, its own refusal included: `lstat` wants the
    /// search permission `execve` wanted, so a directory nothing may search
    /// stops the search where it stopped the spawn — and that is reported as
    /// the third answer rather than as an absence, which is the case where
    /// nothing is established and so nothing is claimed.
    ///
    /// The working directory is asked the same way and for the same reason.
    /// One the child could not enter refused the spawn on its own account,
    /// so nothing a search made from anywhere else is evidence about the
    /// machine.
    ///
    /// The question the error is asked is not whether the child got far
    /// enough to search — nothing in the answer can say, and every predicate
    /// built to ask it is a list of that platform's errors under another
    /// name. It is whether anything is wrong on bdi's own side that the
    /// reader has to be told about. `NotInstalled` is the only verdict that
    /// reports nothing, so it is the only one that has to be earned.
    ///
    /// What that leaves is a refusal carrying an errno that could have come
    /// from the search or from a fork the system would not give, which
    /// nothing here can separate. It is answered from the probe, so a
    /// machine holding no `bd` is told so rather than told that a `bd` it
    /// does not have would not start. The cost is named in
    /// `a_refusal_nothing_can_place_is_answered_by_the_search_that_completed`.
    fn could_not_start(
        program: &str,
        cwd: Option<&Path>,
        env: &Env,
        cause: &std::io::Error,
    ) -> Self {
        match installed(program, cwd, env) {
            _ if ours_to_report(cause, cwd) => Self::unstartable(program, cause),
            UnderThatName::Something => {
                Self::could_not_be_started(FailureKind::InstalledUnstartable, program, cause)
            }
            UnderThatName::Nothing => Self::not_installed(program, cause),
            UnderThatName::Unestablished => Self::unstartable(program, cause),
        }
    }

    /// The read is left blank here and named by `reading`, because the two
    /// places a parse failure arises know different halves of it. A caller
    /// deserialising an answer it asked for knows both; the runner decoding
    /// that answer's bytes was handed a command line rather than the question
    /// that composed it, and cannot tell which of its arguments was the
    /// subcommand.
    pub fn parse(program: &str, cause: impl fmt::Display) -> Self {
        Self {
            kind: FailureKind::Parse,
            program: program.to_string(),
            detail: format!("{program} returned output bdi cannot read: {cause}"),
            unreadable: Some(Unreadable {
                read: String::new(),
                cause: cause.to_string(),
            }),
        }
    }

    /// Name the read this failure came from. A no-op on every kind but
    /// `Parse`: nothing else carries anything for it to name.
    pub fn reading(mut self, read: &str) -> Self {
        if let Some(unreadable) = self.unreadable.as_mut() {
            unreadable.read = read.to_string();
        }
        self
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
            unreadable: None,
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

    /// Every kind a run can fail with. The match is what makes it every one:
    /// a kind added above and not to this chain does not compile.
    pub fn every_failure_kind() -> impl Iterator<Item = FailureKind> {
        std::iter::successors(Some(FailureKind::Auth), |kind| match kind {
            FailureKind::Auth => Some(FailureKind::Unavailable),
            FailureKind::Unavailable => Some(FailureKind::Gone),
            FailureKind::Gone => Some(FailureKind::Busy),
            FailureKind::Busy => Some(FailureKind::NotInstalled),
            FailureKind::NotInstalled => Some(FailureKind::Unstartable),
            FailureKind::Unstartable => Some(FailureKind::InstalledUnstartable),
            FailureKind::InstalledUnstartable => Some(FailureKind::Parse),
            FailureKind::Parse => Some(FailureKind::Unsupported),
            FailureKind::Unsupported => Some(FailureKind::UnknownFlag),
            FailureKind::UnknownFlag => None,
        })
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

    /// A path in this process's own scratch space that nothing holds.
    ///
    /// A test naming something absent has to establish that rather than
    /// borrow it from the machine it runs on: a name that happens to be free
    /// here is a premise the host grants, and the day something holds it the
    /// test measures a case it was never written for.
    fn nothing_holds(named: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("bdi-{named}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let _ = std::fs::remove_file(&path);
        assert!(
            std::fs::symlink_metadata(&path).is_err(),
            "{} is held by something, so a test naming it absent proves nothing",
            path.display()
        );
        path
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

    /// And no failure classified from that text carries anything out of it.
    ///
    /// `unreadable` is the one thing a failure holds beyond its kind, so it
    /// is the one place text could reach the screen from. Every failure that
    /// reads stderr has to arrive without one, whatever was on it — which is
    /// what makes an answer that would not parse a bounded exception rather
    /// than a hole in the rule above. That failure never read stderr: the run
    /// succeeded.
    #[test]
    fn no_failure_classified_from_stderr_carries_anything_out_of_it() {
        for said in [
            REFUSED,
            UNREACHABLE,
            EMBEDDED,
            NO_SUCH_PANE,
            PANE_BUSY,
            UNPLACED,
        ] {
            assert!(
                failing_command(said).unreadable.is_none(),
                "text on stderr reached a failure's own field: {said:?}"
            );
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

    /// The `PATH` is the fixture's rather than this machine's, because the
    /// assertion is an absence: something found anywhere settles the search
    /// wherever it sits, so a test asserting a program is *there* may
    /// inherit a `PATH` safely, and one asserting nothing is there may not.
    /// An inherited entry nothing may search would leave the search
    /// unestablished and turn this red for a reason nothing in it names.
    #[test]
    fn a_program_that_is_not_installed_says_nothing_is_installed() {
        let empty = a_directory_holding_nothing("with-no-program-in-it");
        let mut env = Env::new();
        env.insert(PATH.to_string(), empty.to_string_lossy().to_string());

        let failure = RealRunner
            .run("bdi-no-such-program", &[], None, &env)
            .expect_err("nothing by that name is on PATH");
        std::fs::remove_dir_all(&empty).expect("the directory is ours to remove");

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

        assert_eq!(failure.kind, FailureKind::InstalledUnstartable);
    }

    /// The empty working directory is the one a path join answers wrongly
    /// for: `"".join(".")` is `"."`, which is bdi's own directory and
    /// plainly there, so a probe that only joined would report the child
    /// entering somewhere it was never sent. `chdir` refuses it — measured
    /// `ENOENT` — and with the program absent as well, the reader would be
    /// told the machine has no `bd` and never told the directory was
    /// unusable.
    #[test]
    fn an_empty_working_directory_is_not_one_the_child_entered() {
        let empty = a_directory_holding_nothing("nothing-to-find-from-nowhere");
        let mut env = Env::new();
        env.insert(PATH.to_string(), empty.display().to_string());

        let failure = RealRunner
            .run("bdi-no-such-program", &[], Some(Path::new("")), &env)
            .expect_err("an empty working directory is refused");

        assert_eq!(failure.kind, FailureKind::Unstartable);
    }

    /// A working directory that is not there raises the same `ENOENT` as a
    /// program that is not there, so the directory is asked after and it
    /// decides: a project whose path has gone is something the reader had
    /// and lost, and the silence `Absent` earns belongs to neither.
    ///
    /// It is not the program's failure either. `sh` is installed and fine,
    /// and the directory it was told to run in is bdi's own call, so the
    /// answer names the refusal and stops there rather than sending the
    /// reader to repair an `sh` that would have run anywhere else.
    #[test]
    fn a_directory_that_is_not_there_is_not_a_program_that_was_never_installed() {
        let gone = nothing_holds("no-such-directory");
        let failure = RealRunner
            .run("sh", &["-c", "true"], Some(&gone), &Env::new())
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

        assert_eq!(failure.kind, FailureKind::InstalledUnstartable);
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

        assert_eq!(failure.kind, FailureKind::InstalledUnstartable);
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

        assert_eq!(failure.kind, FailureKind::InstalledUnstartable);
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
        std::os::unix::fs::symlink(nothing_holds("dangling-target"), &program)
            .expect("the link is ours to make");

        let failure = RealRunner
            .run(&program.to_string_lossy(), &[], None, &Env::new())
            .expect_err("the target is not there");
        std::fs::remove_file(&program).expect("the link is ours to remove");

        assert_eq!(failure.kind, FailureKind::InstalledUnstartable);
    }

    /// One `PATH` entry nothing may search is enough to refuse the spawn,
    /// on a machine holding no such program anywhere. The entry need not be
    /// the only one or the first: the child tries every entry, and the
    /// refusal it comes back with is whichever of `EACCES` and `ENOENT` the
    /// libc chose to carry out of the search.
    ///
    /// The probe cannot see past that entry either — `lstat` wants the same
    /// search permission `execve` was refused — and that is the point rather
    /// than a gap: nothing here establishes an installation, so nothing may
    /// be said of one. The reader who is told bd is installed goes hunting a
    /// bd that was never on the machine.
    #[test]
    fn a_spawn_no_search_could_reach_says_nothing_about_an_installation() {
        let locked = an_unsearchable_directory("nothing-inside-it");
        let mut env = Env::new();
        env.insert(PATH.to_string(), locked.display().to_string());

        let failure = RealRunner
            .run("bdi-no-such-program", &[], None, &env)
            .expect_err("the one directory on PATH cannot be searched");
        make_searchable_again(&locked);

        assert_eq!(failure.kind, FailureKind::Unstartable);
    }

    /// A call the child was refused before it looked anywhere, on a machine
    /// where the program is genuinely absent. The search answers, truthfully
    /// and about something else, and answering with it would report a
    /// malformed call as an absent installation — the one failure that is
    /// not a finding, so the reader would lose the notice as well as the
    /// reason. A NUL byte in an argument is the refusal bdi can be handed.
    #[test]
    fn a_call_refused_before_the_child_looked_is_not_an_absent_installation() {
        let empty = a_directory_holding_nothing("nothing-to-find");
        let mut env = Env::new();
        env.insert(PATH.to_string(), empty.display().to_string());

        let failure = RealRunner
            .run("bdi-no-such-program", &["a\0b"], None, &env)
            .expect_err("a NUL byte in an argument is refused before the search");

        assert_eq!(failure.kind, FailureKind::Unstartable);
    }

    /// The argument is not the only place a NUL byte reaches, and the cut is
    /// not a list of the places: `std` builds the environment into the call
    /// too, and refuses the whole call the same way and with the same
    /// absent errno. A cut that named the argument would pass this.
    #[test]
    fn a_nul_byte_in_the_environment_is_refused_before_the_child_looked_too() {
        let empty = a_directory_holding_nothing("nothing-to-find-either");
        let mut env = Env::new();
        env.insert(PATH.to_string(), empty.display().to_string());
        env.insert("BDI_NUL".to_string(), "a\0b".to_string());

        let failure = RealRunner
            .run("bdi-no-such-program", &[], None, &env)
            .expect_err("a NUL byte in the environment is refused before the search");

        assert_eq!(failure.kind, FailureKind::Unstartable);
    }

    /// A call bdi could not build says nothing about the machine whatever
    /// the search found, so the arm that finds the program answers it the
    /// same way as the arm that does not. Told otherwise, a reader is sent
    /// to repair a bd that is installed and fine.
    #[test]
    fn a_call_refused_before_the_child_looked_is_not_an_installation_that_refused() {
        let dir = a_program_without_its_execute_bit("beside-a-call-we-broke");
        let mut env = Env::new();
        env.insert(PATH.to_string(), dir.to_string_lossy().to_string());

        let failure = RealRunner
            .run("bdi-without-its-execute-bit", &["a\0b"], None, &env)
            .expect_err("a NUL byte in an argument is refused before the search");
        std::fs::remove_dir_all(&dir).expect("the directory is ours to remove");

        assert_eq!(failure.kind, FailureKind::Unstartable);
    }

    /// The errors a search can end on are the platform's and unspecified, so
    /// nothing here reads the number: a refusal carrying one nobody has
    /// placed is answered by the search, which completed and found nothing.
    /// An argument list too long for the kernel is such a refusal — it is
    /// `E2BIG`, which no `PATH` search produces and no list here names.
    ///
    /// The limit this states is real. A fork the system will not give
    /// carries an errno too, and is answered the same way, so a machine out
    /// of processes and genuinely without `bd` is told `bd` is absent —
    /// which is true, and silent about the exhaustion. Nothing in a spawn's
    /// answer separates that from a search, and the alternative is to
    /// default to `Unstartable`, which tells a machine holding no `bd` that
    /// a `bd` it does not have would not start.
    #[test]
    fn a_refusal_nothing_can_place_is_answered_by_the_search_that_completed() {
        let empty = a_directory_holding_nothing("nothing-to-find-at-all");
        let mut env = Env::new();
        env.insert(PATH.to_string(), empty.display().to_string());
        let far_too_long = "x".repeat(128 * 1024);
        let arguments: Vec<&str> = (0..256).map(|_| far_too_long.as_str()).collect();

        let failure = RealRunner
            .run("bdi-no-such-program", &arguments, None, &env)
            .expect_err("nothing on PATH holds it and the arguments are too long anyway");

        assert_eq!(
            failure.kind,
            FailureKind::NotInstalled,
            "the spawn answered {:?}",
            failure.detail
        );
    }

    /// The other half of the same errno, and the reason the probe is asked
    /// on every refusal rather than on `ENOENT` alone: a program that is
    /// plainly there without its execute bit fails `EACCES` too, and there
    /// the search permission the probe needs is the one it has. So `lstat`
    /// answers, the installation is established, and the reader is told the
    /// thing they can act on.
    #[test]
    fn a_program_found_on_path_without_its_execute_bit_is_installed_and_broken() {
        let dir = a_program_without_its_execute_bit("on-its-own");
        let mut env = Env::new();
        env.insert(PATH.to_string(), dir.to_string_lossy().to_string());

        let failure = RealRunner
            .run("bdi-without-its-execute-bit", &[], None, &env)
            .expect_err("the file has no execute bit");
        std::fs::remove_dir_all(&dir).expect("the directory is ours to remove");

        assert_eq!(failure.kind, FailureKind::InstalledUnstartable);
    }

    /// A `PATH` entry nothing may search sits beside one holding the program,
    /// and the spawn is refused on the search rather than on the program. The
    /// entry that answered is the one that decides: the child would have
    /// tried every entry, so an installation found at any of them is found,
    /// and one broken entry does not blind the reader to a bd they can see.
    #[test]
    fn an_unsearchable_entry_does_not_hide_a_program_another_entry_holds() {
        let locked = an_unsearchable_directory("beside-one-that-answers");
        let dir = a_program_without_its_execute_bit("beside-one-that-refuses");
        let mut env = Env::new();
        env.insert(
            PATH.to_string(),
            format!("{}:{}", locked.display(), dir.display()),
        );

        let failure = RealRunner
            .run("bdi-without-its-execute-bit", &[], None, &env)
            .expect_err("one entry cannot be searched and the other holds no executable");
        make_searchable_again(&locked);
        std::fs::remove_dir_all(&dir).expect("the directory is ours to remove");

        assert_eq!(failure.kind, FailureKind::InstalledUnstartable);
    }

    /// The distinction the libcs do not report the same way, asked of the
    /// search itself. A directory that will not be searched leaves the
    /// question open where one that answers and holds nothing closes it, and
    /// the `lstat` refuses alike on both platforms where the spawn's own
    /// error is `EACCES` on one and `ENOENT` on the other.
    #[test]
    fn a_directory_that_refuses_the_search_is_not_one_that_holds_nothing() {
        let locked = an_unsearchable_directory("which-will-not-say");
        let open = a_directory_holding_nothing("which-says-so");

        let refused = under_that_name(&locked.join("bdi-no-such-program"));
        let answered = under_that_name(&open.join("bdi-no-such-program"));
        make_searchable_again(&locked);
        std::fs::remove_dir_all(&open).expect("the directory is ours to remove");

        assert_eq!(refused, UnderThatName::Unestablished);
        assert_eq!(answered, UnderThatName::Nothing);
    }

    /// A working directory that is there and will not be entered, against a
    /// `PATH` of absolute entries the search can complete without it. The
    /// child never ran from there, so the completeness is `bdi`'s and not
    /// the child's and says nothing about the machine — and the directory
    /// has to be asked after in its own right to keep that so, because
    /// `is_dir` answers yes for a directory nothing may enter and the
    /// `EACCES` that used to catch it is no longer read.
    #[test]
    fn a_working_directory_nothing_may_enter_is_not_a_search_that_found_nothing() {
        let locked = an_unsearchable_directory("with-no-way-in");
        let open = a_directory_holding_nothing("that-the-search-reaches");
        let mut env = Env::new();
        env.insert(PATH.to_string(), open.to_string_lossy().to_string());

        let failure = RealRunner
            .run("bdi-no-such-program", &[], Some(&locked), &env)
            .expect_err("the working directory cannot be entered");
        make_searchable_again(&locked);
        std::fs::remove_dir_all(&open).expect("the directory is ours to remove");

        assert_eq!(failure.kind, FailureKind::Unstartable);
    }

    /// An empty `PATH` entry names the working directory, which is POSIX and
    /// what the child did. The search has to look there as well, or a bd
    /// sitting in the project's own directory is a bd the reader is told
    /// they never installed.
    ///
    /// The overlay sets `PATH` to nothing rather than leaving it out, and
    /// the two are different questions: an overlay without the key falls
    /// through to `bdi`'s own, which under `cargo test` is a dev shell with
    /// a real bd on it, so such a test asks about this machine instead of
    /// about its fixture and passes without reaching the empty entry.
    #[test]
    fn an_empty_path_entry_is_the_directory_the_child_entered() {
        let dir = a_program_without_its_execute_bit("named-by-an-empty-entry");
        let mut env = Env::new();
        env.insert(PATH.to_string(), String::new());

        let failure = RealRunner
            .run("bdi-without-its-execute-bit", &[], Some(&dir), &env)
            .expect_err("the file has no execute bit");
        std::fs::remove_dir_all(&dir).expect("the directory is ours to remove");

        assert_eq!(failure.kind, FailureKind::InstalledUnstartable);
    }

    /// The refusal reaching the search through the entry rather than through
    /// the directory the child entered: this one it did enter, and a
    /// relative entry under it is what nothing may search. The answer is the
    /// same as for an absolute entry because it is the same refusal, and
    /// scoring it as an absence instead would tell a reader whose bd sits in
    /// that very directory that they never installed one.
    #[test]
    fn a_relative_path_entry_that_refuses_the_search_is_not_an_absence() {
        let dir = std::env::temp_dir().join(format!("bdi-entered-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("the directory is ours to make");
        let locked = dir.join("bin");
        std::fs::create_dir_all(&locked).expect("the directory is ours to make");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644))
            .expect("the mode is ours to set");
        let mut env = Env::new();
        env.insert(PATH.to_string(), "bin".to_string());

        let failure = RealRunner
            .run("bdi-no-such-program", &[], Some(&dir), &env)
            .expect_err("the one directory on PATH cannot be searched");
        make_searchable_again(&locked);
        std::fs::remove_dir_all(&dir).expect("the directory is ours to remove");

        assert_eq!(failure.kind, FailureKind::Unstartable);
    }

    /// A `PATH` entry that is a file rather than a directory holds nothing,
    /// and one machine with one absence gets one answer wherever that entry
    /// sits. What the child answers with does turn on where it sat, and is
    /// why this is asked from both sides: measured on glibc 2.42 on
    /// 2026-09-04, the file ahead of a directory leaves the directory's
    /// `ENOENT` to be carried out, and the file behind it carries its own
    /// `ENOTDIR`. Darwin answers `ENOENT` to both. None of that reaches the
    /// verdict, which is the point — the position of a junk entry is not a
    /// fact about whether a program is installed, and neither is the libc.
    #[test]
    fn a_path_entry_that_is_not_a_directory_holds_nothing_wherever_it_sits() {
        let file = std::env::temp_dir().join(format!("bdi-not-a-directory-{}", std::process::id()));
        std::fs::write(&file, "").expect("the file is ours to write");
        let empty = a_directory_holding_nothing("beside-a-file");

        for path in [
            format!("{}:{}", file.display(), empty.display()),
            format!("{}:{}", empty.display(), file.display()),
        ] {
            let mut env = Env::new();
            env.insert(PATH.to_string(), path.clone());

            let failure = RealRunner
                .run("bdi-no-such-program", &[], None, &env)
                .expect_err("neither entry on PATH holds it");

            assert_eq!(failure.kind, FailureKind::NotInstalled, "PATH was {path}");
        }
        std::fs::remove_file(&file).expect("the file is ours to remove");
    }

    /// Every other way the search can fail is the search failing, not the
    /// search finding nothing — a link that loops here, and elsewhere a name
    /// too long to exist or a process with no file descriptors left. The
    /// kernel refuses the spawn on it rather than searching past it, and
    /// nothing about the machine is established either way.
    #[test]
    fn a_path_entry_the_search_cannot_resolve_establishes_nothing() {
        let looping = std::env::temp_dir().join(format!("bdi-loop-a-{}", std::process::id()));
        let back = std::env::temp_dir().join(format!("bdi-loop-b-{}", std::process::id()));
        let _ = std::fs::remove_file(&looping);
        let _ = std::fs::remove_file(&back);
        std::os::unix::fs::symlink(&back, &looping).expect("the link is ours to make");
        std::os::unix::fs::symlink(&looping, &back).expect("the link is ours to make");
        let mut env = Env::new();
        env.insert(PATH.to_string(), looping.to_string_lossy().to_string());

        let failure = RealRunner
            .run("bdi-no-such-program", &[], None, &env)
            .expect_err("the one entry on PATH points at itself");
        std::fs::remove_file(&looping).expect("the link is ours to remove");
        std::fs::remove_file(&back).expect("the link is ours to remove");

        assert_eq!(failure.kind, FailureKind::Unstartable);
    }

    /// A directory holding one file named as a program, without the execute
    /// bit that would let it run.
    fn a_program_without_its_execute_bit(named: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("bdi-no-execute-bit-{named}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("the directory is ours to make");
        let program = dir.join("bdi-without-its-execute-bit");
        std::fs::write(&program, "#!/bin/sh\nexit 0\n").expect("the file is ours to write");
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o644))
            .expect("the mode is ours to set");
        dir
    }

    /// A directory that is there, may be searched, and holds nothing.
    fn a_directory_holding_nothing(named: &str) -> PathBuf {
        let empty = std::env::temp_dir().join(format!("bdi-empty-{named}-{}", std::process::id()));
        std::fs::create_dir_all(&empty).expect("the directory is ours to make");
        empty
    }

    /// A directory nothing may search.
    fn an_unsearchable_directory(named: &str) -> PathBuf {
        let locked =
            std::env::temp_dir().join(format!("bdi-locked-{named}-{}", std::process::id()));
        std::fs::create_dir_all(&locked).expect("the directory is ours to make");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644))
            .expect("the mode is ours to set");
        locked
    }

    /// Put the mode back before removing it, since nothing may look inside a
    /// directory it cannot search, this test process included.
    fn make_searchable_again(locked: &Path) {
        std::fs::set_permissions(locked, std::fs::Permissions::from_mode(0o755))
            .expect("the mode is ours to set");
        std::fs::remove_dir_all(locked).expect("the directory is ours to remove");
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
    /// does not cost the reader a notice — and it is no evidence that
    /// anything *is* installed either, so it is the one that claims neither.
    #[test]
    fn a_missing_program_in_a_missing_directory_is_reported_as_the_directory() {
        let gone = nothing_holds("no-such-directory-either");
        let empty = a_directory_holding_nothing("with-no-program-in-it-either");
        let mut env = Env::new();
        env.insert(PATH.to_string(), empty.to_string_lossy().to_string());

        let failure = RealRunner
            .run("bdi-no-such-program", &[], Some(&gone), &env)
            .expect_err("neither the directory nor the program is there");
        std::fs::remove_dir_all(&empty).expect("the directory is ours to remove");

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
