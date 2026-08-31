//! A `bd` a test can make hang, so a collection can be held outstanding.
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
    /// finds. First on PATH, so they shadow the real ones and hand on to them.
    pub fn environment(&self) -> Vec<(String, String)> {
        let inherited = std::env::var("PATH").unwrap_or_default();
        vec![
            ("PATH".to_string(), format!("{SHIMS}:{inherited}")),
            (
                "BDI_SHIM_BD_HANGS_WHILE".to_string(),
                self.hangs_while.display().to_string(),
            ),
            (
                "BDI_SHIM_BD_HOLDING".to_string(),
                self.holding.display().to_string(),
            ),
        ]
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
            shim_by_hand()
        );
    }
}

/// What running the `bd` shim outside `bdi` does, for a failure to say with.
///
/// A shim `bdi` could not run at all looks exactly like a `bdi` that never
/// asked for a collection — both are simply no call — and this is what tells
/// them apart. It cost a `nix flake check` to learn that a sandbox holds
/// `/bin/sh` and no `/usr/bin/env`, from a failure that said only that
/// nothing was held.
fn shim_by_hand() -> String {
    match std::process::Command::new(format!("{SHIMS}/bd"))
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
