//! A view whose inbound socket stops taking connections says so at the foot
//! without writing over the frame, and takes the notice off once the socket
//! takes one again. A single descriptor free is room enough to answer a
//! report.
//!
//! Driven through the binary because what this shows is where the word goes.
//! The accept loop's own tests say when it is told, and `Shown`'s say what
//! the foot does with it. Neither can say that a view's stderr, which is the
//! screen, is left alone.
//!
//! Linux only, because the view is brought to its open-files limit through
//! `/proc` and `prlimit`.

#![cfg(target_os = "linux")]

mod terminal;

use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use terminal::descriptors::{limit_open_files, lowest_free_descriptor, open_files};
use terminal::driver::{Driven, GIVING_UP};
use terminal::{
    a_home_naming_one_project, a_socket_of_its_own, contains, the_socket_under,
    whole_frames_until_gone,
};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// The configured project's own header, which is in the first frame before
/// any tracker has answered. A frame holding it is a forest still drawn.
const THE_FIRST_FRAME: &[u8] = "arkham".as_bytes();

/// The notice, from `view::phrase`. Its first words rather than the whole
/// line, because the foot is drawn as a difference from the frame before.
const NOT_TAKEN: &[u8] = "bdi cannot take reports of changes".as_bytes();

/// What `bdi watch` says on stderr of the same failure, which a view saying
/// it there would tear its frame with.
const SAID_ON_STDERR: &[u8] = "cannot take connections".as_bytes();

/// More connections than a run with no descriptor free takes before an
/// accept fails. A thread already blocked in accept holds the descriptor for
/// the next one, so the run takes that one past its limit.
const PAST_THE_LIMIT: usize = 4;

#[test]
fn a_view_out_of_descriptors_says_so_at_the_foot_until_it_has_room_again() {
    let home = a_home_naming_one_project("view-out-of-descriptors");
    let environment = [a_socket_of_its_own(&home)];
    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &environment);
    bdi.read_until(THE_FIRST_FRAME, GIVING_UP);
    let pid = bdi.pid() as u32;
    let room = open_files(pid).rlim_cur;

    limit_open_files(pid, lowest_free_descriptor(pid));
    let _waiting: Vec<UnixStream> = (0..PAST_THE_LIMIT)
        .map(|_| {
            UnixStream::connect(the_socket_under(&home)).expect("the socket queues the connection")
        })
        .collect();
    bdi.read_until(NOT_TAKEN, GIVING_UP);

    let mut rows = ROWS - 1;
    let whole = bdi.resize(rows, COLS);
    let frame = bdi.answer_to(whole, GIVING_UP);
    assert!(
        contains(&frame, NOT_TAKEN) && contains(&frame, THE_FIRST_FRAME),
        "the foot says the reports are not taken, under the forest\n{}",
        bdi.timeline()
    );
    assert!(
        !contains(&bdi.everything(), SAID_ON_STDERR),
        "nothing is written over the frame\n{}",
        bdi.timeline()
    );

    limit_open_files(pid, room);
    let frame = whole_frames_until_gone(&mut bdi, &mut rows, ROWS, COLS, NOT_TAKEN, GIVING_UP);
    assert!(
        !contains(&frame, NOT_TAKEN),
        "the socket takes connections again and the foot still says it does \
         not\n{}",
        bdi.timeline()
    );
    assert!(
        contains(&frame, THE_FIRST_FRAME),
        "and the forest is still there to read it off\n{}",
        bdi.timeline()
    );
}

/// What a producer relies on once a view at its limit has a single
/// descriptor free: the one its report is taken on is all the view needs to
/// answer it.
#[test]
fn a_view_out_of_descriptors_answers_a_report_once_it_has_one_free() {
    let home = a_home_naming_one_project("view-out-of-descriptors-one-free");
    let environment = [a_socket_of_its_own(&home)];
    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &environment);
    bdi.read_until(THE_FIRST_FRAME, GIVING_UP);
    let pid = bdi.pid() as u32;

    limit_open_files(pid, lowest_free_descriptor(pid));
    let (waiting, _taken_past_the_limit) = a_report_past_the_limit(&home);
    limit_open_files(pid, lowest_free_descriptor(pid) + 1);

    assert_eq!(
        answered_within(&waiting, GIVING_UP),
        Answered::With("ok arkham\n".to_string()),
        "{}",
        bdi.timeline()
    );
}

/// What a view did with a report sent to it.
#[derive(Debug, PartialEq)]
enum Answered {
    With(String),
    /// It closed the connection unanswered.
    Dropped,
    /// It has done nothing with it yet.
    Waiting,
}

/// A report sent on a connection the view has not taken.
///
/// A thread already blocked in accept holds the descriptor its next
/// connection takes, so the view can take a connection past the limit before
/// it fails to take one.
fn a_report_past_the_limit(home: &Path) -> (UnixStream, Vec<UnixStream>) {
    let mut taken_past_the_limit = Vec::new();
    loop {
        let mut reporting =
            UnixStream::connect(the_socket_under(home)).expect("the socket queues the connection");
        writeln!(reporting, "arkham").expect("the report is ours to send");
        match answered_within(&reporting, UNTAKEN_AFTER) {
            Answered::Waiting => return (reporting, taken_past_the_limit),
            Answered::With(_) | Answered::Dropped => taken_past_the_limit.push(reporting),
        }
        assert!(
            taken_past_the_limit.len() < PAST_THE_LIMIT,
            "the view kept taking connections past its limit"
        );
    }
}

/// What the view answered on `reporting`, read for `quiet`.
fn answered_within(reporting: &UnixStream, quiet: Duration) -> Answered {
    reporting
        .set_read_timeout(Some(quiet))
        .expect("a read nothing answers is ours to give up on");
    let mut line = String::new();
    match BufReader::new(reporting).read_line(&mut line) {
        Ok(0) => Answered::Dropped,
        Ok(_) => Answered::With(line),
        Err(why) if why.kind() == ErrorKind::ConnectionReset => Answered::Dropped,
        Err(why) if why.kind() == ErrorKind::WouldBlock => Answered::Waiting,
        Err(why) => panic!("the connection is ours to read ({why})"),
    }
}

/// Long enough for a view that is going to take a connection to have taken
/// it.
const UNTAKEN_AFTER: Duration = Duration::from_millis(500);
