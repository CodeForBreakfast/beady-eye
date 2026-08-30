use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::collect::run::{Env, FailureKind, Runner};

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Config {
    #[serde(default)]
    pub projects: Vec<Project>,
    #[serde(default)]
    pub roots: Roots,
    #[serde(default)]
    pub badges: Vec<Badge>,
    #[serde(default)]
    pub anomalies: Anomalies,
    #[serde(default)]
    pub join: Join,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Project {
    pub name: String,
    pub path: PathBuf,
    /// A command whose stdout is this tracker's password, never the password
    /// itself. Absent when the project needs no credential of its own.
    #[serde(default)]
    pub credential_command: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Roots {
    pub metadata_keys: Vec<String>,
    pub explicit: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Badge {
    pub key: String,
    #[serde(rename = "match")]
    pub match_value: Option<String>,
    pub render: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Anomalies {
    pub stale_claim_days: i64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Join {
    pub pane_key: String,
}

impl Default for Anomalies {
    fn default() -> Self {
        Self {
            stale_claim_days: 30,
        }
    }
}

impl Default for Join {
    fn default() -> Self {
        Self {
            pane_key: "agent_pane".to_string(),
        }
    }
}

impl Config {
    pub fn from_toml(s: &str) -> anyhow::Result<Self> {
        let cfg: Config = toml::from_str(s)?;
        if cfg.projects.is_empty() {
            anyhow::bail!("config names no projects; bdi has nothing to read");
        }
        if cfg.projects.len() > 1 {
            let ambient = cfg.projects_on_the_ambient_credential();
            if !ambient.is_empty() {
                anyhow::bail!(
                    "every project needs a credential_command once the config names more \
                     than one, or one tracker's credential reaches another's; missing on: {}",
                    ambient.join(", ")
                );
            }
        }
        Ok(cfg)
    }

    /// The single project `bdi` reads when no config file names one: the
    /// repository the current directory sits in, on the ambient credential.
    ///
    /// bd and git are asked where their own things are rather than walked for
    /// here, so `BEADS_DIR`, a redirect or a worktree resolves the way it does
    /// for any other command run in the same place.
    pub fn from_the_current_directory(
        runner: &dyn Runner,
        cwd: &Path,
        name_from_the_environment: Option<&str>,
    ) -> anyhow::Result<Self> {
        if let Err(failure) = runner.run("bd", &["where", "--json"], Some(cwd), &Env::new()) {
            // bd that never ran has said nothing about this directory.
            if failure.kind == FailureKind::Exec {
                return Err(failure.into());
            }
            anyhow::bail!("{} is not in anything beads tracks", cwd.display());
        }

        let root = git(runner, cwd, &["rev-parse", "--show-toplevel"])
            .map_or_else(|| cwd.to_path_buf(), PathBuf::from);
        let name = name_from_the_environment
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .or_else(|| {
                git(runner, cwd, &["remote", "get-url", "origin"]).map(|url| repository_name(&url))
            })
            .unwrap_or_else(|| directory_name(&root));

        Ok(Self {
            projects: vec![Project {
                name,
                path: root,
                credential_command: None,
            }],
            roots: Roots::default(),
            badges: Vec::new(),
            anomalies: Anomalies::default(),
            join: Join::default(),
        })
    }

    fn projects_on_the_ambient_credential(&self) -> Vec<&str> {
        self.projects
            .iter()
            .filter(|p| p.credential_command.is_none())
            .map(|p| p.name.as_str())
            .collect()
    }
}

/// One line of git's answer, or nothing where git has none to give: no
/// repository, no remote, or no git at all.
fn git(runner: &dyn Runner, cwd: &Path, args: &[&str]) -> Option<String> {
    let said = runner.run("git", args, Some(cwd), &Env::new()).ok()?;
    let line = said.trim();
    (!line.is_empty()).then(|| line.to_string())
}

/// The repository a remote URL names, in any of the spellings git accepts:
/// `git@host:owner/name.git`, `https://host/owner/name`, `/srv/git/name.git`.
fn repository_name(url: &str) -> String {
    let named = url.trim_end_matches('/');
    let named = named.rsplit(['/', ':']).next().unwrap_or(named);
    named.trim_end_matches(".git").to_string()
}

/// What a directory is called, or its whole path where it is called nothing.
fn directory_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

impl Badge {
    /// Render this badge for a metadata value, or `None` if it does not apply.
    /// `{}` in `render` is replaced by the value.
    pub fn apply(&self, value: &str) -> Option<String> {
        if let Some(expected) = &self.match_value {
            if expected != value {
                return None;
            }
        }
        Some(self.render.replace("{}", value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::run::testing::FakeRunner;
    use crate::collect::run::{FailureKind, RunFailure};
    use std::path::Path;

    const EVERY_SECTION: &str = r#"
[[projects]]
name = "atlas"
path = "/home/user/atlas"
credential_command = "secret-tool lookup tracker atlas"

[[projects]]
name = "beacon"
path = "/home/user/dev/beacon"
credential_command = "cat /home/user/dev/beacon/.beads-password"

[roots]
metadata_keys = ["working_topic", "delivery_pr"]
explicit = ["a-1", "b-1"]

[[badges]]
key    = "delivery_pr"
render = "⇢ {}"

[[badges]]
key    = "blocked_on"
match  = "human"
render = "⏸ waiting"

[anomalies]
stale_claim_days = 7

[join]
pane_key = "herdr_pane"
"#;

    const ONE_PROJECT: &str = r#"
[[projects]]
name = "beacon"
path = "/home/user/dev/beacon"
"#;

    const ONE_CREDENTIALLED_ONE_AMBIENT: &str = r#"
[[projects]]
name = "atlas"
path = "/home/user/atlas"
credential_command = "secret-tool lookup tracker atlas"

[[projects]]
name = "beacon"
path = "/home/user/dev/beacon"
"#;

    const TWO_AMBIENT: &str = r#"
[[projects]]
name = "atlas"
path = "/home/user/atlas"
credential_command = "secret-tool lookup tracker atlas"

[[projects]]
name = "beacon"
path = "/home/user/dev/beacon"

[[projects]]
name = "cinder"
path = "/home/user/dev/cinder"
"#;

    #[test]
    fn parses_every_section() {
        let cfg = Config::from_toml(EVERY_SECTION).expect("parses");

        assert_eq!(
            cfg.projects,
            vec![
                Project {
                    name: "atlas".to_string(),
                    path: PathBuf::from("/home/user/atlas"),
                    credential_command: Some("secret-tool lookup tracker atlas".to_string()),
                },
                Project {
                    name: "beacon".to_string(),
                    path: PathBuf::from("/home/user/dev/beacon"),
                    credential_command: Some(
                        "cat /home/user/dev/beacon/.beads-password".to_string()
                    ),
                },
            ]
        );
        assert_eq!(
            cfg.roots,
            Roots {
                metadata_keys: vec!["working_topic".to_string(), "delivery_pr".to_string()],
                explicit: vec!["a-1".to_string(), "b-1".to_string()],
            }
        );
        assert_eq!(
            cfg.badges,
            vec![
                Badge {
                    key: "delivery_pr".to_string(),
                    match_value: None,
                    render: "⇢ {}".to_string(),
                },
                Badge {
                    key: "blocked_on".to_string(),
                    match_value: Some("human".to_string()),
                    render: "⏸ waiting".to_string(),
                },
            ]
        );
        assert_eq!(cfg.anomalies.stale_claim_days, 7);
        assert_eq!(cfg.join.pane_key, "herdr_pane");
    }

    #[test]
    fn a_config_of_one_project_gets_every_default() {
        let cfg = Config::from_toml(ONE_PROJECT).expect("parses");

        assert_eq!(cfg.projects[0].credential_command, None);
        assert_eq!(cfg.roots, Roots::default());
        assert!(cfg.badges.is_empty());
        assert_eq!(cfg.anomalies.stale_claim_days, 30);
        assert_eq!(cfg.join.pane_key, "agent_pane");
    }

    #[test]
    fn a_lone_project_needs_no_credential_command() {
        let cfg = Config::from_toml(ONE_PROJECT).expect("parses");

        assert_eq!(cfg.projects[0].credential_command, None);
    }

    #[test]
    fn a_project_without_a_credential_alongside_one_with_is_rejected() {
        let err = Config::from_toml(ONE_CREDENTIALLED_ONE_AMBIENT)
            .unwrap_err()
            .to_string();

        assert!(err.contains("beacon"), "got: {err}");
        assert!(err.contains("credential_command"), "got: {err}");
    }

    #[test]
    fn rejecting_a_project_does_not_repeat_another_projects_credential_command() {
        let err = Config::from_toml(ONE_CREDENTIALLED_ONE_AMBIENT)
            .unwrap_err()
            .to_string();

        assert!(!err.contains("secret-tool"), "got: {err}");
    }

    #[test]
    fn every_project_without_a_credential_is_named() {
        let err = Config::from_toml(TWO_AMBIENT).unwrap_err().to_string();

        assert!(err.contains("beacon"), "got: {err}");
        assert!(err.contains("cinder"), "got: {err}");
    }

    #[test]
    fn config_without_projects_is_rejected() {
        let err = Config::from_toml("[roots]\nmetadata_keys = []\n").unwrap_err();
        assert!(err.to_string().contains("no projects"), "got: {err}");
    }

    #[test]
    fn badge_without_match_renders_any_value() {
        let b = Badge {
            key: "delivery_pr".to_string(),
            match_value: None,
            render: "⇢ {}".to_string(),
        };
        assert_eq!(b.apply("owner/repo#7"), Some("⇢ owner/repo#7".to_string()));
    }

    #[test]
    fn badge_with_match_is_selective() {
        let b = Badge {
            key: "blocked_on".to_string(),
            match_value: Some("human".to_string()),
            render: "⏸ waiting".to_string(),
        };
        assert_eq!(b.apply("human"), Some("⏸ waiting".to_string()));
        assert_eq!(b.apply("dependency"), None);
    }

    /// A repository beads tracks, as bd and git answer for it. The remote and
    /// the directory disagree deliberately, so a test can tell which was read.
    fn a_tracked_repository() -> FakeRunner {
        FakeRunner::default()
            .with("bd where --json", r#"{"path":"/srv/work/orbital/.beads"}"#)
            .with("git rev-parse --show-toplevel", "/srv/work/orbital\n")
            .with(
                "git remote get-url origin",
                "git@github.com:pilot/ground-station.git\n",
            )
    }

    fn no_such_repository() -> RunFailure {
        RunFailure {
            kind: FailureKind::Unavailable,
            program: "git".to_string(),
            detail: "git exited 128 for a reason bdi cannot place".to_string(),
        }
    }

    #[test]
    fn the_repository_the_directory_sits_in_becomes_the_one_project() {
        let cfg = Config::from_the_current_directory(
            &a_tracked_repository(),
            Path::new("/srv/work/orbital/src"),
            None,
        )
        .expect("the repository is a project");

        assert_eq!(
            cfg.projects,
            vec![Project {
                name: "ground-station".to_string(),
                path: PathBuf::from("/srv/work/orbital"),
                credential_command: None,
            }]
        );
    }

    #[test]
    fn a_synthesised_project_gets_every_other_default() {
        let cfg = Config::from_the_current_directory(
            &a_tracked_repository(),
            Path::new("/srv/work/orbital"),
            None,
        )
        .expect("the repository is a project");

        assert_eq!(cfg.roots, Roots::default());
        assert!(cfg.badges.is_empty());
        assert_eq!(cfg.anomalies.stale_claim_days, 30);
        assert_eq!(cfg.join.pane_key, "agent_pane");
    }

    #[test]
    fn the_environment_names_the_project_ahead_of_git() {
        let cfg = Config::from_the_current_directory(
            &a_tracked_repository(),
            Path::new("/srv/work/orbital"),
            Some("atlas"),
        )
        .expect("the repository is a project");

        assert_eq!(cfg.projects[0].name, "atlas");
    }

    #[test]
    fn an_empty_name_in_the_environment_is_no_name_at_all() {
        let cfg = Config::from_the_current_directory(
            &a_tracked_repository(),
            Path::new("/srv/work/orbital"),
            Some(""),
        )
        .expect("the repository is a project");

        assert_eq!(cfg.projects[0].name, "ground-station");
    }

    #[test]
    fn a_repository_with_no_remote_is_named_by_its_directory() {
        let runner = FakeRunner::default()
            .with("bd where --json", r#"{"path":"/srv/work/orbital/.beads"}"#)
            .with("git rev-parse --show-toplevel", "/srv/work/orbital\n")
            .failing("git remote get-url origin", no_such_repository());

        let cfg = Config::from_the_current_directory(&runner, Path::new("/srv/work/orbital"), None)
            .expect("the repository is a project");

        assert_eq!(cfg.projects[0].name, "orbital");
    }

    #[test]
    fn every_spelling_of_a_remote_names_the_same_project() {
        for url in [
            "git@github.com:pilot/ground-station.git",
            "https://github.com/pilot/ground-station.git",
            "https://github.com/pilot/ground-station",
            "ssh://git@host/~pilot/ground-station.git/",
            "/srv/git/ground-station.git",
        ] {
            let runner = FakeRunner::default()
                .with("bd where --json", r#"{"path":"/srv/work/orbital/.beads"}"#)
                .with("git rev-parse --show-toplevel", "/srv/work/orbital\n")
                .with("git remote get-url origin", &format!("{url}\n"));

            let cfg =
                Config::from_the_current_directory(&runner, Path::new("/srv/work/orbital"), None)
                    .expect("the repository is a project");

            assert_eq!(cfg.projects[0].name, "ground-station", "from {url}");
        }
    }

    /// `BEADS_DIR` reaches a tracker from anywhere, so a directory in no
    /// repository is still worth reading; it is just its own project.
    #[test]
    fn a_tracker_outside_any_repository_is_read_from_where_bdi_was_run() {
        let runner = FakeRunner::default()
            .with("bd where --json", r#"{"path":"/srv/beads/.beads"}"#)
            .failing("git rev-parse --show-toplevel", no_such_repository())
            .failing("git remote get-url origin", no_such_repository());

        let cfg = Config::from_the_current_directory(&runner, Path::new("/srv/loose"), None)
            .expect("the directory is a project");

        assert_eq!(
            cfg.projects,
            vec![Project {
                name: "loose".to_string(),
                path: PathBuf::from("/srv/loose"),
                credential_command: None,
            }]
        );
    }

    #[test]
    fn a_directory_beads_does_not_track_is_reported() {
        let runner = FakeRunner::default().failing(
            "bd where --json",
            RunFailure {
                kind: FailureKind::Unavailable,
                program: "bd".to_string(),
                detail: "bd exited 1 for a reason bdi cannot place".to_string(),
            },
        );

        let err = Config::from_the_current_directory(&runner, Path::new("/srv/loose"), None)
            .unwrap_err()
            .to_string();

        assert!(err.contains("/srv/loose"), "got: {err}");
        assert!(err.contains("beads"), "got: {err}");
    }

    /// bd that never ran has said nothing about this directory, and telling
    /// someone to move is the wrong answer to a missing binary.
    #[test]
    fn a_bd_that_cannot_run_says_so_rather_than_blaming_the_directory() {
        let runner = FakeRunner::default().failing(
            "bd where --json",
            RunFailure::exec("bd", "No such file or directory (os error 2)"),
        );

        let err = Config::from_the_current_directory(&runner, Path::new("/srv/loose"), None)
            .unwrap_err()
            .to_string();

        assert!(err.contains("bd could not be run"), "got: {err}");
        assert!(!err.contains("/srv/loose"), "got: {err}");
    }

    #[test]
    fn the_tracker_is_probed_where_bdi_was_run() {
        let runner = a_tracked_repository();

        Config::from_the_current_directory(&runner, Path::new("/srv/work/orbital/src"), None)
            .expect("the repository is a project");

        assert_eq!(
            runner.call("bd where --json").cwd,
            Some(PathBuf::from("/srv/work/orbital/src"))
        );
    }
}
