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

use terminal::descriptors::{
    a_report_past_the_limit, answered_within, limit_open_files, lowest_free_descriptor, open_files,
    Answered,
};
use terminal::driver::{Driven, GIVING_UP};
use terminal::{
    a_home_naming_one_project, a_socket_of_its_own, contains, the_socket_under, whole_frames_until,
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

/// The notice as the foot cuts it to make room for another, such as one a
/// check of the config made while the view had no descriptor free.
const NOT_TAKEN_CUT: &[u8] = "reports not taken".as_bytes();

/// Whether `frame` says the reports are not taken, in either form.
fn says_not_taken(frame: &[u8]) -> bool {
    contains(frame, NOT_TAKEN) || contains(frame, NOT_TAKEN_CUT)
}

/// What `bdi watch` says on stderr of the same failure, which a view saying
/// it there would tear its frame with.
const SAID_ON_STDERR: &[u8] = "cannot take connections".as_bytes();

#[test]
fn a_view_out_of_descriptors_says_so_at_the_foot_until_it_has_room_again() {
    let home = a_home_naming_one_project("view-out-of-descriptors");
    let environment = [a_socket_of_its_own(&home)];
    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &environment);
    bdi.read_until(THE_FIRST_FRAME, GIVING_UP);
    let pid = bdi.pid() as u32;
    let room = open_files(pid).rlim_cur;

    limit_open_files(pid, lowest_free_descriptor(pid));
    let (waiting, _taken_past_the_limit) = a_report_past_the_limit(&the_socket_under(&home));
    let mut rows = ROWS;
    let frame = whole_frames_until(&mut bdi, &mut rows, ROWS, COLS, GIVING_UP, says_not_taken);
    assert!(
        says_not_taken(&frame) && contains(&frame, THE_FIRST_FRAME),
        "the foot says the reports are not taken, under the forest\n{}",
        bdi.timeline()
    );
    assert!(
        !contains(&bdi.everything(), SAID_ON_STDERR),
        "nothing is written over the frame\n{}",
        bdi.timeline()
    );

    limit_open_files(pid, room);
    assert_eq!(
        answered_within(&waiting, GIVING_UP),
        Answered::With("ok arkham\n".to_string()),
        "the socket takes the report waiting on it\n{}",
        bdi.timeline()
    );
    let frame = whole_frames_until_gone(
        &mut bdi,
        &mut rows,
        ROWS,
        COLS,
        &[NOT_TAKEN, NOT_TAKEN_CUT],
        GIVING_UP,
    );
    assert!(
        !says_not_taken(&frame),
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
    let (waiting, _taken_past_the_limit) = a_report_past_the_limit(&the_socket_under(&home));
    limit_open_files(pid, lowest_free_descriptor(pid) + 1);

    assert_eq!(
        answered_within(&waiting, GIVING_UP),
        Answered::With("ok arkham\n".to_string()),
        "{}",
        bdi.timeline()
    );
}
