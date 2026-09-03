//! The environment each project's tracker is read in.
//!
//! One capture per project: `bdi`'s own environment, what entering the
//! project's directory produces, or what its escape hatch answers — whichever
//! the project's config chose. Every question asked with it is in `bd`.

use std::path::Path;

use crate::collect::run::{Env, RunFailure, Runner};
use crate::config::{Environment, Project};

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
/// A project configured `environment = "direnv"` is read with what entering
/// its directory produces instead. A shell that has entered it is already
/// configured for its tracker — direnv loads the flake, the bd version, and
/// whatever holds the password — so `bdi` reproduces entering the directory
/// rather than reconstructing what entering it would have produced, and the
/// project entry says nothing about what the secret is called or where it
/// lives. That is what a setup keeping one credential per project in each
/// project's own directory wants.
///
/// Captured once per project rather than by wrapping every call, because
/// `direnv exec` reloads the directory each time it runs. Measured against
/// this repository on 2026-08-31: 1.3 to 2.4 seconds per invocation, where a
/// whole collection of both trackers costs 2.5 to 2.7. It also confines a
/// project whose `.envrc` writes to stdout to this one call, whose parser
/// tolerates it, rather than to every answer bd gives.
///
/// A `credential_command` is the escape hatch for a tracker outside direnv's
/// reach, and answers instead of entering the directory.
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
    match (&project.credential_command, project.environment) {
        (Some(command), _) => {
            let password = runner.run("sh", &["-c", command], Some(&project.path), &Env::new())?;
            env.insert(
                CREDENTIAL_VAR.to_string(),
                password.trim_end_matches(['\r', '\n']).to_string(),
            );
        }
        (None, Environment::Direnv) => env.extend(entering(&project.path, runner)?),
        (None, Environment::Ambient) => {}
    }
    Ok(env)
}

/// The variables entering a directory produces.
///
/// direnv is given neither tracker nor credential of `bdi`'s own, so what
/// comes back is what entering that directory produces rather than what the
/// shell `bdi` was launched from was already carrying.
///
/// A directory direnv cannot enter fails this project rather than falling
/// back to the ambient environment. direnv itself fails open — it exits 0
/// and runs with the ambient environment where an `.envrc` is unallowed or a
/// flake will not evaluate — and a mechanism that silently does nothing is
/// indistinguishable from one that worked. What such a fallback cannot do,
/// because `-C` names the tracker, is read another project's database.
fn entering(path: &Path, runner: &dyn Runner) -> Result<Env, RunFailure> {
    let named = path.to_string_lossy();
    let out = runner.run(
        "direnv",
        &["exec", named.as_ref(), "env", "-0"],
        Some(path),
        &Env::new(),
    )?;
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
            environment: Environment::Ambient,
            credential_command: None,
            poll: true,
            worktrees: Vec::new(),
        }
    }

    /// A project that asked to be read with what entering its directory
    /// produces.
    fn entered_with_direnv() -> Project {
        Project {
            environment: Environment::Direnv,
            ..ambient_project()
        }
    }

    fn credentialled() -> Env {
        Env::from([(CREDENTIAL_VAR.to_string(), "hunter2".to_string())])
    }

    /// The direnv call that reproduces entering a project's directory,
    /// spelled as the runner makes it.
    fn entering_the_directory() -> String {
        format!("direnv exec {} env -0", project_dir().display())
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

    /// direnv fails open: it exits 0 and runs with the ambient environment
    /// where an `.envrc` is unallowed or a flake will not evaluate. So a
    /// project whose directory cannot be entered at all is that project's
    /// failure, not a quiet fallback that reads as having worked.
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

    /// The escape hatch answers instead of entering the directory, for a
    /// tracker outside direnv's reach.
    #[test]
    fn a_credential_command_answers_instead_of_entering_the_directory() {
        let runner = FakeRunner::default().with("sh -c op read the/password", "hunter2\n");
        let project = Project {
            name: "atlas".to_string(),
            path: project_dir(),
            environment: Environment::Ambient,
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
            environment: Environment::Ambient,
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
            environment: Environment::Ambient,
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
