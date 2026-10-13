//! A test's home is named for the test and the process, and nothing removes
//! it, so a later run whose process gets the same id is handed the same
//! directory. Asking for the same home twice in one process is that reuse.

mod terminal;

use std::os::unix::net::UnixListener;

use terminal::a_home_naming_one_project_settled;

#[test]
fn a_socket_an_earlier_run_left_in_its_home_does_not_stop_the_next_binding_it() {
    let earlier = a_home_naming_one_project_settled("left-behind", "");
    drop(UnixListener::bind(earlier.join("watcher.sock")).expect("the socket is ours"));

    let later = a_home_naming_one_project_settled("left-behind", "");

    UnixListener::bind(later.join("watcher.sock")).expect("the home holds no socket");
}
