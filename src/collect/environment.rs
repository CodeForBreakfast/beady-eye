//! The environment each project's tracker is read in.
//!
//! One capture per project: `bdi`'s own environment, or what the command the
//! project's config names produces, with its credential command replacing the
//! password in either. Every question asked with it is in `bd`.

use std::path::Path;

use crate::collect::run::{Env, RunFailure, Runner};
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
/// By default it is `bdi`'s own, and nothing is run to find it: a machine
/// with bd and nothing else reads its tracker, and `-C` naming the tracker
/// outright is what makes that safe, because a credential belonging to
/// another tracker can only fail to authenticate against the right database,
/// never open the wrong one.
///
/// A project naming an `environment_command` is read with what that command
/// produces instead. A shell that has entered the project's directory is
/// already configured for its tracker — the mechanism loads the flake, the bd
/// version, and whatever holds the password — so `bdi` reproduces entering the
/// directory rather than reconstructing what entering it would have produced,
/// and the project entry says nothing about what the secret is called or where
/// it lives.
///
/// Which mechanism does that is the config's to name and not `bdi`'s to know.
/// direnv, nix and mise all run a command in an environment, so all three are
/// reached by naming them and by no code here.
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
pub fn tracker_env(
    runner: &dyn Runner,
    project: &Project,
    ambient: Option<&str>,
) -> Result<Env, RunFailure> {
    let mut env = ambient.map_or_else(Env::new, |password| {
        Env::from([(CREDENTIAL_VAR.to_string(), password.to_string())])
    });
    if let Some(command) = &project.environment_command {
        env.extend(entering(&project.path, runner, command)?);
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

    fn project_dir() -> PathBuf {
        PathBuf::from("/tmp/proj")
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
    /// appended, through `sh`.
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
                ("BEADS_DIR", "/tmp/proj/.beads"),
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
            Some("/tmp/proj/.beads"),
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
    #[test]
    fn a_directory_that_cannot_be_entered_fails_the_project_rather_than_falling_back() {
        let runner = FakeRunner::default().failing(
            &entering_the_directory(),
            RunFailure::unstartable("direnv", "No such file or directory"),
        );

        let failure = tracker_env(&runner, &entered_with_direnv(), Some("hunter2")).unwrap_err();

        assert_eq!(failure.kind, FailureKind::Unstartable);
        assert_eq!(failure.program, "direnv");
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
                    ("BEADS_DIR", "/tmp/proj/.beads"),
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
    #[test]
    fn a_wrapper_that_is_not_installed_is_named_rather_than_the_shell() {
        let project = Project {
            environment_command: Some(Command::Line("no-such-wrapper-anywhere exec .".to_string())),
            path: PathBuf::from("."),
            ..ambient_project()
        };

        let failure = tracker_env(&RealRunner, &project, None).unwrap_err();

        assert_eq!(failure.kind, FailureKind::NotInstalled);
        assert_eq!(
            failure.program, "no-such-wrapper-anywhere",
            "the reader was pointed at the wrong program to install"
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
            tracker_env(&runner, &project, None).unwrap_err().kind,
            FailureKind::NotInstalled
        );
    }
}
