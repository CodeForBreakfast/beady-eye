//! The environment each project's tracker is read in.
//!
//! One capture per project: `bdi`'s own environment, or what entering the
//! project's directory produces — by a command its config names, or by the
//! one its directory implies — with its credential command replacing the
//! password in any of them. Every question asked with it is in `bd`.

use std::path::Path;

use crate::collect::run::{found_on_path, Env, RunFailure, Runner};
use crate::collect::tracker::OpenFailure;
use crate::config::{Command, Project};

/// What `bdi` runs inside a project's environment command to read the
/// environment back. NUL-separated, because a value may hold a newline.
///
/// It is appended rather than written by the reader, so a config names the
/// wrapper — `direnv exec .`, `nix develop -c`, `mise exec --` — and every
/// one of those composes with it the same way: each is a program that runs
/// the rest of its own argv, so appending two more words is all it takes.
const PROBE: &str = "env -0";

/// The variable bd authenticates its Dolt server with.
///
/// No subprocess `bdi` launches inherits it — `git`, `herdr` and a project's
/// own `credential_command` are all arbitrary programs that were never given
/// a tracker's password and have no business holding one.
pub const CREDENTIAL_VAR: &str = "BEADS_DOLT_PASSWORD";

/// The variable bd reads to find a tracker. It outranks the working
/// directory, so a project is read from the directory `bdi` chose only where
/// no inherited value overrules it.
pub const TRACKER_VAR: &str = "BEADS_DIR";

/// Which tracker is read and what authenticates to it are one identity, and
/// inheriting either half reaches another project's database. So the runner
/// tells every child both or neither, whatever program the child is: a
/// tracker's credential is kept from `herdr`, `direnv` and a project's own
/// `credential_command` as much as from another project's bd.
pub const NEVER_INHERITED: [&str; 2] = [CREDENTIAL_VAR, TRACKER_VAR];

/// The credential the shell `bdi` was launched from holds, which a project
/// configuring none reaches its tracker on.
pub fn ambient_credential() -> Option<String> {
    std::env::var(CREDENTIAL_VAR).ok()
}

/// The environment one project's tracker is read with.
///
/// A shell that has entered the project's directory is already configured for
/// its tracker — the mechanism loads the flake, the bd version, and whatever
/// holds the password — so `bdi` reproduces entering the directory rather
/// than reconstructing what entering it would have produced, and the project
/// entry says nothing about what the secret is called or where it lives.
///
/// How the directory is entered comes from the config where a project names
/// an `environment_command`, and from the directory itself where it does not
/// and [`detected`] can see how. Both arrive here as a [`Command`] and are
/// captured, parsed and failed identically; the config wins, because a reader
/// who has said how a project is entered has said it.
///
/// Which mechanism it is stays the config's to name and not `bdi`'s to know.
/// direnv, nix and mise all run a command in an environment, so all three are
/// reached by naming them and by no code here.
///
/// A project neither names a command for nor implies one is read with `bdi`'s
/// own environment, and nothing is run to find that out: a machine with bd
/// and nothing else reads its tracker. `-C` naming the tracker outright is
/// what makes that safe, because a credential belonging to another tracker
/// can only fail to authenticate against the right database, never open the
/// wrong one.
///
/// Captured once per project rather than by wrapping every call, because
/// `direnv exec` reloads the directory each time it runs. Measured against a
/// worktree of this repository on 2026-09-04: 136 to 177 milliseconds warm,
/// 1557ms on the first load after the `.envrc` was allowed, and 3 to 5ms for
/// a project with no `.envrc` at all. It also confines a project whose
/// `.envrc` writes to stdout to this one call, whose parser tolerates it,
/// rather than to every answer bd gives.
///
/// A `credential_command` answers after the environment command rather than
/// instead of it, so the two compose: one says how to reach the environment,
/// the other replaces one variable in it. Naming both was refused while the
/// environment was a mechanism, because a credential answering instead of
/// direnv or after it was a precedence nothing on the screen said. A command
/// has no such question — the password is whatever the credential command
/// last wrote.
///
/// The ambient credential underneath all three is what lets a single-tracker
/// setup configure nothing at all.
///
/// A project that asked to be entered and could not be gets no environment at
/// all, and no bd is run for it. Falling back to `bdi`'s own would read that
/// project's tracker with a bd it did not ask for, and reading is not free:
/// bd rewrites `.beads/.local_version` and runs its schema auto-migration on
/// finding itself newer than the bd that last opened a tracker, before the
/// subcommand and whatever the subcommand is. What the reader gets instead is
/// the project reported as having asked for an environment `bdi` could not
/// produce, which is a sentence they can act on rather than a tracker they
/// cannot put back.
///
/// The credential command does not run either, and the order is what says so:
/// a project with no environment has nothing to be read, so running an
/// arbitrary command a config named would be a side effect spent on a read
/// that is not going to happen.
pub fn tracker_env(
    runner: &dyn Runner,
    project: &Project,
    ambient: Option<&str>,
) -> Result<Env, OpenFailure> {
    let mut env = ambient.map_or_else(Env::new, |password| {
        Env::from([(CREDENTIAL_VAR.to_string(), password.to_string())])
    });
    if let Some(command) = project
        .environment_command
        .clone()
        .or_else(|| detected(&project.path))
    {
        let captured =
            entering(&project.path, runner, &command).map_err(|_| OpenFailure::NoEnvironment)?;
        env.extend(captured);
    }
    if let Some(command) = &project.credential_command {
        let password = runner.run("sh", &["-c", command], Some(&project.path), &lending(&env))?;
        env.insert(
            CREDENTIAL_VAR.to_string(),
            password.trim_end_matches(['\r', '\n']).to_string(),
        );
    }
    Ok(env)
}

/// The wrapper a project's own directory asks for without its config saying
/// so: `direnv exec .`, where the directory holds an `.envrc` and the machine
/// holds a direnv.
///
/// Both halves are needed and they answer different questions. The `.envrc`
/// is the project saying how it is entered; the direnv is the machine saying
/// it can. A machine without one reads every project ambient, which is also
/// what a person's own shell gives them in that directory, so nothing has
/// been given up — and it is what keeps a detection that could not have
/// worked from failing a project the way a config naming a wrapper does.
/// That difference is the whole of what makes this safe to do unasked:
/// `bdi` acts on a guess only where the guess is known to be available.
///
/// The `.envrc` is asked first, and not only because it is the cheaper
/// question. It is the selective one: a machine with direnv has it for every
/// project alike, so the `PATH` search would run for each of them and settle
/// nothing about any.
///
/// direnv is the one mechanism detected, because an `.envrc` is a file and
/// the others are not. nix and mise are entered by a command a person types,
/// and a `flake.nix` says a directory *has* a shell rather than that entering
/// it is how this project's tracker is reached — the config's rung above is
/// where a reader says that.
///
/// What comes back is an ordinary [`Command`], so a detected environment is
/// captured, parsed and failed exactly as a configured one is. A detection
/// that fires and then cannot produce an environment is a project that could
/// not be read, not a quiet return to ambient: the directory said how it is
/// entered and the machine said it could, so something is wrong that a reader
/// can fix — an `.envrc` wanting `direnv allow` is the usual one.
fn detected(path: &Path) -> Option<Command> {
    (path.join(ENTERED_DIRECTORY).exists() && found_on_path(DIRENV, Some(path)))
        .then(|| Command::Line(format!("{DIRENV} exec {THE_DIRECTORY_ITSELF}")))
}

/// The file whose presence says a directory is one direnv would enter.
///
/// Nothing reads it. What is in an `.envrc` is direnv's to evaluate, and a
/// `bdi` that read it would be reconstructing the environment rather than
/// reproducing entering the directory.
const ENTERED_DIRECTORY: &str = ".envrc";

/// The program that enters it.
const DIRENV: &str = "direnv";

/// What direnv is asked to enter, written relative for the reason a config
/// writes it that way: the command runs in the project's own directory, so
/// this is the directory being asked about rather than one spelled twice.
const THE_DIRECTORY_ITSELF: &str = ".";

/// What a project's own credential command is run in: the environment its
/// environment command produced, less the two variables nothing inherits.
///
/// The tools a credential command needs are the ones its project's directory
/// supplies — `op`, `secret-tool`, a helper the flake installs — so running it
/// outside the captured environment would leave the two settings composing
/// only on paper, with the password landing in an environment the command
/// that produced it could not have reached.
///
/// It is `NEVER_INHERITED` that must not travel, and the runner cannot strip
/// it here: it removes those variables from what a child *inherits* and then
/// applies what it is handed, so a value passed in this way would arrive.
/// A credential command is an arbitrary program named by a config, and this
/// is the one call where the environment it might be handed holds the very
/// password it is being asked to produce.
fn lending(captured: &Env) -> Env {
    let mut lent = captured.clone();
    for withheld in NEVER_INHERITED {
        lent.remove(withheld);
    }
    lent
}

/// The variables a project's environment command produces.
///
/// The command runs in the project's own directory, which is what lets it be
/// written relative — `direnv exec .` is the directory `bdi` is asking about
/// rather than one the config repeats.
///
/// It is given neither tracker nor credential of `bdi`'s own, so what comes
/// back is what entering that directory produces rather than what the shell
/// `bdi` was launched from was already carrying.
///
/// The wrapper is run directly rather than through `sh -c`, which is what
/// `credential_command` does, and the difference is the diagnosis a reader
/// gets. Under a shell an absent wrapper is the *shell* exiting 127, whose
/// stderr matches none of the phrase lists in `run.rs` and so arrives as
/// `Unavailable` for `sh` — *"sh exited 127 for a reason bdi cannot place"*
/// on a machine whose only problem is that direnv is not installed. Run
/// directly, the spawn fails and the reader is told which program is missing.
/// A wrapper is a program and its arguments, so it loses nothing.
///
/// A command that cannot be run fails this project rather than falling back
/// to the ambient environment, because a mechanism that silently does nothing
/// is indistinguishable from one that worked. Where direnv falls back for
/// itself — a flake that will not evaluate — it exits 0 and this is not the
/// path taken; an unallowed `.envrc` exits 1 with an empty stdout and this
/// is. Measured on direnv 2.37.1, 2026-09-04.
fn entering(path: &Path, runner: &dyn Runner, command: &Command) -> Result<Env, RunFailure> {
    let mut words = command.words().into_iter().chain(PROBE.split_whitespace());
    let program = words.next().unwrap_or_default();
    let argv: Vec<&str> = words.collect();
    let out = runner.run(program, &argv, Some(path), &Env::new())?;
    Ok(variables(&out))
}

/// The variables in an `env -0` answer, tolerating whatever a project's
/// `.envrc` wrote to stdout before it.
///
/// direnv's own log lines reach stderr, measured, but nothing stops a
/// project's `.envrc` printing to stdout, and only this repository's has had
/// that fixed. Such text arrives ahead of the first variable and would
/// otherwise be read as part of its name, losing it. A variable's name holds
/// no newline, so whatever precedes the last one before the `=` is not part
/// of it.
fn variables(out: &str) -> Env {
    out.split('\0')
        .filter_map(|entry| {
            let (named, value) = entry.split_once('=')?;
            let name = named.rsplit('\n').next().unwrap_or(named);
            (!name.is_empty()).then(|| (name.to_string(), value.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::run::testing::FakeRunner;
    use crate::collect::run::{FailureKind, RealRunner};
    use std::path::PathBuf;

    /// A project's directory, named so that nothing is ever there.
    ///
    /// It has to be absent rather than merely unused, because detection reads
    /// the filesystem: a directory holding an `.envrc`, on a machine holding
    /// a direnv, is entered without being asked, and one of the readings here
    /// is that nothing was run at all. Under a real path these would answer
    /// one way on a maintainer's machine and another in a build sandbox.
    fn project_dir() -> PathBuf {
        PathBuf::from("/nowhere/a-project")
    }

    /// A project entry as the config takes it by default: a path, and
    /// nothing else.
    fn ambient_project() -> Project {
        Project {
            name: "atlas".to_string(),
            path: project_dir(),
            environment_command: None,
            credential_command: None,
            poll: true,
            worktrees: Vec::new(),
        }
    }

    /// The wrapper a direnv setup names. Written relative, because the
    /// command runs in the project's own directory.
    const DIRENV: &str = "direnv exec .";

    /// A project that asked to be read with what entering its directory
    /// produces.
    fn entered_with_direnv() -> Project {
        Project {
            environment_command: Some(Command::Line(DIRENV.to_string())),
            ..ambient_project()
        }
    }

    fn credentialled() -> Env {
        Env::from([(CREDENTIAL_VAR.to_string(), "hunter2".to_string())])
    }

    /// The call that reproduces entering a project's directory, spelled as
    /// the runner makes it: the configured wrapper with `bdi`'s own probe
    /// appended.
    fn entering_the_directory() -> String {
        format!("{DIRENV} {PROBE}")
    }

    /// An `env -0` answer: NUL between variables, and no separator after the
    /// last one that would make an empty final entry meaningful.
    fn exported(variables: &[(&str, &str)]) -> String {
        variables
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("\0")
    }

    /// What direnv is for: a shell that has entered the project's directory
    /// is configured for its tracker, so bdi reproduces entering it rather
    /// than restating what it would have produced.
    #[test]
    fn a_project_asking_for_direnv_is_read_by_entering_its_directory() {
        let runner = FakeRunner::default().with(
            &entering_the_directory(),
            &exported(&[
                ("BEADS_DIR", "/nowhere/a-project/.beads"),
                ("BEADS_DOLT_PASSWORD", "the-projects-own-password"),
            ]),
        );

        let env = tracker_env(&runner, &entered_with_direnv(), None).unwrap();

        assert_eq!(
            env.get("BEADS_DOLT_PASSWORD").map(String::as_str),
            Some("the-projects-own-password"),
            "the credential entering the directory produces did not reach bd"
        );
        assert_eq!(
            env.get("BEADS_DIR").map(String::as_str),
            Some("/nowhere/a-project/.beads"),
            "the tracker entering the directory names did not reach bd"
        );
    }

    /// direnv is asked what entering the directory produces, not what bdi was
    /// already carrying. Handed bdi's own tracker and credential it would
    /// answer with them for every project alike, which is the defect `-C` and
    /// the cleared environment exist to stop.
    #[test]
    fn direnv_is_given_no_tracker_and_no_credential_of_bdis_own() {
        let runner = FakeRunner::default().with(&entering_the_directory(), &exported(&[]));

        tracker_env(
            &runner,
            &entered_with_direnv(),
            Some("the-launching-shells-password"),
        )
        .unwrap();

        assert!(
            runner.call(&entering_the_directory()).env.is_empty(),
            "direnv was handed an environment to reproduce"
        );
    }

    /// A project's `.envrc` may print to stdout, and only this repository's
    /// has been fixed not to. The text lands ahead of the first variable, and
    /// reading it as part of that variable's name loses the variable.
    #[test]
    fn text_a_projects_envrc_wrote_first_does_not_lose_the_variable_behind_it() {
        let noise = "entering the atlas shell\n";
        let runner = FakeRunner::default().with(
            &entering_the_directory(),
            &format!(
                "{noise}{}",
                exported(&[("BEADS_DOLT_PASSWORD", "hunter2"), ("PATH", "/nix/bin")])
            ),
        );

        let env = tracker_env(&runner, &entered_with_direnv(), None).unwrap();

        assert_eq!(env.get(CREDENTIAL_VAR).map(String::as_str), Some("hunter2"));
        assert_eq!(
            env.get("PATH").map(String::as_str),
            Some("/nix/bin"),
            "the first variable was read as part of the text in front of it"
        );
        assert!(
            !env.keys().any(|name| name.contains('\n')),
            "text written before the variables became a variable: {env:?}"
        );
    }

    /// A project whose directory cannot be entered at all is that project's
    /// failure, not a quiet fallback that reads as having worked. Where
    /// direnv does fall back for itself — a flake that will not evaluate — it
    /// exits 0 and this is not the path taken.
    ///
    /// It is its own failure rather than one of bd's, and that is the whole
    /// of what the reader gets from it: no bd ran, so nothing about bd is
    /// true of this project, and a sentence about bd would send them after a
    /// program that was never asked anything.
    #[test]
    fn a_directory_that_cannot_be_entered_fails_the_project_rather_than_falling_back() {
        let runner = FakeRunner::default().failing(
            &entering_the_directory(),
            RunFailure::unstartable("direnv", "No such file or directory"),
        );

        let failure = tracker_env(&runner, &entered_with_direnv(), Some("hunter2")).unwrap_err();

        assert_eq!(failure, OpenFailure::NoEnvironment);
    }

    /// Whichever way the capture fails, and there are two that land on
    /// different names: an absent direnv is `NotInstalled` and one that exists
    /// and refuses is `Unavailable`. The project's failure is the same either
    /// way, because what the reader does about it is the same either way.
    #[test]
    fn every_way_the_capture_can_fail_is_the_same_failure_to_the_project() {
        for kind in every_failure_kind() {
            let runner = FakeRunner::default().failing(
                &entering_the_directory(),
                RunFailure {
                    kind,
                    program: "direnv".to_string(),
                    detail: "direnv did not produce an environment".to_string(),
                },
            );

            let failure = tracker_env(&runner, &entered_with_direnv(), None).unwrap_err();

            assert_eq!(failure, OpenFailure::NoEnvironment, "{kind:?}");
        }
    }

    /// Every kind a run can fail with. The match is what makes it every one:
    /// a kind added to `run.rs` and not to this chain does not compile.
    fn every_failure_kind() -> impl Iterator<Item = FailureKind> {
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

    /// A project with no environment has nothing to be read, so its
    /// credential command is not run: an arbitrary command a config named is
    /// a side effect, and spending one on a read that is not going to happen
    /// buys nothing. The fake panics on a call nobody staged, so a credential
    /// command reached here fails in the runner before the assertion.
    #[test]
    fn a_project_with_no_environment_does_not_run_its_credential_command() {
        let runner = FakeRunner::default().failing(
            &entering_the_directory(),
            RunFailure::unstartable("direnv", "No such file or directory"),
        );
        let project = Project {
            credential_command: Some("op read the/password".to_string()),
            ..entered_with_direnv()
        };

        assert_eq!(
            tracker_env(&runner, &project, None).unwrap_err(),
            OpenFailure::NoEnvironment
        );
    }

    /// Naming both settings composes in both directions: the credential
    /// command runs in the environment the environment command produced, so a
    /// helper that only the project's directory supplies is on its `PATH`.
    ///
    /// Not the password, though, and that is the half worth pinning. The
    /// runner strips `NEVER_INHERITED` from what a child inherits and then
    /// applies what it is handed, so a captured environment passed on whole
    /// would put the tracker's own password into an arbitrary program named
    /// by a config — at the one call whose whole purpose is to produce that
    /// password.
    #[test]
    fn a_credential_command_gets_the_captured_tools_but_never_the_captured_password() {
        let runner = FakeRunner::default()
            .with(
                &entering_the_directory(),
                &exported(&[
                    ("PATH", "/nix/bin"),
                    ("BEADS_DOLT_PASSWORD", "the-projects-own-password"),
                    ("BEADS_DIR", "/nowhere/a-project/.beads"),
                ]),
            )
            .with("sh -c op read the/password", "hunter2\n");
        let project = Project {
            credential_command: Some("op read the/password".to_string()),
            ..entered_with_direnv()
        };

        let env = tracker_env(&runner, &project, None).unwrap();

        let call = runner.call("sh -c op read the/password");
        assert_eq!(
            call.env.get("PATH").map(String::as_str),
            Some("/nix/bin"),
            "the credential command could not reach the tools its own directory installs"
        );
        for withheld in NEVER_INHERITED {
            assert!(
                !call.env.contains_key(withheld),
                "{withheld} reached the credential command"
            );
        }
        assert_eq!(
            env.get(CREDENTIAL_VAR).map(String::as_str),
            Some("hunter2"),
            "the credential command's answer did not win over the captured one"
        );
    }

    /// The escape hatch answers instead of entering the directory, for a
    /// tracker outside direnv's reach.
    #[test]
    fn a_credential_command_answers_instead_of_entering_the_directory() {
        let runner = FakeRunner::default().with("sh -c op read the/password", "hunter2\n");
        let project = Project {
            name: "atlas".to_string(),
            path: project_dir(),
            environment_command: None,
            credential_command: Some("op read the/password".to_string()),
            poll: true,
            worktrees: Vec::new(),
        };

        tracker_env(&runner, &project, None).unwrap();

        assert!(
            !runner
                .calls()
                .iter()
                .any(|call| call.argv.starts_with("direnv ")),
            "the directory was entered as well as the escape hatch being used"
        );
    }

    /// The default: a project entry that says nothing about how it is entered
    /// is read with the environment `bdi` itself runs in, and nothing is run
    /// to find out what entering its directory would have produced. A fake
    /// with no answer staged panics on any call, so a direnv reached for here
    /// fails this test in the runner before the assertion is read.
    ///
    /// Saying nothing is not the whole of the default any more — a directory
    /// holding an `.envrc` is entered on a machine holding a direnv, without
    /// the config saying so. What this row still says is that a directory
    /// implying nothing runs nothing, which is why `project_dir` names one
    /// that is never there.
    #[test]
    fn a_project_that_says_nothing_about_its_environment_is_read_without_running_anything() {
        let runner = FakeRunner::default();

        let env = tracker_env(&runner, &ambient_project(), Some("hunter2")).unwrap();

        assert_eq!(env, credentialled());
        assert_eq!(
            runner.calls(),
            Vec::new(),
            "an ambient project ran a program to find its environment"
        );
    }

    /// A wrapper the machine has not got names *itself* as the missing
    /// program, which is what a reader can act on.
    ///
    /// It has to be a real spawn: what is under test is which of
    /// `RealRunner`'s two failure paths the call takes, and a fake answers
    /// whichever it was told to. Run through `sh -c` this comes back
    /// `Unavailable` for `sh`, because the shell starts, fails to find the
    /// wrapper, and exits 127 with stderr matching none of `run.rs`'s phrase
    /// lists — a machine whose only problem is a missing direnv would be told
    /// `sh` failed for a reason `bdi` could not place.
    ///
    /// Asked of `entering` rather than through `tracker_env`, because the
    /// project's failure is one sentence about the project and names no
    /// program, so the classification is made here and shown nowhere. It is
    /// still what running the wrapper directly buys, and it is what a screen
    /// naming the program would have to read.
    #[test]
    fn a_wrapper_that_is_not_installed_is_named_rather_than_the_shell() {
        let wrapper = Command::Line("no-such-wrapper-anywhere exec .".to_string());

        let failure = entering(Path::new("."), &RealRunner, &wrapper).unwrap_err();

        assert_eq!(failure.kind, FailureKind::NotInstalled);
        assert_eq!(
            failure.program, "no-such-wrapper-anywhere",
            "the wrapper's own absence went on record as the shell's"
        );
    }

    /// The parser reads back what `env` actually writes, rather than what we
    /// believe it writes: a real process, and every variable it exported.
    #[test]
    fn the_variables_read_back_are_the_ones_env_wrote() {
        let out = RealRunner
            .run(
                "env",
                &["-0"],
                None,
                &Env::from([("K".to_string(), "v".to_string())]),
            )
            .expect("env runs");

        let read = variables(&out);

        assert_eq!(read.get("K").map(String::as_str), Some("v"));
        assert_eq!(
            read.len(),
            out.split('\0').filter(|entry| !entry.is_empty()).count(),
            "a variable env wrote was not read back"
        );
    }

    #[test]
    fn a_projects_credential_command_supplies_its_password() {
        let runner = FakeRunner::default().with("sh -c op read the/password", "hunter2\n");
        let project = Project {
            name: "atlas".to_string(),
            path: project_dir(),
            environment_command: None,
            credential_command: Some("op read the/password".to_string()),
            poll: true,
            worktrees: Vec::new(),
        };

        let env = tracker_env(&runner, &project, Some("the-launching-shells-password")).unwrap();

        assert_eq!(
            env,
            credentialled(),
            "the trailing newline is not the password"
        );
        let call = runner.call("sh -c op read the/password");
        assert_eq!(call.cwd.as_deref(), Some(project_dir().as_path()));
        assert!(
            call.env.is_empty(),
            "the credential command gets no credential"
        );
    }

    /// A single-tracker setup configures no credential and reaches its tracker
    /// on the ambient one. It is handed that credential rather than left to
    /// inherit it, because nothing bdi launches inherits it any more.
    #[test]
    fn a_project_with_no_credential_command_is_handed_the_ambient_credential() {
        let runner = FakeRunner::default().with(
            &entering_the_directory(),
            &exported(&[("PATH", "/nix/bin")]),
        );

        let env = tracker_env(&runner, &entered_with_direnv(), Some("hunter2")).unwrap();

        assert_eq!(env.get(CREDENTIAL_VAR).map(String::as_str), Some("hunter2"));
        assert_eq!(
            env.get("PATH").map(String::as_str),
            Some("/nix/bin"),
            "the directory was entered but what it produced did not reach bd"
        );
    }

    /// Nothing to hand on is not an empty password: a tracker that wants one
    /// should refuse the call rather than be told the password is "".
    #[test]
    fn a_project_with_no_credential_command_and_no_ambient_one_is_given_nothing() {
        let runner = FakeRunner::default().with(&entering_the_directory(), &exported(&[]));

        assert_eq!(
            tracker_env(&runner, &entered_with_direnv(), None).unwrap(),
            Env::new()
        );
    }

    #[test]
    fn a_credential_command_that_fails_reaches_the_caller() {
        let runner = FakeRunner::default().failing(
            "sh -c op read the/password",
            RunFailure::not_installed("sh", "op: command not found"),
        );
        let project = Project {
            name: "atlas".to_string(),
            path: project_dir(),
            environment_command: None,
            credential_command: Some("op read the/password".to_string()),
            poll: true,
            worktrees: Vec::new(),
        };

        assert_eq!(
            tracker_env(&runner, &project, None).unwrap_err(),
            OpenFailure::Refused(RunFailure::not_installed("sh", "op: command not found"))
        );
    }
}
