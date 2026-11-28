//! A run's open-files limit, read and lowered from outside it, and a report
//! sent until the run takes no more, for a test of what `bdi` does once it
//! holds as many files as it may.
//!
//! Linux only, because the descriptors a run holds are read off `/proc`.

/// The limits on how many files `pid` may hold open.
pub fn open_files(pid: u32) -> libc::rlimit {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    let read = unsafe {
        libc::prlimit(
            pid as libc::pid_t,
            libc::RLIMIT_NOFILE,
            std::ptr::null(),
            &mut limit,
        )
    };
    assert_eq!(read, 0, "{}", std::io::Error::last_os_error());
    limit
}

/// Set the soft limit on how many files `pid` may hold open, leaving its
/// hard limit as it was.
pub fn limit_open_files(pid: u32, soft: libc::rlim_t) {
    let limit = libc::rlimit {
        rlim_cur: soft,
        ..open_files(pid)
    };
    let set = unsafe {
        libc::prlimit(
            pid as libc::pid_t,
            libc::RLIMIT_NOFILE,
            &limit,
            std::ptr::null_mut(),
        )
    };
    assert_eq!(set, 0, "{}", std::io::Error::last_os_error());
}

/// The descriptor `pid` would be given next. A soft limit at it refuses the
/// next one, since a descriptor has to be below the limit.
pub fn lowest_free_descriptor(pid: u32) -> libc::rlim_t {
    let held: std::collections::BTreeSet<libc::rlim_t> =
        std::fs::read_dir(format!("/proc/{pid}/fd"))
            .expect("the run is ours to read")
            .map(|entry| {
                let entry = entry.expect("the run is ours to read");
                entry
                    .file_name()
                    .to_string_lossy()
                    .parse()
                    .expect("a descriptor is a number")
            })
            .collect();
    (0..)
        .find(|descriptor| !held.contains(descriptor))
        .expect("a descriptor is free")
}

/// What a run did with a report sent to it.
#[derive(Debug, PartialEq)]
pub enum Answered {
    With(String),
    /// It closed the connection unanswered.
    Dropped,
    /// It has done nothing with it yet.
    Waiting,
}

/// A report sent on a connection to the socket `at` that its run has not
/// taken, once the run's limit is lowered, and the connections it took
/// before it stopped. Each is sent on its own, so the run runs out of room
/// before the one it does not take.
pub fn a_report_past_the_limit(
    at: &std::path::Path,
) -> (
    std::os::unix::net::UnixStream,
    Vec<std::os::unix::net::UnixStream>,
) {
    use std::io::Write;
    let mut taken_past_the_limit = Vec::new();
    loop {
        let mut reporting =
            std::os::unix::net::UnixStream::connect(at).expect("the socket queues the connection");
        writeln!(reporting, "arkham").expect("the report is ours to send");
        match answered_within(&reporting, UNTAKEN_AFTER) {
            Answered::Waiting => return (reporting, taken_past_the_limit),
            Answered::With(_) | Answered::Dropped => taken_past_the_limit.push(reporting),
        }
        assert!(
            taken_past_the_limit.len() < PAST_THE_LIMIT,
            "the run kept taking connections past its limit"
        );
    }
}

/// What the run answered on `reporting`, read for `quiet`.
pub fn answered_within(
    reporting: &std::os::unix::net::UnixStream,
    quiet: std::time::Duration,
) -> Answered {
    use std::io::{BufRead, ErrorKind};
    reporting
        .set_read_timeout(Some(quiet))
        .expect("a read nothing answers is ours to give up on");
    let mut line = String::new();
    match std::io::BufReader::new(reporting).read_line(&mut line) {
        Ok(0) => Answered::Dropped,
        Ok(_) => Answered::With(line),
        Err(why) if why.kind() == ErrorKind::ConnectionReset => Answered::Dropped,
        Err(why) if why.kind() == ErrorKind::WouldBlock => Answered::Waiting,
        Err(why) => panic!("the connection is ours to read ({why})"),
    }
}

/// Long enough for a run that is going to take a connection to have taken
/// it.
const UNTAKEN_AFTER: std::time::Duration = std::time::Duration::from_millis(500);

/// More connections than a run with no descriptor free takes before an
/// accept fails. A thread already blocked in accept holds the descriptor for
/// the next one, so the run takes that one past its limit. A collection or a
/// check of the config can hold a descriptor for a moment as the limit is
/// lowered, and the run takes a connection on each one that closes.
const PAST_THE_LIMIT: usize = 16;
