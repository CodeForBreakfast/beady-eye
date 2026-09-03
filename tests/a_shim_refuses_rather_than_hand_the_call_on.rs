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
//! What is asserted here is the absence of a call, and an absence assertion
//! wants something that could have occurred. The real herdr cannot be that
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
