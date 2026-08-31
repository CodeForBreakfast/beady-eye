//! The inbound channel any process can poke to say a project's work changed.
//!
//! `bdi` ships the socket and the protocol and nothing that produces for it:
//! what watches a tracker for changes — a Dolt trigger, a git hook, a `bd`
//! wrapper, a systemd path unit, someone typing a command — is the setup's
//! business, and building any one of them in would tie `bdi` to a setup it
//! cannot know.
//!
//! Nothing in here is ever waited on by the loop that draws. The accept loop
//! and each connection block on their own threads, so a writer that connects
//! and never speaks costs one sleeping thread and cannot stop `^C` reaching
//! the loop or the terminal being put back.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};
use std::{fmt, fs, thread};

/// The variable naming the directory this login session owns. A socket under
/// it is reachable by this user and no other, which is the whole of the
/// channel's protection.
const RUNTIME_DIRECTORY: &str = "XDG_RUNTIME_DIR";

/// Where `bdi` puts its socket inside that directory.
const SOCKET: &str = "beady-eye/changes.sock";

/// Nothing this long is a project name, so a writer still building a line at
/// this point is broken. Reading stops here, which is what keeps a writer
/// that never ends its line from being read into memory without limit.
const LONGEST_MESSAGE: usize = 512;

/// Only this user may reach the channel, whatever umask the run was started
/// with. The runtime directory says the same thing; a channel that anything
/// can write to should not depend on being told twice.
const OWNER_ONLY: u32 = 0o600;

/// What `bdi` makes of one message, and what it says back to whoever sent it.
///
/// A message arrives from outside `bdi` and cannot be trusted to be a project
/// name, so every one of these is an answer rather than a failure. The answer
/// goes back down the socket because the writer is the only one who can put a
/// wrong one right — the user is watching a forest, not a log.
#[derive(Debug, PartialEq, Eq)]
pub enum Answer {
    /// A project `bdi` watches. It will be collected for.
    Watched(String),
    /// A name `bdi` watches nothing under.
    Unwatched(String),
    /// Not a project name at all.
    Malformed,
}

impl fmt::Display for Answer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Answer::Watched(project) => write!(f, "ok {project}"),
            Answer::Unwatched(project) => write!(f, "unknown {project}"),
            Answer::Malformed => write!(f, "malformed"),
        }
    }
}

/// The projects `bdi` watches, and when something last said each one changed.
///
/// This is what selects a project's refresh source, and it needs no
/// configuring to do it: nothing can say in advance which projects have a
/// producer, so a project is polled until something reports for it and polled
/// again from the moment that stops.
#[derive(Clone, Default)]
pub struct Reported {
    projects: Arc<Mutex<BTreeMap<String, Option<Instant>>>>,
}

impl Reported {
    pub fn watching<I: IntoIterator<Item = String>>(projects: I) -> Self {
        Self {
            projects: Arc::new(Mutex::new(
                projects
                    .into_iter()
                    .map(|project| (project, None))
                    .collect(),
            )),
        }
    }

    /// Take one message, recording it where it names a project `bdi` watches.
    pub fn take(&self, message: &str) -> Answer {
        let named = message.trim();
        if named.is_empty() || named.len() > LONGEST_MESSAGE {
            return Answer::Malformed;
        }

        match self.projects().get_mut(named) {
            Some(last) => {
                *last = Some(Instant::now());
                Answer::Watched(named.to_string())
            }
            None => Answer::Unwatched(named.to_string()),
        }
    }

    /// What a poll now would still have to find: the projects nothing has
    /// reported for inside `within`.
    ///
    /// Watching nothing leaves everything to be found. There is no poll to
    /// stand down, and a window nothing has to fall inside is one that says
    /// nothing.
    pub fn uncovered(&self, within: Duration) -> Uncovered {
        let projects = self.projects();
        let uncovered: Vec<String> = projects
            .iter()
            .filter(|(_, last)| !last.is_some_and(|at| at.elapsed() < within))
            .map(|(project, _)| project.clone())
            .collect();

        if uncovered.len() == projects.len() {
            Uncovered::Everything
        } else if uncovered.is_empty() {
            Uncovered::Nothing
        } else {
            Uncovered::These(uncovered)
        }
    }

    /// A thread that panicked mid-message poisons the lock. The messages it
    /// left behind are still true, and a view that stopped refreshing because
    /// one writer misbehaved would be exactly the failure this channel is not
    /// allowed to cause.
    fn projects(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Option<Instant>>> {
        self.projects.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// What a poll has left to find.
#[derive(Debug, PartialEq, Eq)]
pub enum Uncovered {
    /// Every project. Nothing is being reported for, so a poll reading them
    /// all at once costs one collection where naming them would cost one
    /// each.
    Everything,
    /// Only these. The rest are being reported for, and a poll would find in
    /// them only what their messages have already said.
    These(Vec<String>),
    /// None of them: every project is covered, so there is nothing to poll.
    Nothing,
}

/// Why `bdi` has no inbound channel.
///
/// Each of these leaves the view working and polled, so each is said rather
/// than fatal.
#[derive(Debug)]
pub enum Refused {
    /// This session owns no runtime directory, so there is nowhere to put a
    /// socket only this user can reach.
    NoRuntimeDirectory,
    /// Another `bdi` is listening there already, so this one has the channel
    /// only when that one lets it go. The reader's remedy, and the only
    /// refusal here that has one.
    AlreadyListening(PathBuf),
    /// The socket could not be made, or could not be made this user's alone.
    Unopenable(PathBuf, std::io::Error),
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "nothing can tell bdi a project changed, so every project is polled: "
        )?;
        match self {
            Refused::NoRuntimeDirectory => {
                write!(
                    f,
                    "this session has no {RUNTIME_DIRECTORY} to put the socket in"
                )
            }
            // The one refusal with a remedy, so the one that says how to
            // reach it. The foot can name the cause and no more; naming a
            // process is a thing to be done here, where there is room for
            // the path and for a way of asking who holds it that is live
            // when the reader asks rather than as old as this line.
            //
            // The restart is half the remedy and not a flourish. `wire` asks
            // for the socket once, before the screen opens, and never binds
            // again — so closing the holder frees the path and gives this run
            // nothing. A remedy that stopped at "close it" would leave the
            // reader watching a channel that was never going to arrive.
            //
            // Two tools rather than one conditioned on the target, because
            // which of them the reader has is a fact about their machine and
            // not about this build: `ss` comes with iproute2 and is not on
            // macOS, `lsof` is in the macOS base system and is not everywhere
            // on Linux. A `cfg` would also put the arm this crate never
            // compiles beyond the reach of the assertion below, which is the
            // only thing holding either command to naming a holder.
            Refused::AlreadyListening(at) => {
                write!(
                    f,
                    "another bdi is listening on {}; ss -lxp or lsof -U names which — close it and restart bdi to get the channel",
                    at.display()
                )
            }
            Refused::Unopenable(at, why) => {
                write!(f, "{} could not be opened ({why})", at.display())
            }
        }
    }
}

/// `bdi`'s end of the inbound channel, for as long as this lives.
///
/// Dropping it takes the socket off the filesystem, so the run that made it
/// is the run that clears it away and the next one has nothing to reclaim.
pub struct Socket {
    at: PathBuf,
}

impl Drop for Socket {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.at);
    }
}

/// Where a writer finds `bdi`, or nothing where this session owns no runtime
/// directory.
pub fn where_writers_find_bdi() -> Option<PathBuf> {
    under(
        std::env::var_os(RUNTIME_DIRECTORY)
            .map(PathBuf::from)
            .as_deref(),
    )
}

fn under(runtime_directory: Option<&Path>) -> Option<PathBuf> {
    runtime_directory.map(|dir| dir.join(SOCKET))
}

/// Open the channel and start listening on it, reporting what stopped it
/// rather than failing: a `bdi` nothing can reach still draws, just polled.
pub fn listen(
    at: Option<PathBuf>,
    reported: &Reported,
    changed: Sender<String>,
) -> Result<Socket, Refused> {
    let at = at.ok_or(Refused::NoRuntimeDirectory)?;
    let listener = bind(&at)?;

    let reported = reported.clone();
    thread::spawn(move || accept(&listener, &reported, &changed));

    Ok(Socket { at })
}

fn bind(at: &Path) -> Result<UnixListener, Refused> {
    if let Some(directory) = at.parent() {
        fs::create_dir_all(directory).map_err(|why| Refused::Unopenable(at.to_path_buf(), why))?;
    }

    let listener = match UnixListener::bind(at) {
        Ok(listener) => listener,
        Err(taken) if taken.kind() == ErrorKind::AddrInUse => reclaim(at)?,
        Err(why) => return Err(Refused::Unopenable(at.to_path_buf(), why)),
    };

    fs::set_permissions(at, fs::Permissions::from_mode(OWNER_ONLY))
        .map_err(|why| Refused::Unopenable(at.to_path_buf(), why))?;

    Ok(listener)
}

/// A socket already at the path is either a live `bdi`'s or the litter of one
/// that crashed — a `UnixListener` leaves its file behind when its process
/// goes. Connecting tells them apart: a live listener accepts, and a file
/// nothing is listening on refuses.
fn reclaim(at: &Path) -> Result<UnixListener, Refused> {
    if UnixStream::connect(at).is_ok() {
        return Err(Refused::AlreadyListening(at.to_path_buf()));
    }

    fs::remove_file(at).map_err(|why| Refused::Unopenable(at.to_path_buf(), why))?;
    UnixListener::bind(at).map_err(|why| Refused::Unopenable(at.to_path_buf(), why))
}

/// Take writers until the socket stops giving them.
///
/// An accept that fails ends the channel rather than being retried: there is
/// no error here a retry would clear, and the poll is what the view falls
/// back to.
fn accept(listener: &UnixListener, reported: &Reported, changed: &Sender<String>) {
    for writer in listener.incoming() {
        let Ok(writer) = writer else { return };

        let (reported, changed) = (reported.clone(), changed.clone());
        thread::spawn(move || hear(writer, &reported, &changed));
    }
}

/// Read one writer's messages until it goes away.
///
/// A thread of its own because a producer that connects once and speaks
/// whenever it has something to say is the shape this channel is for, and a
/// long quiet stretch on such a connection is not a wedge to be timed out.
fn hear(writer: UnixStream, reported: &Reported, changed: &Sender<String>) {
    let Ok(mut answering) = writer.try_clone() else {
        return;
    };
    let mut reading = BufReader::new(writer);
    let mut line = Vec::new();

    loop {
        line.clear();
        let room = LONGEST_MESSAGE as u64 + 1;
        match (&mut reading).take(room).read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }

        // A line that filled the room it was given never ended, so the rest
        // of what this writer is saying cannot be read as messages either.
        let unended = line.len() > LONGEST_MESSAGE;
        let answer = match std::str::from_utf8(&line) {
            Ok(message) if !unended => reported.take(message),
            _ => Answer::Malformed,
        };

        // The name goes with the signal: what a writer said changed is
        // what the collection it triggers has to read, and no more.
        if let Answer::Watched(project) = &answer {
            if changed.send(project.clone()).is_err() {
                return;
            }
        }
        if writeln!(answering, "{answer}").is_err() || unended {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::Path;
    use std::sync::mpsc::{self, Receiver};

    /// Long enough that a channel which was going to report has, and short
    /// enough that a test waiting in vain is not a hang.
    const A_MOMENT: Duration = Duration::from_secs(5);

    /// A window wide enough that nothing in a test falls out of it.
    const A_WHILE: Duration = Duration::from_secs(600);

    fn watching<const N: usize>(projects: [&str; N]) -> Reported {
        Reported::watching(projects.map(str::to_string))
    }

    /// A directory of this test's own, so tests that bind sockets do not
    /// collide with each other or with a previous run.
    fn a_socket_path(named: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bdi-{named}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a directory to put the socket in");
        dir.join("beady-eye").join("changes.sock")
    }

    /// An open channel, and the end of it the loop would be reading.
    fn open(at: &Path, reported: &Reported) -> (Socket, Receiver<String>) {
        let (changed, changes) = mpsc::channel();
        let socket = listen(Some(at.to_path_buf()), reported, changed).expect("the socket opens");
        (socket, changes)
    }

    /// Send `lines` down one connection and read back what bdi said to each.
    fn say(at: &Path, lines: &[&str]) -> Vec<String> {
        let mut writing = UnixStream::connect(at).expect("bdi is listening");
        let mut reading = BufReader::new(writing.try_clone().expect("both ends of the stream"));

        lines
            .iter()
            .map(|line| {
                write!(writing, "{line}").expect("bdi takes the message");
                let mut said = String::new();
                reading.read_line(&mut said).expect("bdi answers");
                said.trim_end().to_string()
            })
            .collect()
    }

    #[test]
    fn a_message_naming_a_watched_project_is_taken() {
        let reported = watching(["atlas", "ferry"]);

        assert_eq!(reported.take("atlas"), Answer::Watched("atlas".to_string()));
    }

    /// The line arrives with the newline that terminated it, and a writer may
    /// have laid its message out to be read by a human as well.
    #[test]
    fn a_name_is_taken_without_the_whitespace_around_it() {
        let reported = watching(["atlas", "ferry"]);

        assert_eq!(
            reported.take("  atlas \n"),
            Answer::Watched("atlas".to_string())
        );
    }

    #[test]
    fn a_message_naming_nothing_bdi_watches_is_said_back_and_nothing_else() {
        let reported = watching(["atlas", "ferry"]);

        assert_eq!(
            reported.take("ghost"),
            Answer::Unwatched("ghost".to_string())
        );
        assert_eq!(
            reported.uncovered(A_WHILE),
            Uncovered::Everything,
            "a name bdi does not watch stands no poll down"
        );
    }

    #[test]
    fn a_message_that_names_no_project_is_malformed() {
        let reported = watching(["atlas", "ferry"]);

        for message in ["", "\n", "   ", "\t \r\n"] {
            assert_eq!(reported.take(message), Answer::Malformed, "for {message:?}");
        }
    }

    /// A writer that streams without ever ending its line is broken, and the
    /// message it is building cannot be a project name by the time it is this
    /// long.
    #[test]
    fn a_message_too_long_to_be_a_project_name_is_malformed() {
        let reported = watching(["atlas", "ferry"]);
        let shout = "a".repeat(LONGEST_MESSAGE + 1);

        assert_eq!(reported.take(&shout), Answer::Malformed);
    }

    #[test]
    fn a_project_something_reports_for_stands_its_poll_down() {
        let reported = watching(["atlas"]);

        assert_eq!(
            reported.uncovered(A_WHILE),
            Uncovered::Everything,
            "nothing has reported for it yet, so it is still polled"
        );
        reported.take("atlas");

        assert_eq!(reported.uncovered(A_WHILE), Uncovered::Nothing);
    }

    /// The saving a mixed setup gets: the project with a producer is left out
    /// of the poll its neighbour still needs, rather than swept up with it.
    #[test]
    fn a_project_nothing_reports_for_is_polled_without_the_ones_that_are() {
        let reported = watching(["atlas", "ferry"]);

        reported.take("atlas");

        assert_eq!(
            reported.uncovered(A_WHILE),
            Uncovered::These(vec!["ferry".to_string()]),
            "ferry has no writer, so the poll still has it to find"
        );
    }

    /// The signal that a live source has gone quiet is the poll resuming: the
    /// view degrades to slow rather than to wrong.
    #[test]
    fn a_project_the_channel_stops_covering_is_polled_again() {
        let reported = watching(["atlas"]);

        reported.take("atlas");

        assert_eq!(
            reported.uncovered(Duration::ZERO),
            Uncovered::Everything,
            "a window that has already closed leaves the project uncovered"
        );
    }

    #[test]
    fn nothing_watched_is_never_covered() {
        let reported = watching([]);

        assert_eq!(
            reported.uncovered(A_WHILE),
            Uncovered::Everything,
            "there is nothing here for a message to stand down"
        );
    }

    #[test]
    fn without_a_runtime_directory_there_is_no_inbound_channel() {
        let (changed, _changes) = mpsc::channel();

        let refused = listen(None, &watching(["atlas", "ferry"]), changed);

        assert!(matches!(refused, Err(Refused::NoRuntimeDirectory)));
    }

    #[test]
    fn a_writer_naming_a_watched_project_wakes_the_loop() {
        let at = a_socket_path("wakes-the-loop");
        let reported = watching(["atlas"]);
        let (_socket, changes) = open(&at, &reported);

        assert_eq!(say(&at, &["atlas\n"]), ["ok atlas"]);

        assert_eq!(
            changes.recv_timeout(A_MOMENT).ok(),
            Some("atlas".to_string()),
            "the loop was told which project to collect"
        );
        assert_eq!(
            reported.uncovered(A_WHILE),
            Uncovered::Nothing,
            "and atlas's poll stood down"
        );
    }

    #[test]
    fn a_writer_naming_something_bdi_does_not_watch_is_told_so_and_the_loop_sleeps_on() {
        let at = a_socket_path("names-a-stranger");
        let (_socket, changes) = open(&at, &watching(["atlas", "ferry"]));

        assert_eq!(say(&at, &["ghost\n"]), ["unknown ghost"]);

        assert!(
            changes.recv_timeout(Duration::from_millis(100)).is_err(),
            "nothing bdi watches changed, so there is nothing to collect"
        );
    }

    /// A message from outside bdi must not be able to take the view down, or
    /// to cost the messages around it.
    #[test]
    fn a_malformed_message_is_dropped_without_disturbing_the_ones_beside_it() {
        let at = a_socket_path("malformed");
        let (_socket, changes) = open(&at, &watching(["atlas", "ferry"]));

        assert_eq!(
            say(&at, &["\n", "atlas\n"]),
            ["malformed", "ok atlas"],
            "the channel carried on from the bad line to the good one"
        );
        assert!(changes.recv_timeout(A_MOMENT).is_ok());
    }

    /// The shape this interface is for: a producer that connects once and
    /// speaks whenever it has something to say.
    #[test]
    fn a_writer_may_stay_and_speak_more_than_once() {
        let at = a_socket_path("stays-and-speaks");
        let (_socket, changes) = open(&at, &watching(["atlas", "ferry"]));

        assert_eq!(say(&at, &["atlas\n", "ferry\n"]), ["ok atlas", "ok ferry"]);

        for said in ["atlas", "ferry"] {
            assert_eq!(
                changes.recv_timeout(A_MOMENT).ok(),
                Some(said.to_string()),
                "the channel did not carry {said} on"
            );
        }
    }

    #[test]
    fn writers_that_know_nothing_of_each_other_are_all_heard() {
        let at = a_socket_path("several-writers");
        let (_socket, changes) = open(&at, &watching(["atlas", "ferry"]));

        assert_eq!(say(&at, &["atlas\n"]), ["ok atlas"]);
        assert_eq!(say(&at, &["ferry\n"]), ["ok ferry"]);

        for said in ["atlas", "ferry"] {
            assert_eq!(
                changes.recv_timeout(A_MOMENT).ok(),
                Some(said.to_string()),
                "the channel did not carry {said} on"
            );
        }
    }

    /// A `UnixListener` leaves its file behind, so a run that crashed leaves
    /// one nothing is listening on.
    #[test]
    fn a_stale_socket_from_a_crashed_run_is_reclaimed() {
        let at = a_socket_path("stale-socket");
        std::fs::create_dir_all(at.parent().expect("the socket is in a directory"))
            .expect("a directory to put the socket in");
        drop(UnixListener::bind(&at).expect("a socket the crashed run left"));
        assert!(at.exists(), "the crashed run's socket is still there");

        let reported = watching(["atlas", "ferry"]);
        let (_socket, changes) = open(&at, &reported);

        assert_eq!(say(&at, &["atlas\n"]), ["ok atlas"]);
        assert!(changes.recv_timeout(A_MOMENT).is_ok());
    }

    /// Two `bdi`s in one session is a different thing from a crashed one, and
    /// stealing the socket would leave the live one deaf.
    #[test]
    fn a_socket_another_bdi_is_listening_on_is_left_alone() {
        let at = a_socket_path("two-bdis");
        let reported = watching(["atlas", "ferry"]);
        let (_first, _changes) = open(&at, &reported);

        let (changed, _changes) = mpsc::channel();
        let second = listen(Some(at.clone()), &reported, changed);

        assert!(matches!(second, Err(Refused::AlreadyListening(_))));
        assert_eq!(
            say(&at, &["atlas\n"]),
            ["ok atlas"],
            "the first bdi is still listening"
        );
    }

    /// The line that reaches the primary screen carries the half no notice
    /// can: the path, and a way of asking who holds it that is live when the
    /// reader asks rather than as old as this line.
    ///
    /// Both commands are asserted whole, flags and all, because each one is
    /// a flag away from a form that reads as having worked. Without `-p`,
    /// `ss` prints the socket's inode where a reader expects a pid —
    /// measured 2026-08-31 against this machine's own squatted socket,
    /// `7385342` against a holder of `355214` — and it still matches, still
    /// prints a line, and there is still a number on it. Without `-U`,
    /// `lsof` answers the same question in 303,936 lines instead of 829,
    /// measured the same day on the same machine. Nothing else in the suite
    /// would go red for a command that had quietly stopped naming a holder.
    #[test]
    fn the_line_left_on_the_primary_screen_says_how_to_find_who_is_holding_it() {
        let said = Refused::AlreadyListening(PathBuf::from("/run/user/1000/x.sock")).to_string();

        assert!(said.contains("another bdi"), "{said}");
        assert!(said.contains("/run/user/1000/x.sock"), "{said}");
        assert!(said.contains("ss -lxp or lsof -U names which"), "{said}");
    }

    /// The remedy is two steps and the second one is the one a reader would
    /// not guess: `listen` is called once, from `wire`, before the screen
    /// opens, and nothing binds again for the life of the run. So a reader
    /// who closes the holder and waits gets a channel that is free and a
    /// `bdi` that will never take it, which looks exactly like the fault
    /// they were trying to clear.
    ///
    /// Held here rather than at the call site because this is the sentence
    /// that makes the promise, and a sentence is what would quietly stop
    /// being true if a retry were ever added and this went unchanged.
    #[test]
    fn the_remedy_says_to_restart_because_the_socket_is_asked_for_only_once() {
        let said = Refused::AlreadyListening(PathBuf::from("/run/user/1000/x.sock")).to_string();

        assert!(said.contains("restart bdi"), "{said}");
    }

    #[test]
    fn the_socket_goes_with_the_run_that_made_it() {
        let at = a_socket_path("removed-on-exit");
        let (socket, _changes) = open(&at, &watching(["atlas", "ferry"]));
        assert!(at.exists());

        drop(socket);

        assert!(!at.exists(), "the next run has nothing to reclaim");
    }

    #[test]
    fn writers_find_bdi_under_the_directory_the_session_owns() {
        let socket = under(Some(Path::new("/run/user/1000")));

        assert_eq!(
            socket,
            Some(PathBuf::from("/run/user/1000/beady-eye/changes.sock"))
        );
        assert_eq!(under(None), None);
    }
}
