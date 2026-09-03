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

/// A `bd` that answers for the beads a test wrote down, or as it always did
/// where a test wrote none, until it is told to hang — and then holds every
/// call until it is dropped.
///
/// A machine with no tracker is the ordinary one in a build sandbox, and a
/// `bdi` that finds none has no row under any project to read. So the beads
/// are written to a file rather than found, for the reason the herdr beside
/// this one gives: what is on the wire is a capture, and every test that
/// needs a row gets the same one whatever the machine is running.
pub struct ShimmedTracker {
    answers: PathBuf,
    unanswered: PathBuf,
    hangs_while: PathBuf,
    holding: PathBuf,
    direnv_ran: PathBuf,
}

/// The statuses bd stores for work that is not finished, spelled as
/// `collect::bd` asks for them. Written out rather than reached for, so a
/// change to that call shows up here as a call the shim could not answer.
const UNFINISHED: &str = "open,in_progress,blocked,deferred";

impl ShimmedTracker {
    /// A tracker answering normally, whose scripts sit under `beside`.
    pub fn beside(beside: &Path) -> Self {
        Self {
            answers: beside.join("bd-answers"),
            unanswered: beside.join("bd-unanswered"),
            hangs_while: beside.join("bd-hangs"),
            holding: beside.join("bd-holding"),
            direnv_ran: beside.join("direnv-ran"),
        }
    }

    /// What `bdi` has to run with for the shims to be the `bd` and `herdr` it
    /// finds.
    pub fn environment(&self) -> Vec<(String, String)> {
        let mut environment = vec![shims_first_on_path()];
        environment.push((
            "BDI_SHIM_BD_ANSWERS".to_string(),
            self.answers.display().to_string(),
        ));
        environment.push((
            "BDI_SHIM_BD_UNANSWERED".to_string(),
            self.unanswered.display().to_string(),
        ));
        environment.push((
            "BDI_SHIM_BD_HANGS_WHILE".to_string(),
            self.hangs_while.display().to_string(),
        ));
        environment.push((
            "BDI_SHIM_BD_HOLDING".to_string(),
            self.holding.display().to_string(),
        ));
        environment.push((
            "BDI_SHIM_DIRENV_RAN".to_string(),
            self.direnv_ran.display().to_string(),
        ));
        environment
    }

    /// Hold these beads — a capture of `bd list --all --json` — and answer
    /// every question a collection asks from them.
    ///
    /// One answer per call, keyed by the call as `bd` is asked it, which is
    /// how the in-process doubles key theirs too. What `bd` would filter is
    /// filtered here — the unfinished rows for discovery, one row for `show`
    /// — so the shim says what the real thing would about the same beads.
    /// It holds no wisps, and reports nothing ready and nothing blocked,
    /// until a test that needs one of those says otherwise.
    pub fn holds(&self, capture: &str) {
        let rows: Vec<serde_json::Value> =
            serde_json::from_str(capture).expect("a capture of bd list --json");
        std::fs::create_dir_all(&self.answers).expect("the answers are ours to write");

        self.answers("list --all --limit 0 --json", &rows);
        let unfinished: Vec<serde_json::Value> = rows
            .iter()
            .filter(|row| row["status"] != "closed")
            .cloned()
            .collect();
        self.answers(
            &format!("list --status {UNFINISHED} --limit 0 --json"),
            &unfinished,
        );
        for row in &rows {
            let id = row["id"].as_str().expect("a bd row names its bead");
            self.answers(&format!("show {id} --json"), std::slice::from_ref(row));
        }
        self.answers("query ephemeral=true --limit 0 --json", &[]);
        self.answers("query ephemeral=true --all --limit 0 --json", &[]);
        self.answers("ready --limit 0 --json", &[]);
        self.answers("blocked --json", &[]);
    }

    /// Say `path` is a directory beads tracks, which is what a `bdi` given no
    /// config asks first: `bd where` answering is what makes the directory
    /// the one project of the run. The tracker it names is the shape a real
    /// answer has, and nothing reads it.
    pub fn tracks(&self, path: &Path) {
        std::fs::create_dir_all(&self.answers).expect("the answers are ours to write");
        let beads = path.join(".beads");
        self.answers_with(
            "where --json",
            &format!(
                r#"{{"database_path": "{}", "path": "{}", "schema_version": 1}}"#,
                beads.join("dolt").display(),
                beads.display()
            ),
        );
    }

    fn answers(&self, asked: &str, with: &[serde_json::Value]) {
        self.answers_with(asked, &serde_json::to_string(with).expect("rows serialise"));
    }

    fn answers_with(&self, asked: &str, text: &str) {
        std::fs::write(self.answers.join(asked), text).expect("the answer is ours to write");
    }

    /// Every call `direnv` was asked, spelled as it was asked. The `direnv`
    /// on PATH beside `bd` is one nobody installed, so a call here is a
    /// project `bdi` tried to enter on a machine that cannot, and an empty
    /// list is the reading that says the project was read without it.
    pub fn direnv_runs(&self) -> Vec<String> {
        std::fs::read_to_string(&self.direnv_ran)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// Every call `bd` was asked that the shim had no answer for, spelled as
    /// it was asked. Such a call went to the real `bd` instead — which,
    /// against a `HOME` with no tracker, is exactly the "nothing here" that
    /// a shim answering nothing would have produced. Empty is the only
    /// reading that says the capture was served.
    pub fn unanswered(&self) -> Vec<String> {
        std::fs::read_to_string(&self.unanswered)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
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
    #[track_caller]
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
/// them — bar `direnv`, which the suite runs without. All three live in the
/// one directory, so all three are shadowed together whichever of them a
/// test came for.
///
/// It prepends, so nothing here takes a program off `PATH`: the real binary
/// is still behind the shim, which is what lets a shim hand on to it. A test
/// asserting what `bdi` does with a program **absent** cannot use this, and
/// cannot tell that it did not: on a machine that has the program it would be
/// testing the shim, and on one that does not it would pass without having
/// established anything. Both are green. `shims_first_with_nothing_called`
/// below is what an absence test wants.
pub fn shims_first_on_path() -> (String, String) {
    let inherited = std::env::var("PATH").unwrap_or_default();
    ("PATH".to_string(), format!("{SHIMS}:{inherited}"))
}

/// The same, with `absent` on `PATH` nowhere: not shimmed, and not behind the
/// shims either.
///
/// The shims are linked into a directory of their own under `beside`, minus
/// the one named, because a program cannot be taken off `PATH` while the
/// directory it shares with the others is on it. Every directory of the
/// inherited `PATH` that holds one is dropped as well, which is the half that
/// makes the answer the same on a machine that has the program and one that
/// never did.
pub fn shims_first_with_nothing_called(absent: &str, beside: &Path) -> (String, String) {
    let ours = beside.join(format!("shims-without-{absent}"));
    std::fs::create_dir_all(&ours).expect("the directory is ours to make");
    for shim in std::fs::read_dir(SHIMS).expect("the shims are where they are written down") {
        let shim = shim.expect("the shim directory is readable").path();
        let named = ours.join(shim.file_name().expect("a shim is a file"));
        if shim.ends_with(absent) || named.exists() {
            continue;
        }
        std::os::unix::fs::symlink(&shim, &named).expect("the link is ours to make");
    }

    let inherited = std::env::var("PATH").unwrap_or_default();
    let elsewhere: Vec<&str> = inherited
        .split(':')
        .filter(|directory| !Path::new(directory).join(absent).exists())
        .collect();
    (
        "PATH".to_string(),
        format!("{}:{}", ours.display(), elsewhere.join(":")),
    )
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
    reads: PathBuf,
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
            reads: beside.join("herdr-reads"),
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
            (
                "BDI_SHIM_HERDR_READS".to_string(),
                self.reads.display().to_string(),
            ),
        ]
    }

    /// How many times the band has asked for a pane.
    ///
    /// The count rather than the clock, for a test about how often the band
    /// reads: an answer's latency is the gap to whatever wakes the loop next,
    /// and every deadline the loop holds is in that gap. How many times it
    /// asked over a window of its own choosing is the band's alone, and the
    /// slowest thing that could otherwise wake it — a project's line ageing —
    /// ticks once a second and puts a ceiling on what a band with no clock
    /// can reach.
    pub fn reads(&self) -> usize {
        std::fs::read_to_string(&self.reads)
            .unwrap_or_default()
            .lines()
            .count()
    }

    /// What every pane read from here answers with, in place of what it
    /// answered before.
    ///
    /// The shim `cat`s one file, so this is how a pane that is *doing*
    /// something is reached: a second answer that differs from the first is
    /// what the band's own clock is for, and a band that never read again
    /// would go on showing the first for ever.
    ///
    /// Written beside and renamed onto, because the reader is another process
    /// and a `cat` of a file half written is an answer neither text.
    pub fn shows(&self, said: &str) {
        let beside = self.visible.with_extension("next");
        std::fs::write(&beside, said).expect("the pane is ours to write");
        std::fs::rename(&beside, &self.visible).expect("the pane is ours to replace");
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
    #[track_caller]
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
