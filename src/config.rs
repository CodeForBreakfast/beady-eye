//! What a setup tells `bdi`: the shape of the config file, what each setting
//! means, and what `bdi` refuses to read.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::TimeDelta;

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
    /// Which of `projects` this run reads, and what chose them. The rest stay
    /// here rather than being dropped: a pane is placed by which configured
    /// project holds its directory, whether or not that project is read.
    #[serde(skip)]
    pub scope: Scope,
}

/// The projects a run reads, out of every one the config names, and what
/// decided it. A function of the config, the directory `bdi` was started in
/// and the command line — never a value the config file can carry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Scope {
    /// Every configured project: nothing asked for fewer, and no project
    /// holds the directory `bdi` was started in.
    #[default]
    Everything,
    /// The projects `--project` named. The reader typed them, so the screen
    /// says nothing about the ones left out.
    Asked(Vec<String>),
    /// The project holding the directory `bdi` was started in, and any the
    /// roots named on the command line widened the read set to. The reader
    /// did not type this one, so the screen says the directory chose.
    Directory {
        project: String,
        widened: Vec<String>,
    },
}

impl Scope {
    pub fn reads(&self, name: &str) -> bool {
        match self {
            Scope::Everything => true,
            Scope::Asked(named) => named.iter().any(|n| n == name),
            Scope::Directory { project, widened } => {
                project == name || widened.iter().any(|n| n == name)
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Project {
    pub name: String,
    pub path: PathBuf,
    /// The environment this project's tracker is read in.
    #[serde(default)]
    pub environment: Environment,
    /// A command whose stdout is this tracker's password, never the password
    /// itself. The escape hatch for a tracker outside direnv's reach: it
    /// answers in the ambient environment, and a project asking for direnv
    /// as well is refused, because it is entered one way.
    #[serde(default)]
    pub credential_command: Option<String>,
    /// Whether this project asks for itself every interval, or leaves saying
    /// its work has moved to whatever reports for it on the inbound channel.
    ///
    /// Per project because a producer is per tracker: a consumer filtered to
    /// one project's database covers that project and no other, and a setup
    /// that has deployed one for some of its trackers should not have to poll
    /// all of them or none.
    ///
    /// Off is a claim, not a saving. It says something else reports this
    /// project's changes, so a producer that dies takes the project's
    /// freshness with it and nothing here quietly covers for that — an
    /// automatic fallback would hide the very failure the operator needs to
    /// see. `bdi` polls until told otherwise, which is why this defaults on.
    #[serde(default = "polls")]
    pub poll: bool,
    /// Where this project is worked: the place it names, in each working
    /// tree git lists for its repository. Measured rather than configured, so
    /// nothing written by hand can outrank what git says.
    #[serde(skip)]
    pub worktrees: Vec<PathBuf>,
}

/// A project says nothing about polling until it says it does not.
fn polls() -> bool {
    true
}

/// How a project's tracker is reached: with the environment `bdi` itself
/// runs in, or with what entering the project's directory under direnv
/// produces.
///
/// Ambient is the default because it is what a machine with bd and nothing
/// else can run, and `-C` naming the tracker outright is what makes it safe:
/// a credential belonging to another tracker can only fail to authenticate
/// against the right database, never open the wrong one. direnv is for a
/// setup that keeps one credential per project in each project's own
/// directory, and it is asked for by name rather than inferred from an
/// `.envrc`, so the config says which mechanism a project is read by.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    #[default]
    Ambient,
    Direnv,
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
    /// How long a project waits after one read before it asks for the next,
    /// where it polls at all. A collection is several `bd` subprocesses
    /// against each tracker's server, per instance running, so this is
    /// measured in seconds.
    ///
    /// A gap after a read rather than a period a read happens inside, and the
    /// difference is worth reading twice: the next ask is armed by the read
    /// that came back, so the effective period is this plus however long a
    /// read takes — `bdi-rer.4` measured 1.53s for the cascade. What it buys
    /// is that each project's schedule comes from its own history and nothing
    /// else, so projects drift apart rather than all paying the cascade on
    /// one tick, and a slow project delays only itself.
    ///
    /// Measured at `40f4eb5` against Dolt-backed trackers, one of 129 beads
    /// and one larger: 1.1 to 1.5 seconds for the small one alone, 2.3 to
    /// 2.4 for the larger alone, 3.5 to 4.1 for both together.
    ///
    /// Most of that is fixed per project rather than per row — a project
    /// costs seven-plus processes before its rows are read at all — so the
    /// cost follows the number of projects configured as much as the size of
    /// any one tracker, and a config naming twice as many wants a longer
    /// interval than this one.
    ///
    /// Setting it below a collection is allowed and is bounded, because the
    /// gap does not start until the read ends: a project asks again this long
    /// after its last answer, never sooner and never twice over. So a short
    /// interval buys back-to-back collections with no idle gap, and a view as
    /// fresh as the collection allows rather than as the interval promised.
    pub refresh_seconds: u64,

    /// How long a collection may go unanswered before `bdi` reports the
    /// tracker as having stopped answering rather than as being read.
    ///
    /// A collection blocks in `Command::output()`, which has no deadline, and
    /// reports nothing until it is done — so without this a tracker hung for
    /// an hour is drawn exactly as one asked half a second ago.
    ///
    /// Configured rather than fixed for the same reason the interval above is,
    /// and by the same measurement: what a healthy collection costs follows
    /// the number of projects, so a config naming twice as many waits longer
    /// before anything is wrong. The default is around eight times the 3.5 to
    /// 4.1 seconds measured for the two projects that measurement was taken
    /// on.
    ///
    /// Passing it abandons nothing. The collection runs on, and a tracker that
    /// answers at last puts its rows up — a deadline that cut the collection
    /// off would leave a merely slow tracker permanently unreadable, which is
    /// the disappearance `bdi` is built not to do.
    pub unanswered_after_seconds: u64,

    /// How long the tail waits after herdr answers before it asks for the
    /// selected pane again. The one interval here counted in milliseconds,
    /// because it is the one that is under a second: the tail is a live
    /// view of a pane, and seconds cannot say how live.
    ///
    /// A gap after an answer rather than a period, as `refresh_seconds` is:
    /// a slow herdr stretches the gap rather than piling asks up behind
    /// itself. The read is one `herdr` process, measured at 2–5 ms, so at
    /// the default four a second cost about a hundredth of a core — and four
    /// a second is where a reader stops being able to tell the band from the
    /// pane it is reading.
    pub tail_refresh_millis: u64,
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
            unanswered_after_seconds: 30,
            tail_refresh_millis: 250,
        }
    }
}

impl Tui {
    pub fn refresh(&self) -> Duration {
        Duration::from_secs(self.refresh_seconds)
    }

    pub fn tail_refresh(&self) -> Duration {
        Duration::from_millis(self.tail_refresh_millis)
    }

    /// The same, as the clock arithmetic beside a project's name counts in.
    pub fn unanswered_after(&self) -> TimeDelta {
        TimeDelta::seconds(self.unanswered_after_seconds.try_into().unwrap_or(i64::MAX))
    }
}

impl Config {
    /// A config naming these projects and nothing else, with every other
    /// setting at its default: what discovery builds where no file says more.
    pub fn naming(projects: Vec<Project>) -> Self {
        Config {
            projects,
            roots: Roots::default(),
            badges: Vec::new(),
            anomalies: Anomalies::default(),
            join: Join::default(),
            tui: Tui::default(),
            scope: Scope::default(),
        }
    }

    pub fn from_toml(s: &str) -> anyhow::Result<Self> {
        let table: toml::Table = toml::from_str(s)?;
        if table
            .get("roots")
            .and_then(|roots| roots.get("metadata_keys"))
            .is_some()
        {
            anyhow::bail!(
                "[roots] metadata_keys is gone: every unfinished bead is a root, so a key \
                 could name nothing bd's statuses do not; remove it"
            );
        }
        let cfg: Config = table.try_into()?;
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
        if let Some(project) = cfg
            .projects
            .iter()
            .find(|p| p.environment == Environment::Direnv && p.credential_command.is_some())
        {
            anyhow::bail!(
                "{} names both environment = \"direnv\" and a credential_command; a project \
                 is entered one way, so say which",
                project.name
            );
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

    /// The projects this run reads, in the order the config names them.
    ///
    /// Every site that gathers reads through this — the collection loop, the
    /// order the trees are drawn in, the forest drawn before any tracker has
    /// answered, the working trees git is asked for — so a project outside
    /// the scope is one nothing can go and read: the projects left out are
    /// not gathered, rather than gathered and hidden. `projects` itself stays
    /// whole for the sites that place a pane.
    pub fn read(&self) -> impl Iterator<Item = &Project> {
        self.projects.iter().filter(|p| self.reads(&p.name))
    }

    pub fn reads(&self, name: &str) -> bool {
        self.scope.reads(name)
    }

    /// The config scoped to the projects `--project` named, or left whole
    /// where none is. What decides that is whether a scope was asked for,
    /// never how many projects one selected: a scope that selected nothing is
    /// refused below rather than obeyed.
    ///
    /// Applied before the roots the command line names, so a *positional*
    /// under a project the scope left out is refused: one command line asking
    /// for a project's root and asking not to read that project contradicts
    /// itself, and the other order would accept it and then draw nothing.
    /// `roots.explicit` is read only inside a project's own collection, so a
    /// root under a project no collection reaches is never consulted.
    ///
    /// A root the *config file* names under an excluded project is not that
    /// contradiction and is left alone — see the test below.
    pub fn scoped_to(mut self, names: &[String]) -> anyhow::Result<Self> {
        if names.is_empty() {
            return Ok(self);
        }
        let unknown: Vec<&str> = names
            .iter()
            .map(String::as_str)
            .filter(|named| !self.is_configured(named))
            .collect();
        if !unknown.is_empty() {
            anyhow::bail!(
                "--project names {}, which is no project of this config; bdi is \
                 configured for {}",
                unknown.join(", "),
                names_of(&self.projects).join(", ")
            );
        }
        self.scope = Scope::Asked(names.to_vec());
        Ok(self)
    }

    /// The config scoped to the project holding `path` — the directory `bdi`
    /// was started in — or left whole where no project holds it. The
    /// deepest project wins, as it does when the join places a pane.
    pub fn scoped_to_the_project_holding(self, path: &Path) -> Self {
        self.scoped_to_the_project_holding_any_of(&[path.to_path_buf()])
    }

    /// The same, over the places one directory is: its counterpart in each
    /// working tree of the repository it sits in.
    pub fn scoped_to_the_project_holding_any_of(mut self, places: &[PathBuf]) -> Self {
        let holding = places
            .iter()
            .flat_map(|place| {
                self.projects
                    .iter()
                    .filter_map(move |p| Some((p.holds(place)?, p)))
            })
            .max_by_key(|(depth, _)| *depth)
            .map(|(_, project)| project.name.clone());
        if let Some(project) = holding {
            self.scope = Scope::Directory {
                project,
                widened: Vec::new(),
            };
        }
        self
    }

    /// Roots named on the command line join those named in config: discovery
    /// rule 3 has two spellings and one meaning. `<project>:<bead-id>` says
    /// whose tracker holds the bead; a bare id can only mean the one project
    /// being read, so the terse form survives exactly as far as it is
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
    ///
    /// A root under a project the scope left out is a contradiction only
    /// when the reader typed the scope. A scope the directory chose is
    /// widened to take the project in: `bdi homelab:hl-1` from another
    /// project's desktop reads both.
    fn placed(&mut self, named: &str) -> anyhow::Result<(String, String)> {
        let Some((project, id)) = named.split_once(':') else {
            let reading: Vec<&Project> = self.read().collect();
            return match reading.as_slice() {
                [only] => Ok((only.name.clone(), named.to_string())),
                several => anyhow::bail!(
                    "{named} names no project, and bdi is reading {}; write it as \
                     <project>:{named}",
                    several
                        .iter()
                        .map(|p| p.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            };
        };
        if project.is_empty() || id.is_empty() {
            anyhow::bail!("{named} is not <project>:<bead-id>");
        }
        if !self.is_configured(project) {
            anyhow::bail!(
                "{named} gives {id} to {project}, which is not among the projects \
                 bdi is configured for: {}",
                names_of(&self.projects).join(", ")
            );
        }
        if !self.reads(project) {
            match &mut self.scope {
                Scope::Directory { widened, .. } => widened.push(project.to_string()),
                Scope::Asked(_) | Scope::Everything => anyhow::bail!(
                    "{named} gives {id} to {project}, which is not among the projects \
                     bdi is reading: {}",
                    self.read()
                        .map(|p| p.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }
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
unanswered_after_seconds = 90
tail_refresh_millis = 100
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
                    environment: Environment::Ambient,
                    credential_command: Some("secret-tool lookup tracker atlas".to_string()),
                    poll: true,
                    worktrees: Vec::new(),
                },
                Project {
                    name: "beacon".to_string(),
                    path: PathBuf::from("/home/user/dev/beacon"),
                    environment: Environment::Ambient,
                    credential_command: Some(
                        "cat /home/user/dev/beacon/.beads-password".to_string()
                    ),
                    poll: true,
                    worktrees: Vec::new(),
                },
            ]
        );
        assert_eq!(
            cfg.roots,
            Roots {
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
        assert_eq!(cfg.tui.unanswered_after_seconds, 90);
        assert_eq!(cfg.tui.tail_refresh_millis, 100);
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
        assert_eq!(cfg.tui.unanswered_after_seconds, 30);
        assert_eq!(cfg.tui.tail_refresh_millis, 250);
    }

    /// The interval is written in seconds and read as a duration; nothing
    /// downstream should be doing that arithmetic.
    #[test]
    fn the_refresh_interval_is_read_as_a_duration() {
        let cfg = Config::from_toml(EVERY_SECTION).expect("parses");

        assert_eq!(cfg.tui.refresh(), Duration::from_secs(5));
        assert_eq!(Tui::default().refresh(), Duration::from_secs(30));
    }

    /// The tail's interval is the one written in milliseconds, and it is
    /// read as a duration all the same.
    #[test]
    fn the_tails_interval_is_read_as_a_duration() {
        let cfg = Config::from_toml(EVERY_SECTION).expect("parses");

        assert_eq!(cfg.tui.tail_refresh(), Duration::from_millis(100));
        assert_eq!(Tui::default().tail_refresh(), Duration::from_millis(250));
    }

    /// The same for how long a collection may go unanswered, which is counted
    /// against a `chrono` clock rather than a `std` one because what it dates
    /// is the instant the collection was asked for.
    #[test]
    fn how_long_a_collection_may_go_unanswered_is_read_as_a_duration() {
        let cfg = Config::from_toml(EVERY_SECTION).expect("parses");

        assert_eq!(cfg.tui.unanswered_after(), TimeDelta::seconds(90));
        assert_eq!(Tui::default().unanswered_after(), TimeDelta::seconds(30));
    }

    /// The whole of a project entry: a path. A tracker is read in `bdi`'s own
    /// environment unless its entry says otherwise, so a config restates
    /// neither where a tracker is nor how to authenticate to it, however many
    /// it names.
    #[test]
    fn a_project_needs_only_a_path_however_many_the_config_names() {
        for spelling in [ONE_PROJECT, TWO_AMBIENT, ONE_CREDENTIALLED_ONE_AMBIENT] {
            let cfg = Config::from_toml(spelling).expect("a path is the whole of an entry");

            assert!(cfg.projects.iter().any(|p| p.credential_command.is_none()));
        }
    }

    /// The escape hatch survives, for a tracker outside direnv's reach.
    #[test]
    fn a_project_may_still_name_a_credential_command() {
        let cfg = Config::from_toml(ONE_CREDENTIALLED_ONE_AMBIENT).expect("parses");

        assert_eq!(
            cfg.projects[0].credential_command.as_deref(),
            Some("secret-tool lookup tracker atlas")
        );
    }

    /// The default is the one a machine with bd and nothing else can run:
    /// `bdi`'s own environment, with no program run to reproduce a shell's.
    #[test]
    fn a_project_saying_nothing_about_its_environment_is_read_in_bdis_own() {
        let cfg = Config::from_toml(ONE_PROJECT).expect("parses");

        assert_eq!(cfg.projects[0].environment, Environment::Ambient);
    }

    const ONE_ENTERED_WITH_DIRENV: &str = r#"
[[projects]]
name = "beacon"
path = "/home/user/dev/beacon"
environment = "direnv"
"#;

    #[test]
    fn a_project_may_ask_to_be_entered_with_direnv() {
        let cfg = Config::from_toml(ONE_ENTERED_WITH_DIRENV).expect("parses");

        assert_eq!(cfg.projects[0].environment, Environment::Direnv);
    }

    const ENTERED_TWO_WAYS: &str = r#"
[[projects]]
name = "beacon"
path = "/home/user/dev/beacon"
environment = "direnv"
credential_command = "secret-tool lookup tracker beacon"
"#;

    /// One project, one way in. A credential command answering instead of
    /// direnv, or after it, would be a precedence nothing on the screen
    /// says, so a config asking for both is refused rather than resolved.
    #[test]
    fn a_project_entered_two_ways_is_refused() {
        let err = Config::from_toml(ENTERED_TWO_WAYS).unwrap_err().to_string();

        assert!(err.contains("beacon"), "got: {err}");
        assert!(err.contains("environment"), "got: {err}");
        assert!(err.contains("credential_command"), "got: {err}");
    }

    const ENTERED_SOME_OTHER_WAY: &str = r#"
[[projects]]
name = "beacon"
path = "/home/user/dev/beacon"
environment = "nix-shell"
"#;

    #[test]
    fn a_way_in_bdi_does_not_have_is_refused_by_name() {
        let err = Config::from_toml(ENTERED_SOME_OTHER_WAY)
            .unwrap_err()
            .to_string();

        assert!(err.contains("nix-shell"), "got: {err}");
        assert!(err.contains("ambient"), "got: {err}");
        assert!(err.contains("direnv"), "got: {err}");
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

    const ROOT_IN_A_SECOND_PROJECT: &str = r#"
[[projects]]
name = "atlas"
path = "/home/user/atlas"
credential_command = "secret-tool lookup tracker atlas"

[[projects]]
name = "beacon"
path = "/home/user/dev/beacon"
credential_command = "cat /home/user/dev/beacon/.beads-password"

[roots.explicit]
beacon = ["b-7"]
"#;

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

    /// The projects a config reads, by name and in its order.
    fn read_by(cfg: &Config) -> Vec<&str> {
        cfg.read().map(|p| p.name.as_str()).collect()
    }

    /// Scoping is what stops a reader working in one project paying for the
    /// others: every site that gathers reads the projects through `read`, so
    /// a project outside the scope is one nothing can go and read.
    #[test]
    fn a_scope_reads_only_the_projects_it_names() {
        let cfg = two_projects()
            .scoped_to(&["beacon".to_string()])
            .expect("beacon is configured");

        assert_eq!(read_by(&cfg), ["beacon"]);
    }

    /// The projects a scope leaves out stay known. A pane is placed by which
    /// configured project holds its directory, and a run that had forgotten
    /// the other projects would report every pane on another desktop as in a
    /// directory no project covers.
    #[test]
    fn a_scope_leaves_the_config_naming_every_project() {
        let cfg = two_projects()
            .scoped_to(&["beacon".to_string()])
            .expect("beacon is configured");

        assert_eq!(names_of(&cfg.projects), ["atlas", "beacon"]);
        assert!(cfg.reads("beacon"));
        assert!(!cfg.reads("atlas"));
    }

    /// The forest is drawn in the order the config names, so a scope is a
    /// filter over the config rather than a running order of its own.
    #[test]
    fn a_scope_leaves_the_projects_it_keeps_in_the_order_the_config_names() {
        let cfg = two_projects()
            .scoped_to(&["beacon".to_string(), "atlas".to_string()])
            .expect("both are configured");

        assert_eq!(read_by(&cfg), ["atlas", "beacon"]);
    }

    /// Asking for no particular project is not asking for none. What decides
    /// it is whether a scope was requested, never how many projects one
    /// selected — a `bdi` run with no arguments has to start.
    #[test]
    fn naming_no_project_leaves_every_project() {
        let cfg = two_projects()
            .scoped_to(&[])
            .expect("a scope of nothing scopes nothing");

        assert_eq!(read_by(&cfg), ["atlas", "beacon"]);
        assert_eq!(cfg.scope, Scope::Everything);
    }

    /// The directory `bdi` is started in decides the read set: the project
    /// holding it is the one read, and the scope says the directory chose.
    #[test]
    fn the_project_holding_the_directory_is_the_one_read() {
        let cfg =
            two_projects().scoped_to_the_project_holding(Path::new("/home/user/dev/beacon/src"));

        assert_eq!(read_by(&cfg), ["beacon"]);
        assert_eq!(
            cfg.scope,
            Scope::Directory {
                project: "beacon".to_string(),
                widened: Vec::new(),
            }
        );
        assert_eq!(
            names_of(&cfg.projects),
            ["atlas", "beacon"],
            "the projects the directory left out stay known"
        );
    }

    /// A repository inside another resolves to the inner one, which is the
    /// tie the join already breaks the same way when it places a pane.
    #[test]
    fn the_deepest_project_holding_the_directory_is_the_one_read() {
        let cfg = Config::from_toml(
            r#"
[[projects]]
name = "outer"
path = "/home/user/dev"

[[projects]]
name = "inner"
path = "/home/user/dev/inner"
"#,
        )
        .expect("the config parses")
        .scoped_to_the_project_holding(Path::new("/home/user/dev/inner/src"));

        assert_eq!(read_by(&cfg), ["inner"]);
    }

    /// Started outside every configured project there is nothing to scope to
    /// and nothing was asked for, so `bdi` reads everything, as it does today.
    #[test]
    fn a_directory_no_project_holds_leaves_every_project_read() {
        let cfg = two_projects().scoped_to_the_project_holding(Path::new("/home/user/elsewhere"));

        assert_eq!(read_by(&cfg), ["atlas", "beacon"]);
        assert_eq!(cfg.scope, Scope::Everything);
    }

    /// A positional under a project the directory left out widens the read
    /// set to that project: `bdi homelab:hl-1` from another project's desktop
    /// reads both. Only an explicit `--project` makes that a contradiction.
    #[test]
    fn a_root_under_a_project_the_directory_left_out_widens_the_read_set() {
        let cfg = two_projects()
            .scoped_to_the_project_holding(Path::new("/home/user/dev/beacon"))
            .with_roots_named_on_the_command_line(&["atlas:a-1".to_string()])
            .expect("a root elsewhere widens a scope the directory chose");

        assert_eq!(read_by(&cfg), ["atlas", "beacon"]);
        assert_eq!(
            cfg.roots.explicit,
            BTreeMap::from([("atlas".to_string(), vec!["a-1".to_string()])])
        );
        assert_eq!(
            cfg.scope,
            Scope::Directory {
                project: "beacon".to_string(),
                widened: vec!["atlas".to_string()],
            }
        );
    }

    /// A bare id belongs to the one project being read, however the scope
    /// that left one was arrived at.
    #[test]
    fn a_bare_root_belongs_to_the_project_the_directory_chose() {
        let cfg = two_projects()
            .scoped_to_the_project_holding(Path::new("/home/user/dev/beacon"))
            .with_roots_named_on_the_command_line(&["b-7".to_string()])
            .expect("the directory leaves only beacon");

        assert_eq!(
            cfg.roots.explicit,
            BTreeMap::from([("beacon".to_string(), vec!["b-7".to_string()])])
        );
    }

    /// A scope that quietly selected less than it named would start `bdi` on
    /// a forest the reader did not ask for and could not tell from the one
    /// they did, so a name matching nothing is refused the way every other
    /// unknown project name here is.
    #[test]
    fn a_scope_naming_no_configured_project_is_rejected() {
        let err = two_projects()
            .scoped_to(&["cinder".to_string()])
            .unwrap_err()
            .to_string();

        assert!(err.contains("cinder"), "got: {err}");
        assert!(err.contains("atlas"), "got: {err}");
        assert!(err.contains("beacon"), "got: {err}");
    }

    /// A root the *config* names under an excluded project is not the
    /// contradiction the command line can state, and is allowed. What is
    /// refused is a scope and a positional asking for opposite things in one
    /// invocation; a config root is a standing preference this run overrides,
    /// and a tree the reader excluded is silent by the same rule that makes
    /// scoping itself silent.
    ///
    /// The entry stays where it is rather than being pruned. It is only ever
    /// read inside a project's own collection, so an entry under a project no
    /// collection reaches is never consulted, and taking it out would be work
    /// to reach the state leaving it alone already gives.
    #[test]
    fn a_configured_root_under_a_project_the_scope_left_out_is_kept_and_unread() {
        let cfg = Config::from_toml(ROOT_IN_A_SECOND_PROJECT)
            .expect("the config parses")
            .scoped_to(&["atlas".to_string()])
            .expect("a config root elsewhere is not a contradiction");

        assert_eq!(read_by(&cfg), ["atlas"]);
        assert_eq!(
            cfg.roots.explicit["beacon"],
            ["b-7"],
            "nothing reads it, so nothing has to take it out"
        );
    }

    /// A root in a project the scope left out asks `bdi` to draw a tree out
    /// of a tracker it was told not to read. Refusing says so; keeping it
    /// would put the root in `roots.explicit` under a project no collection
    /// ever reaches, where nothing reads it and nothing reports it.
    #[test]
    fn a_root_naming_a_project_the_scope_left_out_is_rejected() {
        let err = two_projects()
            .scoped_to(&["atlas".to_string()])
            .expect("atlas is configured")
            .with_roots_named_on_the_command_line(&["beacon:b-7".to_string()])
            .unwrap_err()
            .to_string();

        assert!(err.contains("beacon"), "got: {err}");
        assert!(err.contains("atlas"), "got: {err}");
    }

    /// What a bare id was ever ambiguous about is which of the trackers being
    /// read holds it, so a scope that leaves one project settles it.
    #[test]
    fn a_bare_root_belongs_to_the_only_project_a_scope_leaves() {
        let cfg = two_projects()
            .scoped_to(&["beacon".to_string()])
            .expect("beacon is configured")
            .with_roots_named_on_the_command_line(&["b-7".to_string()])
            .expect("the scope leaves only beacon");

        assert_eq!(
            cfg.roots.explicit,
            BTreeMap::from([("beacon".to_string(), vec!["b-7".to_string()])])
        );
    }

    #[test]
    fn config_without_projects_is_rejected() {
        let err = Config::from_toml("[roots]\n").unwrap_err();
        assert!(err.to_string().contains("no projects"), "got: {err}");
    }

    /// `[roots] metadata_keys` marked live work while discovery took only two
    /// statuses. Every unfinished bead is a root now, so a key could name
    /// nothing the statuses do not, and the field went with the feature. A
    /// config still naming it is told so, rather than having it read and
    /// ignored.
    #[test]
    fn a_config_naming_the_retired_metadata_keys_is_told_the_field_is_gone() {
        let err = Config::from_toml(
            r#"
[[projects]]
name = "atlas"
path = "/home/user/atlas"

[roots]
metadata_keys = ["working_topic"]
"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("metadata_keys"), "got: {err}");
        assert!(err.to_string().contains("gone"), "got: {err}");
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
