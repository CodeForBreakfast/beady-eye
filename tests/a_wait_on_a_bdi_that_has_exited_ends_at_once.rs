//! What a wait does when the `bdi` it is waiting on has already exited.
//!
//! Every wait in the driver is a poll with a deadline, and a `bdi` that has
//! exited will never satisfy one. A wait that runs to its deadline on one is
//! a deadline's worth of nothing, reported as "never said" with no word about
//! why — and under mutation it is not reported at all: cargo-mutants gives a
//! test run five times its baseline with a floor of twenty seconds, so a wait
//! that outlives that scores `Timeout` on the tally, where the suite would
//! have failed and named the wait. bdi-2bb.28 counted three mutants scored
//! that way for a `bdi` that did nothing but exit.
//!
//! So these start a `bdi` that cannot get as far as its screen — its config
//! does not parse — and ask each wait to give up at once, naming the exit.
//! The last one asks what the failure says, because that is what is left of
//! a failure nobody was watching: bdi-kpu lost one to a run whose only record
//! was the summary line, and seven waits across two tests were all it could
//! name.

mod terminal;

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::process::Command;
use std::time::{Duration, Instant};

use terminal::driver::{Driven, GIVING_UP};
use terminal::{a_home_whose_config_does_not_parse, said_by, ENTER_ALTERNATE_SCREEN};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// Long enough to tell a refusal from a wait run to its deadline: a `bdi`
/// that cannot read its config exits within milliseconds, and the deadline
/// is [`GIVING_UP`].
const A_REFUSAL_TAKES: Duration = Duration::from_secs(5);

/// A quiet window a `bdi` that has exited would satisfy by being dead, if
/// the wait let it.
const A_SILENCE: Duration = Duration::from_millis(300);

#[test]
fn reading_until_it_says_something_ends_at_once() {
    let mut bdi = a_bdi_that_exits_before_drawing("exited-before-reading");

    let (refusal, took) = refused(|| bdi.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP));

    assert_it_named_the_exit(&refusal, took);
    assert!(
        refusal.contains("1049h"),
        "the refusal does not say what it was waiting for: {refusal}"
    );
}

#[test]
fn settling_ends_at_once() {
    let mut bdi = a_bdi_that_exits_before_drawing("exited-before-settling");

    let (refusal, took) = refused(|| bdi.settle(A_SILENCE, GIVING_UP));

    assert_it_named_the_exit(&refusal, took);
}

#[test]
fn waiting_for_an_answer_ends_at_once() {
    let mut bdi = a_bdi_that_exits_before_drawing("exited-before-answering");
    // The error it exits with is an answer to anything asked before it, so
    // the mark has to come after its last word — which the first refusal
    // has read, since it reads everything a `bdi` wrote on the way out. A
    // resize is the one way to a mark that does not first insist on the
    // screen, which this `bdi` never opens.
    refused(|| bdi.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP));
    let asked = bdi.resize(ROWS, COLS);

    let (refusal, took) = refused(|| {
        bdi.answer_to(asked, GIVING_UP);
    });

    assert_it_named_the_exit(&refusal, took);
}

/// The wait the test below runs this binary again to watch fail. Ignored
/// because it is a fixture rather than a test: it fails on purpose, and what
/// it says on the way is the subject.
#[test]
#[ignore = "the subject of a_failed_wait_names_the_test_and_the_line_it_waited_on, which runs it"]
fn a_wait_that_fails_on_an_exited_bdi() {
    let mut bdi = a_bdi_that_exits_before_drawing("exited-and-reported");
    println!("{THE_WAIT_IS_ON_LINE}{}", line!() + 1);
    bdi.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP);
}

/// What the fixture says about itself before it waits, so the test watching
/// it knows which line to look for in the failure.
const THE_WAIT_IS_ON_LINE: &str = "the wait is on line ";

/// A failed wait names the test it was in and the line of that test it was
/// on, on the one line a reader of the run sees first.
///
/// The test name is what libtest calls the thread; the line is the wait's
/// caller, not the driver's. Without the second, every expired wait in the
/// suite panics at the same line of `driver.rs`, and a test with seven waits
/// has failed at one of seven.
#[test]
fn a_failed_wait_names_the_test_and_the_line_it_waited_on() {
    let ran = Command::new(std::env::current_exe().expect("this binary has a path"))
        .args([
            "--exact",
            "--ignored",
            "--nocapture",
            "a_wait_that_fails_on_an_exited_bdi",
        ])
        .output()
        .expect("this binary runs");
    let stdout = String::from_utf8_lossy(&ran.stdout);
    let stderr = String::from_utf8_lossy(&ran.stderr);

    assert!(
        !ran.status.success(),
        "the fixture waited on a bdi that had exited, and passed: {stdout}{stderr}"
    );
    let line = stdout
        .lines()
        .find_map(|said| said.strip_prefix(THE_WAIT_IS_ON_LINE))
        .expect("the fixture says which line it waits on");
    // On one line, the way the panic hook writes it: the thread, which
    // libtest names after the test, then where it panicked.
    let the_test = "thread 'a_wait_that_fails_on_an_exited_bdi'";
    let the_wait = format!("panicked at {}:{line}:", file!());
    assert!(
        stderr
            .lines()
            .any(|said| said.starts_with(the_test) && said.contains(&the_wait)),
        "the failure does not name the test and the line it waited on — \
         looked for a line opening {the_test:?} and holding {the_wait:?} in: {stderr}"
    );
}

/// A `bdi` whose config does not parse, which is one that exits before it
/// has opened its screen.
fn a_bdi_that_exits_before_drawing(named: &str) -> Driven {
    let home = a_home_whose_config_does_not_parse(named);
    Driven::bdi(ROWS, COLS, home, &[])
}

/// What the wait said when it refused, and how long it took to.
fn refused(wait: impl FnOnce()) -> (String, Duration) {
    let waited_from = Instant::now();
    let waited = catch_unwind(AssertUnwindSafe(wait));
    let took = waited_from.elapsed();
    let refusal =
        waited.expect_err("the wait passed against a bdi that had exited without drawing");
    (said_by(&refusal), took)
}

fn assert_it_named_the_exit(refusal: &str, took: Duration) {
    assert!(
        refusal.contains("exited"),
        "the refusal does not say bdi had exited: {refusal}"
    );
    assert!(
        took < A_REFUSAL_TAKES,
        "the wait took {took:?} to refuse, which is a deadline rather than a refusal"
    );
}
