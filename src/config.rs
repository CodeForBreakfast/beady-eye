use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

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
    /// Where this project is worked: the place it names, in each working
    /// tree git lists for its repository. Measured rather than configured, so
    /// nothing written by hand can outrank what git says.
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
    ///
    /// The default is roughly three times what a collection costs, so a
    /// `bdi` nothing reports to spends about a third of its time collecting:
    /// dozens of queries against each tracker's server, for as long as it is
    /// open, per instance running. Measured against one Dolt-backed tracker
    /// of a hundred open beads, a whole collection took 9.4 to 11.2 seconds,
    /// of which 5.2 to 6.3 was the one `bd dep tree` over the largest tree.
    /// The cost follows the size of the biggest tree rather than the number
    /// of them, so a config drawing two efforts that size collects most of
    /// the time and wants a longer interval than this one.
    ///
    /// Setting it below a collection is allowed and is bounded. Collections
    /// never overlap: one runs, at most one waits behind it, and every
    /// interval that passes meanwhile collapses into that one. So an interval
    /// shorter than a collection buys back-to-back collections with no idle
    /// gap — one per collection, never one per interval — and a view as fresh
    /// as the collection allows rather than as the interval promised.
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
}
