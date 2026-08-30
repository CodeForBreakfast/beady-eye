use std::path::PathBuf;

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

    fn projects_on_the_ambient_credential(&self) -> Vec<&str> {
        self.projects
            .iter()
            .filter(|p| p.credential_command.is_none())
            .map(|p| p.name.as_str())
            .collect()
    }
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
}
