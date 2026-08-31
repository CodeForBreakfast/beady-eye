//! A `bd` a test can make hang and a `herdr` it can answer for, so a
//! collection or a pane read can be held outstanding.
//!
//! `bdi` reaches `bd` and `herdr` through `Command::new` — a PATH lookup
//! (src/collect/run.rs) — so a script earlier on PATH can hold either of them
//! up without `bdi` knowing there is a test at all. That is what makes a hung
//! tracker reachable: it is otherwise a state only a broken network produces,
//! and it is the one state that tells a stalled loop from a slow one.
//!
//! The scripts themselves are `tests/shims/`, so a person debugging by hand
//! runs the same ones the suite does. `tests/shims/bd --help` is not a thing;
//! read the comment at the top of the file.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Where the scripts live. The tests run from the built binary's directory
/// rather than the source tree, so this is the one thing that has to be
/// written down.
const SHIMS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/shims");

/// A `bd` that answers as it always did until it is told to hang, and then
/// holds every call until it is dropped.
pub struct ShimmedTracker {
    hangs_while: PathBuf,
    holding: PathBuf,
}

impl ShimmedTracker {
    /// A tracker answering normally, whose scripts sit under `beside`.
    pub fn beside(beside: &Path) -> Self {
        Self {
            hangs_while: beside.join("bd-hangs"),
            holding: beside.join("bd-holding"),
        }
    }

    /// What `bdi` has to run with for the shims to be the `bd` and `herdr` it
    /// finds.
    pub fn environment(&self) -> Vec<(String, String)> {
        let mut environment = vec![shims_first_on_path()];
        environment.push((
            "BDI_SHIM_BD_HANGS_WHILE".to_string(),
            self.hangs_while.display().to_string(),
        ));
        environment.push((
            "BDI_SHIM_BD_HOLDING".to_string(),
            self.holding.display().to_string(),
        ));
        environment
    }

    /// Stop answering. Every `bd` call from here waits until this is dropped.
    pub fn hang(&self) {
        std::fs::write(&self.hangs_while, "").expect("the flag is ours to raise");
    }

    /// Wait until a call is actually being held.
    ///
    /// The screen cannot be asked this. `bdi` redraws its foot when a
    /// collection starts, but the terminal is written as a difference from the
    /// frame before, so "collected 12:34:56" becoming "collecting" puts three
    /// letters on the wire and no word a test could look for. The shim says it
    /// outright instead.
    pub fn wait_until_holding(&self, patience: Duration) {
        let giving_up = Instant::now() + patience;
        while Instant::now() < giving_up {
            if self.holding.exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!(
            "no bd call was held within {patience:?}, so nothing was \
             outstanding to answer a keystroke during. Run by hand the shim \
             says: {}",
            shim_by_hand("bd")
        );
    }
}

/// The shims ahead of the real programs, so they shadow them and hand on to
/// them. Both shims live in the one directory, so both are shadowed together
/// whichever of them a test came for.
pub fn shims_first_on_path() -> (String, String) {
    let inherited = std::env::var("PATH").unwrap_or_default();
    ("PATH".to_string(), format!("{SHIMS}:{inherited}"))
}

/// A `herdr` that reports the session a test wrote down, and holds its pane
/// reads on command.
///
/// A machine with no herdr is the ordinary one in a build sandbox, and a `bdi`
/// that finds none has no pane on any row to read. So the session is written
/// to a file rather than found: what is on the wire is a capture, and every
/// test that needs a pane gets the same one whatever the machine is running.
pub struct ShimmedHerdr {
    agents: PathBuf,
    visible: PathBuf,
    hangs_while: PathBuf,
    holding: PathBuf,
}

/// The one pane every test here reads. Its `cwd` is under no configured
/// project, so it is drawn among the panes `bdi` has nothing to join to —
/// which is the shortest road to a row that names a pane.
pub const A_PANE: &str = "wT:p1";

impl ShimmedHerdr {
    /// A herdr reporting one working pane, whose scripts sit under `beside`.
    pub fn beside(beside: &Path) -> Self {
        let herdr = Self {
            agents: beside.join("herdr-agents.json"),
            visible: beside.join("herdr-visible"),
            hangs_while: beside.join("herdr-hangs"),
            holding: beside.join("herdr-holding"),
        };
        std::fs::write(
            &herdr.agents,
            format!(
                r#"{{"result":{{"agents":[{{"pane_id":"{A_PANE}","cwd":"/","agent_status":"working"}}]}}}}"#
            ),
        )
        .expect("the session is ours to write");
        std::fs::write(&herdr.visible, "rebuilt .#thinkpad, generation 541\n")
            .expect("the pane is ours to write");
        herdr
    }

    /// What `bdi` has to run with for the shim to be the `herdr` it finds.
    pub fn environment(&self) -> Vec<(String, String)> {
        vec![
            shims_first_on_path(),
            (
                "BDI_SHIM_HERDR_AGENTS".to_string(),
                self.agents.display().to_string(),
            ),
            (
                "BDI_SHIM_HERDR_VISIBLE".to_string(),
                self.visible.display().to_string(),
            ),
            (
                "BDI_SHIM_HERDR_HANGS_WHILE".to_string(),
                self.hangs_while.display().to_string(),
            ),
            (
                "BDI_SHIM_HERDR_HOLDING".to_string(),
                self.holding.display().to_string(),
            ),
        ]
    }

    /// Stop answering pane reads. Every one from here waits until this is
    /// dropped or `let_go` is called.
    pub fn hang(&self) {
        std::fs::write(&self.hangs_while, "").expect("the flag is ours to raise");
    }

    /// Answer pane reads again, letting go of whichever is held.
    pub fn let_go(&self) {
        let _ = std::fs::remove_file(&self.hangs_while);
    }

    /// Whether a pane read is being held right now.
    pub fn holding(&self) -> bool {
        self.holding.exists()
    }

    /// Wait until a pane read is actually being held.
    ///
    /// The screen cannot be asked this: a `bdi` that has not yet asked herdr
    /// and one waiting on it draw the same band. The shim says it outright
    /// instead, which is what makes the wait an ordering rather than a sleep.
    pub fn wait_until_holding(&self, patience: Duration) {
        let giving_up = Instant::now() + patience;
        while Instant::now() < giving_up {
            if self.holding() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!(
            "no pane read was held within {patience:?}, so nothing was \
             outstanding to answer a keystroke during. Run by hand the shim \
             says: {}",
            shim_by_hand("herdr")
        );
    }
}

impl Drop for ShimmedHerdr {
    /// Let go of whatever is held, for the reason the tracker beside this one
    /// gives: the shim is `bdi`'s child rather than the test's.
    fn drop(&mut self) {
        self.let_go();
    }
}

/// What running the `bd` shim outside `bdi` does, for a failure to say with.
///
/// A shim `bdi` could not run at all looks exactly like a `bdi` that never
/// asked for a collection — both are simply no call — and this is what tells
/// them apart. It cost a `nix flake check` to learn that a sandbox holds
/// `/bin/sh` and no `/usr/bin/env`, from a failure that said only that
/// nothing was held.
fn shim_by_hand(named: &str) -> String {
    match std::process::Command::new(format!("{SHIMS}/{named}"))
        .arg("--version")
        .output()
    {
        Ok(ran) => format!(
            "exit {:?}, stderr {:?}",
            ran.status.code(),
            String::from_utf8_lossy(&ran.stderr)
        ),
        Err(refused) => format!("it will not run at all: {refused}"),
    }
}

impl Drop for ShimmedTracker {
    /// Let go of whatever is held. The shim is `bdi`'s child rather than the
    /// test's, so killing `bdi` does not end it; lowering the flag does.
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.hangs_while);
    }
}
