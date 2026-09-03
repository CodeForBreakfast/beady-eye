//! A shim answers for the program it shadows, or refuses. It never hands the
//! call to the real one.
//!
//! `tests/shims/shadowed` runs the first program on `PATH` outside the shim
//! directory, and the `herdr` shim used to end by handing everything it had
//! not learned to it. On the machine that builds this suite that reaches
//! nothing and exits 127, which is why the arrangement read as safe. On the
//! machine somebody runs the suite on by hand it reaches the herdr they are
//! sitting in — the one holding their panes — and `agent focus` is a write.
//! So the safety was present where nothing could go wrong and absent where
//! everything could.
//!
//! The `bd` shim ended the same way, and its one call that carries no `-C`
//! is the same shape of hazard: `bd where --json` resolves its tracker by
//! walking up from the working directory, so a handed-on call reaches the
//! maintainer's own `.beads` from this repository and answers exit 0 —
//! which is what decides the directory is a project. Its `sql` fall-through
//! was deliberate rather than accidental, and refusing keeps what that
//! decision protected: the working root is never answered, so every refresh
//! reads in full.
//!
//! What is asserted here is the absence of a call, and an absence assertion
//! wants something that could have occurred. The real program cannot be that
//! something: reaching it is the hazard, so a test that let the handing on
//! land would prove the point by doing the damage. A stand-in the test owns
//! sits where the real one would be found instead, and writes down what it
//! was asked. The file it writes is the red.

mod terminal;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use terminal::shims::{shim, shims_first_over_a_stand_in, A_PANE, A_SESSION};

/// Where the stand-in writes down what it was asked. It is handed over in the
/// environment rather than written into the script, because the path is a
/// temporary directory's and a `TMPDIR` holding a space would otherwise be
/// split by the shell that runs it — a redirect to somewhere else, or none.
const WAS_HANDED: &str = "BDI_STAND_IN_WAS_HANDED";

/// A subcommand the shim has never been taught, spelled as herdr takes it.
/// `bdi` does not ask for this one; the bead is about the next command it
/// learns, and this stands for that one.
const NOT_LEARNED: [&str; 2] = ["pane", "send-text"];

/// A `herdr` this test owns, standing where the real one would be found.
///
/// It records every call and answers nothing useful, because nothing here
/// wants an answer from it: what it is for is to be *reachable*, so that a
/// shim which hands a call on leaves a mark instead of quietly succeeding.
struct AStandIn {
    directory: PathBuf,
    handed_on: PathBuf,
}

impl AStandIn {
    /// A stand-in for `named`, in a directory of this test's own under
    /// `beside`.
    fn called(named: &str, beside: &Path) -> Self {
        let directory = beside.join(format!("stand-in-for-{named}"));
        std::fs::create_dir_all(&directory).expect("the directory is ours to make");
        let handed_on = beside.join(format!("{named}-was-handed"));
        let _ = std::fs::remove_file(&handed_on);

        let program = directory.join(named);
        std::fs::write(
            &program,
            format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >>\"${WAS_HANDED}\"\n"),
        )
        .expect("the stand-in is ours to write");
        std::fs::set_permissions(
            &program,
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .expect("the stand-in is ours to make runnable");

        Self {
            directory,
            handed_on,
        }
    }

    /// Every call that reached it, spelled as it was asked. Empty is the
    /// reading that says nothing was handed on.
    fn was_handed(&self) -> Vec<String> {
        std::fs::read_to_string(&self.handed_on)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// A run of `program` with this stand-in behind the shims, and every file
    /// a shim answers from taken away — so what the shim does is decided by
    /// the arguments and by what the test puts back, rather than by whatever
    /// the shell running the suite happens to hold.
    fn asking(&self, program: &Path) -> Command {
        let (path, entries) = shims_first_over_a_stand_in(&self.directory);
        let mut command = Command::new(program);
        command.env(path, entries).env(WAS_HANDED, &self.handed_on);
        for answered_from in [
            "BDI_SHIM_HERDR_SESSIONS",
            "BDI_SHIM_HERDR_AGENTS",
            "BDI_SHIM_HERDR_VISIBLE",
            "BDI_SHIM_HERDR_READS",
            "BDI_SHIM_HERDR_HANGS_WHILE",
            "BDI_SHIM_HERDR_HOLDING",
            "BDI_SHIM_HERDR_READ_DELAY",
            "BDI_SHIM_BD_ANSWERS",
            "BDI_SHIM_BD_UNANSWERED",
            "BDI_SHIM_BD_HANGS_WHILE",
            "BDI_SHIM_BD_HOLDING",
        ] {
            command.env_remove(answered_from);
        }
        command
    }

    /// The same, run.
    fn running(&self, program: &Path, arguments: &[&str]) -> Output {
        self.asking(program)
            .args(arguments)
            .output()
            .expect("the shim runs")
    }
}

/// A directory of this test's own, named for the case that wants it, so the
/// cases can run at once without treading on each other's stand-ins.
fn a_directory_for(case: &str) -> PathBuf {
    let ours = std::env::temp_dir().join(format!("bdi-shim-{case}-{}", std::process::id()));
    std::fs::create_dir_all(&ours).expect("the directory is ours to make");
    ours
}

/// The bead: a subcommand the shim has not learned is refused, and the real
/// herdr never hears about it.
#[test]
fn a_subcommand_the_shim_has_not_learned_is_refused_and_not_handed_on() {
    let stand_in = AStandIn::called("herdr", &a_directory_for("not-learned"));

    let mut asked = vec!["--session", A_SESSION];
    asked.extend(NOT_LEARNED);
    let ran = stand_in.running(&shim("herdr"), &asked);

    assert_eq!(
        stand_in.was_handed(),
        Vec::<String>::new(),
        "the shim handed a subcommand it has not learned to the herdr it \
         shadows, which on the machine a reader runs this on is their own \
         live session"
    );
    assert!(
        !ran.status.success(),
        "the shim answered a subcommand it has not learned: {ran:?}"
    );
    let said = String::from_utf8_lossy(&ran.stderr);
    assert!(
        said.contains(&NOT_LEARNED.join(" ")),
        "the refusal did not say what it was asked, so a reader has to go \
         and find out: {said}"
    );
    assert!(
        said.contains(A_SESSION),
        "the refusal did not say which session it was asked about. `bdi` \
         names one on every call and a pane id means nothing without it, so \
         a reader debugging the refusal cannot tell which session's herdr \
         would have been reached: {said}"
    );
}

/// The write, which needs no new subcommand to reach. `agent focus` is
/// learned, but the shim answers it only where a test named the file to
/// record it in — and every other test on this machine leaves that unset. So
/// the one write `bdi` performs was a hand-on away from a real pane in a real
/// workspace, and this is the row that says it no longer is.
#[test]
fn a_learned_subcommand_with_nothing_to_answer_from_is_refused_too() {
    let stand_in = AStandIn::called("herdr", &a_directory_for("nothing-to-answer-from"));

    let ran = stand_in.running(
        &shim("herdr"),
        &["--session", A_SESSION, "agent", "focus", A_PANE],
    );

    assert_eq!(
        stand_in.was_handed(),
        Vec::<String>::new(),
        "the shim handed a focus to the herdr it shadows, which is a write \
         to whatever session that herdr answers for"
    );
    assert!(
        !ran.status.success(),
        "the shim reported a focus nobody recorded: {ran:?}"
    );
}

/// What the two rows above rest on: the stand-in is reachable, so an empty
/// `was_handed` is a reading rather than a hole. `shadowed` is what the shim
/// used to end with, and running it directly is the shortest way to ask
/// whether the arrangement could have carried a call at all.
#[test]
fn the_stand_in_is_where_a_call_handed_on_would_land() {
    let stand_in = AStandIn::called("herdr", &a_directory_for("reachable"));

    // `shadowed` is told which program it is standing in front of, and takes
    // it as the first argument — which is what the shim used to give it.
    let ran = stand_in.running(
        &shim("shadowed"),
        &["herdr", "--session", A_SESSION, "agent", "focus", A_PANE],
    );

    assert!(
        ran.status.success(),
        "nothing stood where a handed-on call would land, so the two rows \
         above assert the absence of something that could not have happened \
         anyway: {ran:?}"
    );
    assert_eq!(
        stand_in.was_handed(),
        vec![format!("--session {A_SESSION} agent focus {A_PANE}")],
        "the stand-in was reached but did not write down what it was asked"
    );
}

/// The same reading, taken where the path holding it has a space in it — a
/// `TMPDIR` that does is all it takes. The stand-in is a shell script, so a
/// path written into it rather than handed over in the environment would be
/// split into words here: the redirect would land somewhere else or not
/// parse, and the row above would report an absence it had caused itself.
#[test]
fn a_stand_in_under_a_path_with_a_space_still_writes_down_what_it_was_asked() {
    let stand_in = AStandIn::called("herdr", &a_directory_for("a reachable space"));

    let ran = stand_in.running(&shim("shadowed"), &["herdr", "agent", "focus", A_PANE]);

    assert!(
        ran.status.success(),
        "the stand-in could not be run from a path with a space in it: {ran:?}"
    );
    assert_eq!(
        stand_in.was_handed(),
        vec![format!("agent focus {A_PANE}")],
        "the stand-in ran and wrote nowhere this test can read, which is what \
         a path split into words does"
    );
}

/// What the isolation rests on now: precedence, and nothing removed. The
/// helper used to drop every directory holding a `herdr`, which on a machine
/// keeping one in a shared `bin` takes `dirname` — `shadowed`'s own — with
/// it. Order is enough, so nothing needs dropping, and this is the row that
/// says nothing is.
#[test]
fn isolating_the_stand_in_takes_nothing_off_the_path() {
    let ours = a_directory_for("nothing-dropped");
    let (_, entries) = shims_first_over_a_stand_in(&ours);

    let inherited = std::env::var("PATH").unwrap_or_default();
    let kept: Vec<&str> = entries.split(':').collect();
    for directory in inherited.split(':').filter(|at| !at.is_empty()) {
        assert!(
            kept.contains(&directory),
            "{directory} was dropped from PATH, and every program in it with \
             it: {entries}"
        );
    }
    assert!(
        !inherited.is_empty(),
        "the inherited PATH was empty, so the loop above checked nothing"
    );
}

/// The other half of the same question: refusing is not the shim having
/// stopped working. What it has learned it still answers, from the file the
/// test names, without the stand-in hearing anything.
#[test]
fn what_the_shim_has_learned_it_still_answers() {
    let ours = a_directory_for("still-answers");
    let stand_in = AStandIn::called("herdr", &ours);
    let sessions = ours.join("herdr-sessions.json");
    let listed =
        format!(r#"{{"sessions":[{{"default":true,"name":"{A_SESSION}","running":true}}]}}"#);
    std::fs::write(&sessions, &listed).expect("the sessions are ours to write");

    let ran = stand_in
        .asking(&shim("herdr"))
        .args(["session", "list", "--json"])
        .env("BDI_SHIM_HERDR_SESSIONS", &sessions)
        .output()
        .expect("the shim runs");

    assert!(
        ran.status.success(),
        "the shim refused a subcommand it has learned: {ran:?}"
    );
    assert_eq!(
        String::from_utf8_lossy(&ran.stdout),
        listed,
        "the shim did not answer with what the test wrote down"
    );
    assert_eq!(
        stand_in.was_handed(),
        Vec::<String>::new(),
        "the shim answered and handed the call on as well"
    );
}

/// The `bd` call that carries no tracker, spelled as `bdi` asks it.
///
/// `src/collect/discovery.rs` runs this to decide whether the directory is
/// one beads tracks, and it is the whole of `bdi`'s bd surface that names no
/// tracker: every other call goes through `Reader::asked` in
/// `src/collect/bd.rs`, which opens with `-C <path> --readonly`.
/// `RealRunner` strips `BEADS_DIR` from every child as well, so a delegated
/// one resolves the tracker by walking up from the working directory —
/// which in this repository reaches the maintainer's own `.beads` and
/// answers **exit 0**, measured 2026-09-03. Answering is what decides the
/// directory is a project, so handing this on does not read the wrong
/// tracker quietly: it hands the run a success no test wrote down, on the
/// machine somebody runs the suite by hand on and on no other.
const NAMES_NO_TRACKER: [&str; 2] = ["where", "--json"];

/// The statement `bdi` asks the tracker's working root with, spelled as
/// `src/collect/bd.rs` composes it.
const THE_WORKING_ROOT: &str = "SELECT dolt_hashof_db() AS h";

/// The bead: the one call that names no tracker is refused, and the bd that
/// would have answered for the maintainer's own never hears it.
#[test]
fn the_bd_call_that_names_no_tracker_is_refused_and_not_handed_on() {
    let stand_in = AStandIn::called("bd", &a_directory_for("bd-names-no-tracker"));

    let ran = stand_in.running(&shim("bd"), &NAMES_NO_TRACKER);

    assert_eq!(
        stand_in.was_handed(),
        Vec::<String>::new(),
        "the shim handed on the one call that names no tracker, which \
         against this repository resolves to the maintainer's own"
    );
    assert!(
        !ran.status.success(),
        "the shim answered a call nothing wrote an answer for: {ran:?}"
    );
    let said = String::from_utf8_lossy(&ran.stderr);
    assert!(
        said.contains(&NAMES_NO_TRACKER.join(" ")),
        "the refusal did not say what it was asked, so a reader running the \
         shim by hand has to go and find out: {said}"
    );
}

/// `sql` is refused whatever is written down for it, and a file is exactly
/// the hazard rather than the remedy: `bdi` asks it for the tracker's
/// working root and skips the whole read while the answer has not moved
/// (`src/app/tracker.rs`), so a constant would have the first read stand for
/// every refresh after it.
/// `a_collection_that_drops_the_shown_bead_takes_the_window_down` is the row
/// that goes red when one does.
#[test]
fn the_working_root_is_refused_even_where_a_file_answers_it() {
    let ours = a_directory_for("bd-sql-answered");
    let stand_in = AStandIn::called("bd", &ours);
    let answers = ours.join("bd-answers");
    std::fs::create_dir_all(&answers).expect("the answers are ours to write");
    let a_constant = r#"[{"h":"a hash that never moves"}]"#;
    std::fs::write(
        answers.join(format!("sql --json {THE_WORKING_ROOT}")),
        a_constant,
    )
    .expect("the answer is ours to write");

    let ran = stand_in
        .asking(&shim("bd"))
        .args(["-C", "/nowhere", "--readonly", "sql", "--json"])
        .arg(THE_WORKING_ROOT)
        .env("BDI_SHIM_BD_ANSWERS", &answers)
        .output()
        .expect("the shim runs");

    assert_eq!(
        stand_in.was_handed(),
        Vec::<String>::new(),
        "the shim handed the working root to the bd it shadows"
    );
    assert!(
        !String::from_utf8_lossy(&ran.stdout).contains("a hash that never moves"),
        "the shim said what the file held, so every refresh after the first \
         would find the tracker unmoved and read nothing"
    );
    assert!(
        !ran.status.success(),
        "the shim reported success for the working root, which is the one \
         answer it may never give: {ran:?}"
    );
}

/// A call a file could have answered and none did is refused too, and
/// written down where a test reads it. The writing down is the whole of what
/// a refusal leaves behind: `bdi` captures a child's stderr and drops it
/// after classifying the failure (`src/collect/run.rs`), so nothing the shim
/// says there reaches whoever is reading the run.
#[test]
fn a_bd_call_no_file_answers_is_refused_and_written_down() {
    let ours = a_directory_for("bd-unanswered");
    let stand_in = AStandIn::called("bd", &ours);
    let unanswered = ours.join("bd-unanswered");
    let _ = std::fs::remove_file(&unanswered);

    let ran = stand_in
        .asking(&shim("bd"))
        .args(["-C", "/nowhere", "--readonly"])
        .args(["list", "--all", "--limit", "0", "--json"])
        .env("BDI_SHIM_BD_UNANSWERED", &unanswered)
        .output()
        .expect("the shim runs");

    assert_eq!(
        stand_in.was_handed(),
        Vec::<String>::new(),
        "the shim handed on a call it had no answer for"
    );
    assert!(
        !ran.status.success(),
        "the shim answered a call nothing wrote an answer for: {ran:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&unanswered).unwrap_or_default(),
        "list --all --limit 0 --json\n",
        "the refusal was not written down, and the stderr saying it is \
         dropped by bdi, so a test that trips one has nothing to read"
    );
}

/// What the three rows above rest on: the stand-in is reachable under the
/// name `bd` as well, so an empty `was_handed` is a reading rather than a
/// hole.
#[test]
fn the_stand_in_is_where_a_bd_call_handed_on_would_land() {
    let stand_in = AStandIn::called("bd", &a_directory_for("bd-reachable"));

    let mut asked = vec!["bd"];
    asked.extend(NAMES_NO_TRACKER);
    let ran = stand_in.running(&shim("shadowed"), &asked);

    assert!(
        ran.status.success(),
        "nothing stood where a handed-on call would land, so the rows above \
         assert the absence of something that could not have happened \
         anyway: {ran:?}"
    );
    assert_eq!(
        stand_in.was_handed(),
        vec![NAMES_NO_TRACKER.join(" ")],
        "the stand-in was reached but did not write down what it was asked"
    );
}

/// The other half, as for herdr: refusing is not the shim having stopped
/// working. What a file answers it still answers, and the stand-in hears
/// nothing.
#[test]
fn what_a_file_answers_the_bd_shim_still_says() {
    let ours = a_directory_for("bd-still-answers");
    let stand_in = AStandIn::called("bd", &ours);
    let answers = ours.join("bd-answers");
    std::fs::create_dir_all(&answers).expect("the answers are ours to write");
    let listed = r#"[{"id":"bdi-1","title":"a bead","status":"open"}]"#;
    std::fs::write(answers.join("list --all --limit 0 --json"), listed)
        .expect("the answer is ours to write");

    let ran = stand_in
        .asking(&shim("bd"))
        .args(["-C", "/nowhere", "--readonly"])
        .args(["list", "--all", "--limit", "0", "--json"])
        .env("BDI_SHIM_BD_ANSWERS", &answers)
        .output()
        .expect("the shim runs");

    assert!(
        ran.status.success(),
        "the shim refused a call a file answers: {ran:?}"
    );
    assert_eq!(
        String::from_utf8_lossy(&ran.stdout),
        listed,
        "the shim did not answer with what the test wrote down"
    );
    assert_eq!(
        stand_in.was_handed(),
        Vec::<String>::new(),
        "the shim answered and handed the call on as well"
    );
}
