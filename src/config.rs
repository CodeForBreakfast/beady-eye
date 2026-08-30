use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

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
    #[serde(default)]
    pub tui: Tui,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Project {
    pub name: String,
    pub path: PathBuf,
    /// A command whose stdout is this tracker's password, never the password
    /// itself. Absent when the project needs no credential of its own.
    #[serde(default)]
    pub credential_command: Option<String>,
    /// Every working tree of this project's repository, as git listed them:
    /// the main checkout and each linked worktree. Discovered rather than
    /// configured, so nothing written by hand can outrank what git says.
    #[serde(skip)]
    pub worktrees: Vec<PathBuf>,
}

impl Project {
    /// How deeply this project holds a directory, or nothing where it holds
    /// it at all: the depth of the deepest working tree the directory sits
    /// under, so a project inside another wins the paths they share.
    pub fn holds(&self, path: &Path) -> Option<usize> {
        std::iter::once(&self.path)
            .chain(&self.worktrees)
            .filter(|tree| path.starts_with(tree))
            .map(|tree| tree.components().count())
            .max()
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Roots {
    pub metadata_keys: Vec<String>,
    /// The roots named outright, under the project whose tracker holds each.
    /// Bead prefixes are per-tracker and uncoordinated, so an id on its own
    /// names nothing bdi can go and read.
    pub explicit: BTreeMap<String, Vec<String>>,
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

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Tui {
    /// How long the fallback timer waits between collections, for the
    /// projects nothing else reports changes for. A collection is dozens of
    /// remote round trips per project, so this is measured in seconds.
    pub refresh_seconds: u64,
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

impl Default for Tui {
    fn default() -> Self {
        Self {
            refresh_seconds: 30,
        }
    }
}

impl Tui {
    pub fn refresh(&self) -> Duration {
        Duration::from_secs(self.refresh_seconds)
    }
}

impl Config {
    pub fn from_toml(s: &str) -> anyhow::Result<Self> {
        let cfg: Config = toml::from_str(s)?;
        if cfg.projects.is_empty() {
            anyhow::bail!("config names no projects; bdi has nothing to read");
        }
        let repeated = cfg.names_borne_by_more_than_one_project();
        if !repeated.is_empty() {
            anyhow::bail!(
                "a project's name is how bdi tells its beads from another tracker's, so \
                 two projects cannot answer to one; repeated: {}",
                repeated.join(", ")
            );
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
        for (named, ids) in &cfg.roots.explicit {
            if !cfg.is_configured(named) {
                anyhow::bail!(
                    "[roots.explicit] gives {} to {named}, which is no project of this \
                     config; bdi is reading {}",
                    ids.join(", "),
                    names_of(&cfg.projects).join(", ")
                );
            }
        }
        Ok(cfg)
    }

    /// Roots named on the command line join those named in config: discovery
    /// rule 3 has two spellings and one meaning. `<project>:<bead-id>` says
    /// whose tracker holds the bead; a bare id can only mean the one project
    /// there is, so the terse form survives exactly as far as it is
    /// unambiguous.
    pub fn with_roots_named_on_the_command_line(
        mut self,
        beads: &[String],
    ) -> anyhow::Result<Self> {
        for named in beads {
            let (project, id) = self.placed(named)?;
            self.roots.explicit.entry(project).or_default().push(id);
        }
        Ok(self)
    }

    /// The project and bead a command-line root names, or why it names
    /// neither.
    fn placed(&self, named: &str) -> anyhow::Result<(String, String)> {
        let Some((project, id)) = named.split_once(':') else {
            return match self.projects.as_slice() {
                [only] => Ok((only.name.clone(), named.to_string())),
                several => anyhow::bail!(
                    "{named} names no project, and bdi is reading {}; write it as \
                     <project>:{named}",
                    names_of(several).join(", ")
                ),
            };
        };
        if project.is_empty() || id.is_empty() {
            anyhow::bail!("{named} is not <project>:<bead-id>");
        }
        if !self.is_configured(project) {
            anyhow::bail!(
                "{named} gives {id} to {project}, which is no project of this config; \
                 bdi is reading {}",
                names_of(&self.projects).join(", ")
            );
        }
        Ok((project.to_string(), id.to_string()))
    }

    fn is_configured(&self, name: &str) -> bool {
        self.projects.iter().any(|p| p.name == name)
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

        let repository = git(runner, cwd, &["rev-parse", "--show-toplevel"]).map(PathBuf::from);
        // A directory in no repository has no worktrees to list, and asking
        // git for them is only a second way to hear that.
        let worktrees = match &repository {
            Some(_) => worktrees_of(runner, cwd),
            None => Vec::new(),
        };
        let root = repository.unwrap_or_else(|| cwd.to_path_buf());
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
                worktrees,
            }],
            roots: Roots::default(),
            badges: Vec::new(),
            anomalies: Anomalies::default(),
            join: Join::default(),
            tui: Tui::default(),
        })
    }

    fn names_borne_by_more_than_one_project(&self) -> Vec<&str> {
        let mut seen = BTreeSet::new();
        let mut repeated = BTreeSet::new();
        for name in self.projects.iter().map(|p| p.name.as_str()) {
            if !seen.insert(name) {
                repeated.insert(name);
            }
        }
        repeated.into_iter().collect()
    }

    fn projects_on_the_ambient_credential(&self) -> Vec<&str> {
        self.projects
            .iter()
            .filter(|p| p.credential_command.is_none())
            .map(|p| p.name.as_str())
            .collect()
    }
}

fn names_of(projects: &[Project]) -> Vec<&str> {
    projects.iter().map(|p| p.name.as_str()).collect()
}

/// Every working tree of the repository the directory sits in, as
/// `git worktree list` reports them, and empty where git reports none.
///
/// A seat that works in its own worktree leaves its siblings' panes in
/// directories under neither each other nor the checkout, so a project's
/// territory is all of its working trees rather than only the one bdi was
/// run from. Each porcelain record opens with the directory and continues
/// with the commit and the branch, which name no directory at all.
fn worktrees_of(runner: &dyn Runner, cwd: &Path) -> Vec<PathBuf> {
    let Some(listed) = git(runner, cwd, &["worktree", "list", "--porcelain"]) else {
        return Vec::new();
    };
    listed
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .map(PathBuf::from)
        .collect()
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
    use crate::collect::run::{FailureKind, RealRunner, RunFailure};
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

[roots.explicit]
atlas  = ["a-1", "a-9"]
beacon = ["b-1"]

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

[tui]
refresh_seconds = 5
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
                    worktrees: Vec::new(),
                },
                Project {
                    name: "beacon".to_string(),
                    path: PathBuf::from("/home/user/dev/beacon"),
                    credential_command: Some(
                        "cat /home/user/dev/beacon/.beads-password".to_string()
                    ),
                    worktrees: Vec::new(),
                },
            ]
        );
        assert_eq!(
            cfg.roots,
            Roots {
                metadata_keys: vec!["working_topic".to_string(), "delivery_pr".to_string()],
                explicit: BTreeMap::from([
                    (
                        "atlas".to_string(),
                        vec!["a-1".to_string(), "a-9".to_string()]
                    ),
                    ("beacon".to_string(), vec!["b-1".to_string()]),
                ]),
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
        assert_eq!(cfg.tui.refresh_seconds, 5);
    }

    #[test]
    fn a_config_of_one_project_gets_every_default() {
        let cfg = Config::from_toml(ONE_PROJECT).expect("parses");

        assert_eq!(cfg.projects[0].credential_command, None);
        assert_eq!(cfg.roots, Roots::default());
        assert!(cfg.badges.is_empty());
        assert_eq!(cfg.anomalies.stale_claim_days, 30);
        assert_eq!(cfg.join.pane_key, "agent_pane");
        assert_eq!(cfg.tui.refresh_seconds, 30);
    }

    /// The interval is written in seconds and read as a duration; nothing
    /// downstream should be doing that arithmetic.
    #[test]
    fn the_refresh_interval_is_read_as_a_duration() {
        let cfg = Config::from_toml(EVERY_SECTION).expect("parses");

        assert_eq!(cfg.tui.refresh(), Duration::from_secs(5));
        assert_eq!(Tui::default().refresh(), Duration::from_secs(30));
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

    const TWO_PROJECTS: &str = r#"
[[projects]]
name = "atlas"
path = "/home/user/atlas"
credential_command = "secret-tool lookup tracker atlas"

[[projects]]
name = "beacon"
path = "/home/user/dev/beacon"
credential_command = "cat /home/user/dev/beacon/.beads-password"
"#;

    const ROOT_IN_NO_CONFIGURED_PROJECT: &str = r#"
[[projects]]
name = "atlas"
path = "/home/user/atlas"
credential_command = "secret-tool lookup tracker atlas"

[[projects]]
name = "beacon"
path = "/home/user/dev/beacon"
credential_command = "cat /home/user/dev/beacon/.beads-password"

[roots.explicit]
cinder = ["c-1"]
"#;

    #[test]
    fn an_explicit_root_under_a_project_the_config_does_not_name_is_rejected() {
        let err = Config::from_toml(ROOT_IN_NO_CONFIGURED_PROJECT)
            .unwrap_err()
            .to_string();

        assert!(err.contains("cinder"), "got: {err}");
        assert!(err.contains("atlas"), "got: {err}");
        assert!(err.contains("beacon"), "got: {err}");
    }

    const ONE_NAME_ON_TWO_PROJECTS: &str = r#"
[[projects]]
name = "atlas"
path = "/home/user/atlas"
credential_command = "secret-tool lookup tracker atlas"

[[projects]]
name = "atlas"
path = "/home/user/dev/atlas-fork"
credential_command = "cat /home/user/dev/atlas-fork/.beads-password"
"#;

    #[test]
    fn two_projects_of_one_name_are_rejected() {
        let err = Config::from_toml(ONE_NAME_ON_TWO_PROJECTS)
            .unwrap_err()
            .to_string();

        assert!(err.contains("atlas"), "got: {err}");
    }

    const ONE_NAME_ON_TWO_AMBIENT_PROJECTS: &str = r#"
[[projects]]
name = "atlas"
path = "/home/user/atlas"

[[projects]]
name = "atlas"
path = "/home/user/dev/atlas-fork"
"#;

    /// Both guards have something to say about this config, and only one of
    /// them says the thing that is actually wrong with it.
    #[test]
    fn a_repeated_name_is_reported_before_a_missing_credential() {
        let err = Config::from_toml(ONE_NAME_ON_TWO_AMBIENT_PROJECTS)
            .unwrap_err()
            .to_string();

        assert!(err.contains("atlas"), "got: {err}");
        assert!(!err.contains("credential_command"), "got: {err}");
    }

    fn two_projects() -> Config {
        Config::from_toml(TWO_PROJECTS).expect("the config parses")
    }

    #[test]
    fn a_qualified_root_from_the_command_line_goes_to_the_project_it_names() {
        let cfg = two_projects()
            .with_roots_named_on_the_command_line(&["beacon:b-7".to_string()])
            .expect("beacon is configured");

        assert_eq!(
            cfg.roots.explicit,
            BTreeMap::from([("beacon".to_string(), vec!["b-7".to_string()])])
        );
    }

    #[test]
    fn a_root_from_the_command_line_joins_those_the_config_names() {
        let cfg = Config::from_toml(EVERY_SECTION)
            .expect("the config parses")
            .with_roots_named_on_the_command_line(&["atlas:a-3".to_string()])
            .expect("atlas is configured");

        assert_eq!(
            cfg.roots.explicit["atlas"],
            ["a-1", "a-9", "a-3"],
            "the command line appends rather than replacing"
        );
    }

    #[test]
    fn a_bare_root_belongs_to_the_only_project_there_is() {
        let cfg = Config::from_toml(ONE_PROJECT)
            .expect("the config parses")
            .with_roots_named_on_the_command_line(&["b-7".to_string()])
            .expect("there is only one project it can mean");

        assert_eq!(
            cfg.roots.explicit,
            BTreeMap::from([("beacon".to_string(), vec!["b-7".to_string()])])
        );
    }

    #[test]
    fn a_bare_root_with_several_projects_configured_is_rejected() {
        let err = two_projects()
            .with_roots_named_on_the_command_line(&["b-7".to_string()])
            .unwrap_err()
            .to_string();

        assert!(err.contains("b-7"), "got: {err}");
        assert!(err.contains("atlas"), "got: {err}");
        assert!(err.contains("beacon"), "got: {err}");
    }

    #[test]
    fn a_root_from_the_command_line_naming_no_configured_project_is_rejected() {
        let err = two_projects()
            .with_roots_named_on_the_command_line(&["cinder:c-1".to_string()])
            .unwrap_err()
            .to_string();

        assert!(err.contains("cinder"), "got: {err}");
        assert!(err.contains("beacon"), "got: {err}");
    }

    #[test]
    fn a_root_that_is_all_colon_and_no_bead_is_rejected() {
        for named in ["atlas:", ":a-1", ":"] {
            let err = two_projects()
                .with_roots_named_on_the_command_line(&[named.to_string()])
                .unwrap_err()
                .to_string();

            assert!(err.contains(named), "got: {err}");
        }
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
            .with("git worktree list --porcelain", ONE_CHECKOUT)
    }

    /// `git worktree list --porcelain` for a repository nobody has added a
    /// worktree to: the checkout itself, and nothing else.
    const ONE_CHECKOUT: &str = "\
worktree /srv/work/orbital
HEAD 4d3c1f0e9b8a7c6d5e4f3a2b1c0d9e8f7a6b5c4d
branch refs/heads/main
";

    /// The same, for a repository worked in the way this one is: a checkout
    /// and a worktree per seat, each somewhere else entirely.
    const A_WORKTREE_PER_SEAT: &str = "\
worktree /srv/work/orbital
HEAD 4d3c1f0e9b8a7c6d5e4f3a2b1c0d9e8f7a6b5c4d
branch refs/heads/main

worktree /tmp/seat-a/wt
HEAD 1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b
detached

worktree /tmp/seat-b/wt
HEAD 9f8e7d6c5b4a39281706f5e4d3c2b1a09f8e7d6c
detached
";

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
                worktrees: vec![PathBuf::from("/srv/work/orbital")],
            }]
        );
    }

    /// The bug this fixes: a seat works in its own worktree, its siblings
    /// work in theirs, and the panes are in none of the directories bdi was
    /// run from. All of them are the project.
    #[test]
    fn every_worktree_of_the_repository_belongs_to_the_project() {
        let runner =
            a_tracked_repository().with("git worktree list --porcelain", A_WORKTREE_PER_SEAT);

        let cfg = Config::from_the_current_directory(&runner, Path::new("/tmp/seat-a/wt"), None)
            .expect("the repository is a project");

        assert_eq!(
            cfg.projects[0].worktrees,
            vec![
                PathBuf::from("/srv/work/orbital"),
                PathBuf::from("/tmp/seat-a/wt"),
                PathBuf::from("/tmp/seat-b/wt"),
            ]
        );
    }

    /// Degrade, never disappear: git that answers nothing leaves the project
    /// holding the one directory it was found in.
    #[test]
    fn a_repository_git_lists_no_worktrees_for_still_holds_its_own_path() {
        let runner = FakeRunner::default()
            .with("bd where --json", r#"{"path":"/srv/work/orbital/.beads"}"#)
            .with("git rev-parse --show-toplevel", "/srv/work/orbital\n")
            .with(
                "git remote get-url origin",
                "git@github.com:pilot/ground-station.git\n",
            )
            .failing("git worktree list --porcelain", no_such_repository());

        let cfg = Config::from_the_current_directory(&runner, Path::new("/srv/work/orbital"), None)
            .expect("the repository is a project");

        assert!(cfg.projects[0].worktrees.is_empty());
        assert!(
            cfg.projects[0]
                .holds(Path::new("/srv/work/orbital/src"))
                .is_some(),
            "a project git listed no worktrees for holds nothing at all"
        );
    }

    /// A porcelain record is more than its first line, and only the first
    /// line names a directory.
    #[test]
    fn only_the_worktree_lines_of_the_listing_are_directories() {
        let runner =
            a_tracked_repository().with("git worktree list --porcelain", A_WORKTREE_PER_SEAT);

        let cfg = Config::from_the_current_directory(&runner, Path::new("/tmp/seat-a/wt"), None)
            .expect("the repository is a project");

        assert!(
            cfg.projects[0]
                .worktrees
                .iter()
                .all(|w| w.starts_with("/srv") || w.starts_with("/tmp")),
            "a HEAD or a branch was read as a directory: {:?}",
            cfg.projects[0].worktrees
        );
    }

    /// The working trees a project occupies are git's answer about a
    /// repository, so a config file saying otherwise says nothing.
    #[test]
    fn a_config_cannot_write_the_worktrees_a_project_occupies() {
        let cfg = Config::from_toml(
            r#"
[[projects]]
name = "beacon"
path = "/home/user/dev/beacon"
worktrees = ["/home/user/anywhere-at-all"]
"#,
        )
        .expect("parses");

        assert!(cfg.projects[0].worktrees.is_empty());
        assert_eq!(
            cfg.projects[0].holds(Path::new("/home/user/anywhere-at-all")),
            None,
            "a hand-written directory reached the project anyway"
        );
    }

    /// A directory of our own to build a repository in, outside anything
    /// this checkout tracks.
    fn a_scratch_directory(named: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("bdi-{named}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("the directory is ours to make");
        std::fs::canonicalize(&path).expect("the directory we just made resolves")
    }

    /// git run with an identity and a default branch of our own, so the test
    /// says the same thing on a machine whose git is configured differently
    /// and on one where it is not configured at all.
    fn git_in(cwd: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=bdi tests",
                "-c",
                "user.email=tests@beady-eye.invalid",
                "-c",
                "init.defaultBranch=main",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?} in {}: {}",
            cwd.display(),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// The parse is worth only what git actually prints, so this one builds a
    /// repository and a linked worktree and reads git's own answer through
    /// the runner that really runs it.
    ///
    /// It reads from the *linked* worktree, because that is the seat that saw
    /// nothing: a worktree that cannot name the checkout it came from staffs
    /// no row in it.
    #[test]
    fn a_worktree_lists_the_checkout_it_was_added_from_and_itself() {
        let scratch = a_scratch_directory("worktree-listing");
        let checkout = scratch.join("checkout");
        std::fs::create_dir_all(&checkout).expect("the directory is ours to make");
        git_in(&checkout, &["init"]);
        git_in(&checkout, &["commit", "--allow-empty", "-m", "root"]);

        let linked = scratch.join("seat/wt");
        git_in(
            &checkout,
            &["worktree", "add", "--detach", &linked.display().to_string()],
        );

        assert_eq!(
            worktrees_of(&RealRunner, &linked),
            vec![checkout, linked],
            "git did not list both working trees the way the parse expects"
        );

        std::fs::remove_dir_all(&scratch).expect("the directory is ours to remove");
    }

    /// The listing is asked for where bdi was run, because a worktree only
    /// knows its siblings from inside the repository.
    #[test]
    fn the_worktrees_are_listed_from_where_bdi_was_run() {
        let runner = a_tracked_repository();

        Config::from_the_current_directory(&runner, Path::new("/srv/work/orbital/src"), None)
            .expect("the repository is a project");

        assert_eq!(
            runner.call("git worktree list --porcelain").cwd,
            Some(PathBuf::from("/srv/work/orbital/src"))
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
        assert_eq!(cfg.tui, Tui::default());
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
            .with("git worktree list --porcelain", ONE_CHECKOUT)
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
                .with("git worktree list --porcelain", ONE_CHECKOUT)
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
                worktrees: Vec::new(),
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
