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

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
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

/// The projects `bdi` watches, which is the whole of what the channel needs
/// to know: a message either names one of them or it does not, and that is
/// the answer the writer gets back.
///
/// Nothing here decides whether a project is polled. What a poll would have
/// found is already said by the read a report causes: each read arms the
/// project's next ask one interval further out, so a project something keeps
/// reporting for never comes due, and one whose producer stops comes due an
/// interval after its last read. See `tui::armed`.
///
/// **The answer is a claim about the config in force, so this is written
/// whenever that config is.** A set settled at startup goes stale in both
/// directions and only one of them can be seen: a project the reader adds is
/// refused, and a project they remove is still accepted and still asks for a
/// read of a project no longer collected. The writer is the only party who
/// could put a wrong name right, and both halves of that mislead them.
///
/// **The critical section is a pointer copy and cannot panic.** The set is
/// replaced whole rather than edited in place, and a connection thread takes
/// the current one out from under the lock before it looks anything up — so
/// the lookup holds nothing, a reload never waits on a connection, and a
/// writer that panics mid-message can take nothing down with it. There is
/// still no state for it to leave half-written, which is what a channel
/// anything may write to has to be able to say.
#[derive(Clone, Default)]
pub struct Reported {
    projects: Arc<Mutex<Arc<BTreeSet<String>>>>,
}

impl Reported {
    pub fn watching<I: IntoIterator<Item = String>>(projects: I) -> Self {
        Self {
            projects: Arc::new(Mutex::new(Arc::new(projects.into_iter().collect()))),
        }
    }

    /// The projects `bdi` reads from here on, as a config the reader has
    /// written names them.
    ///
    /// Called with the same list that decides which projects poll, so what
    /// the channel accepts and what the loop asks for cannot come apart.
    pub fn now_watching<I: IntoIterator<Item = String>>(&self, projects: I) {
        *self.held() = Arc::new(projects.into_iter().collect());
    }

    /// Take one message, and say what `bdi` made of it.
    pub fn take(&self, message: &str) -> Answer {
        let named = message.trim();
        if named.is_empty() || named.len() > LONGEST_MESSAGE {
            return Answer::Malformed;
        }

        // Taken out from under the lock, so the lookup below holds nothing:
        // a set arriving between here and there is one this message was
        // sent too early to be answered against, and waiting for it would
        // mean holding the lock across a search on a writer's behalf.
        let watching = Arc::clone(&self.held());
        if watching.contains(named) {
            Answer::Watched(named.to_string())
        } else {
            Answer::Unwatched(named.to_string())
        }
    }

    /// The set, for the moment it takes to copy a pointer or replace one.
    ///
    /// A poisoned lock is recovered from rather than propagated. Nothing
    /// inside the critical section can panic, so nothing here can poison it
    /// — and if something one day did, the value under it is a whole set
    /// swapped for another and not a half-written one, so there is nothing
    /// for a refusal to protect. Answering `unknown` to every writer for the
    /// rest of the run because an unrelated thread died is the disappearance
    /// this project is built not to do.
    fn held(&self) -> MutexGuard<'_, Arc<BTreeSet<String>>> {
        self.projects.lock().unwrap_or_else(PoisonError::into_inner)
    }
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
    use std::any::Any;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::Shutdown;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::panic::AssertUnwindSafe;
    use std::path::Path;
    use std::sync::mpsc::{self, Receiver};
    use std::time::Duration;

    /// Long enough that a channel which was going to report has, and short
    /// enough that a test waiting in vain is not a hang.
    const A_MOMENT: Duration = Duration::from_secs(5);

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

    /// Both ends of one connection to bdi: what messages go down, and what
    /// its answers come back up.
    fn connect(at: &Path) -> (UnixStream, BufReader<UnixStream>) {
        let writing = UnixStream::connect(at).expect("bdi is listening");
        let reading = BufReader::new(writing.try_clone().expect("both ends of the stream"));
        (writing, reading)
    }

    /// Hold one conversation with bdi: each line goes down the connection
    /// with the answer expected back for it.
    ///
    /// Each answer is asserted before the line after it is sent. A bdi that
    /// hangs up on a writer breaks the next write as well as the answer that
    /// never came, and a broken pipe would fail the test without saying which
    /// line it was; so the answer is what a test fails on, and a bdi that has
    /// gone is reported by the line it was not there for.
    #[track_caller]
    fn say(at: &Path, exchanges: &[(&str, &str)]) {
        let (mut writing, mut reading) = connect(at);
        let mut answered = Vec::new();

        for (line, expected) in exchanges {
            let mut said = String::new();
            let heard = write!(writing, "{line}").and_then(|()| reading.read_line(&mut said));
            assert!(
                matches!(heard, Ok(1..)),
                "bdi let the writer go at {line:?}, after answering {answered:?}"
            );
            assert_eq!(said.trim_end(), *expected, "bdi's answer to {line:?}");
            answered.push(said.trim_end().to_string());
        }
    }

    fn message_of(panic: Box<dyn Any + Send>) -> String {
        panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|said| said.to_string()))
            .expect("the failure carried a message")
    }

    /// Every conversation with bdi in this module goes through `say`, so
    /// what it reports when bdi stops answering is what a mutant that
    /// hangs up on a writer is caught by. A stand-in that answers one line
    /// and drops the next shows the red naming the message bdi was not
    /// there for, and how far the conversation got, rather than the pipe
    /// that closed under the write.
    #[test]
    fn a_bdi_that_lets_a_writer_go_is_reported_by_the_message_it_never_answered() {
        let at = a_socket_path("lets-the-writer-go");
        std::fs::create_dir_all(at.parent().expect("the socket is in a directory"))
            .expect("a directory to put the socket in");
        let listener = UnixListener::bind(&at).expect("a stand-in for bdi");
        let stand_in = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("the writer connects");
            let mut reading = BufReader::new(stream.try_clone().expect("both ends of the stream"));
            let mut line = String::new();
            reading.read_line(&mut line).expect("the first message");
            writeln!(&stream, "ok atlas").expect("the answer goes back");
            line.clear();
            reading.read_line(&mut line).expect("the second message");
        });

        let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
            say(&at, &[("atlas\n", "ok atlas"), ("ferry\n", "ok ferry")]);
        }));
        stand_in.join().expect("the stand-in ran to its end");

        let red = message_of(outcome.expect_err("say noticed bdi had gone"));
        assert!(
            red.contains("\"ferry\\n\""),
            "the red names the message bdi never answered: {red}"
        );
        assert!(
            red.contains("\"ok atlas\""),
            "the red says how far the conversation got: {red}"
        );
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
    }

    /// Both directions of the one write, because they fail apart: a set that
    /// only gained the new names would answer for every project the reader
    /// ever configured, which is the accepting half of the same staleness.
    #[test]
    fn the_projects_written_are_the_ones_taken_from_then_on() {
        let reported = watching(["atlas"]);

        reported.now_watching(["ferry".to_string()]);

        assert_eq!(reported.take("ferry"), Answer::Watched("ferry".to_string()));
        assert_eq!(
            reported.take("atlas"),
            Answer::Unwatched("atlas".to_string())
        );
    }

    /// The point of the write, and what a set copied into each connection
    /// would not do: a thread holding its own `Reported` since before the
    /// write answers against what was written, not against what it was
    /// handed. Every connection thread is exactly this clone.
    #[test]
    fn a_clone_taken_before_the_write_takes_against_what_was_written() {
        let reported = watching(["atlas"]);
        let held_by_a_connection = reported.clone();

        reported.now_watching(["ferry".to_string()]);

        assert_eq!(
            held_by_a_connection.take("ferry"),
            Answer::Watched("ferry".to_string())
        );
        assert_eq!(
            held_by_a_connection.take("atlas"),
            Answer::Unwatched("atlas".to_string())
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

        say(&at, &[("atlas\n", "ok atlas")]);

        assert_eq!(
            changes.recv_timeout(A_MOMENT).ok(),
            Some("atlas".to_string()),
            "the loop was told which project to collect"
        );
    }

    #[test]
    fn a_writer_naming_something_bdi_does_not_watch_is_told_so_and_the_loop_sleeps_on() {
        let at = a_socket_path("names-a-stranger");
        let (_socket, changes) = open(&at, &watching(["atlas", "ferry"]));

        say(&at, &[("ghost\n", "unknown ghost")]);

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

        say(&at, &[("\n", "malformed"), ("atlas\n", "ok atlas")]);
        assert!(changes.recv_timeout(A_MOMENT).is_ok());
    }

    /// A writer still building a line when the room runs out has stopped
    /// speaking messages, and what bdi has read of it is a prefix rather
    /// than a name — however much the prefix looks like one. Acting on it
    /// would refresh a project on the strength of half a line, and reading
    /// on would answer the rest of that line as if each piece were its own
    /// message, so the writer is answered once and let go.
    ///
    /// The write half is closed after the line so that a bdi which reads on
    /// runs out of bytes and answers rather than waiting: a test that hangs
    /// where it should fail says nothing.
    #[test]
    fn a_line_that_never_ends_is_malformed_and_the_writer_is_let_go() {
        let at = a_socket_path("never-ends");
        let reported = watching(["atlas"]);
        let (_socket, changes) = open(&at, &reported);

        let (mut writing, mut reading) = connect(&at);
        let unending = format!("atlas{}", " ".repeat(LONGEST_MESSAGE * 2));
        write!(writing, "{unending}").expect("bdi takes the message");
        writing
            .shutdown(Shutdown::Write)
            .expect("the writer has said all it is going to");

        let mut said = String::new();
        reading.read_line(&mut said).expect("bdi answers");
        assert_eq!(
            said.trim_end(),
            "malformed",
            "the prefix bdi read is not the name it spells"
        );

        let mut afterwards = String::new();
        reading
            .read_to_string(&mut afterwards)
            .expect("bdi is done");
        assert_eq!(
            afterwards, "",
            "bdi let the writer go rather than answering the rest of its line"
        );
        assert!(
            changes.recv_timeout(Duration::from_millis(100)).is_err(),
            "nothing bdi watches was named, so there is nothing to collect"
        );
    }

    /// The line carries its own newline, so the longest name that still
    /// fits in the room is one byte short of it. A writer sitting on that
    /// boundary is heard, and the message after it is still read as the
    /// next message rather than as the tail of this one.
    #[test]
    fn the_longest_message_that_still_ends_is_taken() {
        let at = a_socket_path("longest-that-ends");
        let brink = "a".repeat(LONGEST_MESSAGE - 1);
        let reported = watching([brink.as_str(), "atlas"]);
        let (_socket, changes) = open(&at, &reported);

        say(
            &at,
            &[
                (&format!("{brink}\n"), &format!("ok {brink}")),
                ("atlas\n", "ok atlas"),
            ],
        );

        assert_eq!(
            changes.recv_timeout(A_MOMENT).ok(),
            Some(brink),
            "the loop was told to collect for the name on the boundary"
        );
    }

    /// The shape this interface is for: a producer that connects once and
    /// speaks whenever it has something to say.
    #[test]
    fn a_writer_may_stay_and_speak_more_than_once() {
        let at = a_socket_path("stays-and-speaks");
        let (_socket, changes) = open(&at, &watching(["atlas", "ferry"]));

        say(&at, &[("atlas\n", "ok atlas"), ("ferry\n", "ok ferry")]);

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

        say(&at, &[("atlas\n", "ok atlas")]);
        say(&at, &[("ferry\n", "ok ferry")]);

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

        say(&at, &[("atlas\n", "ok atlas")]);
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
        say(&at, &[("atlas\n", "ok atlas")]);
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
    /// Only the sentence is asserted, and nothing holds it to the bind it
    /// describes. A retry added to `wire` would make the restart a lie and
    /// leave this green, and nothing else in the suite would go red either.
    /// Tying the two means driving a `bdi` against a held socket, freeing it,
    /// and waiting out a retry interval that does not exist — a timeout
    /// standing in for an assertion, over a property that is true by
    /// construction today. So a retry is a change to this sentence too, and
    /// this paragraph is what says so.
    #[test]
    fn the_remedy_for_a_held_socket_says_to_restart_bdi() {
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
