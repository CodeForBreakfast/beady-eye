//! What `bdi` does when it is run: the arguments it takes, the config those
//! arguments resolve against, and whether the snapshot is drawn or printed.

use std::io::{ErrorKind, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use chrono::Utc;
use clap::Parser;

use crate::app::Asked;
use crate::collect::agents::Agents;
use crate::collect::bd;
use crate::collect::discovery;
use crate::collect::herdr;
use crate::collect::run::{RealRunner, Runner};
use crate::config::Config;
use crate::model::snapshot::Filter;
use crate::tui::{Armed, Arming, Reload, CHECKED_EVERY};

/// Where the config lives when nothing says otherwise.
const DEFAULT_CONFIG: &str = "~/.config/beady-eye/config.toml";

/// The variable that names the project when no config file names one, ahead
/// of the name its repository or directory would give it.
const PROJECT_IN_THE_ENVIRONMENT: &str = "BDI_PROJECT";

/// The view is drawn on the alternate screen, so a `bdi` whose output is a
/// pipe has nowhere to draw and `--json` is the only thing it can give.
const NO_TERMINAL: u8 = 2;

#[derive(Parser)]
#[command(name = "bdi", version, about = "A tree of work in flight")]
struct Cli {
    /// Draw the tree this bead roots, alongside the trees bdi discovers.
    /// Write it as <project>:<bead-id> where bdi is reading more than one
    /// project; a bare id means the one project being read. A root under a
    /// project the directory left out reads that project too.
    #[arg(value_name = "BEAD-ID")]
    beads: Vec<String>,

    /// Read the configuration from this file, rather than
    /// ~/.config/beady-eye/config.toml.
    #[arg(long)]
    config: Option<String>,

    /// Read only the projects named, repeating the option for each, from
    /// wherever bdi is started. The ones left out are not read at all,
    /// rather than read and hidden.
    #[arg(long = "project", value_name = "NAME")]
    projects: Vec<String>,

    /// Read every configured project, wherever bdi is started. Without it,
    /// bdi started under a configured project's directory reads that
    /// project alone.
    #[arg(long = "all-projects", conflicts_with = "projects")]
    all_projects: bool,

    /// Emit the snapshot as JSON.
    #[arg(long)]
    json: bool,

    /// Draw every tree, including those with no live agent.
    #[arg(long)]
    all: bool,

    /// Poll every project this run, whatever the config says about each.
    #[arg(long, conflicts_with = "no_poll")]
    poll: bool,

    /// Poll no project this run, whatever the config says about each.
    #[arg(long = "no-poll")]
    no_poll: bool,
}

/// What this run said about polling, over what its config says about each
/// project.
///
/// A run-level switch rather than a second way of naming projects: `--project`
/// already narrows the set, and what this is for is bisecting — turning the
/// poll on to see whether a suspect producer was the only thing wrong, or off
/// to see whether it was working at all — on a machine nobody wants to
/// redeploy to find out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Polling {
    /// Each project as its own `poll` key says.
    AsConfigured,
    Everything,
    Nothing,
}

impl Polling {
    fn asked_for(cli: &Cli) -> Self {
        match (cli.poll, cli.no_poll) {
            (true, _) => Polling::Everything,
            (_, true) => Polling::Nothing,
            _ => Polling::AsConfigured,
        }
    }

    /// How long after a read this project waits before asking for the next,
    /// or nothing where it does not ask at all.
    fn after_a_read(self, project: &crate::config::Project, every: Duration) -> Option<Duration> {
        let polls = match self {
            Polling::AsConfigured => project.poll,
            Polling::Everything => true,
            Polling::Nothing => false,
        };
        polls.then_some(every)
    }
}

/// Which of the configured projects this run reads, as the command line
/// said.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Reading {
    /// Nothing said: the project holding the directory `bdi` was started in,
    /// or every project where none holds it.
    WhereBdiWasStarted,
    /// `--all-projects`.
    EveryProject,
    /// `--project`, once for each.
    Named(Vec<String>),
}

impl Reading {
    fn asked_for(cli: &Cli) -> Self {
        if cli.all_projects {
            Reading::EveryProject
        } else if cli.projects.is_empty() {
            Reading::WhereBdiWasStarted
        } else {
            Reading::Named(cli.projects.clone())
        }
    }
}

/// Where `bdi` was started and what its command line said — which, with the
/// config, is everything the read set is a function of.
struct Launch<'a> {
    cwd: &'a Path,
    reading: Reading,
    roots: &'a [String],
}

/// The config this run works to, and the file it came out of.
///
/// What settling the config left `bdi` unable to do travels on the config
/// itself rather than beside it, as a fact for the model to publish and the
/// view to make words of. That is what keeps the two mouths saying the same
/// thing: a fact carried beside the config reaches whichever of them the
/// caller hands it to, and only the screen is ever handed anything.
#[derive(Debug)]
struct Settled {
    config: Config,
    /// The config file to watch for edits, or nothing where none was read.
    read_from: Option<PathBuf>,
}

pub fn run() -> anyhow::Result<ExitCode> {
    let cli = Cli::parse();

    let home = std::env::var_os("HOME").map(PathBuf::from);
    let cwd = std::env::current_dir().context("finding the current directory")?;
    let launch = Launch {
        cwd: &cwd,
        reading: Reading::asked_for(&cli),
        roots: &cli.beads,
    };
    let Settled {
        config: mut cfg,
        read_from,
    } = match &cli.config {
        Some(named) => read_config(&RealRunner, &expand_tilde(named, home), &launch),
        None => config_for_wherever_bdi_was_run(
            &RealRunner,
            &expand_tilde(DEFAULT_CONFIG, home),
            &launch,
        ),
    }?;

    let filter = if cli.all {
        Filter::All
    } else {
        Filter::LiveAgents
    };
    if cli.json {
        let snapshot = crate::app::run(
            &cfg,
            &herdr::Herdr::new(&RealRunner as &dyn Runner),
            &bd::Cli::new(&RealRunner),
            filter,
            Utc::now(),
        );
        println!("{}", serde_json::to_string_pretty(&snapshot)?);
        return Ok(ExitCode::SUCCESS);
    }

    if !std::io::stdout().is_terminal() {
        eprintln!("bdi's view needs a terminal; re-run with --json");
        return Ok(ExitCode::from(NO_TERMINAL));
    }

    // The config the run starts on, kept back for the loop to be set up
    // from. The collector takes the other copy below and works to whatever
    // the reader writes after it, so this is the one that says what the
    // first frame draws.
    let started_on = cfg.clone();
    let polling = Polling::asked_for(&cli);
    // Asked again whenever the reader writes a config, so the set of
    // projects that poll is the set the file names. The command line is what
    // it carries that a config cannot: `--poll` and `--no-poll` overrule
    // every project's own key, and they are settled here.
    let arms: Arming = Box::new(move |cfg: &Config| {
        cfg.read()
            .map(|project| {
                Armed::polling(
                    project.name.clone(),
                    polling.after_a_read(project, cfg.tui.refresh()),
                )
            })
            .collect()
    });
    // Built before the config goes to the collector, and holding a copy of
    // it: what a re-read is compared against is the config this run is
    // working to, and after this line the collector owns the only other one.
    //
    // The file is read on the loop's own thread and what a re-read produces
    // goes behind the collector's seam — `drive::looked_at` is where that
    // split is decided and why.
    let reload = read_from.map(|path| {
        let cwd = cwd.clone();
        let reading = Reading::asked_for(&cli);
        let roots = cli.beads.clone();
        Reload::watching(
            path,
            CHECKED_EVERY,
            cfg.clone(),
            Box::new(move |text| {
                config_for_this_run(
                    text,
                    &RealRunner,
                    &Launch {
                        cwd: &cwd,
                        reading: reading.clone(),
                        roots: &roots,
                    },
                )
            }),
            Utc::now(),
        )
    });
    let mut collection = crate::app::Collection::default();
    let trackers = bd::Cli::new(&RealRunner);
    // One provider for the run, asked by the collection on its thread and by
    // the tail on another. Which one it is is chosen here and nowhere below.
    let agents: Arc<dyn Agents> = Arc::new(herdr::Herdr::new(&RealRunner as &dyn Runner));
    let listing = Arc::clone(&agents);
    crate::tui::run(
        &started_on,
        filter,
        arms,
        agents,
        Box::new(move |asked| match asked {
            // Nothing is drawn for a config the reader has written: what it
            // changes is what every read after it reads, and the loop asks
            // for one of those behind it.
            Asked::Reloaded(written) => {
                cfg = *written;
                None
            }
            Asked::Read(wanted) => {
                Some(collection.collect(&cfg, &listing, &trackers, &wanted, filter, Utc::now()))
            }
        }),
        reload,
    )?;

    Ok(ExitCode::SUCCESS)
}

/// A path the user named is read as written: a config that is not there is an
/// error, never a reason to look somewhere else.
///
/// The path comes back beside the config because it is the file the run goes
/// on looking at, and this is where which file that is gets settled.
fn read_config(runner: &dyn Runner, path: &Path, launch: &Launch<'_>) -> anyhow::Result<Settled> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading the config at {}", path.display()))?;
    Ok(Settled {
        config: config_for_this_run(&text, runner, launch)?,
        read_from: Some(path.to_path_buf()),
    })
}

/// A config file's text, as the config this run reads: scoped, holding the
/// roots the command line names, and with each project read knowing its
/// working trees. The file says where a project is; git says where else the
/// same project is, because a seat working in a linked worktree is working
/// in the project.
///
/// In that order. The scope is settled before git is asked where each
/// project is worked, because asking is a `git worktree list` in the
/// project's own directory — a project the run left out would otherwise
/// still be gone to, which is the gathering scoping exists to avoid. The
/// roots come between, because one under a project the directory left out
/// widens the scope, and the project it brought in is asked like any other.
/// The whole config is parsed first either way: what `[roots.explicit]` is
/// checked against is the config as written, not the part of it this run
/// reads.
fn config_for_this_run(
    text: &str,
    runner: &dyn Runner,
    launch: &Launch<'_>,
) -> anyhow::Result<Config> {
    let cfg = scoped(Config::from_toml(text)?, runner, launch)?
        .with_roots_named_on_the_command_line(launch.roots)?;
    Ok(discovery::with_the_working_trees_git_lists(cfg, runner))
}

/// The config scoped as the command line said, or as the directory says
/// where it said nothing.
fn scoped(cfg: Config, runner: &dyn Runner, launch: &Launch<'_>) -> anyhow::Result<Config> {
    match &launch.reading {
        Reading::Named(names) => cfg.scoped_to(names),
        Reading::EveryProject => Ok(cfg),
        Reading::WhereBdiWasStarted => {
            Ok(discovery::scoped_to_the_directory(cfg, runner, launch.cwd))
        }
    }
}

/// The config, or — where there is no config file at all — the repository the
/// current directory sits in. Only an absent file falls back; one that is
/// there and will not open is still an error.
///
/// The one project discovery finds is everything there is, so the directory
/// has nothing to choose between and the run reads it as everything: a
/// `--project` naming something else is still refused, and a `--project`
/// naming it is still obeyed.
fn config_for_wherever_bdi_was_run(
    runner: &dyn Runner,
    path: &Path,
    launch: &Launch<'_>,
) -> anyhow::Result<Settled> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Settled {
            config: config_for_this_run(&text, runner, launch)?,
            read_from: Some(path.to_path_buf()),
        }),
        Err(absent) if absent.kind() == ErrorKind::NotFound => {
            let named = std::env::var(PROJECT_IN_THE_ENVIRONMENT).ok();
            let discovered =
                discovery::from_the_current_directory(runner, launch.cwd, named.as_deref())
                    .with_context(|| {
                        format!(
                            "there is no config at {}, so bdi read the current directory",
                            path.display()
                        )
                    })?;
            let cfg = match &launch.reading {
                Reading::Named(names) => discovered.scoped_to(names)?,
                Reading::EveryProject | Reading::WhereBdiWasStarted => discovered,
            };
            // No file, so nothing to look at again. A config file written
            // while this run is going is a config file this run never read,
            // and what it says about scope, roots and where each project is
            // worked was settled against a directory rather than against it.
            Ok(Settled {
                config: cfg.with_roots_named_on_the_command_line(launch.roots)?,
                read_from: None,
            })
        }
        Err(unreadable) => {
            Err(unreadable).with_context(|| format!("reading the config at {}", path.display()))
        }
    }
}

/// `~` belongs to the shell, so a config path written with one is expanded
/// here rather than handed to the filesystem verbatim.
fn expand_tilde(path: &str, home: Option<PathBuf>) -> PathBuf {
    match (path.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::run::testing::FakeRunner;
    use crate::collect::run::{Env, RunFailure};
    use crate::config::Scope;
    use clap::CommandFactory;
    use std::collections::BTreeMap;

    /// A config file naming one project, and git's answer for where that
    /// project's repository is worked in.
    const ONE_PROJECT: &str = r#"
[[projects]]
name = "orbital"
path = "/srv/work/orbital"
"#;

    const TWO_PROJECTS: &str = r#"
[[projects]]
name = "orbital"
path = "/srv/work/orbital"

[[projects]]
name = "ferry"
path = "/srv/work/ferry"
"#;

    const A_WORKTREE_PER_SEAT: &str = "\
worktree /srv/work/orbital
HEAD 4d3c1f0e9b8a7c6d5e4f3a2b1c0d9e8f7a6b5c4d
branch refs/heads/main

worktree /tmp/seat-a/wt
HEAD 1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b
detached
";

    /// A config file of our own, written where the test can hand its path to
    /// the thing that reads it.
    fn a_config_file(named: &str) -> PathBuf {
        a_config_file_holding(named, ONE_PROJECT)
    }

    fn a_config_file_holding(named: &str, text: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("bdi-{named}-{}.toml", std::process::id()));
        std::fs::write(&path, text).expect("the file is ours to write");
        path
    }

    /// A run started in `cwd` with no flag about which projects to read and
    /// no root named, which is how one `bdi` per desktop is started.
    fn started_in(cwd: &'static str) -> Launch<'static> {
        Launch {
            cwd: Path::new(cwd),
            reading: Reading::WhereBdiWasStarted,
            roots: &[],
        }
    }

    /// A run started somewhere no configured project holds, asking for
    /// every project, for the tests that are not about scoping.
    fn every_project() -> Launch<'static> {
        Launch {
            reading: Reading::EveryProject,
            ..started_in("/home/elsewhere")
        }
    }

    fn read_by(cfg: &Config) -> Vec<&str> {
        cfg.read().map(|p| p.name.as_str()).collect()
    }

    /// The directory each call was made in. The argv is the same line
    /// whichever project it is asked about, so the directory is what says
    /// which project a call was for.
    fn directories_entered(runner: &FakeRunner) -> Vec<Option<PathBuf>> {
        runner.calls().into_iter().map(|c| c.cwd).collect()
    }

    /// A runner that fails the test on any call made under a directory the
    /// scope left out. Not entering an excluded project is the property
    /// scoping exists for, so it is measured on every call rather than read
    /// back off the ones a test thought to look for.
    struct NeverEntering {
        forbidden: PathBuf,
        inner: FakeRunner,
    }

    fn never_entering(forbidden: &str, inner: FakeRunner) -> NeverEntering {
        NeverEntering {
            forbidden: PathBuf::from(forbidden),
            inner,
        }
    }

    impl Runner for NeverEntering {
        fn run(
            &self,
            program: &str,
            args: &[&str],
            cwd: Option<&Path>,
            env: &Env,
        ) -> Result<String, RunFailure> {
            if let Some(entered) = cwd.filter(|cwd| cwd.starts_with(&self.forbidden)) {
                panic!(
                    "`{program} {}` was run in {}, which the scope left out",
                    args.join(" "),
                    entered.display()
                );
            }
            self.inner.run(program, args, cwd, env)
        }
    }

    /// git as a machine with two projects checked out answers it: orbital
    /// worked in its checkout and a seat's worktree, ferry in its checkout
    /// alone.
    fn two_repositories() -> FakeRunner {
        FakeRunner::default().with("git worktree list --porcelain", A_WORKTREE_PER_SEAT)
    }

    // ---- the directory decides the read set --------------------------------

    /// One `bdi` per desktop: started under one of the configured projects
    /// it reads that project, and nothing goes near the other.
    #[test]
    fn bdi_started_under_a_configured_project_reads_that_project_alone() {
        let path = a_config_file_holding("started-in-orbital", TWO_PROJECTS);
        let runner = never_entering("/srv/work/ferry", two_repositories());

        let cfg = read_config(&runner, &path, &started_in("/srv/work/orbital/src"))
            .expect("the config is ours to read")
            .config;

        assert_eq!(read_by(&cfg), ["orbital"]);
        assert_eq!(
            cfg.scope,
            Scope::Directory {
                project: "orbital".to_string(),
                widened: Vec::new(),
            }
        );
        assert!(
            cfg.projects[0]
                .holds(Path::new("/tmp/seat-a/wt/src"))
                .is_some(),
            "the project read still learns its working trees"
        );

        std::fs::remove_file(&path).expect("the file is ours to remove");
    }

    /// A project may be a directory holding several repositories — a desktop
    /// of them — and a `bdi` started in any one of those is in the project.
    #[test]
    fn bdi_started_in_a_repository_inside_a_project_reads_that_project() {
        let path = a_config_file_holding("started-inside", TWO_PROJECTS);
        let runner = never_entering("/srv/work/ferry", two_repositories());

        let cfg = read_config(
            &runner,
            &path,
            &started_in("/srv/work/orbital/ground-station/src"),
        )
        .expect("the config is ours to read")
        .config;

        assert_eq!(read_by(&cfg), ["orbital"]);

        std::fs::remove_file(&path).expect("the file is ours to remove");
    }

    /// A seat works in a linked worktree outside the project's tree, and the
    /// scope is decided before any project has been asked where it is
    /// worked. One `git worktree list` from the directory itself names the
    /// checkout it was cut from, and that is under the project.
    #[test]
    fn bdi_started_in_a_linked_worktree_of_a_project_reads_that_project() {
        let path = a_config_file_holding("started-in-a-worktree", TWO_PROJECTS);
        let runner = never_entering("/srv/work/ferry", two_repositories());

        let cfg = read_config(&runner, &path, &started_in("/tmp/seat-a/wt/src"))
            .expect("the config is ours to read")
            .config;

        assert_eq!(read_by(&cfg), ["orbital"]);
        let from_the_directory: Vec<String> = runner
            .inner
            .calls()
            .into_iter()
            .filter(|c| c.cwd.as_deref() == Some(Path::new("/tmp/seat-a/wt/src")))
            .map(|c| c.argv)
            .collect();
        assert_eq!(
            from_the_directory,
            ["git worktree list --porcelain"],
            "one git call from the directory, and nothing else runs there"
        );

        std::fs::remove_file(&path).expect("the file is ours to remove");
    }

    /// Started outside every configured project there is nothing to scope
    /// to, and nothing was asked for: `bdi` reads everything, as it did
    /// before the directory had a say.
    #[test]
    fn bdi_started_outside_every_configured_project_reads_all_of_them() {
        let path = a_config_file_holding("started-elsewhere", TWO_PROJECTS);
        let runner = FakeRunner::default().with(
            "git worktree list --porcelain",
            "worktree /home/elsewhere\nHEAD 4d3c1f0e9b8a7c6d5e4f3a2b1c0d9e8f7a6b5c4d\nbranch refs/heads/main\n",
        );

        let cfg = read_config(&runner, &path, &started_in("/home/elsewhere/notes"))
            .expect("the config is ours to read")
            .config;

        assert_eq!(read_by(&cfg), ["orbital", "ferry"]);
        assert_eq!(cfg.scope, Scope::Everything);

        std::fs::remove_file(&path).expect("the file is ours to remove");
    }

    /// `--all-projects` is the opt-out: the session watching everything from
    /// one project's checkout runs it, and the directory is not consulted.
    #[test]
    fn all_projects_reads_every_configured_project_wherever_bdi_was_started() {
        let path = a_config_file_holding("all-projects", TWO_PROJECTS);
        let runner = two_repositories();

        let cfg = read_config(
            &runner,
            &path,
            &Launch {
                reading: Reading::EveryProject,
                ..started_in("/srv/work/orbital/src")
            },
        )
        .expect("the config is ours to read")
        .config;

        assert_eq!(read_by(&cfg), ["orbital", "ferry"]);
        assert_eq!(cfg.scope, Scope::Everything);
        assert!(
            !directories_entered(&runner).contains(&Some(PathBuf::from("/srv/work/orbital/src"))),
            "the directory was consulted when the command line had already decided"
        );

        std::fs::remove_file(&path).expect("the file is ours to remove");
    }

    /// `--project` stays the way to ask for a different project, or two,
    /// from anywhere: it outranks the directory, and the directory is not
    /// consulted.
    #[test]
    fn an_explicit_project_outranks_the_directory() {
        let path = a_config_file_holding("project-outranks", TWO_PROJECTS);
        let runner = never_entering("/srv/work/orbital", two_repositories());

        let cfg = read_config(
            &runner,
            &path,
            &Launch {
                reading: Reading::Named(vec!["ferry".to_string()]),
                ..started_in("/srv/work/orbital/src")
            },
        )
        .expect("the config is ours to read")
        .config;

        assert_eq!(read_by(&cfg), ["ferry"]);
        assert_eq!(cfg.scope, Scope::Asked(vec!["ferry".to_string()]));

        std::fs::remove_file(&path).expect("the file is ours to remove");
    }

    /// `bdi ferry:fer-1` from orbital's desktop reads both. The widening
    /// happens before git is asked where each project is worked, so the
    /// project a root brought in learns its working trees like any other.
    #[test]
    fn a_root_under_a_project_the_directory_left_out_widens_the_read_set() {
        let path = a_config_file_holding("root-widens", TWO_PROJECTS);
        let runner = two_repositories();

        let cfg = read_config(
            &runner,
            &path,
            &Launch {
                roots: &["ferry:fer-1".to_string()],
                ..started_in("/srv/work/orbital/src")
            },
        )
        .expect("a root elsewhere widens a scope the directory chose")
        .config;

        assert_eq!(read_by(&cfg), ["orbital", "ferry"]);
        assert_eq!(
            cfg.roots.explicit,
            BTreeMap::from([("ferry".to_string(), vec!["fer-1".to_string()])])
        );
        assert!(
            directories_entered(&runner).contains(&Some(PathBuf::from("/srv/work/ferry"))),
            "ferry is read now, so git was asked where it is worked"
        );

        std::fs::remove_file(&path).expect("the file is ours to remove");
    }

    /// The same root against a scope the reader typed is a contradiction
    /// inside one command line, and stays refused.
    #[test]
    fn a_root_under_a_project_an_explicit_scope_left_out_is_still_refused() {
        let path = a_config_file_holding("root-refused", TWO_PROJECTS);
        let runner = never_entering("/srv/work/ferry", two_repositories());

        let refused = read_config(
            &runner,
            &path,
            &Launch {
                reading: Reading::Named(vec!["orbital".to_string()]),
                roots: &["ferry:fer-1".to_string()],
                ..started_in("/srv/work/orbital/src")
            },
        )
        .expect_err("asking for ferry's root and asking not to read ferry");

        assert!(format!("{refused:#}").contains("ferry"), "got: {refused:#}");

        std::fs::remove_file(&path).expect("the file is ours to remove");
    }

    /// A bare id belongs to the one project being read, so `bdi orb-7` from
    /// orbital's checkout needs no project name however many the config
    /// names.
    #[test]
    fn a_bare_root_belongs_to_the_project_the_directory_chose() {
        let path = a_config_file_holding("bare-root", TWO_PROJECTS);
        let runner = never_entering("/srv/work/ferry", two_repositories());

        let cfg = read_config(
            &runner,
            &path,
            &Launch {
                roots: &["orb-7".to_string()],
                ..started_in("/srv/work/orbital/src")
            },
        )
        .expect("the directory leaves only orbital")
        .config;

        assert_eq!(
            cfg.roots.explicit,
            BTreeMap::from([("orbital".to_string(), vec!["orb-7".to_string()])])
        );

        std::fs::remove_file(&path).expect("the file is ours to remove");
    }

    /// The no-config run is unchanged: the one project it discovers is
    /// everything there is, and the screen has nothing to say about a scope.
    #[test]
    fn a_run_with_no_config_file_reads_the_discovered_project_as_everything() {
        let absent =
            std::env::temp_dir().join(format!("bdi-absent-everything-{}.toml", std::process::id()));
        let runner = FakeRunner::default()
            .with("bd where --json", "{}")
            .with("git rev-parse --show-toplevel", "/srv/work/orbital")
            .with("git worktree list --porcelain", A_WORKTREE_PER_SEAT)
            .with("git remote get-url origin", "git@host:owner/orbital.git");

        let cfg =
            config_for_wherever_bdi_was_run(&runner, &absent, &started_in("/srv/work/orbital/src"))
                .expect("the directory is a project")
                .config;

        assert_eq!(read_by(&cfg), ["orbital"]);
        assert_eq!(cfg.scope, Scope::Everything);
    }

    /// git, as far as reading a config needs it: the one listing, whatever
    /// is asked and wherever it is asked from.
    struct ARepositoryWorkedInTwoPlaces;

    impl Runner for ARepositoryWorkedInTwoPlaces {
        fn run(
            &self,
            _program: &str,
            _args: &[&str],
            _cwd: Option<&Path>,
            _env: &Env,
        ) -> Result<String, RunFailure> {
            Ok(A_WORKTREE_PER_SEAT.to_string())
        }
    }

    /// A config file is where a project is written down, and nothing written
    /// down can say where the seats are. Both ways of reading one ask git.
    #[test]
    fn a_config_the_command_line_names_learns_its_working_trees() {
        let path = a_config_file("named-config");

        let cfg = read_config(&ARepositoryWorkedInTwoPlaces, &path, &every_project())
            .expect("the config is ours to read")
            .config;

        assert_eq!(
            cfg.projects[0].worktrees,
            vec![
                PathBuf::from("/srv/work/orbital"),
                PathBuf::from("/tmp/seat-a/wt"),
            ]
        );

        std::fs::remove_file(&path).expect("the file is ours to remove");
    }

    #[test]
    fn the_config_found_where_bdi_looks_learns_its_working_trees() {
        let path = a_config_file("default-config");

        let cfg =
            config_for_wherever_bdi_was_run(&ARepositoryWorkedInTwoPlaces, &path, &every_project())
                .expect("the config is ours to read")
                .config;

        assert!(
            cfg.projects[0]
                .holds(Path::new("/tmp/seat-a/wt/src"))
                .is_some(),
            "a seat in a linked worktree is working in the project"
        );

        std::fs::remove_file(&path).expect("the file is ours to remove");
    }

    /// Only an absent config falls back. One that is there and will not open
    /// is an error, because reading the current directory instead would answer
    /// a question the user did not ask and look like it had answered theirs.
    #[test]
    fn a_config_that_will_not_open_is_an_error_rather_than_a_fallback() {
        let a_directory = std::env::temp_dir();

        let refused = config_for_wherever_bdi_was_run(
            &ARepositoryWorkedInTwoPlaces,
            &a_directory,
            &every_project(),
        )
        .expect_err("a directory is not a config file");

        assert!(
            format!("{refused:#}")
                .contains(&format!("reading the config at {}", a_directory.display())),
            "a config that will not open should be reported as itself, not as \
             a missing one; said: {refused:#}"
        );
    }

    /// Asking git where a project is worked is a subprocess in that project's
    /// own directory, so it is gathering like any other and a scope has to
    /// come first. It did not: `git worktree list` ran in every configured
    /// project's path before the scope was applied, which the collection
    /// tests could not see because they are handed a config that has already
    /// been through this.
    #[test]
    fn a_project_the_scope_left_out_is_not_even_asked_where_it_is_worked() {
        let path = a_config_file_holding("scoped-worktrees", TWO_PROJECTS);
        let runner =
            FakeRunner::default().with("git worktree list --porcelain", A_WORKTREE_PER_SEAT);

        read_config(
            &runner,
            &path,
            &Launch {
                reading: Reading::Named(vec!["orbital".to_string()]),
                ..every_project()
            },
        )
        .expect("the config is ours to read");

        let asked = directories_entered(&runner);
        assert!(
            !asked.contains(&Some(PathBuf::from("/srv/work/ferry"))),
            "ferry was scoped out, so nothing should have gone to its directory; asked {asked:?}"
        );
        assert!(
            asked.contains(&Some(PathBuf::from("/srv/work/orbital"))),
            "orbital was scoped in, so git was asked where it is worked; asked {asked:?}"
        );

        std::fs::remove_file(&path).expect("the file is ours to remove");
    }

    /// A run with no config file at all still has a scope to honour. The
    /// project it discovers is one project, so a scope naming a different one
    /// selects nothing and is refused — rather than starting on the project
    /// the reader did not ask for, which is what a scope the fallback ignored
    /// would do.
    #[test]
    fn a_scope_naming_no_discovered_project_is_refused_with_no_config_file() {
        let absent = std::env::temp_dir().join(format!("bdi-absent-{}.toml", std::process::id()));
        let runner = FakeRunner::default()
            .with("bd where --json", "{}")
            .with("git rev-parse --show-toplevel", "/srv/work/orbital")
            .with("git worktree list --porcelain", A_WORKTREE_PER_SEAT)
            .with("git remote get-url origin", "git@host:owner/orbital.git");

        let refused = config_for_wherever_bdi_was_run(
            &runner,
            &absent,
            &Launch {
                reading: Reading::Named(vec!["nothing-of-the-sort".to_string()]),
                ..started_in("/srv/work/orbital")
            },
        )
        .expect_err("the scope names no project the current directory is in");

        assert!(
            format!("{refused:#}").contains("nothing-of-the-sort"),
            "got: {refused:#}"
        );
    }

    #[test]
    fn the_command_line_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn no_config_is_named_unless_one_is_asked_for() {
        assert_eq!(Cli::parse_from(["bdi"]).config, None);
    }

    #[test]
    fn a_config_named_on_the_command_line_is_taken_as_written() {
        let cli = Cli::parse_from(["bdi", "--config", "/etc/beady-eye.toml"]);

        assert_eq!(cli.config.as_deref(), Some("/etc/beady-eye.toml"));
    }

    #[test]
    fn bead_ids_are_taken_as_arguments() {
        let cli = Cli::parse_from(["bdi", "orb-7", "ferry:fer-9", "--json"]);

        assert_eq!(cli.beads, ["orb-7", "ferry:fer-9"]);
        assert!(cli.json);
    }

    #[test]
    fn the_projects_to_draw_are_taken_as_a_repeatable_option() {
        let cli = Cli::parse_from(["bdi", "--project", "orbital", "--project", "ferry"]);

        assert_eq!(cli.projects, ["orbital", "ferry"]);
    }

    #[test]
    fn no_project_is_named_unless_one_is_asked_for() {
        assert!(Cli::parse_from(["bdi"]).projects.is_empty());
    }

    /// The run says nothing about which projects to read unless it is asked
    /// to, and the directory then decides.
    #[test]
    fn a_run_that_names_no_project_leaves_it_to_the_directory() {
        assert_eq!(
            Reading::asked_for(&Cli::parse_from(["bdi"])),
            Reading::WhereBdiWasStarted
        );
    }

    #[test]
    fn a_run_can_ask_for_every_project_or_name_the_ones_it_wants() {
        assert_eq!(
            Reading::asked_for(&Cli::parse_from(["bdi", "--all-projects"])),
            Reading::EveryProject
        );
        assert_eq!(
            Reading::asked_for(&Cli::parse_from(["bdi", "--project", "ferry"])),
            Reading::Named(vec!["ferry".to_string()])
        );
    }

    /// Every project and only these is a command line that contradicts
    /// itself, and clap says so rather than one of them quietly winning.
    #[test]
    fn a_run_cannot_ask_for_every_project_and_name_only_some() {
        assert!(Cli::try_parse_from(["bdi", "--all-projects", "--project", "ferry"]).is_err());
    }

    /// `--all` is the view filter — every tree, including those with no
    /// live agent — and keeps that meaning; reading every project is a
    /// different flag.
    #[test]
    fn all_is_the_filter_and_not_the_read_set() {
        let cli = Cli::parse_from(["bdi", "--all"]);

        assert!(cli.all);
        assert_eq!(Reading::asked_for(&cli), Reading::WhereBdiWasStarted);
    }

    /// The run says nothing about polling unless it is asked to, and each
    /// project is then read as its own config key says.
    #[test]
    fn a_run_that_says_nothing_about_polling_leaves_it_to_the_config() {
        assert_eq!(
            Polling::asked_for(&Cli::parse_from(["bdi"])),
            Polling::AsConfigured
        );
    }

    #[test]
    fn a_run_can_turn_the_poll_on_or_off_for_every_project() {
        assert_eq!(
            Polling::asked_for(&Cli::parse_from(["bdi", "--poll"])),
            Polling::Everything
        );
        assert_eq!(
            Polling::asked_for(&Cli::parse_from(["bdi", "--no-poll"])),
            Polling::Nothing
        );
    }

    /// Asking for both is a command line that contradicts itself, and clap
    /// says so rather than one of them quietly winning.
    #[test]
    fn a_run_cannot_ask_for_the_poll_and_against_it_at_once() {
        assert!(Cli::try_parse_from(["bdi", "--poll", "--no-poll"]).is_err());
    }

    /// The interval each project's next ask is armed from, or nothing where
    /// it does not ask: what the run said, over what its config says.
    #[test]
    fn what_the_run_said_outranks_what_each_project_says() {
        let every = Duration::from_secs(30);
        let polled = a_project(true);
        let pushed = a_project(false);

        assert_eq!(
            Polling::AsConfigured.after_a_read(&polled, every),
            Some(every)
        );
        assert_eq!(Polling::AsConfigured.after_a_read(&pushed, every), None);
        assert_eq!(
            Polling::Everything.after_a_read(&pushed, every),
            Some(every)
        );
        assert_eq!(Polling::Nothing.after_a_read(&polled, every), None);
    }

    fn a_project(poll: bool) -> crate::config::Project {
        crate::config::Project {
            name: "orbital".to_string(),
            path: PathBuf::from("/srv/work/orbital"),
            environment_command: None,
            credential_command: None,
            poll,
            worktrees: Vec::new(),
        }
    }

    #[test]
    fn a_leading_tilde_becomes_the_home_directory() {
        let path = expand_tilde(DEFAULT_CONFIG, Some(PathBuf::from("/home/pilot")));

        assert_eq!(
            path,
            PathBuf::from("/home/pilot/.config/beady-eye/config.toml")
        );
    }

    #[test]
    fn a_path_that_names_no_home_is_left_as_written() {
        for path in ["/etc/beady-eye.toml", "~elsewhere/config.toml"] {
            assert_eq!(
                expand_tilde(path, Some(PathBuf::from("/home/pilot"))),
                PathBuf::from(path)
            );
        }
    }

    /// Without a `HOME` there is nothing to expand to, and a path we cannot
    /// resolve is better reported by the open that fails than guessed at.
    #[test]
    fn without_a_home_the_path_is_left_as_written() {
        assert_eq!(
            expand_tilde(DEFAULT_CONFIG, None),
            PathBuf::from(DEFAULT_CONFIG)
        );
    }
}
