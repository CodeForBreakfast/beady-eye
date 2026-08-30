# beady-eye Core + JSON Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A `bdi --json` binary that reads one or more beads trackers and, where available, a herdr session, and emits a tree of work per root annotated with the live agent on each node.

**Architecture:** Three units. A collector does all I/O by shelling out to `bd` and `herdr` and parsing their JSON. A pure model assembles rows into ordered trees, joins agents onto nodes, applies badge config and anomaly rules, and filters. A renderer serialises the model. The model never shells out, so every rule in it is unit-testable against fixtures.

**Tech Stack:** Rust 2021, `serde`/`serde_json`, `toml`, `clap`, `chrono`, `anyhow`. Nix flake dev shell. No async — the collector runs a handful of subprocesses.

**Spec:** `docs/design.md`

## Global Constraints

- **Never write to bd.** No `bd` subcommand other than `list`, `show`, `dep tree`. No `herdr` subcommand other than `agent list`, `agent read`, `agent get`. (`agent focus` arrives with the TUI, not here.)
- **The key is `(project, id)`, never `id` alone.** Bead prefixes are per-tracker and uncoordinated.
- **Degrade, never disappear.** A tracker that cannot be read, a node whose parent is missing, a truncated subtree — each is reported in the output, never silently dropped.
- **`--limit 0` on every `bd list`.** The default is 50 and truncation silently changes meaning.
- **`herdr`'s `agent_status` is a terminal property**, never a bead status and never a badge. Keep them in separate fields.
- **Fail loudly on an unexpected JSON shape** rather than rendering a wrong tree. A missing optional field is fine; a field with the wrong type is an error.
- Rust edition 2021, MSRV 1.74. No `unsafe`. `#![deny(warnings)]` is not set (it breaks on toolchain bumps); CI runs `cargo clippy -- -D warnings`.

---

### Task 1: Scaffold the crate and dev shell

**Files:**
- Create: `Cargo.toml`
- Create: `flake.nix`
- Create: `src/main.rs`
- Create: `src/lib.rs`
- Create: `rust-toolchain.toml`

**Interfaces:**
- Consumes: nothing.
- Produces: crate `beady_eye` (lib) and binary `bdi`. Every later task adds modules under `src/`.

- [ ] **Step 1: Write the failing test**

Create `src/lib.rs`:

```rust
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_not_empty() {
        assert!(!version().is_empty());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test`
Expected: FAIL — there is no `Cargo.toml` yet, so cargo errors with "could not find `Cargo.toml`".

- [ ] **Step 3: Write minimal implementation**

Create `Cargo.toml`:

```toml
[package]
name = "beady-eye"
version = "0.1.0"
edition = "2021"
rust-version = "1.74"
description = "A tree of work in flight: bead graphs annotated with the live agents working them"
repository = "https://github.com/CodeForBreakfast/beady-eye"

[lib]
name = "beady_eye"
path = "src/lib.rs"

[[bin]]
name = "bdi"
path = "src/main.rs"

[dependencies]
anyhow = "1"
chrono = { version = "0.4", default-features = false, features = ["std", "clock", "serde"] }
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"

[dev-dependencies]
pretty_assertions = "1"
```

Create `rust-toolchain.toml`:

```toml
[toolchain]
channel = "1.83"
components = ["rustfmt", "clippy"]
```

Create `src/main.rs`:

```rust
fn main() {
    println!("bdi {}", beady_eye::version());
}
```

Create `flake.nix`:

```nix
{
  description = "beady-eye: a tree of work in flight";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let pkgs = nixpkgs.legacyPackages.${system};
      in {
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [ cargo rustc rustfmt clippy rust-analyzer ];
        };

        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "beady-eye";
          version = "0.1.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
        };
      });
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test`
Expected: PASS — `test tests::version_is_not_empty ... ok`

Then run: `cargo run` — expected output `bdi 0.1.0`.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock rust-toolchain.toml flake.nix src/lib.rs src/main.rs
git commit -m "feat: scaffold the crate and dev shell"
```

---

### Task 2: Configuration

**Files:**
- Create: `src/config.rs`
- Modify: `src/lib.rs` (add `pub mod config;`)

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `config::Config { projects: Vec<Project>, roots: Roots, badges: Vec<Badge>, anomalies: Anomalies, join: Join }`
  - `config::Project { name: String, path: PathBuf }`
  - `config::Roots { metadata_keys: Vec<String>, explicit: Vec<String> }`
  - `config::Badge { key: String, match_value: Option<String>, render: String }`
  - `config::Anomalies { stale_claim_days: i64 }`
  - `config::Join { pane_key: String }`
  - `Config::from_toml(&str) -> anyhow::Result<Config>`
  - `Badge::apply(&self, value: &str) -> Option<String>`

The defaults matter: `stale_claim_days` is 30, matching `bd stale --days`, `pane_key` is `"agent_pane"`, and every list defaults empty. A config naming no projects is an error, because there is nothing to read.

- [ ] **Step 1: Write the failing test**

Create `src/config.rs`:

```rust
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
        Self { stale_claim_days: 30 }
    }
}

impl Default for Join {
    fn default() -> Self {
        Self { pane_key: "agent_pane".to_string() }
    }
}

impl Config {
    pub fn from_toml(s: &str) -> anyhow::Result<Self> {
        let cfg: Config = toml::from_str(s)?;
        if cfg.projects.is_empty() {
            anyhow::bail!("config names no projects; bdi has nothing to read");
        }
        Ok(cfg)
    }
}

impl Badge {
    /// Render this badge for a metadata value, or None if it does not apply.
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

    const SAMPLE: &str = r#"
[[projects]]
name = "summit-works"
path = "/tmp/bdi-ground/summit-works"

[roots]
metadata_keys = ["working_topic"]

[[badges]]
key = "delivery_pr"
render = "PR {}"

[[badges]]
key = "blocked_on"
match = "human"
render = "waiting"
"#;

    #[test]
    fn parses_projects_badges_and_defaults() {
        let cfg = Config::from_toml(SAMPLE).expect("parses");

        assert_eq!(cfg.projects.len(), 1);
        assert_eq!(cfg.projects[0].name, "summit-works");
        assert_eq!(cfg.roots.metadata_keys, vec!["working_topic".to_string()]);
        assert_eq!(cfg.badges.len(), 2);

        // Defaults apply when the sections are absent.
        assert_eq!(cfg.anomalies.stale_claim_days, 30);
        assert_eq!(cfg.join.pane_key, "agent_pane");
    }

    #[test]
    fn badge_without_match_renders_any_value() {
        let b = Badge { key: "delivery_pr".into(), match_value: None, render: "PR {}".into() };
        assert_eq!(b.apply("owner/repo#7"), Some("PR owner/repo#7".to_string()));
    }

    #[test]
    fn badge_with_match_is_selective() {
        let b = Badge { key: "blocked_on".into(), match_value: Some("human".into()), render: "waiting".into() };
        assert_eq!(b.apply("human"), Some("waiting".to_string()));
        assert_eq!(b.apply("dependency"), None);
    }

    #[test]
    fn config_without_projects_is_rejected() {
        let err = Config::from_toml("[roots]\nmetadata_keys = []\n").unwrap_err();
        assert!(err.to_string().contains("no projects"), "got: {err}");
    }
}
```

Add to `src/lib.rs`:

```rust
pub mod config;
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test config::`
Expected: FAIL — compile error, `toml` is declared but `src/config.rs` was only just added; if it compiles, the four tests must still be run and pass. If any assertion fails, that is the failure to fix.

- [ ] **Step 3: Write minimal implementation**

The code in Step 1 is the implementation — this task's test and implementation are one file, because a config struct's test is its parse. If `cargo test` reported a failure in Step 2, fix it here rather than adding code.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test config::`
Expected: PASS — 4 tests.

- [ ] **Step 5: Commit**

```bash
git add src/config.rs src/lib.rs
git commit -m "feat: configuration with badge rules and defaults"
```

---

### Task 3: Parse bd's dependency-tree JSON

**Files:**
- Create: `src/model/mod.rs`
- Create: `src/model/types.rs`
- Create: `src/collect/mod.rs`
- Create: `src/collect/bd.rs`
- Create: `tests/fixtures/bd_dep_tree.json`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `model::types::{Bead, Status, Edge}`
  - `Status::{Open, InProgress, Blocked, Closed, Deferred, Other(String)}`
  - `Edge::{ParentChild, Blocks, Other(String)}`
  - `collect::bd::parse_dep_tree(&str) -> anyhow::Result<Vec<Bead>>`

`Bead` mirrors only the fields we use. Unknown fields are ignored; a present field of the wrong type is an error.

- [ ] **Step 1: Write the failing test**

Create `tests/fixtures/bd_dep_tree.json` — this is the real shape, trimmed to six rows:

```json
[
  {"id":"nix-1","title":"Root epic","status":"open","priority":1,"issue_type":"epic","owner":"g@example.com","created_at":"2026-08-29T10:00:00Z","updated_at":"2026-08-30T08:00:00Z","metadata":{},"depth":0,"truncated":false},
  {"id":"nix-1.20","title":"wallpaper timer calls dms","status":"closed","priority":2,"issue_type":"bug","owner":"g@example.com","created_at":"2026-08-30T07:57:06Z","updated_at":"2026-08-30T08:33:47Z","closed_at":"2026-08-30T08:33:47Z","metadata":{"working_topic":"proj/topic-20"},"depth":1,"parent_id":"nix-1","edge_from_parent":"parent-child","truncated":false},
  {"id":"nix-1.14","title":"weather widget has no location","status":"open","priority":3,"issue_type":"task","owner":"g@example.com","created_at":"2026-08-29T11:00:00Z","updated_at":"2026-08-29T11:00:00Z","metadata":{},"depth":2,"parent_id":"nix-1.20","edge_from_parent":"blocks","truncated":false},
  {"id":"nix-1.1","title":"wire the niri theme include","status":"blocked","priority":2,"issue_type":"task","owner":"g@example.com","created_at":"2026-08-29T10:30:00Z","updated_at":"2026-08-30T09:00:00Z","started_at":"2026-08-29T12:00:00Z","metadata":{"blocked_on":"human","agent_pane":"wCM:p6"},"depth":1,"parent_id":"nix-1","edge_from_parent":"parent-child","truncated":false},
  {"id":"nix-1.4","title":"restore app theming","status":"open","priority":2,"issue_type":"task","owner":"g@example.com","created_at":"2026-08-29T10:31:00Z","updated_at":"2026-08-29T10:31:00Z","metadata":{},"depth":2,"parent_id":"nix-1.1","edge_from_parent":"blocks","truncated":false},
  {"id":"nix-1.16","title":"guard a key in both layers","status":"in_progress","priority":3,"issue_type":"task","owner":"g@example.com","created_at":"2026-08-29T10:32:00Z","updated_at":"2026-07-01T10:32:00Z","started_at":"2026-07-01T10:32:00Z","metadata":{},"depth":1,"parent_id":"nix-1","edge_from_parent":"parent-child","truncated":false}
]
```

Create `src/model/types.rs`:

```rust
use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Open,
    InProgress,
    Blocked,
    Closed,
    Deferred,
    #[serde(untagged)]
    Other(String),
}

impl Status {
    /// Render order: work in flight first, finished last.
    pub fn rank(&self) -> u8 {
        match self {
            Status::InProgress => 0,
            Status::Blocked => 1,
            Status::Open => 2,
            Status::Deferred => 3,
            Status::Closed => 4,
            Status::Other(_) => 5,
        }
    }

    pub fn is_closed(&self) -> bool {
        matches!(self, Status::Closed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Edge {
    ParentChild,
    Blocks,
    #[serde(untagged)]
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Bead {
    pub id: String,
    pub title: String,
    pub status: Status,
    #[serde(default)]
    pub priority: u8,
    #[serde(default)]
    pub issue_type: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub edge_from_parent: Option<Edge>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub truncated: bool,
}
```

Create `src/model/mod.rs`:

```rust
pub mod types;
```

Create `src/collect/bd.rs`:

```rust
use crate::model::types::Bead;

/// Parse the output of `bd dep tree <root> --direction=up --json`.
///
/// bd returns a flat array already in its own render order, each row carrying
/// `parent_id`, `depth` and `edge_from_parent`. We keep the rows and re-order
/// them ourselves; see `model::tree`.
pub fn parse_dep_tree(s: &str) -> anyhow::Result<Vec<Bead>> {
    let beads: Vec<Bead> = serde_json::from_str(s)?;
    Ok(beads)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::types::{Edge, Status};

    const FIXTURE: &str = include_str!("../../tests/fixtures/bd_dep_tree.json");

    #[test]
    fn parses_every_row() {
        let beads = parse_dep_tree(FIXTURE).expect("parses");
        assert_eq!(beads.len(), 6);
    }

    #[test]
    fn root_has_no_parent_and_children_carry_their_edge() {
        let beads = parse_dep_tree(FIXTURE).unwrap();

        let root = &beads[0];
        assert_eq!(root.id, "nix-1");
        assert_eq!(root.parent_id, None);
        assert_eq!(root.edge_from_parent, None);

        let child = beads.iter().find(|b| b.id == "nix-1.14").unwrap();
        assert_eq!(child.parent_id.as_deref(), Some("nix-1.20"));
        assert_eq!(child.edge_from_parent, Some(Edge::Blocks));
    }

    #[test]
    fn statuses_map_onto_the_enum() {
        let beads = parse_dep_tree(FIXTURE).unwrap();
        let by = |id: &str| beads.iter().find(|b| b.id == id).unwrap().status.clone();

        assert_eq!(by("nix-1.20"), Status::Closed);
        assert_eq!(by("nix-1.1"), Status::Blocked);
        assert_eq!(by("nix-1.16"), Status::InProgress);
        assert_eq!(by("nix-1.4"), Status::Open);
    }

    #[test]
    fn metadata_is_carried_inline() {
        let beads = parse_dep_tree(FIXTURE).unwrap();
        let b = beads.iter().find(|b| b.id == "nix-1.1").unwrap();

        assert_eq!(b.metadata.get("blocked_on").map(String::as_str), Some("human"));
        assert_eq!(b.metadata.get("agent_pane").map(String::as_str), Some("wCM:p6"));
    }

    #[test]
    fn a_wrongly_typed_field_is_an_error_not_a_default() {
        let bad = r#"[{"id":"x","title":"t","status":"open","priority":"high"}]"#;
        assert!(parse_dep_tree(bad).is_err());
    }
}
```

Create `src/collect/mod.rs`:

```rust
pub mod bd;
```

Add to `src/lib.rs`:

```rust
pub mod collect;
pub mod model;
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test collect::bd::`
Expected: FAIL on first write — the fixture path or module wiring is what breaks first. Fix until it compiles, then all five tests must pass.

- [ ] **Step 3: Write minimal implementation**

The Step 1 code is the implementation. If `Status::Other` or `Edge::Other` misbehave, note that `#[serde(untagged)]` on a unit-struct variant requires the variant to hold the raw string — that is why both are `Other(String)`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test collect::bd::`
Expected: PASS — 5 tests.

- [ ] **Step 5: Commit**

```bash
git add src/model src/collect src/lib.rs tests/fixtures/bd_dep_tree.json
git commit -m "feat: parse bd dependency-tree JSON into typed rows"
```

---

### Task 4: Assemble and order the tree

**Files:**
- Create: `src/model/tree.rs`
- Modify: `src/model/mod.rs`

**Interfaces:**
- Consumes: `model::types::{Bead, Status}` from Task 3.
- Produces:
  - `model::tree::Placed { bead: Bead, depth: u16 }`
  - `model::tree::Assembled { rows: Vec<Placed>, dangling: Vec<String> }`
  - `model::tree::assemble(beads: Vec<Bead>) -> Assembled`

Siblings sort by `(status.rank(), priority, id)`. Depth is recomputed from the parent chain rather than trusted from bd. A bead whose `parent_id` names a bead not in the input is placed at depth 1 under the root and its id recorded in `dangling` — reported, never dropped. A parent cycle is broken by a visited set so assembly always terminates.

- [ ] **Step 1: Write the failing test**

Create `src/model/tree.rs`:

```rust
use std::collections::{BTreeMap, HashSet};

use crate::model::types::Bead;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub bead: Bead,
    pub depth: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assembled {
    pub rows: Vec<Placed>,
    /// Ids whose declared parent was not present in the input.
    pub dangling: Vec<String>,
}

/// Order a flat set of bd rows into render order.
///
/// bd has already resolved and deduplicated the tree, so this re-parents by
/// `parent_id` and sorts siblings; it does not walk dependency edges.
pub fn assemble(beads: Vec<Bead>) -> Assembled {
    let present: HashSet<String> = beads.iter().map(|b| b.id.clone()).collect();

    let root_id = beads
        .iter()
        .find(|b| b.parent_id.is_none())
        .map(|b| b.id.clone());

    let mut dangling = Vec::new();
    let mut children: BTreeMap<String, Vec<Bead>> = BTreeMap::new();
    let mut root: Option<Bead> = None;

    for bead in beads {
        match &bead.parent_id {
            None => root = Some(bead),
            Some(parent) if present.contains(parent) => {
                children.entry(parent.clone()).or_default().push(bead);
            }
            Some(_) => {
                dangling.push(bead.id.clone());
                if let Some(r) = &root_id {
                    children.entry(r.clone()).or_default().push(bead);
                }
            }
        }
    }

    for kids in children.values_mut() {
        kids.sort_by(|a, b| {
            a.status
                .rank()
                .cmp(&b.status.rank())
                .then(a.priority.cmp(&b.priority))
                .then(a.id.cmp(&b.id))
        });
    }

    let mut rows = Vec::new();
    if let Some(root) = root {
        let mut seen = HashSet::new();
        walk(root, 0, &children, &mut seen, &mut rows);
    }

    dangling.sort();
    Assembled { rows, dangling }
}

fn walk(
    bead: Bead,
    depth: u16,
    children: &BTreeMap<String, Vec<Bead>>,
    seen: &mut HashSet<String>,
    out: &mut Vec<Placed>,
) {
    if !seen.insert(bead.id.clone()) {
        return;
    }
    let id = bead.id.clone();
    out.push(Placed { bead, depth });

    if let Some(kids) = children.get(&id) {
        for kid in kids {
            walk(kid.clone(), depth + 1, children, seen, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::bd::parse_dep_tree;

    const FIXTURE: &str = include_str!("../../tests/fixtures/bd_dep_tree.json");

    fn ids(a: &Assembled) -> Vec<&str> {
        a.rows.iter().map(|p| p.bead.id.as_str()).collect()
    }

    #[test]
    fn every_bead_appears_exactly_once() {
        let a = assemble(parse_dep_tree(FIXTURE).unwrap());
        assert_eq!(a.rows.len(), 6);

        let mut seen = ids(&a);
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 6, "a bead was emitted more than once");
    }

    #[test]
    fn root_is_first_and_at_depth_zero() {
        let a = assemble(parse_dep_tree(FIXTURE).unwrap());
        assert_eq!(a.rows[0].bead.id, "nix-1");
        assert_eq!(a.rows[0].depth, 0);
    }

    #[test]
    fn depth_is_recomputed_from_the_parent_chain() {
        let a = assemble(parse_dep_tree(FIXTURE).unwrap());
        let depth = |id: &str| a.rows.iter().find(|p| p.bead.id == id).unwrap().depth;

        assert_eq!(depth("nix-1.1"), 1);
        assert_eq!(depth("nix-1.4"), 2);
        assert_eq!(depth("nix-1.14"), 2);
    }

    #[test]
    fn siblings_order_in_flight_before_open_before_closed() {
        let a = assemble(parse_dep_tree(FIXTURE).unwrap());
        let order = ids(&a);

        let pos = |id: &str| order.iter().position(|x| *x == id).unwrap();

        // .16 is in_progress, .1 is blocked, .20 is closed — all children of the root.
        assert!(pos("nix-1.16") < pos("nix-1.1"), "in_progress sorts before blocked");
        assert!(pos("nix-1.1") < pos("nix-1.20"), "blocked sorts before closed");
    }

    #[test]
    fn a_child_follows_its_own_parent_not_the_next_sibling() {
        let a = assemble(parse_dep_tree(FIXTURE).unwrap());
        let order = ids(&a);
        let pos = |id: &str| order.iter().position(|x| *x == id).unwrap();

        // .4 hangs off .1, so it must sit between .1 and whatever follows it.
        assert_eq!(pos("nix-1.4"), pos("nix-1.1") + 1);
        assert_eq!(pos("nix-1.14"), pos("nix-1.20") + 1);
    }

    #[test]
    fn a_bead_whose_parent_is_absent_is_reported_and_kept() {
        let json = r#"[
          {"id":"nix-1","title":"root","status":"open","priority":1,"metadata":{},"truncated":false},
          {"id":"nix-1.9","title":"orphan","status":"open","priority":2,"parent_id":"nix-1.404","edge_from_parent":"blocks","metadata":{},"truncated":false}
        ]"#;
        let a = assemble(parse_dep_tree(json).unwrap());

        assert_eq!(a.dangling, vec!["nix-1.9".to_string()]);
        assert_eq!(a.rows.len(), 2, "the orphan is kept, not dropped");
        assert_eq!(a.rows[1].depth, 1, "the orphan is placed under the root");
    }

    #[test]
    fn a_parent_cycle_terminates() {
        let json = r#"[
          {"id":"a","title":"a","status":"open","priority":1,"metadata":{},"truncated":false},
          {"id":"b","title":"b","status":"open","priority":1,"parent_id":"c","edge_from_parent":"blocks","metadata":{},"truncated":false},
          {"id":"c","title":"c","status":"open","priority":1,"parent_id":"b","edge_from_parent":"blocks","metadata":{},"truncated":false}
        ]"#;
        let a = assemble(parse_dep_tree(json).unwrap());
        // b and c are unreachable from the root; the point is that this returns at all.
        assert_eq!(a.rows[0].bead.id, "a");
    }
}
```

Add to `src/model/mod.rs`:

```rust
pub mod tree;
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test model::tree::`
Expected: FAIL — `model::tree` is not yet declared in `mod.rs` on the first run.

- [ ] **Step 3: Write minimal implementation**

The Step 1 code is the implementation. If `a_child_follows_its_own_parent_not_the_next_sibling` fails, the cause is emitting all of one depth before recursing; `walk` must recurse into each child before moving to the next sibling.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test model::tree::`
Expected: PASS — 7 tests.

- [ ] **Step 5: Commit**

```bash
git add src/model/tree.rs src/model/mod.rs
git commit -m "feat: assemble bd rows into ordered render trees"
```

---

### Task 5: Parse herdr's agent list

**Files:**
- Create: `src/collect/herdr.rs`
- Create: `tests/fixtures/herdr_agent_list.json`
- Modify: `src/collect/mod.rs`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `collect::herdr::{Pane, PaneStatus}`
  - `PaneStatus::{Idle, Working, Blocked, Done, Other(String)}`
  - `collect::herdr::parse_agent_list(&str) -> anyhow::Result<Vec<Pane>>`

`herdr agent list` wraps its payload: `{"id":..., "result":{"agents":[...],"type":"agent_list"}}`. The parser unwraps it. A pane with no `display_agent` is kept — that is the unattributed case, not an error.

- [ ] **Step 1: Write the failing test**

Create `tests/fixtures/herdr_agent_list.json`:

```json
{"id":"cli:agent:list","result":{"type":"agent_list","agents":[
  {"agent":"claude","agent_status":"working","cwd":"/tmp/bdi-ground/summit-works","display_agent":"nix-1.16","title":"guard a key in both layers","pane_id":"wCM:pB","tab_id":"wCM:t1","workspace_id":"wCM"},
  {"agent":"claude","agent_status":"idle","cwd":"/tmp/bdi-ground/summit-works","display_agent":"nix-1.1","title":"theme wiring","pane_id":"wCM:p6","tab_id":"wCM:t1","workspace_id":"wCM","state_labels":{"idle":"asleep: needs eyes on the focus ring","working":"verifying theme wiring"}},
  {"agent":"claude","agent_status":"working","cwd":"/tmp/bdi-ground/summit-works","display_agent":"nix-1.20","title":"wallpaper timer","pane_id":"wCM:p9","tab_id":"wCM:t1","workspace_id":"wCM"},
  {"agent":"claude","agent_status":"blocked","cwd":"/tmp/bdi-ground/summit-works","pane_id":"wCM:pD","tab_id":"wCM:t1","workspace_id":"wCM"}
]}}
```

Create `src/collect/herdr.rs`:

```rust
use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaneStatus {
    Idle,
    Working,
    Blocked,
    Done,
    #[serde(untagged)]
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Pane {
    pub pane_id: String,
    pub cwd: PathBuf,
    #[serde(default)]
    pub display_agent: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub state_labels: BTreeMap<String, String>,
    pub agent_status: PaneStatus,
    #[serde(default)]
    pub workspace_id: Option<String>,
}

impl Pane {
    /// The line to show for this pane: its state label for the state it is
    /// actually in, falling back to its title.
    pub fn caption(&self) -> Option<&str> {
        let key = match &self.agent_status {
            PaneStatus::Idle => "idle",
            PaneStatus::Working => "working",
            PaneStatus::Blocked => "blocked",
            PaneStatus::Done => "done",
            PaneStatus::Other(s) => s.as_str(),
        };
        self.state_labels
            .get(key)
            .map(String::as_str)
            .or(self.title.as_deref())
    }
}

#[derive(Deserialize)]
struct Envelope {
    result: AgentList,
}

#[derive(Deserialize)]
struct AgentList {
    agents: Vec<Pane>,
}

/// Parse the output of `herdr agent list`.
pub fn parse_agent_list(s: &str) -> anyhow::Result<Vec<Pane>> {
    let env: Envelope = serde_json::from_str(s)?;
    Ok(env.result.agents)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../tests/fixtures/herdr_agent_list.json");

    #[test]
    fn unwraps_the_envelope() {
        let panes = parse_agent_list(FIXTURE).expect("parses");
        assert_eq!(panes.len(), 4);
    }

    #[test]
    fn a_pane_without_display_agent_is_kept() {
        let panes = parse_agent_list(FIXTURE).unwrap();
        let p = panes.iter().find(|p| p.pane_id == "wCM:pD").unwrap();

        assert_eq!(p.display_agent, None);
        assert_eq!(p.agent_status, PaneStatus::Blocked);
    }

    #[test]
    fn caption_prefers_the_label_for_the_current_state() {
        let panes = parse_agent_list(FIXTURE).unwrap();
        let p = panes.iter().find(|p| p.pane_id == "wCM:p6").unwrap();

        assert_eq!(p.agent_status, PaneStatus::Idle);
        assert_eq!(p.caption(), Some("asleep: needs eyes on the focus ring"));
    }

    #[test]
    fn caption_falls_back_to_title_when_no_label_matches() {
        let panes = parse_agent_list(FIXTURE).unwrap();
        let p = panes.iter().find(|p| p.pane_id == "wCM:p9").unwrap();

        assert_eq!(p.caption(), Some("wallpaper timer"));
    }
}
```

Add to `src/collect/mod.rs`:

```rust
pub mod herdr;
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test collect::herdr::`
Expected: FAIL — `collect::herdr` is not yet declared on the first run.

- [ ] **Step 3: Write minimal implementation**

The Step 1 code is the implementation.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test collect::herdr::`
Expected: PASS — 4 tests.

- [ ] **Step 5: Commit**

```bash
git add src/collect/herdr.rs src/collect/mod.rs tests/fixtures/herdr_agent_list.json
git commit -m "feat: parse herdr agent list into typed panes"
```

---

### Task 6: Join agents onto beads, and apply badges

**Files:**
- Create: `src/model/join.rs`
- Modify: `src/model/mod.rs`

**Interfaces:**
- Consumes: `model::tree::Placed` (Task 4), `collect::herdr::{Pane, PaneStatus}` (Task 5), `config::{Badge, Join}` (Task 2).
- Produces:
  - `model::join::{AgentRef, JoinSource, Badged}`
  - `JoinSource::{AgentPane, DisplayAgent}`
  - `model::join::resolve(rows: &[Placed], panes: &[Pane], join: &Join) -> BTreeMap<String, AgentRef>`
  - `model::join::badges_for(bead: &Bead, badges: &[Badge]) -> Vec<Badged>`
  - `model::join::unattributed<'a>(panes: &'a [Pane], claimed: &BTreeMap<String, AgentRef>) -> Vec<&'a Pane>`

The bead→pane direction wins over pane→bead, because it is exact rather than inferred. A pane named by a bead's key but absent from the live list resolves to nothing — that absence is what Task 7 turns into `orphan-claim`.

- [ ] **Step 1: Write the failing test**

Create `src/model/join.rs`:

```rust
use std::collections::{BTreeMap, HashSet};

use serde::Serialize;

use crate::collect::herdr::{Pane, PaneStatus};
use crate::config::{Badge, Join};
use crate::model::tree::Placed;
use crate::model::types::Bead;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JoinSource {
    /// The bead named the pane. Exact.
    AgentPane,
    /// The pane named the bead. Inferred from free text.
    DisplayAgent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentRef {
    pub pane: String,
    pub pane_status: PaneStatus,
    pub title: Option<String>,
    pub source: JoinSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Badged {
    pub key: String,
    pub text: String,
}

/// Map bead id -> the live pane working it.
pub fn resolve(rows: &[Placed], panes: &[Pane], join: &Join) -> BTreeMap<String, AgentRef> {
    let live: BTreeMap<&str, &Pane> = panes.iter().map(|p| (p.pane_id.as_str(), p)).collect();
    let mut out: BTreeMap<String, AgentRef> = BTreeMap::new();

    // Exact direction first: the bead names its pane.
    for row in rows {
        if let Some(pane_id) = row.bead.metadata.get(&join.pane_key) {
            if let Some(pane) = live.get(pane_id.as_str()) {
                out.insert(
                    row.bead.id.clone(),
                    AgentRef {
                        pane: pane.pane_id.clone(),
                        pane_status: pane.agent_status.clone(),
                        title: pane.caption().map(str::to_string),
                        source: JoinSource::AgentPane,
                    },
                );
            }
        }
    }

    // Inferred direction fills the gaps only.
    let known: HashSet<String> = rows.iter().map(|r| r.bead.id.clone()).collect();
    for pane in panes {
        let Some(bead_id) = &pane.display_agent else { continue };
        if !known.contains(bead_id) || out.contains_key(bead_id) {
            continue;
        }
        out.insert(
            bead_id.clone(),
            AgentRef {
                pane: pane.pane_id.clone(),
                pane_status: pane.agent_status.clone(),
                title: pane.caption().map(str::to_string),
                source: JoinSource::DisplayAgent,
            },
        );
    }

    out
}

/// Render the configured badges that apply to this bead.
pub fn badges_for(bead: &Bead, badges: &[Badge]) -> Vec<Badged> {
    badges
        .iter()
        .filter_map(|b| {
            let value = bead.metadata.get(&b.key)?;
            let text = b.apply(value)?;
            Some(Badged { key: b.key.clone(), text })
        })
        .collect()
}

/// Live panes that resolved to no bead in any tree.
pub fn unattributed<'a>(panes: &'a [Pane], claimed: &BTreeMap<String, AgentRef>) -> Vec<&'a Pane> {
    let taken: HashSet<&str> = claimed.values().map(|a| a.pane.as_str()).collect();
    panes.iter().filter(|p| !taken.contains(p.pane_id.as_str())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::bd::parse_dep_tree;
    use crate::collect::herdr::parse_agent_list;
    use crate::model::tree::assemble;

    const BEADS: &str = include_str!("../../tests/fixtures/bd_dep_tree.json");
    const PANES: &str = include_str!("../../tests/fixtures/herdr_agent_list.json");

    fn fixture() -> (Vec<Placed>, Vec<Pane>) {
        let rows = assemble(parse_dep_tree(BEADS).unwrap()).rows;
        let panes = parse_agent_list(PANES).unwrap();
        (rows, panes)
    }

    #[test]
    fn the_bead_naming_its_pane_resolves_exactly() {
        let (rows, panes) = fixture();
        let joined = resolve(&rows, &panes, &Join::default());

        // nix-1.1 carries agent_pane=wCM:p6.
        let a = joined.get("nix-1.1").expect("resolved");
        assert_eq!(a.pane, "wCM:p6");
        assert_eq!(a.source, JoinSource::AgentPane);
    }

    #[test]
    fn a_pane_naming_its_bead_resolves_as_inferred() {
        let (rows, panes) = fixture();
        let joined = resolve(&rows, &panes, &Join::default());

        // nix-1.16 has no agent_pane; the pane's display_agent supplies it.
        let a = joined.get("nix-1.16").expect("resolved");
        assert_eq!(a.pane, "wCM:pB");
        assert_eq!(a.source, JoinSource::DisplayAgent);
    }

    #[test]
    fn a_bead_naming_a_dead_pane_resolves_to_nothing() {
        let (mut rows, panes) = fixture();
        rows[0]
            .bead
            .metadata
            .insert("agent_pane".into(), "wCM:pGONE".into());

        let joined = resolve(&rows, &panes, &Join::default());
        assert!(!joined.contains_key(&rows[0].bead.id));
    }

    #[test]
    fn a_pane_with_no_bead_is_unattributed() {
        let (rows, panes) = fixture();
        let joined = resolve(&rows, &panes, &Join::default());
        let loose = unattributed(&panes, &joined);

        assert_eq!(loose.len(), 1);
        assert_eq!(loose[0].pane_id, "wCM:pD");
    }

    #[test]
    fn badges_render_only_where_the_key_and_match_agree() {
        let (rows, _) = fixture();
        let bead = &rows.iter().find(|r| r.bead.id == "nix-1.1").unwrap().bead;

        let cfg = vec![
            Badge { key: "blocked_on".into(), match_value: Some("human".into()), render: "waiting".into() },
            Badge { key: "blocked_on".into(), match_value: Some("dependency".into()), render: "dep".into() },
            Badge { key: "delivery_pr".into(), match_value: None, render: "PR {}".into() },
        ];

        let got = badges_for(bead, &cfg);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].text, "waiting");
    }
}
```

Add to `src/model/mod.rs`:

```rust
pub mod join;
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test model::join::`
Expected: FAIL — module not declared on the first run.

- [ ] **Step 3: Write minimal implementation**

The Step 1 code is the implementation. `PaneStatus` must derive `Clone` for `resolve` to copy it onto `AgentRef`; that derive is already in Task 5.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test model::join::`
Expected: PASS — 5 tests.

- [ ] **Step 5: Commit**

```bash
git add src/model/join.rs src/model/mod.rs
git commit -m "feat: join agents onto beads and render configured badges"
```

---

### Task 7: Anomaly rules

**Files:**
- Create: `src/model/anomaly.rs`
- Modify: `src/model/mod.rs`

**Interfaces:**
- Consumes: `model::types::{Bead, Status}`, `model::join::AgentRef`, `config::Anomalies`.
- Produces:
  - `model::anomaly::Anomaly::{StaleClaim { days: i64 }, OrphanClaim, StalePane}`
  - `model::anomaly::detect(bead: &Bead, agent: Option<&AgentRef>, cfg: &Anomalies, now: DateTime<Utc>) -> Option<Anomaly>`

`now` is a parameter, not `Utc::now()` inside, so the age rule is testable. Precedence when several could fire: `StalePane`, then `OrphanClaim`, then `StaleClaim`. A blocked bead with a live pane yields nothing — an agent parked on blocked work is normal.

- [ ] **Step 1: Write the failing test**

Create `src/model/anomaly.rs`:

```rust
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::config::Anomalies;
use crate::model::join::AgentRef;
use crate::model::types::{Bead, Status};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "rule", rename_all = "kebab-case")]
pub enum Anomaly {
    /// in_progress and untouched for longer than the configured window.
    StaleClaim { days: i64 },
    /// in_progress with no live pane behind it.
    OrphanClaim,
    /// Closed, but its pane is still alive.
    StalePane,
}

pub fn detect(
    bead: &Bead,
    agent: Option<&AgentRef>,
    cfg: &Anomalies,
    now: DateTime<Utc>,
) -> Option<Anomaly> {
    if bead.status.is_closed() {
        return agent.map(|_| Anomaly::StalePane);
    }

    if bead.status != Status::InProgress {
        // A live pane on a blocked or open bead is ordinary.
        return None;
    }

    if agent.is_none() {
        return Some(Anomaly::OrphanClaim);
    }

    let updated = bead.updated_at?;
    let days = (now - updated).num_days();
    (days >= cfg.stale_claim_days).then_some(Anomaly::StaleClaim { days })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::bd::parse_dep_tree;
    use crate::collect::herdr::PaneStatus;
    use crate::model::join::JoinSource;

    const BEADS: &str = include_str!("../../tests/fixtures/bd_dep_tree.json");

    fn bead(id: &str) -> Bead {
        parse_dep_tree(BEADS).unwrap().into_iter().find(|b| b.id == id).unwrap()
    }

    fn live() -> AgentRef {
        AgentRef {
            pane: "wCM:p1".into(),
            pane_status: PaneStatus::Working,
            title: None,
            source: JoinSource::AgentPane,
        }
    }

    fn now() -> DateTime<Utc> {
        "2026-08-30T12:00:00Z".parse().unwrap()
    }

    #[test]
    fn closed_bead_with_a_live_pane_is_a_stale_pane() {
        let got = detect(&bead("nix-1.20"), Some(&live()), &Anomalies::default(), now());
        assert_eq!(got, Some(Anomaly::StalePane));
    }

    #[test]
    fn closed_bead_with_no_pane_is_fine() {
        let got = detect(&bead("nix-1.20"), None, &Anomalies::default(), now());
        assert_eq!(got, None);
    }

    #[test]
    fn in_progress_with_no_pane_is_an_orphan_claim() {
        let got = detect(&bead("nix-1.16"), None, &Anomalies::default(), now());
        assert_eq!(got, Some(Anomaly::OrphanClaim));
    }

    #[test]
    fn in_progress_and_long_untouched_is_a_stale_claim() {
        // nix-1.16 was last updated 2026-07-01; that is 60 days before now().
        let got = detect(&bead("nix-1.16"), Some(&live()), &Anomalies::default(), now());
        assert_eq!(got, Some(Anomaly::StaleClaim { days: 60 }));
    }

    #[test]
    fn a_recent_in_progress_claim_with_a_pane_is_fine() {
        let mut b = bead("nix-1.16");
        b.updated_at = Some("2026-08-29T12:00:00Z".parse().unwrap());

        let got = detect(&b, Some(&live()), &Anomalies::default(), now());
        assert_eq!(got, None);
    }

    #[test]
    fn a_blocked_bead_with_a_live_pane_is_never_flagged() {
        // This is a sleeping agent, which is a normal state.
        let got = detect(&bead("nix-1.1"), Some(&live()), &Anomalies::default(), now());
        assert_eq!(got, None);
    }

    #[test]
    fn the_age_window_is_configurable() {
        let cfg = Anomalies { stale_claim_days: 90 };
        let got = detect(&bead("nix-1.16"), Some(&live()), &cfg, now());
        assert_eq!(got, None, "60 days is inside a 90-day window");
    }
}
```

Add to `src/model/mod.rs`:

```rust
pub mod anomaly;
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test model::anomaly::`
Expected: FAIL — module not declared on the first run.

- [ ] **Step 3: Write minimal implementation**

The Step 1 code is the implementation. `Status` needs `PartialEq` for the `!=` comparison; that derive is already in Task 3.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test model::anomaly::`
Expected: PASS — 7 tests.

- [ ] **Step 5: Commit**

```bash
git add src/model/anomaly.rs src/model/mod.rs
git commit -m "feat: anomaly rules for stale panes, orphan claims and stale claims"
```

---

### Task 8: Assemble the snapshot, with counts and the live-agent filter

**Files:**
- Create: `src/model/snapshot.rs`
- Modify: `src/model/mod.rs`

**Interfaces:**
- Consumes: everything from Tasks 2 to 7.
- Produces:
  - `model::snapshot::{Snapshot, Tree, Node, Counts, TrackerState, HerdrState, Filter, HiddenTree, LoosePane}`
  - `model::snapshot::build_tree(project, root_id, assembled, panes, cfg, now) -> Tree`
  - `model::snapshot::build(trees: Vec<Tree>, panes: &[Pane], herdr: HerdrState, filter: Filter, now) -> Snapshot`

`build` partitions trees into `trees` and `hidden_trees` by the filter. A tree with an unreachable tracker is **never** hidden — it has no nodes to count agents from, so hiding it would be indistinguishable from having no work.

- [ ] **Step 1: Write the failing test**

Create `src/model/snapshot.rs`:

```rust
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::collect::herdr::{Pane, PaneStatus};
use crate::config::Config;
use crate::model::anomaly::{self, Anomaly};
use crate::model::join::{self, AgentRef, Badged};
use crate::model::tree::Assembled;
use crate::model::types::Status;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HerdrState {
    Ok,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Filter {
    LiveAgents,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "state")]
pub enum TrackerState {
    Ok,
    Unreachable { error: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Counts {
    pub total: usize,
    pub closed: usize,
    pub live_agents: usize,
    pub anomalies: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Node {
    pub id: String,
    pub title: String,
    pub status: Status,
    pub priority: u8,
    pub depth: u16,
    /// Open, with every dependency satisfied. From `bd ready`, which is the
    /// only thing that knows — a tree row carries its tree parent, not its
    /// full blocker set.
    pub ready: bool,
    pub edge: Option<crate::model::types::Edge>,
    pub badges: Vec<Badged>,
    pub agent: Option<AgentRef>,
    pub anomaly: Option<Anomaly>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Tree {
    pub project: String,
    pub root: String,
    pub title: String,
    pub counts: Counts,
    #[serde(flatten)]
    pub tracker: TrackerState,
    pub nodes: Vec<Node>,
    /// Ids whose declared parent was absent from the tracker's answer.
    pub dangling: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HiddenTree {
    pub project: String,
    pub root: String,
    pub title: String,
    pub reason: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LoosePane {
    pub pane: String,
    pub cwd: String,
    pub pane_status: PaneStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Snapshot {
    pub generated_at: DateTime<Utc>,
    pub herdr: HerdrState,
    pub filter: Filter,
    pub trees: Vec<Tree>,
    pub hidden_trees: Vec<HiddenTree>,
    pub unattributed: Vec<LoosePane>,
}

pub fn build_tree(
    project: &str,
    assembled: Assembled,
    panes: &[Pane],
    cfg: &Config,
    now: DateTime<Utc>,
) -> Tree {
    let joined = join::resolve(&assembled.rows, panes, &cfg.join);

    let nodes: Vec<Node> = assembled
        .rows
        .iter()
        .map(|placed| {
            let agent = joined.get(&placed.bead.id).cloned();
            let anomaly = anomaly::detect(&placed.bead, agent.as_ref(), &cfg.anomalies, now);
            Node {
                id: placed.bead.id.clone(),
                title: placed.bead.title.clone(),
                status: placed.bead.status.clone(),
                priority: placed.bead.priority,
                depth: placed.depth,
                edge: placed.bead.edge_from_parent.clone(),
                badges: join::badges_for(&placed.bead, &cfg.badges),
                agent,
                anomaly,
                truncated: placed.bead.truncated,
            }
        })
        .collect();

    let counts = Counts {
        total: nodes.len(),
        closed: nodes.iter().filter(|n| n.status.is_closed()).count(),
        live_agents: nodes.iter().filter(|n| n.agent.is_some()).count(),
        anomalies: nodes.iter().filter(|n| n.anomaly.is_some()).count(),
    };

    let (root, title) = nodes
        .first()
        .map(|n| (n.id.clone(), n.title.clone()))
        .unwrap_or_default();

    Tree {
        project: project.to_string(),
        root,
        title,
        counts,
        tracker: TrackerState::Ok,
        nodes,
        dangling: assembled.dangling,
    }
}

pub fn build(
    trees: Vec<Tree>,
    panes: &[Pane],
    herdr: HerdrState,
    filter: Filter,
    now: DateTime<Utc>,
) -> Snapshot {
    let (shown, hidden): (Vec<Tree>, Vec<Tree>) = match (herdr, filter) {
        (HerdrState::Ok, Filter::LiveAgents) => trees.into_iter().partition(|t| {
            t.counts.live_agents > 0 || matches!(t.tracker, TrackerState::Unreachable { .. })
        }),
        _ => (trees, Vec::new()),
    };

    let claimed = shown
        .iter()
        .chain(hidden.iter())
        .flat_map(|t| t.nodes.iter())
        .filter_map(|n| n.agent.as_ref().map(|a| (n.id.clone(), a.clone())))
        .collect();

    let unattributed = join::unattributed(panes, &claimed)
        .into_iter()
        .map(|p| LoosePane {
            pane: p.pane_id.clone(),
            cwd: p.cwd.display().to_string(),
            pane_status: p.agent_status.clone(),
        })
        .collect();

    Snapshot {
        generated_at: now,
        herdr,
        filter,
        trees: shown,
        hidden_trees: hidden
            .into_iter()
            .map(|t| HiddenTree {
                project: t.project,
                root: t.root,
                title: t.title,
                reason: "no-live-agent",
            })
            .collect(),
        unattributed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::bd::parse_dep_tree;
    use crate::collect::herdr::parse_agent_list;
    use crate::model::tree::assemble;

    const BEADS: &str = include_str!("../../tests/fixtures/bd_dep_tree.json");
    const PANES: &str = include_str!("../../tests/fixtures/herdr_agent_list.json");

    fn cfg() -> Config {
        Config::from_toml(
            r#"
[[projects]]
name = "summit-works"
path = "/tmp/bdi-ground/summit-works"

[[badges]]
key = "blocked_on"
match = "human"
render = "waiting"
"#,
        )
        .unwrap()
    }

    fn now() -> DateTime<Utc> {
        "2026-08-30T12:00:00Z".parse().unwrap()
    }

    fn tree() -> Tree {
        let assembled = assemble(parse_dep_tree(BEADS).unwrap());
        let panes = parse_agent_list(PANES).unwrap();
        build_tree("summit-works", assembled, &panes, &cfg(), now())
    }

    #[test]
    fn counts_come_from_the_nodes() {
        let t = tree();
        assert_eq!(t.counts.total, 6);
        assert_eq!(t.counts.closed, 1);
        assert_eq!(t.counts.live_agents, 3);
    }

    #[test]
    fn the_root_is_the_first_node() {
        let t = tree();
        assert_eq!(t.root, "nix-1");
        assert_eq!(t.nodes[0].depth, 0);
    }

    #[test]
    fn badges_reach_the_node() {
        let t = tree();
        let n = t.nodes.iter().find(|n| n.id == "nix-1.1").unwrap();
        assert_eq!(n.badges.len(), 1);
        assert_eq!(n.badges[0].text, "waiting");
    }

    #[test]
    fn anomalies_reach_the_node() {
        let t = tree();
        let stale = t.nodes.iter().find(|n| n.id == "nix-1.20").unwrap();
        assert_eq!(stale.anomaly, Some(Anomaly::StalePane));
    }

    #[test]
    fn the_filter_hides_a_tree_with_no_live_agent() {
        let mut empty = tree();
        empty.project = "other".into();
        empty.root = "oth-1".into();
        empty.counts.live_agents = 0;
        empty.nodes.iter_mut().for_each(|n| n.agent = None);

        let panes = parse_agent_list(PANES).unwrap();
        let snap = build(vec![tree(), empty], &panes, HerdrState::Ok, Filter::LiveAgents, now());

        assert_eq!(snap.trees.len(), 1);
        assert_eq!(snap.hidden_trees.len(), 1);
        assert_eq!(snap.hidden_trees[0].root, "oth-1");
        assert_eq!(snap.hidden_trees[0].reason, "no-live-agent");
    }

    #[test]
    fn an_unreachable_tracker_is_never_hidden() {
        let mut broken = tree();
        broken.root = "oth-1".into();
        broken.counts.live_agents = 0;
        broken.nodes.clear();
        broken.tracker = TrackerState::Unreachable { error: "access denied".into() };

        let panes = parse_agent_list(PANES).unwrap();
        let snap = build(vec![broken], &panes, HerdrState::Ok, Filter::LiveAgents, now());

        assert_eq!(snap.trees.len(), 1, "a tracker we could not read must still be visible");
        assert!(snap.hidden_trees.is_empty());
    }

    #[test]
    fn without_herdr_nothing_is_filtered() {
        let mut empty = tree();
        empty.counts.live_agents = 0;
        empty.nodes.iter_mut().for_each(|n| n.agent = None);

        let snap = build(vec![empty], &[], HerdrState::Unavailable, Filter::LiveAgents, now());
        assert_eq!(snap.trees.len(), 1);
        assert!(snap.hidden_trees.is_empty());
    }

    #[test]
    fn a_pane_on_no_bead_lands_in_unattributed() {
        let panes = parse_agent_list(PANES).unwrap();
        let snap = build(vec![tree()], &panes, HerdrState::Ok, Filter::LiveAgents, now());

        assert_eq!(snap.unattributed.len(), 1);
        assert_eq!(snap.unattributed[0].pane, "wCM:pD");
    }
}
```

Add to `src/model/mod.rs`:

```rust
pub mod anomaly;
pub mod join;
pub mod snapshot;
pub mod tree;
pub mod types;
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test model::snapshot::`
Expected: FAIL — module not declared on the first run.

- [ ] **Step 3: Write minimal implementation**

The Step 1 code is the implementation. `Status` and `Edge` need `Clone`; both derives are in Task 3. `AgentRef` and `Badged` need `Clone`; both derives are in Task 6.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test model::snapshot::`
Expected: PASS — 8 tests.

- [ ] **Step 5: Commit**

```bash
git add src/model/snapshot.rs src/model/mod.rs
git commit -m "feat: assemble snapshots with counts, badges, anomalies and the live-agent filter"
```

---

### Task 9: Run bd and herdr, per project, degrading on failure

**Files:**
- Modify: `src/collect/bd.rs`
- Modify: `src/collect/herdr.rs`
- Create: `src/collect/run.rs`
- Modify: `src/collect/mod.rs`

**Interfaces:**
- Consumes: `config::Project`.
- Produces:
  - `collect::run::{Runner, RealRunner, Output}`
  - `Runner::run(&self, program: &str, args: &[&str], cwd: Option<&Path>) -> anyhow::Result<String>`
  - `collect::bd::{dep_tree, discover_roots}` taking `&dyn Runner`
  - `collect::herdr::agent_list(&dyn Runner) -> anyhow::Result<Vec<Pane>>`

`Runner` is a trait so every call site is testable without spawning anything. Each `bd` invocation is given the project's directory via `-C`, which is how bd finds that project's credential environment. A non-zero exit is an error carrying stderr — the caller turns it into `TrackerState::Unreachable` rather than aborting.

- [ ] **Step 1: Write the failing test**

Create `src/collect/run.rs`:

```rust
use std::path::Path;
use std::process::Command;

pub trait Runner {
    fn run(&self, program: &str, args: &[&str], cwd: Option<&Path>) -> anyhow::Result<String>;
}

pub struct RealRunner;

impl Runner for RealRunner {
    fn run(&self, program: &str, args: &[&str], cwd: Option<&Path>) -> anyhow::Result<String> {
        let mut cmd = Command::new(program);
        cmd.args(args);
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        let out = cmd.output()?;
        if !out.status.success() {
            anyhow::bail!(
                "{program} exited {}: {}",
                out.status.code().unwrap_or(-1),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8(out.stdout)?)
    }
}

#[cfg(test)]
pub mod testing {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// A Runner that replays canned output keyed by the joined argv.
    #[derive(Default)]
    pub struct FakeRunner {
        pub responses: HashMap<String, anyhow::Result<String>>,
        pub calls: Mutex<Vec<String>>,
    }

    impl FakeRunner {
        pub fn with(mut self, argv: &str, out: &str) -> Self {
            self.responses.insert(argv.to_string(), Ok(out.to_string()));
            self
        }

        pub fn failing(mut self, argv: &str, err: &str) -> Self {
            self.responses
                .insert(argv.to_string(), Err(anyhow::anyhow!("{}", err)));
            self
        }
    }

    impl Runner for FakeRunner {
        fn run(&self, program: &str, args: &[&str], _cwd: Option<&Path>) -> anyhow::Result<String> {
            let key = format!("{program} {}", args.join(" "));
            self.calls.lock().unwrap().push(key.clone());
            match self.responses.get(&key) {
                Some(Ok(s)) => Ok(s.clone()),
                Some(Err(e)) => Err(anyhow::anyhow!("{e}")),
                None => anyhow::bail!("FakeRunner has no response for: {key}"),
            }
        }
    }
}
```

Append to `src/collect/bd.rs`:

```rust
use std::collections::BTreeSet;
use std::path::Path;

use crate::collect::run::Runner;

/// `bd dep tree <root> --direction=up --json`, run in the project's directory
/// so bd picks up that project's credential environment.
pub fn dep_tree(runner: &dyn Runner, cwd: &Path, root: &str) -> anyhow::Result<Vec<Bead>> {
    let out = runner.run(
        "bd",
        &["dep", "tree", root, "--direction=up", "--json"],
        Some(cwd),
    )?;
    parse_dep_tree(&out)
}

/// Ids that beads considers ready: open, with every dependency satisfied.
/// The tree JSON cannot answer this — a row carries only its tree parent, not
/// its blockers — so we ask bd, which already computes it.
pub fn ready_ids(runner: &dyn Runner, cwd: &Path) -> anyhow::Result<BTreeSet<String>> {
    let out = runner.run("bd", &["ready", "--limit", "0", "--json"], Some(cwd))?;
    Ok(parse_dep_tree(&out)?.into_iter().map(|b| b.id).collect())
}

/// Beads that mark live work: bd's own in-flight statuses, plus any bead
/// carrying one of the configured metadata keys.
pub fn discover_roots(
    runner: &dyn Runner,
    cwd: &Path,
    metadata_keys: &[String],
) -> anyhow::Result<Vec<Bead>> {
    let mut found: Vec<Bead> = Vec::new();

    for status in ["in_progress", "blocked"] {
        let out = runner.run(
            "bd",
            &["list", "--status", status, "--limit", "0", "--json"],
            Some(cwd),
        )?;
        found.extend(parse_dep_tree(&out)?);
    }

    for key in metadata_keys {
        let out = runner.run(
            "bd",
            &["list", "--has-metadata-key", key, "--limit", "0", "--json"],
            Some(cwd),
        )?;
        found.extend(parse_dep_tree(&out)?);
    }

    found.sort_by(|a, b| a.id.cmp(&b.id));
    found.dedup_by(|a, b| a.id == b.id);
    Ok(found)
}
```

Append to `src/collect/bd.rs`'s test module:

```rust
    use crate::collect::run::testing::FakeRunner;
    use std::path::PathBuf;

    #[test]
    fn dep_tree_asks_bd_in_the_projects_directory() {
        let runner = FakeRunner::default()
            .with("bd dep tree nix-1 --direction=up --json", FIXTURE);

        let beads = dep_tree(&runner, &PathBuf::from("/tmp/proj"), "nix-1").unwrap();
        assert_eq!(beads.len(), 6);
    }

    #[test]
    fn discovery_unions_statuses_and_metadata_keys_without_duplicates() {
        let one = r#"[{"id":"nix-1.16","title":"a","status":"in_progress","priority":3,"metadata":{},"truncated":false}]"#;
        let two = r#"[{"id":"nix-1.1","title":"b","status":"blocked","priority":2,"metadata":{},"truncated":false}]"#;
        // The same bead comes back from the metadata query as from the status query.
        let three = r#"[{"id":"nix-1.16","title":"a","status":"in_progress","priority":3,"metadata":{},"truncated":false}]"#;

        let runner = FakeRunner::default()
            .with("bd list --status in_progress --limit 0 --json", one)
            .with("bd list --status blocked --limit 0 --json", two)
            .with("bd list --has-metadata-key working_topic --limit 0 --json", three);

        let got = discover_roots(
            &runner,
            &PathBuf::from("/tmp/proj"),
            &["working_topic".to_string()],
        )
        .unwrap();

        let ids: Vec<&str> = got.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, vec!["nix-1.1", "nix-1.16"]);
    }

    #[test]
    fn ready_ids_returns_the_set_bd_considers_startable() {
        let out = r#"[{"id":"nix-1.1","title":"a","status":"open","priority":2,"metadata":{},"truncated":false},
                      {"id":"nix-1.3","title":"b","status":"open","priority":1,"metadata":{},"truncated":false}]"#;
        let runner = FakeRunner::default().with("bd ready --limit 0 --json", out);

        let got = ready_ids(&runner, &PathBuf::from("/tmp/proj")).unwrap();
        assert!(got.contains("nix-1.1"));
        assert!(got.contains("nix-1.3"));
        assert!(!got.contains("nix-1.4"), "a blocked bead is not ready");
    }

    #[test]
    fn a_failing_bd_surfaces_its_stderr() {
        let runner = FakeRunner::default()
            .failing("bd dep tree nix-1 --direction=up --json", "Access denied for user 'other'");

        let err = dep_tree(&runner, &PathBuf::from("/tmp/proj"), "nix-1").unwrap_err();
        assert!(err.to_string().contains("Access denied"), "got: {err}");
    }
```

Append to `src/collect/herdr.rs`:

```rust
use crate::collect::run::Runner;

/// `herdr agent list`. A failure here is not fatal — the caller degrades to
/// the bd-only tier.
pub fn agent_list(runner: &dyn Runner) -> anyhow::Result<Vec<Pane>> {
    let out = runner.run("herdr", &["agent", "list"], None)?;
    parse_agent_list(&out)
}
```

Append to `src/collect/herdr.rs`'s test module:

```rust
    use crate::collect::run::testing::FakeRunner;

    #[test]
    fn agent_list_shells_out_and_parses() {
        let runner = FakeRunner::default().with("herdr agent list", FIXTURE);
        assert_eq!(agent_list(&runner).unwrap().len(), 4);
    }

    #[test]
    fn a_missing_herdr_is_an_error_the_caller_can_degrade_on() {
        let runner = FakeRunner::default().failing("herdr agent list", "no such file");
        assert!(agent_list(&runner).is_err());
    }
```

Add to `src/collect/mod.rs`:

```rust
pub mod run;
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test collect::`
Expected: FAIL — `collect::run` is not declared, and `FakeRunner` is unresolved in the two test modules.

- [ ] **Step 3: Write minimal implementation**

The Step 1 code is the implementation. Note `#[cfg(test)] pub mod testing` inside `run.rs` — `FakeRunner` must not ship in the binary, and other modules' `#[cfg(test)]` blocks can still reach it.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test collect::`
Expected: PASS — 5 bd tests plus 3 new, 4 herdr tests plus 2 new.

- [ ] **Step 5: Commit**

```bash
git add src/collect
git commit -m "feat: run bd per project and herdr once, behind a testable Runner"
```

---

### Task 10: Wire the binary and emit the JSON contract

**Files:**
- Modify: `src/main.rs`
- Create: `src/app.rs`
- Modify: `src/lib.rs`
- Create: `tests/snapshot_json.rs`

**Interfaces:**
- Consumes: everything above.
- Produces:
  - `app::run(cfg: &Config, runner: &dyn Runner, filter: Filter, now) -> Snapshot`
  - `bdi --json`, `bdi --config <path>`, `bdi --all`

`app::run` walks each configured project: discover roots, walk each root to its topmost `parent-child` ancestor, fetch its tree, build it. A project whose `bd` call fails contributes one `Tree` with `TrackerState::Unreachable` rather than aborting the run. A herdr failure downgrades the whole snapshot to `HerdrState::Unavailable`.

- [ ] **Step 1: Write the failing test**

Create `src/app.rs`:

```rust
use std::collections::BTreeSet;

use chrono::{DateTime, Utc};

use crate::collect::run::Runner;
use crate::collect::{bd, herdr};
use crate::config::Config;
use crate::model::snapshot::{
    self, Counts, Filter, HerdrState, Snapshot, TrackerState, Tree,
};
use crate::model::tree::assemble;

/// Walk a bead id to its topmost ancestor by trimming `.N` suffixes is NOT
/// safe — ids are opaque. We ask bd instead: the tree walk from any bead
/// reaches its root, so we take the row whose `parent_id` is absent.
fn root_of(runner: &dyn Runner, cwd: &std::path::Path, id: &str) -> anyhow::Result<String> {
    let rows = bd::dep_tree(runner, cwd, id)?;
    Ok(rows
        .iter()
        .find(|b| b.parent_id.is_none())
        .map(|b| b.id.clone())
        .unwrap_or_else(|| id.to_string()))
}

pub fn run(
    cfg: &Config,
    runner: &dyn Runner,
    filter: Filter,
    now: DateTime<Utc>,
) -> Snapshot {
    let (panes, herdr_state) = match herdr::agent_list(runner) {
        Ok(p) => (p, HerdrState::Ok),
        Err(_) => (Vec::new(), HerdrState::Unavailable),
    };

    let mut trees: Vec<Tree> = Vec::new();

    for project in &cfg.projects {
        let discovered = match bd::discover_roots(runner, &project.path, &cfg.roots.metadata_keys) {
            Ok(d) => d,
            Err(e) => {
                trees.push(unreachable_tree(&project.name, &e.to_string()));
                continue;
            }
        };

        let mut roots: BTreeSet<String> = cfg.roots.explicit.iter().cloned().collect();
        for bead in &discovered {
            match root_of(runner, &project.path, &bead.id) {
                Ok(r) => {
                    roots.insert(r);
                }
                Err(e) => trees.push(unreachable_tree(&project.name, &e.to_string())),
            }
        }

        for root in roots {
            match bd::dep_tree(runner, &project.path, &root) {
                Ok(rows) => trees.push(snapshot::build_tree(
                    &project.name,
                    assemble(rows),
                    &panes,
                    cfg,
                    now,
                )),
                Err(e) => trees.push(unreachable_tree(&project.name, &e.to_string())),
            }
        }
    }

    snapshot::build(trees, &panes, herdr_state, filter, now)
}

fn unreachable_tree(project: &str, error: &str) -> Tree {
    Tree {
        project: project.to_string(),
        root: String::new(),
        title: String::new(),
        counts: Counts { total: 0, closed: 0, live_agents: 0, anomalies: 0 },
        tracker: TrackerState::Unreachable { error: error.to_string() },
        nodes: Vec::new(),
        dangling: Vec::new(),
    }
}
```

Create `tests/snapshot_json.rs`:

```rust
use beady_eye::collect::run::Runner;
use beady_eye::config::Config;
use beady_eye::model::snapshot::Filter;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::path::Path;

const BEADS: &str = include_str!("fixtures/bd_dep_tree.json");
const PANES: &str = include_str!("fixtures/herdr_agent_list.json");

struct Canned(HashMap<String, String>);

impl Runner for Canned {
    fn run(&self, program: &str, args: &[&str], _cwd: Option<&Path>) -> anyhow::Result<String> {
        let key = format!("{program} {}", args.join(" "));
        self.0
            .get(&key)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no canned response for {key}"))
    }
}

fn now() -> DateTime<Utc> {
    "2026-08-30T12:00:00Z".parse().unwrap()
}

fn canned() -> Canned {
    let empty = "[]";
    let in_progress = r#"[{"id":"nix-1.16","title":"a","status":"in_progress","priority":3,"metadata":{},"truncated":false}]"#;

    let mut m = HashMap::new();
    m.insert("herdr agent list".into(), PANES.to_string());
    m.insert("bd list --status in_progress --limit 0 --json".into(), in_progress.to_string());
    m.insert("bd list --status blocked --limit 0 --json".into(), empty.to_string());
    m.insert("bd dep tree nix-1.16 --direction=up --json".into(), BEADS.to_string());
    m.insert("bd dep tree nix-1 --direction=up --json".into(), BEADS.to_string());
    Canned(m)
}

fn cfg() -> Config {
    Config::from_toml(
        r#"
[[projects]]
name = "summit-works"
path = "/tmp/bdi-ground/summit-works"
"#,
    )
    .unwrap()
}

#[test]
fn emits_one_tree_rooted_at_the_discovered_epic() {
    let snap = beady_eye::app::run(&cfg(), &canned(), Filter::LiveAgents, now());

    assert_eq!(snap.trees.len(), 1);
    assert_eq!(snap.trees[0].root, "nix-1");
    assert_eq!(snap.trees[0].nodes.len(), 6);
}

#[test]
fn the_json_carries_the_contract_fields() {
    let snap = beady_eye::app::run(&cfg(), &canned(), Filter::LiveAgents, now());
    let v: serde_json::Value = serde_json::to_value(&snap).unwrap();

    assert_eq!(v["herdr"], "ok");
    assert_eq!(v["filter"], "live-agents");
    assert!(v["trees"].is_array());
    assert!(v["hidden_trees"].is_array());
    assert!(v["unattributed"].is_array());

    let node = &v["trees"][0]["nodes"][0];
    for key in ["id", "title", "status", "priority", "depth", "badges", "agent", "anomaly"] {
        assert!(node.get(key).is_some(), "node is missing {key}");
    }
}

#[test]
fn a_project_whose_tracker_fails_is_reported_not_dropped() {
    let mut m = canned();
    m.0.remove("bd list --status in_progress --limit 0 --json");

    let snap = beady_eye::app::run(&cfg(), &m, Filter::LiveAgents, now());

    assert_eq!(snap.trees.len(), 1);
    let v = serde_json::to_value(&snap.trees[0]).unwrap();
    assert_eq!(v["state"], "unreachable");
}

#[test]
fn without_herdr_the_snapshot_says_so_and_still_renders() {
    let mut m = canned();
    m.0.remove("herdr agent list");

    let snap = beady_eye::app::run(&cfg(), &m, Filter::LiveAgents, now());

    assert_eq!(serde_json::to_value(&snap).unwrap()["herdr"], "unavailable");
    assert_eq!(snap.trees.len(), 1, "trees still render without liveness");
}
```

Replace `src/main.rs`:

```rust
use std::path::PathBuf;

use anyhow::Context;
use chrono::Utc;
use clap::Parser;

use beady_eye::collect::run::RealRunner;
use beady_eye::config::Config;
use beady_eye::model::snapshot::Filter;

#[derive(Parser)]
#[command(name = "bdi", version, about = "A tree of work in flight")]
struct Cli {
    /// Path to the config file.
    #[arg(long, default_value = "~/.config/beady-eye/config.toml")]
    config: String,

    /// Emit JSON rather than the interactive view.
    #[arg(long)]
    json: bool,

    /// Include trees with no live agent.
    #[arg(long)]
    all: bool,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let path = expand_tilde(&cli.config);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("reading config at {}", path.display()))?;
    let cfg = Config::from_toml(&text)?;

    let filter = if cli.all { Filter::All } else { Filter::LiveAgents };
    let snap = beady_eye::app::run(&cfg, &RealRunner, filter, Utc::now());

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&snap)?);
    } else {
        // The interactive view arrives in the next plan; until then, --json is
        // the only output, and saying so beats printing nothing.
        eprintln!("the interactive view is not built yet; re-run with --json");
        std::process::exit(2);
    }
    Ok(())
}

fn expand_tilde(s: &str) -> PathBuf {
    match s.strip_prefix("~/") {
        Some(rest) => match std::env::var_os("HOME") {
            Some(home) => PathBuf::from(home).join(rest),
            None => PathBuf::from(s),
        },
        None => PathBuf::from(s),
    }
}
```

Add to `src/lib.rs`:

```rust
pub mod app;
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test snapshot_json`
Expected: FAIL — `beady_eye::app` does not resolve on the first run.

- [ ] **Step 3: Write minimal implementation**

The Step 1 code is the implementation. Two things to watch: `tests/snapshot_json.rs` is an integration test so it can only use `pub` items, which is why `Runner` and `app::run` are public; and `Tree`'s `#[serde(flatten)]` on `TrackerState` is what puts `"state":"ok"` at the tree's top level rather than nesting it.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test`
Expected: PASS — every unit test plus 4 integration tests.

Then run `cargo clippy -- -D warnings` and `cargo fmt --check`; fix anything they report.

- [ ] **Step 5: Commit**

```bash
git add src/app.rs src/main.rs src/lib.rs tests/snapshot_json.rs
git commit -m "feat: wire the bdi binary and emit the JSON contract"
```

---

## What this plan deliberately leaves out

- **The TUI.** It gets its own plan, written against the types this one produces rather than against a guess at them.
- **The Noctalia widget.** Deferred behind the contract, per the design.
- **`herdr agent read` / `focus`.** Both belong to the TUI.
- **Real credentials.** Every test drives a `Runner` fake. Proving the per-project credential path against a live multi-tracker server is the first task of a follow-up, and the design names it as the leading risk.

## Self-review

**Spec coverage.** Discovery (Task 9), conventions-as-configuration (Tasks 2, 6), the two-tier degradation (Tasks 9, 10), the default filter (Task 8), tree assembly and dedup (Task 4), the join in both directions (Task 6), all four anomaly rules — `stale-claim`, `orphan-claim`, `stale-pane` in Task 7 and `unattributed` in Tasks 6 and 8 — and the JSON contract (Tasks 8, 10). The `(project, id)` rule is carried by `Tree.project` scoping every node.

**Gap, stated rather than hidden.** The design's `agent.source` distinction is implemented, but nothing yet *renders* the "inferred rather than confirmed" caveat to a reader — that lands with the TUI.

**Type consistency.** `Status`, `Edge`, `Bead` (Task 3) are used unchanged in Tasks 4, 6, 7, 8. `Placed`/`Assembled` (Task 4) feed Tasks 6 and 8. `AgentRef`/`Badged`/`JoinSource` (Task 6) feed Tasks 7 and 8. `Runner` (Task 9) is taken by `bd::dep_tree`, `bd::discover_roots`, `herdr::agent_list` and `app::run` with one signature.
