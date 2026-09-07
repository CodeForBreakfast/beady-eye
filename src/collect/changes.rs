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
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::{fmt, fs, thread};

/// The variable naming the directory this login session owns, which is where
/// the socket goes when nothing tells `bdi` where to put it.
///
/// A directory this session owns is one no other user can reach and none is
/// needed to create in, so where a derived socket sat said who could reach
/// it. A told path can sit anywhere, so that is no longer a fact about every
/// run: [`OWNER_ONLY`] is what each run does for itself about who may reach
/// the socket, and [`only_this_user_may_take_a_name_under`] is what it asks about who
/// may replace it.
const RUNTIME_DIRECTORY: &str = "XDG_RUNTIME_DIR";

/// Where `bdi` puts its socket inside that directory.
const SOCKET: &str = "beady-eye/changes.sock";

/// Nothing this long is a project name, so a writer still building a line at
/// this point is broken. Reading stops here, which is what keeps a writer
/// that never ends its line from being read into memory without limit.
const LONGEST_MESSAGE: usize = 512;

/// The mode the socket is created with, whatever umask the run was started
/// under and wherever it was told to put it.
///
/// Set on every run rather than left to where the socket sits, because where
/// it sits stopped being a fact about it: a derived path is under a directory
/// no other user can reach, and a told path need not be and often will not —
/// `/tmp` is world-traversable.
///
/// Both platforms `bdi` runs on check these bits when something connects, so
/// this is what keeps other users off the channel rather than a hope about
/// where the socket sits. Measured 2026-09-07 on Linux 7.2.3 and on Darwin
/// 25.6.0 with one program: a socket its own owner sets to `0400` refuses
/// that owner and one set to `0200` takes them, which is write permission
/// being checked rather than the bits being read and ignored; and a socket
/// `0600` under another user refuses this one. Darwin's `unix(4)` says the
/// same in its own words — *the destination of a `connect(2)` or `sendto(2)`
/// must be writable*.
const OWNER_ONLY: u32 = 0o600;

/// The mode of a directory `bdi` makes to put a socket in.
///
/// Execute as well as read, since entering is what a directory is for. It
/// covers the moment between the socket appearing and [`OWNER_ONLY`] being
/// set on it, which nothing about the socket itself can.
const ONLY_THIS_USER_MAY_ENTER: u32 = 0o700;

/// The bits by which a directory lets a group put a name in it and take one
/// out, and [`ANYBODY_MAY_TAKE_NAMES`] the same for everybody else.
///
/// Write and search, both. Making a name needs the directory searched as well
/// as written, so reading the write bit alone would refuse a directory
/// nobody but its owner can touch. Measured 2026-09-07 on Linux 7.2.3 and on
/// Darwin 25.6.0: a directory its owner sets to `0600` refuses that owner a
/// new name and one set to `0300` takes it.
const A_GROUP_MAY_TAKE_NAMES: u32 = 0o030;
const ANYBODY_MAY_TAKE_NAMES: u32 = 0o003;

/// The bit by which a directory keeps each of its names for whoever owns
/// them, however many people may write there. `/tmp` and `/var/tmp` carry it
/// on both platforms.
///
/// It keeps a name from everybody but the directory's own owner, who may
/// still take any name in it — so it is worth something only where that owner
/// is somebody this run is content to be interfered with by.
const NAMES_STAY_THEIR_OWNERS: u32 = 0o1000;

/// The one owner besides this user that a directory on the socket's way down
/// may have.
///
/// `root` can reach anything on the machine whatever a directory says, so a
/// rule refusing it would buy nothing. It is also who owns every shared
/// directory a reader reaches for — `/tmp` and `/var/tmp` on both platforms.
const THE_SYSTEM: u32 = 0;

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
    /// Nothing told this run where to listen and this session owns no runtime
    /// directory to put a socket under, so there is no path to open. A
    /// machine that has no runtime directory at all — macOS — is refused for
    /// this reason until it is told one.
    NoRuntimeDirectory,
    /// Another `bdi` is listening there already, so this one has the channel
    /// only when that one lets it go — or when one of them is told a
    /// different path.
    AlreadyListening(PathBuf),
    /// Something that is not a socket is already at the path, so the path is
    /// not this run's to take. Reachable only where a run was told where to
    /// listen: a derived path names a file `bdi` puts there itself.
    NotASocket(PathBuf),
    /// A directory on the way down to the socket is one another user may take
    /// a name in, so nothing bound beneath it stays what was bound. Carries
    /// that directory rather than the socket's own, which may be several
    /// below it. Reachable only where a run was told where to listen: a
    /// directory `bdi` makes is [`ONLY_THIS_USER_MAY_ENTER`], and the runtime
    /// directory a derived path sits under is this session's alone.
    NameOthersMayTake(PathBuf),
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
            // The refusal a machine can be in for ever, so the one whose
            // remedy has to travel with it. A reader here has no runtime
            // directory to make appear and nothing to close, and until they
            // are told the path exists the sentence reads as a verdict on
            // their machine rather than as something to set. Both ways of
            // telling it, because they answer different questions: the key
            // is what a Mac wants every run, the flag is what a second `bdi`
            // beside a first wants once.
            Refused::NoRuntimeDirectory => {
                write!(
                    f,
                    "this session has no {RUNTIME_DIRECTORY} to put the socket in — name a path with --socket, or with socket under [changes] in the config, and bdi listens there"
                )
            }
            // The one refusal a reader answers by closing something, so the
            // one that says how to find what to close. The foot can name the
            // cause and no more; naming a process is a thing to be done here,
            // where there is room for the path and for a way of asking who
            // holds it that is live when the reader asks rather than as old
            // as this line.
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
            // The remedy is a different path, and which path is the reader's
            // to choose — so what this owes them is what is in the way. A
            // reader who typed one character wrong recognises the name and
            // needs nothing else; one who meant it learns that `bdi` will not
            // take the file, which is the answer either way.
            Refused::NotASocket(at) => {
                write!(
                    f,
                    "{} is not a socket and bdi will not take it — name another path with --socket, or with socket under [changes] in the config",
                    at.display()
                )
            }
            // The directory named is the one at fault, which is not always
            // the one the reader typed the socket into — so the remedy is a
            // path with nothing of that kind above it rather than a deeper
            // name, which under a shared directory would be advice to go
            // further into it.
            Refused::NameOthersMayTake(directory) => {
                write!(
                    f,
                    "another user may take a name in {} — name a socket path with no such directory above it, with --socket or with socket under [changes] in the config",
                    directory.display()
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
    /// The file this run bound, so the one it clears away is that file and
    /// not whatever holds the name by then.
    bound: Option<File>,
}

/// Which file a name holds, as the filesystem tells them apart. Nothing where
/// the name holds nothing, or holds something that cannot be read.
///
/// Whose hands the name can change into is what
/// [`only_this_user_may_take_a_name_under`] settles, and it leaves this user's own: a
/// reader may remove the socket and put something else at the name while
/// `bdi` is up, and that file is theirs. So asking is still worth it, and
/// what it buys is the direction it is wrong in — every case it catches and
/// every case it cannot read leave the file alone, and the only cost of
/// leaving a socket of ours behind is that the next run reclaims it.
type File = (u64, u64);

fn file_at(named: &Path) -> Option<File> {
    fs::symlink_metadata(named)
        .ok()
        .map(|what| (what.dev(), what.ino()))
}

impl Drop for Socket {
    /// Only where the name still holds the file this run bound. The reader
    /// owns the name and may unlink the socket and give the name to something
    /// else while `bdi` is up — and removing that on the way out is their
    /// file gone, exactly as reclaiming it would have been at the other end
    /// of the run.
    ///
    /// Leaving one behind instead costs nothing: a socket of ours that
    /// nothing is listening on is what the next run reclaims.
    fn drop(&mut self) {
        if file_at(&self.at) == self.bound {
            let _ = fs::remove_file(&self.at);
        }
    }
}

/// Where a writer finds `bdi`: the path this run was told to listen on, or
/// the one under the directory this session owns where it was told none.
///
/// Nothing where neither, which is the one way left to have nowhere to put a
/// socket and is what [`Refused::NoRuntimeDirectory`] reports.
pub fn where_writers_find_bdi(told: Option<PathBuf>) -> Option<PathBuf> {
    told.or_else(|| {
        under(
            std::env::var_os(RUNTIME_DIRECTORY)
                .map(PathBuf::from)
                .as_deref(),
        )
    })
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
    // Read before anything else this run does, so the name has had as little
    // time as it can to change hands. It cannot be read from the listener
    // instead: a bound socket's descriptor stats as its own inode on sockfs,
    // which is a different device from the directory entry the name is.
    let bound = file_at(&at);

    let reported = reported.clone();
    thread::spawn(move || accept(&listener, &reported, &changed));

    Ok(Socket { at, bound })
}

fn bind(at: &Path) -> Result<UnixListener, Refused> {
    let directory = directory_holding(at);

    // Made this user's own from the moment it exists, rather than left to
    // umask and narrowed afterwards. A socket is connectable the instant
    // `bind` returns and takes its mode from umask until the line below
    // changes it, so a directory nobody else may enter is what covers
    // that. It reaches only directories this run makes: `recursive` takes
    // one that is already there as it stands, which is right — an
    // existing directory is somebody's, and how it is set is theirs.
    fs::DirBuilder::new()
        .recursive(true)
        .mode(ONLY_THIS_USER_MAY_ENTER)
        .create(directory)
        .map_err(|why| Refused::Unopenable(at.to_path_buf(), why))?;

    // Asked after the directory is made, so a directory `bdi` made answers
    // for itself rather than not being there to answer.
    only_this_user_may_take_a_name_under(directory)?;

    let listener = match UnixListener::bind(at) {
        Ok(listener) => listener,
        Err(taken) if taken.kind() == ErrorKind::AddrInUse => reclaim(at)?,
        Err(why) => return Err(Refused::Unopenable(at.to_path_buf(), why)),
    };

    fs::set_permissions(at, fs::Permissions::from_mode(OWNER_ONLY))
        .map_err(|why| Refused::Unopenable(at.to_path_buf(), why))?;

    Ok(listener)
}

/// The directory the socket's name is in, as a directory rather than as what
/// `Path` says about the spelling.
///
/// A bare name is a name in the directory the run was started in, and
/// `Path::parent` gives that as the empty path — which nothing can be asked
/// about, and which would leave `--socket changes.sock` unjudged in whatever
/// directory a reader happened to run `bdi` from.
fn directory_holding(at: &Path) -> &Path {
    match at.parent() {
        Some(directory) if !directory.as_os_str().is_empty() => directory,
        _ => Path::new("."),
    }
}

/// Whether the whole way down to the socket is this user's or the system's,
/// refusing for the nearest directory somebody else may take a name in — the
/// nearest one, that is, that puts the socket within their reach.
///
/// Every directory on the way, rather than the one the socket sits in.
/// Renaming a directory aside and putting your own there gives you every name
/// beneath it, so a private directory under a shared one is as open as the
/// shared one — and the remedy `bdi` names is a directory deeper, which would
/// otherwise be advice to walk further into the same hole.
///
/// Both how the path is spelled and what it resolves to, because neither
/// covers the other. A link is followed to somewhere else entirely, so what a
/// name means is what the links in it point at; and a link is *reached
/// through* the directory holding it without appearing anywhere beneath what
/// it points at, so a resolved way down alone would judge where a link goes
/// and never the directory anybody may repoint it from.
///
/// Together they are what makes the answer keep: every directory either way
/// down is one no other user may write in, so there is nobody left to move a
/// link or a directory between this answer and the `bind` that follows it.
///
/// **A way down that cannot be read is refused, not passed.** Reading it is
/// how the socket's own directory is cleared, so an unreadable one is a
/// directory nothing has cleared — and the owner of a directory above it can
/// make the reading fail on purpose. A symlink pointed at itself for the
/// moment this runs, and back afterwards, would otherwise carry a path
/// straight through to `bind` with every check skipped.
///
/// That is the whole reason it does not defer to `bind`, which touches the
/// path next and would say what went wrong with it in hand. `bind` asks a
/// different question, and gets its answer after this one has been acted on.
fn only_this_user_may_take_a_name_under(under: &Path) -> Result<(), Refused> {
    let unreadable = |directory: &Path| {
        let directory = directory.to_path_buf();
        move |why| Refused::Unopenable(directory, why)
    };

    let resolved = fs::canonicalize(under).map_err(unreadable(under))?;
    let this_user = this_user();

    for directory in directories_on(under).chain(directories_on(&resolved)) {
        let what = fs::metadata(directory).map_err(unreadable(directory))?;
        if others_may_take_a_name_in(what.permissions().mode(), what.uid(), this_user) {
            return Err(Refused::NameOthersMayTake(directory.to_path_buf()));
        }
    }

    Ok(())
}

/// The directory at the end of this way down and every one above it, nearest
/// first.
///
/// **The root is among them only where it is the end, and the two halves of
/// that are both load-bearing.** Above the socket's own directory it is
/// nobody's to answer for: a refusal is answered by naming another path, no
/// path leaves the root out, and a root somebody else owns is a whole
/// filesystem somebody else owns — `/` inside a nix build sandbox belongs to
/// `65534`. Where the socket's own name is *in* the root there is another
/// path to name, one directory deeper, so that one is judged like any other.
/// Skipping it there instead is an unjudged bind on a directory everybody can
/// see, which is what this whole walk exists to stop.
fn directories_on(way: &Path) -> impl Iterator<Item = &Path> {
    way.ancestors()
        .enumerate()
        .filter(|(above, directory)| *above == 0 || directory.parent().is_some())
        .map(|(_, directory)| directory)
}

/// Whom this run is, which is the only party besides [`THE_SYSTEM`] a
/// directory on the socket's way down may belong to.
///
/// There is no safe `std` call that says which user a process is.
fn this_user() -> u32 {
    // SAFETY: `getuid` takes no arguments, reads no memory and is defined to
    // succeed on every unix.
    unsafe { libc::getuid() }
}

/// Whether somebody other than this user could put their own file at a name
/// in a directory owned and set like this.
///
/// An owner may always take any name in their own directory, so a directory
/// belonging to somebody else is one they may take the socket's name in
/// however narrowly it is set — a directory of theirs at `0755` as much as one
/// at `0777`. That is the first half, and it is the half a mode cannot say.
///
/// The second is who else may write there. Write permission outside the owner
/// is what lets anybody else touch the directory's names at all, and the
/// sticky bit takes it back for names that already exist, since with it only a
/// name's own owner may remove or rename it. So a socket bound in `/tmp` is
/// still that socket for as long as it is there, however many people may write
/// beside it.
///
/// This is what lets the two unlinks be safe rather than merely careful. Each
/// looks at what is at the name and then removes it, and no unlink takes a
/// file to check against, so on their own they narrow the window rather than
/// closing it. Where the name cannot change hands there is no window: the
/// file is this user's and removing it is this user's to do, or it is
/// somebody else's and the remove is refused.
fn others_may_take_a_name_in(how: u32, owner: u32, this_user: u32) -> bool {
    if owner != this_user && owner != THE_SYSTEM {
        return true;
    }
    let anybody_else = how & A_GROUP_MAY_TAKE_NAMES == A_GROUP_MAY_TAKE_NAMES
        || how & ANYBODY_MAY_TAKE_NAMES == ANYBODY_MAY_TAKE_NAMES;
    anybody_else && how & NAMES_STAY_THEIR_OWNERS == 0
}

/// A socket already at the path is either a live `bdi`'s or the litter of one
/// that crashed — a `UnixListener` leaves its file behind when its process
/// goes. Connecting tells them apart: a live listener accepts, and a file
/// nothing is listening on refuses.
///
/// Anything that is not a socket is neither, and removing it is how a
/// mistyped path costs a reader a file. `bind` answers *address already in
/// use* for every kind of thing in the way, and a `connect` to a regular file
/// is refused exactly as a dead socket's is — so what is there has to be
/// looked at rather than inferred from either of them.
fn reclaim(at: &Path) -> Result<UnixListener, Refused> {
    if UnixStream::connect(at).is_ok() {
        return Err(Refused::AlreadyListening(at.to_path_buf()));
    }

    if !is_a_socket(at) {
        return Err(Refused::NotASocket(at.to_path_buf()));
    }

    fs::remove_file(at).map_err(|why| Refused::Unopenable(at.to_path_buf(), why))?;
    UnixListener::bind(at).map_err(|why| Refused::Unopenable(at.to_path_buf(), why))
}

/// Whether the path holds a socket, as it stands now.
///
/// A path that cannot be read at all is not one to remove either, so it
/// answers no: the caller refuses, and the reader is told what stood in the
/// way rather than losing it.
fn is_a_socket(at: &Path) -> bool {
    fs::symlink_metadata(at).is_ok_and(|what| what.file_type().is_socket())
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

    /// A socket named straight in a directory whose mode is the thing under
    /// test, rather than in one `bdi` made and set for itself.
    ///
    /// The directory under test sits inside one nobody else may enter, so a
    /// test that opens a directory up opens nothing up in `/tmp`.
    fn a_socket_in_a_directory_moded(named: &str, how: u32) -> PathBuf {
        let around = std::env::temp_dir().join(format!("bdi-{named}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&around);
        std::fs::create_dir_all(&around).expect("a directory to keep the test's own in");
        std::fs::set_permissions(
            &around,
            std::fs::Permissions::from_mode(ONLY_THIS_USER_MAY_ENTER),
        )
        .expect("and nobody else may enter it");

        let told = around.join("told");
        std::fs::create_dir_all(&told).expect("a directory to put the socket in");
        std::fs::set_permissions(&told, std::fs::Permissions::from_mode(how))
            .expect("set as the test means it rather than as umask left it");
        told.join("changes.sock")
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

    /// A socket is connectable the instant `bind` returns and wears whatever
    /// umask gave it until its mode is set, so nothing about the socket
    /// itself covers that moment. The directory does, for as long as it is
    /// one this run made.
    #[test]
    fn the_directory_bdi_makes_for_its_socket_is_this_users_own() {
        let at = a_socket_path("directory-mode");
        let (_socket, _changes) = open(&at, &watching(["atlas"]));

        let directory = at.parent().expect("the socket is in a directory");
        let mode = std::fs::metadata(directory)
            .expect("the directory bdi made")
            .permissions()
            .mode()
            & 0o777;

        assert_eq!(
            mode, ONLY_THIS_USER_MAY_ENTER,
            "nobody else may enter the directory bdi made to put its socket in"
        );
    }

    /// Where the name can change hands, looking at what is at it buys a
    /// narrower window and never a closed one, because no unlink takes a file
    /// to check against. So the remedy is the directory rather than a sharper
    /// look: `bdi` does not bind where the name is not its own to keep.
    #[test]
    fn a_directory_others_may_take_a_name_in_is_one_bdi_will_not_bind_in() {
        let at = a_socket_in_a_directory_moded("open-directory", 0o777);

        let (changed, _changes) = mpsc::channel();
        let refused = listen(Some(at.clone()), &watching(["atlas"]), changed);

        let named = match refused.err() {
            Some(Refused::NameOthersMayTake(directory)) => directory,
            otherwise => panic!("a directory anyone may write in is refused, not {otherwise:?}"),
        };
        assert_eq!(
            std::fs::canonicalize(&named).ok(),
            at.parent()
                .and_then(|directory| std::fs::canonicalize(directory).ok()),
            "and it is that directory the refusal names"
        );
        assert!(!at.exists(), "and nothing of bdi's is left at the name");
    }

    /// An unreadable way down is one nothing has cleared, so it is refused
    /// rather than let through. The owner of a directory above the socket can
    /// make the reading fail whenever they like — a link pointed at itself
    /// for the moment the check runs, and back before `bind` follows it — so
    /// passing an unreadable way down would hand them every check at once.
    ///
    /// Asked of the walk directly, because `bind` makes the directory before
    /// it asks and would meet the same link one call earlier.
    #[test]
    fn a_way_down_that_cannot_be_read_is_refused_rather_than_passed() {
        let around = std::env::temp_dir().join(format!("bdi-loop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&around);
        std::fs::create_dir_all(&around).expect("a directory to keep the test's own in");

        let itself = around.join("itself");
        std::os::unix::fs::symlink("itself", &itself).expect("a link pointed at its own name");

        match only_this_user_may_take_a_name_under(&itself) {
            Err(Refused::Unopenable(named, _)) => assert_eq!(
                named, itself,
                "and the refusal names the way down it could not read"
            ),
            otherwise => panic!("a way down that cannot be read is refused, not {otherwise:?}"),
        }
    }

    /// Renaming a directory aside and putting your own there gives you every
    /// name beneath it, so a directory of this user's own under a shared one
    /// is as open as the shared one — and the remedy the refusal names would
    /// otherwise be advice to walk further into it.
    #[test]
    fn a_private_directory_under_one_others_may_take_a_name_in_is_refused_for_that_one() {
        let shared = a_socket_in_a_directory_moded("shared-above", 0o777)
            .parent()
            .expect("the socket is in a directory")
            .to_path_buf();
        let at = shared.join("mine").join("changes.sock");

        let (changed, _changes) = mpsc::channel();
        let refused = listen(Some(at.clone()), &watching(["atlas"]), changed);

        let named = match refused.err() {
            Some(Refused::NameOthersMayTake(directory)) => directory,
            otherwise => panic!("a path under a shared directory is refused, not {otherwise:?}"),
        };
        assert_eq!(
            std::fs::canonicalize(&named).ok(),
            std::fs::canonicalize(&shared).ok(),
            "and the refusal names the directory at fault rather than the socket's own"
        );
    }

    /// A link is reached through the directory holding it and appears nowhere
    /// beneath what it points at, so a resolved way down on its own judges
    /// where the link goes and never the directory anybody may repoint it
    /// from — leaving the socket to be bound wherever it is pointed next.
    #[test]
    fn a_link_out_of_a_shared_directory_is_refused_for_the_directory_holding_it() {
        let shared = a_socket_in_a_directory_moded("shared-holding-a-link", 0o777)
            .parent()
            .expect("the socket is in a directory")
            .to_path_buf();
        let mine = shared
            .parent()
            .expect("the test's own directory is around it")
            .join("mine");
        std::fs::create_dir_all(&mine).expect("somewhere of this user's own to point at");
        std::os::unix::fs::symlink(&mine, shared.join("link")).expect("a link anybody may repoint");

        let at = shared.join("link").join("changes.sock");
        let (changed, _changes) = mpsc::channel();
        let refused = listen(Some(at), &watching(["atlas"]), changed);

        let named = match refused.err() {
            Some(Refused::NameOthersMayTake(directory)) => directory,
            otherwise => panic!("a path through a shared directory is refused, not {otherwise:?}"),
        };
        assert_eq!(
            std::fs::canonicalize(&named).ok(),
            std::fs::canonicalize(&shared).ok(),
            "and it is the directory holding the link that is named, which resolving loses"
        );
    }

    /// A bare name is judged in the directory the run was started in, which
    /// is what `--socket changes.sock` means and is not what `Path::parent`
    /// says it means. Left as the empty path it is a name nothing can be
    /// asked about, in whatever directory a reader happened to be.
    #[test]
    fn a_socket_named_without_a_directory_is_judged_where_the_run_was_started() {
        assert_eq!(directory_holding(Path::new("changes.sock")), Path::new("."));
        assert_eq!(
            directory_holding(Path::new("beady-eye/changes.sock")),
            Path::new("beady-eye")
        );
        assert_eq!(
            directory_holding(Path::new("/tmp/changes.sock")),
            Path::new("/tmp")
        );
    }

    /// The root is left alone above the socket, because no path a reader
    /// could name instead leaves it out. Where the socket's own name is in
    /// it there is such a path — one directory deeper — so it is judged like
    /// any other, and skipping it there would be the empty parent again in
    /// another spelling.
    #[test]
    fn the_root_is_judged_where_the_name_is_in_it_and_left_alone_above() {
        assert_eq!(
            directories_on(Path::new("/")).collect::<Vec<_>>(),
            [Path::new("/")],
            "a socket named in the root is judged by the root"
        );
        assert_eq!(
            directories_on(Path::new("/tmp/beady-eye")).collect::<Vec<_>>(),
            [Path::new("/tmp/beady-eye"), Path::new("/tmp")],
            "and above the socket's own directory the root is left out"
        );
    }

    /// Two things a mode cannot say on its own, and the rows that say them.
    ///
    /// A directory belonging to somebody else is one they may take any name
    /// in however narrowly it is set, so `0755` under them is no better than
    /// `0777` under anybody. And the sticky bit is worth something only under
    /// an owner this run is content to be interfered with by, since that
    /// owner may take a name in it whatever the bit says.
    ///
    /// Asked of the rule rather than of a directory, because no machine a
    /// test runs on has a directory in somebody else's name to point it at.
    #[test]
    fn a_directory_is_this_users_to_bind_under_by_its_owner_as_well_as_its_mode() {
        let me = 501;
        let them = 1000;

        for (how, owner, taken, what) in [
            (0o700, me, false, "a directory of this user's own"),
            (0o755, THE_SYSTEM, false, "one of the system's"),
            (0o755, them, true, "a narrow one somebody else owns"),
            (0o700, them, true, "even a private one somebody else owns"),
            (
                0o777,
                me,
                true,
                "one of this user's anybody may take a name in",
            ),
            (0o770, me, true, "one of this user's their group may"),
            (0o760, me, false, "one their group may write and not search"),
            (0o1777, me, false, "a sticky one of this user's"),
            (
                0o1777,
                THE_SYSTEM,
                false,
                "a sticky one of the system's, which /tmp is",
            ),
            (0o1777, them, true, "a sticky one somebody else owns"),
        ] {
            assert_eq!(
                others_may_take_a_name_in(how, owner, me),
                taken,
                "{what} ({how:04o}, owner {owner})"
            );
        }
    }

    /// The name is the reader's, and they may take the socket off it and put
    /// their own file there while the run is up — so what holds the name when
    /// a run ends need not be what that run bound. Removing it then is the
    /// same file loss reclaiming would have been, at the other end of the run.
    #[test]
    fn a_file_that_replaced_the_socket_under_a_run_outlives_it() {
        let at = a_socket_path("replaced-socket");
        let (socket, _changes) = open(&at, &watching(["atlas"]));

        std::fs::remove_file(&at).expect("somebody else takes the name");
        std::fs::write(&at, "what they put there").expect("and leaves their own file at it");

        drop(socket);

        assert_eq!(
            std::fs::read_to_string(&at).ok().as_deref(),
            Some("what they put there"),
            "the name no longer holds the socket this run bound, so it is not this run's to clear"
        );
    }

    /// A path a run is told is a path a person typed, and one keystroke is
    /// all that separates the name of a socket from the name of a file they
    /// need. What is there is neither a live `bdi` nor a crashed one's
    /// litter, so it is not `bdi`'s to clear away to make room.
    #[test]
    fn a_path_holding_something_that_is_not_a_socket_is_left_where_it_is() {
        let at = a_socket_path("not-a-socket");
        std::fs::create_dir_all(at.parent().expect("the socket is in a directory"))
            .expect("a directory to put the socket in");
        std::fs::write(&at, "what the reader meant to keep").expect("a file to be typed over");

        let (changed, _changes) = mpsc::channel();
        let refused = listen(Some(at.clone()), &watching(["atlas"]), changed);

        assert!(
            matches!(refused, Err(Refused::NotASocket(_))),
            "a path holding something else is refused for what is there"
        );
        assert_eq!(
            std::fs::read_to_string(&at).ok().as_deref(),
            Some("what the reader meant to keep"),
            "the file is still the reader's"
        );
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

    /// A reader with no runtime directory cannot make one appear and has
    /// nothing to close, so without the remedy the line is a verdict on their
    /// machine. Both ways of naming a path are asserted because they answer
    /// different questions and a reader arrives with one of them: a Mac wants
    /// the key on every run, a second `bdi` beside a first wants the flag
    /// once.
    #[test]
    fn the_line_for_a_session_that_owns_no_directory_says_how_to_name_a_path() {
        let said = Refused::NoRuntimeDirectory.to_string();

        assert!(said.contains(RUNTIME_DIRECTORY), "{said}");
        assert!(said.contains("--socket"), "{said}");
        assert!(said.contains("socket under [changes]"), "{said}");
    }

    #[test]
    fn the_socket_goes_with_the_run_that_made_it() {
        let at = a_socket_path("removed-on-exit");
        let (socket, _changes) = open(&at, &watching(["atlas", "ferry"]));
        assert!(at.exists());

        drop(socket);

        assert!(!at.exists(), "the next run has nothing to reclaim");
    }

    /// Told nothing, `bdi` listens where it has always listened, so a run
    /// with no config keeps the path every producer already written against
    /// it uses.
    /// Told nothing, `bdi` listens where it has always listened, so a run
    /// with no config keeps the path every producer already written against
    /// it uses.
    #[test]
    fn writers_find_bdi_under_the_directory_the_session_owns() {
        let socket = under(Some(Path::new("/run/user/1000")));

        assert_eq!(
            socket,
            Some(PathBuf::from("/run/user/1000/beady-eye/changes.sock"))
        );
        assert_eq!(under(None), None);
    }

    /// Told where to listen, `bdi` listens there, and what the session owns
    /// is not consulted at all — which is what makes this deterministic
    /// wherever it runs. That is what lets two `bdi`s in one session each
    /// have a channel, and it is the only way a machine with no runtime
    /// directory has one.
    #[test]
    fn a_run_told_where_to_listen_listens_there() {
        let told = PathBuf::from("/var/folders/T/bdi/changes.sock");

        assert_eq!(where_writers_find_bdi(Some(told.clone())), Some(told));
    }
}
