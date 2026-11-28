//! A run's open-files limit, read and lowered from outside it, for a test
//! of what `bdi` does once it holds as many files as it may.
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
