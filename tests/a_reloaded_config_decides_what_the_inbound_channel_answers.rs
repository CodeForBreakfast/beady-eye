//! What the inbound channel says `ok` to follows the config the reader has
//! written, not the one the run started on.
//!
//! The socket's answer is a claim about which projects `bdi` is reading —
//! *the name must match a project's `name` in the config* — and it is the
//! only thing a writer ever learns, because the answer goes back down the
//! socket rather than onto a screen the writer is not watching. So a set
//! settled at startup does not merely go stale in the direction the reader
//! notices. It goes stale in both, and only one of them can be seen: a
//! project the reader has added is refused, and a project they have removed
//! is still accepted and still asks for a read of a project no longer
//! collected.
//!
//! Driven through the binary because nothing below it can say this. The
//! socket is opened once in `wire`, before the screen, and the config is read
//! again on the loop's own deadline — what is being asked is whether the
//! second reaches the first, and the two meet nowhere else.
//!
//! One connection, opened before the edit and used after it, because that is
//! the shape the protocol is for: a long-running producer connects once and
//! speaks whenever it has something to say. A test that reconnected would ask
//! a weaker question, since a fresh connection is entitled to a fresh
//! reading of anything.
//!
//! The screen is what the edit is waited on, not a sleep. The project the
//! edit gained is drawn only after the reload has been taken and the
//! collection it asked for has come back, and the accepted names are settled
//! on the loop's own thread before that collection is asked for — so a frame
//! carrying the new project is proof the new names are in force, on every
//! machine and at no fixed speed.

mod terminal;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::ShimmedTracker;
use terminal::{a_socket_of_its_own, THE_DESCRIBED_SUBTREE};

const ROWS: u16 = 40;
const COLS: u16 = 120;

const ATLAS: &[u8] = "atlas".as_bytes();
const FERRY: &[u8] = "ferry".as_bytes();

/// Long enough for a check to fall due, the collection it asks for to be
/// made, and the frame that draws what came back. The check's own interval
/// is `tui::reload::CHECKED_EVERY`, which is not a config setting and so
/// cannot be shortened for a test.
const A_RELOAD_AND_ITS_COLLECTION: Duration = Duration::from_secs(20);

/// Long enough that an answer which was coming has, and short enough that a
/// test waiting for one that is not is a failure rather than a hang.
const AN_ANSWER: Duration = Duration::from_secs(10);

/// A `HOME` whose config names `projects`, each in a directory of its own
/// under it.
///
/// Under it rather than at it, so the directory `bdi` is started in — the
/// `HOME` itself — belongs to no project and the run reads every project the
/// file names. A project at the directory `bdi` was started in would scope
/// the run to itself, and a project added beside it would then be left out
/// for that reason rather than for any reason this test is about.
fn a_home_naming(named: &str, projects: &[&str]) -> PathBuf {
    let home = std::env::temp_dir().join(format!("bdi-{named}-{}", std::process::id()));
    std::fs::create_dir_all(home.join(".config/beady-eye")).expect("the directory is ours to make");
    naming(&home, projects);
    home
}

/// The config rewritten to name `projects` and nothing else, as a reader
/// editing the file leaves it.
fn naming(home: &Path, projects: &[&str]) {
    let written: String = projects
        .iter()
        .map(|project| {
            let path = home.join(project);
            std::fs::create_dir_all(&path).expect("the directory is ours to make");
            format!(
                "[[projects]]\nname = \"{project}\"\npath = \"{}\"\n\n",
                path.display()
            )
        })
        .collect();
    std::fs::write(home.join(".config/beady-eye/config.toml"), written)
        .expect("the config is ours to write");
}

/// Something outside `bdi` saying a project's work has moved on, holding its
/// connection open across the reader's edit the way a real one does.
struct Producer {
    speaking: UnixStream,
    listening: BufReader<UnixStream>,
}

impl Producer {
    /// Connected to the run whose runtime directory is this `HOME`.
    fn connected_to(home: &Path) -> Self {
        let at = home.join("beady-eye/changes.sock");
        let speaking = UnixStream::connect(&at)
            .unwrap_or_else(|why| panic!("bdi is listening on {} ({why})", at.display()));
        let listening = speaking
            .try_clone()
            .expect("the connection is ours to read");
        listening
            .set_read_timeout(Some(AN_ANSWER))
            .expect("a read that is not answered is ours to give up on");
        Self {
            speaking,
            listening: BufReader::new(listening),
        }
    }

    /// Say one project's work has moved, and hand back what `bdi` answered.
    fn says(&mut self, project: &str) -> String {
        writeln!(self.speaking, "{project}").expect("the message is ours to send");
        let mut answer = String::new();
        self.listening
            .read_line(&mut answer)
            .expect("bdi answers every line");
        answer.trim_end().to_string()
    }
}

#[test]
fn the_names_the_channel_accepts_follow_the_config_the_reader_wrote() {
    let home = a_home_naming("channel-follows-config", &["atlas"]);
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_DESCRIBED_SUBTREE);
    let mut environment = tracker.environment();
    environment.push(a_socket_of_its_own(&home));

    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &environment);
    bdi.read_until(ATLAS, GIVING_UP);

    let mut producer = Producer::connected_to(&home);
    assert_eq!(
        producer.says("atlas"),
        "ok atlas",
        "the run started on a config naming atlas, so the channel accepts \
         it\n{}",
        bdi.timeline()
    );
    assert_eq!(
        producer.says("ferry"),
        "unknown ferry",
        "and refuses a project no config has ever named\n{}",
        bdi.timeline()
    );

    naming(&home, &["ferry"]);
    bdi.read_until(FERRY, A_RELOAD_AND_ITS_COLLECTION);

    // One edit, both directions, judged together. They are two halves of one
    // staleness rather than two facts, and only the first half has anything
    // on screen to show it — so a pair of assertions would stop at the half
    // the reader can already see and leave the other unmeasured on exactly
    // the runs where it had moved.
    let answers = (producer.says("ferry"), producer.says("atlas"));

    assert_eq!(
        (answers.0.as_str(), answers.1.as_str()),
        ("ok ferry", "unknown atlas"),
        "the config the reader wrote decides what the channel accepts: the \
         project they added is one bdi is now reading, and the one they took \
         out is not. An `ok` for the removed project asks for a read of one \
         bdi no longer collects, and tells the only party who could put it \
         right that bdi is watching a project it is not.\n{}",
        bdi.timeline()
    );
}
