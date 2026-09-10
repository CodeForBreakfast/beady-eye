//! What a setup tells `bdi`: the shape of the config file, what each setting
//! means, and what `bdi` refuses to read.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::TimeDelta;

use regex_lite::{Captures, Regex};
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
    pub changes: Changes,
    #[serde(default)]
    pub tui: Tui,
    #[serde(default)]
    pub theme: Theme,
    /// Which of `projects` this run reads, and what chose them. The rest stay
    /// here rather than being dropped: a pane is placed by which configured
    /// project holds its directory, whether or not that project is read.
    #[serde(skip)]
    pub scope: Scope,
    /// git could not be run, so the projects here are named after the
    /// directories their trackers sit at the top of rather than after
    /// remotes.
    ///
    /// Discovery sets it and nothing else does, so it is only ever true of
    /// the one project a run with no config file draws: a file names its own
    /// projects, and `BDI_PROJECT` names the discovered one outright. That is
    /// also why a config the reader writes mid-run cannot carry it stale —
    /// where this is true there is no file to re-read.
    ///
    /// Like `scope`, a fact about how this run's config came to be rather
    /// than anything a config file could carry.
    #[serde(skip)]
    pub named_without_git: bool,
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

/// An unknown key is refused rather than dropped, which is serde's default.
/// A project entry is the one place a reader hand-writes the name of a
/// mechanism, and a key `bdi` silently ignores is read as the default — so a
/// misspelling, or a config written against a key that has since gone, would
/// have its tracker read in an environment it asked not to be read in.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub name: String,
    pub path: PathBuf,
    /// A command that runs another command in the environment this project's
    /// tracker is read in — `direnv exec .`, `nix develop -c`, `mise exec --`.
    /// `bdi` appends the probe that reads the environment back, so the config
    /// names the wrapper and nothing else.
    ///
    /// A project naming none is read in the environment `bdi` itself runs in,
    /// and nothing is run to find that out.
    #[serde(default)]
    pub environment_command: Option<Command>,
    /// A command whose stdout is this tracker's password, never the password
    /// itself. The rung for a setup whose only exotic need is the credential:
    /// its stdout is captured where an environment command's argv is visible
    /// to `ps`, so it stays rather than folding into one.
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
    /// Badges this project draws in place of the ones `[[badges]]` names, for
    /// the keys it names and no others.
    ///
    /// A link template on a shared badge cannot name a repository or a host,
    /// so a fleet-wide list cannot give one project's `delivery_pr` its own
    /// destination. This is where that project says so, while every key it
    /// stays silent about keeps drawing what the shared list says.
    #[serde(default)]
    pub badges: Vec<Badge>,
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

/// A program and its arguments, written either way round.
///
/// A line is what almost every wrapper wants — `direnv exec .` is three words
/// and no argument holds a space — so that is what the common case writes.
/// It is split on whitespace and nothing else: no quotes are honoured, and a
/// config relying on them would have `bdi` run an argv the reader did not
/// write, which is the class of silent wrong answer this key exists to close.
///
/// So an argument that holds a space is written as a list, where each entry
/// is one argument whatever is inside it:
///
/// ```toml
/// environment_command = ["nix", "develop", ".#dev shell", "-c"]
/// ```
///
/// The exotic case pays a more precise config and the common one pays
/// nothing, rather than every reader learning a quoting rule for a space
/// almost none of them has.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Command {
    Line(String),
    Words(Vec<String>),
}

impl Command {
    /// The program and its arguments, in order.
    pub fn words(&self) -> Vec<&str> {
        match self {
            Command::Line(line) => line.split_whitespace().collect(),
            Command::Words(words) => words.iter().map(String::as_str).collect(),
        }
    }

    /// Whether it names no program at all, which the config refuses. `bdi`
    /// appends its own probe, so an empty command would run that probe alone
    /// — reading the project in `bdi`'s environment while its config says it
    /// was read in its own.
    ///
    /// A list is the way to write an empty *word*, not only an empty command:
    /// `[""]` has an entry and still names nothing, where a line cannot,
    /// because splitting on whitespace never yields one. So it is the first
    /// word that has to be there rather than any word.
    pub fn names_no_program(&self) -> bool {
        self.words()
            .first()
            .is_none_or(|program| program.is_empty())
    }
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

/// A badge opens a table, so every key written after `[[projects.badges]]`
/// lands in it. A badge that took a project's `path` would leave the project
/// reporting a key the reader did in fact write as missing.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Badge {
    pub key: String,
    #[serde(rename = "match")]
    pub match_value: Option<Pattern>,
    pub render: String,
    /// Where the badge points, as a template over the same captures `render`
    /// reads.
    ///
    /// A brace pair naming nothing the value supplied leaves the badge with
    /// no link at all: a destination built out of a part that was never
    /// there points somewhere else.
    pub link: Option<String>,
}

/// A badge's `match`: the pattern a setup wrote, and that pattern compiled.
///
/// Anchored against the whole value. `match` was an exact-value test before
/// it was a pattern, and anchoring is what keeps every config written then
/// saying what it said: unanchored, `human` would begin drawing on
/// `inhumane`.
///
/// Compiled here, as the config is read, because badges are applied to every
/// bead of every collection.
#[derive(Debug, Clone)]
pub struct Pattern {
    source: String,
    anchored: Regex,
}

impl Pattern {
    pub fn new(source: &str) -> Result<Self, regex_lite::Error> {
        Ok(Self {
            source: source.to_string(),
            anchored: Regex::new(&format!("^(?:{source})$"))?,
        })
    }

    fn captures<'v>(&self, value: &'v str) -> Option<Captures<'v>> {
        self.anchored.captures(value)
    }
}

/// The compiled pattern is a function of the source text, so the source text
/// is the whole of what two patterns can differ by. Written out because no
/// regex implements `PartialEq`, and this equality is load-bearing: it is how
/// a re-read config is judged against the one in force.
impl PartialEq for Pattern {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
    }
}

impl Eq for Pattern {}

impl<'de> Deserialize<'de> for Pattern {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let source = String::deserialize(deserializer)?;
        Pattern::new(&source)
            .map_err(|e| serde::de::Error::custom(format!("{source:?} is no pattern: {e}")))
    }
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

/// Where `bdi` listens for something saying a project's work has moved on.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Changes {
    /// The socket to listen on, rather than the one under the directory this
    /// login session owns.
    ///
    /// Told rather than derived because the two parties that have to agree on
    /// it can be in different login sessions: `bdi` is a TUI a human runs and
    /// a producer is a daemon, and a runtime directory scopes to exactly the
    /// session. Derived, each is free to be right about a different path;
    /// named here, it is one fact both are given. A machine that owns no
    /// runtime directory at all — macOS — has nothing to derive and gets its
    /// channel from this key or not at all.
    ///
    /// The socket is created `0600` wherever it goes, and both platforms
    /// `bdi` runs on check that mode when something connects, so the channel
    /// is this user's for the same reason on either. Who may replace the
    /// socket is the directories above it to say, so `bdi` reads the way down
    /// to it as well: where a directory on that way is one somebody else may
    /// take a name in, `bdi` names that directory and polls.
    ///
    /// Per user, so it cannot be what two simultaneous `bdi` runs differ by —
    /// both read this file and derive this path. `--socket` is what one of
    /// them overrides it with.
    pub socket: Option<PathBuf>,
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

    /// How far one notch of the wheel moves the forest, and the bead window
    /// over it.
    ///
    /// Settled here rather than in code because no one number serves every
    /// device: a wheel reports a detent, and a high-precision trackpad
    /// reports once per cell of travel, so the same value is a nudge on one
    /// and a leap on the other.
    ///
    /// The terminal's own knob cannot reach this. A terminal scaling the
    /// wheel for its scrollback neutralises that scaling to its sign while a
    /// program is reading mouse reports, so what arrives here is one report
    /// per detent whatever the reader set.
    pub wheel_notch_lines: usize,
}

/// What the reader's terminal is, in the one respect `bdi` can neither see
/// nor ask.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Theme {
    pub background: Background,
}

/// The background the reader's terminal draws on.
///
/// The reader says it because `bdi` cannot find it out. A terminal query
/// degrades either to a wait or to a confident wrong answer, and a wrong
/// answer of that kind is intermittent — right in one terminal and wrong in
/// another, right outside a multiplexer and wrong inside it — so nobody can
/// see what is producing it. A declaration is wrong the same way on every
/// terminal from the first frame, which is what makes it something the
/// reader notices and one line fixes.
///
/// Their background rather than their theme, and that is the whole axis
/// rather than a stand-in for a richer one. A theme brings its own
/// foreground and its own colour 8, so `bdi` needs neither; what no theme
/// can tell it is which side of the background a treatment of `bdi`'s own
/// will land on.
///
/// Dark is what an undeclared reader gets, because it is what the shipped
/// tones were chosen against: the default changes nothing for anyone
/// already reading `bdi`.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Background {
    #[default]
    Dark,
    Light,
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
            wheel_notch_lines: 3,
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

    /// The same, as the clock arithmetic beside a project's name counts in,
    /// or the longest interval there is where the config named a patience
    /// longer than that.
    ///
    /// Saturating rather than refusing, because every value up there says the
    /// same thing — a collection this patience gives up on is one no run
    /// reaches — and because the patience is only ever compared against, so
    /// the longest interval there is is an answer every reader of it holds.
    /// `TimeDelta` runs out twice on the way: at `i64` seconds, and again
    /// three decimal places short of that, so a patience past the second
    /// limit is a thousandth of the way to the first.
    pub fn unanswered_after(&self) -> TimeDelta {
        i64::try_from(self.unanswered_after_seconds)
            .ok()
            .and_then(TimeDelta::try_seconds)
            .unwrap_or(TimeDelta::MAX)
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
            changes: Changes::default(),
            tui: Tui::default(),
            theme: Theme::default(),
            scope: Scope::default(),
            named_without_git: false,
        }
    }

    /// The badges this project draws: the global list, with a project's own
    /// entries standing in for every global entry that shares a key with one
    /// of them.
    ///
    /// A shadowed key's entries stand where the global list's first entry for
    /// that key stood, so overriding one badge does not reorder the row. Keys
    /// only the project names follow the rest.
    pub fn badges_for_project(&self, project: &str) -> Vec<Badge> {
        let Some(own) = self
            .projects
            .iter()
            .find(|p| p.name == project)
            .map(|p| p.badges.as_slice())
            .filter(|own| !own.is_empty())
        else {
            return self.badges.clone();
        };
        let mut drawn: Vec<Badge> = Vec::new();
        let mut stood_in_for: BTreeSet<&str> = BTreeSet::new();
        for global in &self.badges {
            match own.iter().any(|b| b.key == global.key) {
                false => drawn.push(global.clone()),
                true => {
                    if stood_in_for.insert(&global.key) {
                        drawn.extend(own.iter().filter(|b| b.key == global.key).cloned());
                    }
                }
            }
        }
        drawn.extend(
            own.iter()
                .filter(|b| !stood_in_for.contains(b.key.as_str()))
                .cloned(),
        );
        drawn
    }

    /// The projects this run reads whose names git did not give.
    ///
    /// One call for both mouths: the snapshot the screen draws and the
    /// snapshot `--json` prints are built from this, so neither can qualify
    /// a name the other does not.
    pub fn projects_named_without_git(&self) -> Vec<String> {
        match self.named_without_git {
            true => self.read().map(|project| project.name.clone()).collect(),
            false => Vec::new(),
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
        if let Some(project) = cfg.projects.iter().find(|p| {
            p.environment_command
                .as_ref()
                .is_some_and(Command::names_no_program)
        }) {
            anyhow::bail!(
                "{} names an environment_command with no program in it; bdi appends its own \
                 probe to what you write, so an empty one would read the project in bdi's \
                 environment while saying it was read in its own",
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
    /// widened to take the project in: `bdi meadow:mdw-1` from another
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
    /// `{}` in `render` is replaced by the whole value, and `{name}` by what
    /// the pattern's capture of that name took. A brace pair naming nothing
    /// the pattern captured is left as it was written, and the pair taken is
    /// the innermost, so `{{}}` still draws braces around the value.
    ///
    /// One pass, so what is placed is never read again: a value spelled like
    /// a placeholder is a value.
    pub fn apply(&self, value: &str) -> Option<String> {
        Some(self.fill(&self.render, value)?.text)
    }

    /// Where this badge points for a metadata value: its `link` filled in
    /// from the captures `render` reads, or `None` where the config names no
    /// link, the badge does not apply, or a brace pair in the template named
    /// nothing the value supplied.
    pub fn link_for(&self, value: &str) -> Option<String> {
        let filled = self.fill(self.link.as_ref()?, value)?;
        filled.whole.then_some(filled.text)
    }

    /// `template` filled in for `value`, or `None` where this badge does not
    /// apply to the value at all.
    fn fill(&self, template: &str, value: &str) -> Option<Filled> {
        let taken = match &self.match_value {
            Some(pattern) => Some(pattern.captures(value)?),
            None => None,
        };

        let mut text = String::new();
        let mut whole = true;
        let mut rest = template;
        while let Some(close) = rest.find('}') {
            let Some(open) = rest[..close].rfind('{') else {
                text.push_str(&rest[..=close]);
                rest = &rest[close + 1..];
                continue;
            };
            let name = &rest[open + 1..close];
            let placed = match name {
                "" => Some(value),
                _ => taken
                    .as_ref()
                    .and_then(|taken| taken.name(name))
                    .map(|capture| capture.as_str()),
            };
            whole &= placed.is_some();
            text.push_str(&rest[..open]);
            text.push_str(placed.unwrap_or(&rest[open..=close]));
            rest = &rest[close + 1..];
        }
        text.push_str(rest);
        Some(Filled { text, whole })
    }
}

/// One template filled in for one value.
struct Filled {
    text: String,
    /// Whether every brace pair in the template named something the value
    /// supplied. `render` draws a pair that named nothing as it was written,
    /// and a `link` carrying one is dropped, so the two need telling apart.
    whole: bool,
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

[[projects.badges]]
key    = "delivery_pr"
render = "⇢ beacon/{}"

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

[changes]
socket = "/var/folders/T/beady-eye/changes.sock"

[tui]
refresh_seconds = 5
unanswered_after_seconds = 90
tail_refresh_millis = 100
wheel_notch_lines = 1

[theme]
background = "light"
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
                    environment_command: None,
                    credential_command: Some("secret-tool lookup tracker atlas".to_string()),
                    poll: true,
                    badges: Vec::new(),
                    worktrees: Vec::new(),
                },
                Project {
                    name: "beacon".to_string(),
                    path: PathBuf::from("/home/user/dev/beacon"),
                    environment_command: None,
                    credential_command: Some(
                        "cat /home/user/dev/beacon/.beads-password".to_string()
                    ),
                    poll: true,
                    badges: vec![Badge {
                        key: "delivery_pr".to_string(),
                        match_value: None,
                        render: "⇢ beacon/{}".to_string(),
                        link: None,
                    }],
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
                    link: None,
                },
                Badge {
                    key: "blocked_on".to_string(),
                    match_value: Some(pattern("human")),
                    render: "⏸ waiting".to_string(),
                    link: None,
                },
            ]
        );
        assert_eq!(cfg.anomalies.stale_claim_days, 7);
        assert_eq!(cfg.join.pane_key, "herdr_pane");
        assert_eq!(
            cfg.changes.socket,
            Some(PathBuf::from("/var/folders/T/beady-eye/changes.sock"))
        );
        assert_eq!(cfg.tui.refresh_seconds, 5);
        assert_eq!(cfg.tui.unanswered_after_seconds, 90);
        assert_eq!(cfg.tui.tail_refresh_millis, 100);
        assert_eq!(cfg.tui.wheel_notch_lines, 1);
        assert_eq!(cfg.theme.background, Background::Light);
    }

    fn pattern(source: &str) -> Pattern {
        Pattern::new(source).expect("the pattern compiles")
    }

    fn badge(key: &str, render: &str) -> Badge {
        Badge {
            key: key.to_string(),
            match_value: None,
            render: render.to_string(),
            link: None,
        }
    }

    fn matching(key: &str, value: &str, render: &str) -> Badge {
        Badge {
            match_value: Some(pattern(value)),
            ..badge(key, render)
        }
    }

    fn drawing(name: &str, badges: Vec<Badge>) -> Project {
        Project {
            name: name.to_string(),
            path: PathBuf::from("/home/user").join(name),
            environment_command: None,
            credential_command: None,
            poll: true,
            badges,
            worktrees: Vec::new(),
        }
    }

    #[test]
    fn a_projects_badge_stands_where_the_global_one_it_shadows_stood() {
        let cfg = Config {
            badges: vec![
                badge("delivery_pr", "⇢ {}"),
                matching("blocked_on", "human", "⏸ waiting"),
            ],
            ..Config::naming(vec![drawing(
                "beacon",
                vec![badge("delivery_pr", "⇢ beacon/{}"), badge("epic", "▣ {}")],
            )])
        };

        assert_eq!(
            cfg.badges_for_project("beacon"),
            vec![
                badge("delivery_pr", "⇢ beacon/{}"),
                matching("blocked_on", "human", "⏸ waiting"),
                badge("epic", "▣ {}"),
            ]
        );
    }

    /// The global list may name one key several times, matched on a different
    /// value each time. A project overriding that key replaces the whole group
    /// rather than one of its entries: shadowing half a key would leave the
    /// project drawing the shared wording for every value it did not name.
    #[test]
    fn a_projects_badge_shadows_every_global_entry_for_its_key() {
        let cfg = Config {
            badges: vec![
                matching("blocked_on", "human", "⏸ waiting"),
                matching("blocked_on", "dependency", "⏸ blocked"),
            ],
            ..Config::naming(vec![drawing(
                "beacon",
                vec![matching("blocked_on", "human", "⏸ ask Ada")],
            )])
        };

        assert_eq!(
            cfg.badges_for_project("beacon"),
            vec![matching("blocked_on", "human", "⏸ ask Ada")]
        );
    }

    #[test]
    fn a_project_naming_no_badges_draws_the_global_list() {
        let cfg = Config {
            badges: vec![badge("delivery_pr", "⇢ {}")],
            ..Config::naming(vec![drawing("atlas", Vec::new())])
        };

        assert_eq!(
            cfg.badges_for_project("atlas"),
            vec![badge("delivery_pr", "⇢ {}")]
        );
    }

    /// The reader is told about the key they wrote, in the place they wrote
    /// it, rather than about the project that quietly lost it.
    ///
    /// `docs/configuration.md` quotes this sentence, so a badge that gains a
    /// key has to update both.
    #[test]
    fn a_project_key_written_after_its_badges_is_refused_by_the_badge() {
        let misplaced = r#"
[[projects]]
name = "beacon"

[[projects.badges]]
key    = "delivery_pr"
render = "⇢ beacon/{}"

path = "/home/user/dev/beacon"
"#;

        let refused = Config::from_toml(misplaced).expect_err("a badge has no path");

        let said = refused.to_string();
        assert!(
            said.contains("unknown field `path`, expected one of `key`, `match`, `render`, `link`"),
            "{said}"
        );
        assert!(!said.contains("missing field"), "{said}");
    }

    #[test]
    fn a_config_of_one_project_gets_every_default() {
        let cfg = Config::from_toml(ONE_PROJECT).expect("parses");

        assert_eq!(cfg.projects[0].credential_command, None);
        assert_eq!(cfg.roots, Roots::default());
        assert!(cfg.badges.is_empty());
        assert_eq!(cfg.anomalies.stale_claim_days, 30);
        assert_eq!(cfg.join.pane_key, "agent_pane");
        assert_eq!(cfg.changes.socket, None);
        assert_eq!(cfg.tui.refresh_seconds, 30);
        assert_eq!(cfg.tui.unanswered_after_seconds, 30);
        assert_eq!(cfg.tui.tail_refresh_millis, 250);
        assert_eq!(
            cfg.tui.wheel_notch_lines, 3,
            "three lines a notch is the convention a reader who says nothing gets"
        );
        assert_eq!(cfg.theme.background, Background::Dark);
    }

    /// `bdi` cannot see the reader's background, so a reader who says
    /// nothing is answered from the shipped default rather than from
    /// anything about the machine. That is what makes a wrong answer stable
    /// and attributable: it is wrong the same way on every terminal, and
    /// one documented key fixes it for good.
    #[test]
    fn an_undeclared_background_is_the_fallback() {
        assert_eq!(Theme::default().background, Background::Dark);
        assert_eq!(
            Config::from_toml(ONE_PROJECT)
                .expect("parses")
                .theme
                .background,
            Background::Dark
        );
    }

    /// And a background the reader misspelled is refused rather than read
    /// as the fallback. A typo answered silently with the default is the
    /// failure the key exists to remove, arriving through the key: the
    /// reader has said which background they are on, believes they have been
    /// heard, and has nothing on screen to tell them otherwise.
    #[test]
    fn a_background_that_is_not_one_of_the_two_is_refused() {
        let mistyped = format!("{ONE_PROJECT}\n[theme]\nbackground = \"Light\"\n");

        let refused =
            Config::from_toml(&mistyped).expect_err("a background bdi has no palette for");

        assert!(refused.to_string().contains("background"), "{refused}");
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

    /// The bound itself, taken off `chrono` rather than written down: the
    /// longest patience an interval can hold is read back as itself, and one
    /// second more is the longest interval there is.
    #[test]
    fn a_patience_longer_than_an_interval_can_hold_is_the_longest_there_is() {
        let longest: u64 = TimeDelta::MAX
            .num_seconds()
            .try_into()
            .expect("the longest interval there is runs forwards");

        assert_eq!(
            patient_for(longest).unanswered_after(),
            TimeDelta::seconds(TimeDelta::MAX.num_seconds())
        );
        assert_eq!(patient_for(longest + 1).unanswered_after(), TimeDelta::MAX);
    }

    /// The other limit, a thousandfold past the one above: a patience too
    /// large to be a signed count of seconds at all. No config file reaches
    /// it — TOML counts in signed 64-bit and refuses the literal — so what
    /// stands here is the crate's own `Tui`, whose fields anything may set,
    /// and nothing but a value up here tells the two limits apart.
    #[test]
    fn a_patience_too_large_to_count_in_signed_seconds_is_the_longest_there_is() {
        assert_eq!(patient_for(u64::MAX).unanswered_after(), TimeDelta::MAX);
    }

    /// The same rule as far out as a config file can put it: `i64::MAX`
    /// seconds, the largest integer TOML carries, already a thousandfold past
    /// what an interval holds — and the exact value the old fallback
    /// substituted for every value it caught.
    #[test]
    fn a_config_naming_a_patience_no_interval_can_hold_is_read_as_the_longest_there_is() {
        let cfg = Config::from_toml(&format!(
            "{ONE_PROJECT}[tui]\nunanswered_after_seconds = {}\n",
            i64::MAX
        ))
        .expect("parses");

        assert_eq!(cfg.tui.unanswered_after(), TimeDelta::MAX);
    }

    fn patient_for(seconds: u64) -> Tui {
        Tui {
            unanswered_after_seconds: seconds,
            ..Tui::default()
        }
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

    /// A project that names no command is read in the environment `bdi`
    /// itself runs in, and nothing is run to reproduce a shell's.
    #[test]
    fn a_project_saying_nothing_about_its_environment_is_read_in_bdis_own() {
        let cfg = Config::from_toml(ONE_PROJECT).expect("parses");

        assert_eq!(cfg.projects[0].environment_command, None);
    }

    const ONE_ENTERED_WITH_DIRENV: &str = r#"
[[projects]]
name = "beacon"
path = "/home/user/dev/beacon"
environment_command = "direnv exec ."
"#;

    /// The wrapper is what the config names. What `bdi` runs inside it is
    /// `bdi`'s own business, so the reader writes no probe.
    #[test]
    fn a_project_may_name_the_command_that_gives_its_environment() {
        let cfg = Config::from_toml(ONE_ENTERED_WITH_DIRENV).expect("parses");

        assert_eq!(
            cfg.projects[0].environment_command,
            Some(Command::Line("direnv exec .".to_string()))
        );
    }

    const ENTERED_WITH_A_SPACE_IN_AN_ARGUMENT: &str = r#"
[[projects]]
name = "beacon"
path = "/home/user/dev/beacon"
environment_command = ["nix", "develop", ".#dev shell", "-c"]
"#;

    /// A line is split on whitespace and no quoting is honoured, so an
    /// argument holding a space is written as a list instead. Without this
    /// the config would name one argv and `bdi` would run another.
    #[test]
    fn an_argument_holding_a_space_is_written_as_a_list() {
        let cfg = Config::from_toml(ENTERED_WITH_A_SPACE_IN_AN_ARGUMENT).expect("parses");

        assert_eq!(
            cfg.projects[0]
                .environment_command
                .as_ref()
                .expect("the project named one")
                .words(),
            vec!["nix", "develop", ".#dev shell", "-c"],
        );
    }

    /// The common case is a line, and it is the same command either way
    /// round.
    #[test]
    fn a_line_and_a_list_of_its_words_name_the_same_command() {
        assert_eq!(
            Command::Line("direnv exec .".to_string()).words(),
            Command::Words(vec!["direnv".into(), "exec".into(), ".".into()]).words(),
        );
    }

    const ENTERED_SOME_OTHER_WAY: &str = r#"
[[projects]]
name = "beacon"
path = "/home/user/dev/beacon"
environment_command = "nix develop -c"
"#;

    /// Any wrapper that runs a command, not a list `bdi` holds. nix and mise
    /// are reached by a config that names them and by no change here, which
    /// is the whole of why the enum went.
    #[test]
    fn a_mechanism_bdi_has_never_heard_of_is_named_the_same_way() {
        let cfg = Config::from_toml(ENTERED_SOME_OTHER_WAY).expect("parses");

        assert_eq!(
            cfg.projects[0].environment_command,
            Some(Command::Line("nix develop -c".to_string()))
        );
    }

    /// An empty command is refused, both ways it can be written. `bdi`
    /// appends its own probe, so an empty one would run `env -0` alone and
    /// hand back the ambient environment — the project read in `bdi`'s
    /// environment while its config says otherwise, which is the silent
    /// wrong read the whole setting exists to close.
    #[test]
    fn an_environment_command_with_no_program_in_it_is_refused() {
        for empty in [r#""""#, r#"" ""#, "[]", r#"[""]"#, r#"["", "exec"]"#] {
            let err = Config::from_toml(&format!(
                r#"
[[projects]]
name = "beacon"
path = "/home/user/dev/beacon"
environment_command = {empty}
"#
            ))
            .unwrap_err()
            .to_string();

            assert!(err.contains("beacon"), "for {empty}, got: {err}");
            assert!(
                err.contains("environment_command"),
                "for {empty}, got: {err}"
            );
        }
    }

    const ENTERED_THE_OLD_WAY: &str = r#"
[[projects]]
name = "beacon"
path = "/home/user/dev/beacon"
environment = "direnv"
"#;

    /// A config written against the `environment` key is refused rather than
    /// ignored. serde drops an unknown field by default, so without this the
    /// setup that most needs the new key — one already naming direnv — would
    /// be read in `bdi`'s own environment instead, silently, which is the
    /// failure the key exists to stop.
    #[test]
    fn a_config_still_naming_the_key_this_replaced_is_refused() {
        let err = Config::from_toml(ENTERED_THE_OLD_WAY)
            .unwrap_err()
            .to_string();

        assert!(err.contains("environment"), "got: {err}");
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
    /// set to that project: `bdi meadow:mdw-1` from another project's desktop
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
            link: None,
        };
        assert_eq!(b.apply("owner/repo#7"), Some("⇢ owner/repo#7".to_string()));
    }

    #[test]
    fn badge_with_match_is_selective() {
        let b = Badge {
            key: "blocked_on".to_string(),
            match_value: Some(pattern("human")),
            render: "⏸ waiting".to_string(),
            link: None,
        };
        assert_eq!(b.apply("human"), Some("⏸ waiting".to_string()));
        assert_eq!(b.apply("dependency"), None);
    }

    /// What anchoring buys, stated over every pair a corpus makes rather
    /// than over one example. `match` was an exact-value test before it was
    /// a pattern, so a config written then names one value and no other:
    /// unanchored, `human` would begin drawing on `inhumane`.
    #[test]
    fn a_match_written_as_a_literal_draws_on_that_value_and_no_other() {
        let values = ["human", "dependency", "pr", "a", "owner/repo#7", "⏸"];
        let anything_near = |v: &str| {
            [
                v.to_string(),
                format!("in{v}"),
                format!("{v}e"),
                format!("in{v}e"),
                format!("{v} {v}"),
                format!(" {v}"),
                format!("{v}\n"),
                v.to_uppercase(),
                String::new(),
            ]
        };

        for value in values {
            let badge = Badge {
                key: "blocked_on".to_string(),
                match_value: Some(pattern(value)),
                render: "drawn".to_string(),
                link: None,
            };
            for candidate in values.iter().flat_map(|v| anything_near(v)) {
                assert_eq!(
                    badge.apply(&candidate).is_some(),
                    candidate == value,
                    "{value:?} against {candidate:?}"
                );
            }
        }
    }

    #[test]
    fn render_substitutes_a_capture_by_name_and_braces_by_the_whole_value() {
        let b = Badge {
            key: "delivery_pr".to_string(),
            match_value: Some(pattern(r"[^/]+/(?<repo>[^#]+)#(?<number>[0-9]+)")),
            render: "⇢ {repo} #{number} of {}".to_string(),
            link: None,
        };
        assert_eq!(
            b.apply("owner/atlas#7"),
            Some("⇢ atlas #7 of owner/atlas#7".to_string())
        );
        assert_eq!(b.apply("owner/atlas"), None);
    }

    #[test]
    fn braces_written_around_the_braces_are_drawn_around_the_value() {
        let b = Badge {
            key: "delivery_pr".to_string(),
            match_value: None,
            render: "{{}}".to_string(),
            link: None,
        };
        assert_eq!(b.apply("owner/repo#7"), Some("{owner/repo#7}".to_string()));
    }

    /// A value is placed, never read: what a capture took is not itself a
    /// template, however it happens to be spelled.
    #[test]
    fn a_value_spelled_like_a_placeholder_is_placed_and_not_read() {
        let b = Badge {
            key: "working_topic".to_string(),
            match_value: Some(pattern(r"(?<channel>[^/]+)/(?<topic>.+)")),
            render: "{channel} · {topic}".to_string(),
            link: None,
        };
        assert_eq!(
            b.apply("{topic}/atlas"),
            Some("{topic} · atlas".to_string())
        );
    }

    /// A `link` is a template over the same captures `render` reads, which is
    /// what lets one global list build a URL out of a reference held as
    /// `owner/repo#number`: a `render` alone has no way to name a host.
    #[test]
    fn a_link_is_built_from_the_captures_render_reads() {
        let b = Badge {
            key: "delivery_pr".to_string(),
            match_value: Some(pattern(r"(?<owner>[^/]+)/(?<repo>[^#]+)#(?<number>[0-9]+)")),
            render: "⇢ #{number}".to_string(),
            link: Some("https://forge.invalid/{owner}/{repo}/pull/{number}".to_string()),
        };
        assert_eq!(b.apply("orbital/atlas#7"), Some("⇢ #7".to_string()));
        assert_eq!(
            b.link_for("orbital/atlas#7"),
            Some("https://forge.invalid/orbital/atlas/pull/7".to_string())
        );
    }

    /// A capture the pattern names but this value never supplied leaves the
    /// badge with no link. A `delivery_pr` is held as a bare number as well
    /// as a qualified reference, and a URL built round an owner and a
    /// repository that were never there points somewhere else entirely.
    #[test]
    fn a_link_missing_one_of_its_captures_is_no_link_at_all() {
        let b = Badge {
            key: "delivery_pr".to_string(),
            match_value: Some(pattern(
                r"(?:(?<owner>[^/]+)/(?<repo>[^#]+))?#?(?<number>[0-9]+)",
            )),
            render: "⇢ #{number}".to_string(),
            link: Some("https://forge.invalid/{owner}/{repo}/pull/{number}".to_string()),
        };
        assert_eq!(b.apply("12"), Some("⇢ #12".to_string()));
        assert_eq!(b.link_for("12"), None);
        assert_eq!(
            b.link_for("orbital/atlas#12"),
            Some("https://forge.invalid/orbital/atlas/pull/12".to_string())
        );
    }

    /// And a name the pattern has no capture for at all, which is the same
    /// mistake written in the config rather than met in a value.
    #[test]
    fn a_link_naming_a_capture_the_pattern_never_had_is_no_link() {
        let b = Badge {
            key: "delivery_pr".to_string(),
            match_value: Some(pattern(r"(?<number>[0-9]+)")),
            render: "⇢ #{number}".to_string(),
            link: Some("https://forge.invalid/{repo}/pull/{number}".to_string()),
        };
        assert_eq!(b.link_for("12"), None);
    }

    #[test]
    fn a_badge_that_does_not_apply_points_nowhere() {
        let b = Badge {
            key: "blocked_on".to_string(),
            match_value: Some(pattern("human")),
            render: "⏸ waiting".to_string(),
            link: Some("https://forge.invalid/waiting".to_string()),
        };
        assert_eq!(
            b.link_for("human"),
            Some("https://forge.invalid/waiting".to_string())
        );
        assert_eq!(b.link_for("dependency"), None);
    }

    #[test]
    fn a_badge_whose_config_names_no_link_points_nowhere() {
        let b = Badge {
            key: "delivery_pr".to_string(),
            match_value: None,
            render: "⇢ {}".to_string(),
            link: None,
        };
        assert_eq!(b.link_for("orbital/atlas#7"), None);
    }

    #[test]
    fn a_badges_link_is_read_out_of_the_config() {
        let cfg = Config::from_toml(&format!(
            r#"{ONE_PROJECT}
[[badges]]
key    = "delivery_pr"
match  = "(?<owner>[^/]+)/(?<repo>[^#]+)#(?<number>[0-9]+)"
render = "⇢ #{{number}}"
link   = "https://forge.invalid/{{owner}}/{{repo}}/pull/{{number}}"
"#
        ))
        .expect("the config reads");

        assert_eq!(
            cfg.badges[0].link_for("orbital/atlas#7"),
            Some("https://forge.invalid/orbital/atlas/pull/7".to_string())
        );
    }

    /// The config is refused whole, which is what leaves the one in force
    /// standing and puts the reason at the foot of the screen.
    #[test]
    fn a_match_that_does_not_parse_refuses_the_config() {
        let err = Config::from_toml(&format!(
            r#"{ONE_PROJECT}
[[badges]]
key    = "blocked_on"
match  = "(unclosed"
render = "⏸ waiting"
"#
        ))
        .unwrap_err();
        assert!(err.to_string().contains("(unclosed"), "got: {err}");
    }

    /// The working trees a project occupies are git's answer about a
    /// repository, so a config file cannot write them. It is now told so:
    /// the line was dropped in silence while unknown fields were, and a
    /// reader whose hand-written directory never arrives has no way to find
    /// out that the key was never theirs to set.
    #[test]
    fn a_config_cannot_write_the_worktrees_a_project_occupies() {
        let err = Config::from_toml(
            r#"
[[projects]]
name = "beacon"
path = "/home/user/dev/beacon"
worktrees = ["/home/user/anywhere-at-all"]
"#,
        )
        .unwrap_err()
        .to_string();

        assert!(err.contains("worktrees"), "got: {err}");
    }
}
