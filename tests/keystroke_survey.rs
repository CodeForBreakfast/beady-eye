//! How big the forest the maintainer's own config draws is, and what one
//! keystroke over it costs.
//!
//! Reads the real trackers, so it needs the maintainer shell and is ignored:
//!
//!     cargo test --release --test keystroke_survey -- --ignored --nocapture
//!
//! `BDI_CONFIG` names the config; unset, it is the one `bdi` reads by default.
//! The numbers it prints are the ones `bdi-9jj` and its children are measured
//! by, so print them rather than assert them: what counts as fast is a
//! judgement, and a threshold here would fail on a loaded machine.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use beady_eye::app;
use beady_eye::collect::run::RealRunner;
use beady_eye::config::Config;
use beady_eye::model::snapshot::{Filter, Snapshot, Tree};
use beady_eye::view::forest::flatten;
use beady_eye::view::{Action, Motion};
use chrono::Utc;

const PRESSES: usize = 50;

fn config() -> Config {
    let path = std::env::var_os("BDI_CONFIG")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join(".config/beady-eye/config.toml"))
        })
        .expect("BDI_CONFIG or HOME names the config");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading the config at {}: {e}", path.display()));
    Config::from_toml(&text).expect("the config parses")
}

/// What each tree holds, summed over the trees: the thing every subtree
/// question the layout asks scales with.
fn stored(trees: &[Arc<Tree>]) -> usize {
    trees.iter().map(|tree| tree.beads.len()).sum()
}

/// How many beads those are, each `(project, id)` counted once.
fn distinct(trees: &[Arc<Tree>]) -> usize {
    trees
        .iter()
        .flat_map(|tree| {
            tree.beads
                .iter()
                .map(move |node| (tree.project.as_str(), node.id.as_str()))
        })
        .collect::<BTreeSet<_>>()
        .len()
}

/// The rows `--json` writes for the shown trees: one per way down to a bead.
fn emitted(snapshot: &Snapshot) -> usize {
    let json = serde_json::to_value(snapshot).expect("the snapshot serialises");
    json["trees"]
        .as_array()
        .expect("trees is an array")
        .iter()
        .map(|tree| tree["nodes"].as_array().expect("nodes is an array").len())
        .sum()
}

fn press(snapshot: &Snapshot) -> (usize, Duration) {
    let mut forest = flatten(snapshot.clone());
    let lines = forest.lines().len();
    let started = Instant::now();
    for _ in 0..PRESSES {
        forest.apply(Action::Move(Motion::NextRow));
    }
    (lines, started.elapsed() / PRESSES as u32)
}

/// How many times a refresh is timed, so one reading is not one scheduler
/// hiccup.
const REFRESHES: u32 = 5;

/// What landing a collection costs: re-applying the filter to a snapshot in
/// hand, flattening one into a forest, and a forest taking a snapshot
/// identical to the one it holds — the floor a refresh can never get under.
fn landing(name: &str, snapshot: &Snapshot) {
    let mut refiltered = snapshot.clone();
    let started = Instant::now();
    refiltered.refilter(snapshot.filter);
    let refilter_alone = started.elapsed();
    drop(refiltered);

    let handed = snapshot.clone();
    let started = Instant::now();
    let mut forest = flatten(handed);
    let flatten_alone = started.elapsed();

    let mut spent = Duration::ZERO;
    for _ in 0..REFRESHES {
        let again = snapshot.clone();
        let started = Instant::now();
        forest.refresh(again);
        spent += started.elapsed();
    }

    println!("[{name}: landing a collection]");
    println!("  refilter alone                   {refilter_alone:?}");
    println!("  flatten (first layout)           {flatten_alone:?}");
    println!(
        "  Forest::refresh, identical snapshot {:?}",
        spent / REFRESHES
    );
}

fn report(name: &str, snapshot: &Snapshot) {
    let (lines, per_press) = press(snapshot);
    println!("[{name}]");
    println!(
        "  rows in Snapshot.collected       {}",
        stored(&snapshot.collected)
    );
    println!(
        "  distinct (project, id)           {}",
        distinct(&snapshot.collected)
    );
    println!(
        "  rows in Snapshot.trees           {}",
        stored(&snapshot.trees)
    );
    println!("  rows --json writes for them      {}", emitted(snapshot));
    println!("  lines drawn                      {lines}");
    println!("  Forest::apply(Move) per press    {per_press:?}");
    landing(name, snapshot);
}

#[test]
#[ignore = "reads the maintainer's own trackers, which need the maintainer shell"]
fn rows_and_keystrokes_over_the_configured_trackers() {
    let cfg = config();
    let started = Instant::now();
    let snapshot = app::run(&cfg, &RealRunner, Filter::LiveAgents, Utc::now());
    println!("collected in {:?}", started.elapsed());
    let mut largest: Vec<(usize, &str, &str)> = snapshot
        .collected
        .iter()
        .map(|tree| (tree.beads.len(), tree.project.as_str(), tree.root.as_str()))
        .collect();
    largest.sort_unstable_by(|a, b| b.cmp(a));
    for (rows, project, root) in largest.iter().take(5) {
        println!("  {project} {root}: {rows} rows");
    }

    report("default filter", &snapshot);
    let mut all = snapshot;
    all.refilter(Filter::All);
    report("--all", &all);
}
