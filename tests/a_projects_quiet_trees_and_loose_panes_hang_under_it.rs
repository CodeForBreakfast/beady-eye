//! Everything beneath a project sits under its one line: the trees the
//! live-agent filter holds back, and the panes working in its paths that no
//! bead claims. Graeme: *"at the moment i have to look in 3 different places
//! to see everything"*.
//!
//! One project, read from a shimmed `bd`, with two roots: one a pane is
//! working in, one nobody is. A shimmed `herdr` reports that pane and a
//! second one in the same directory that names no bead. What the binary then
//! draws under the project's line, counting rows from the top, is:
//!
//! ```text
//! 0 ▾ atlas
//! 1   ├── ○ atl-1  raise the beacon
//! 2   │   └── ◐ .1  trim the wick          ◍ wT:p2 working
//! 3   ├─▸ 1 tree with no live agent        a to show all
//! 4   └── ⚠ 1 unattributed pane
//! 5       └── ◍ wT:p3 idle                 <the project's path>
//! 6 ▸ ⚠ 1 project whose tracker could not be read
//! ```
//!
//! The rows are read by what a click on them does, because a repaint reaches
//! the wire a word at a time and a row is not a string a test can look for.
//! Clicking a row selects it, so a key pressed next answers for that row and
//! the answer says what the row was.
//!
//! The second project is there to be the last line: a group below the trees
//! rather than under a project, so the rows above it are the project's or
//! the screen has drawn the quiet tree and the loose pane below the trees
//! rather than under the project. It is a project whose directory does not
//! exist, which is the cheapest tracker that cannot be read.

mod terminal;

use std::time::Duration;

use std::path::PathBuf;
use terminal::driver::{clicked_on, Driven, GIVING_UP};
use terminal::shims::{ShimmedHerdr, ShimmedTracker};

use terminal::{a_socket_of_its_own, contains, ENTER_ALTERNATE_SCREEN};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// A gap this long between bytes means the collection is over and drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// Two roots of the one project. `atl-1.1` names the pane working on it, so
/// its tree has a live agent and rests open onto it; nobody is on `atl-2`,
/// so the filter holds that tree back.
const THE_TRACKER: &str = r#"[
  {"id":"atl-1","title":"raise the beacon","status":"open",
   "priority":1,"issue_type":"epic","dependencies":[]},
  {"id":"atl-1.1","title":"trim the wick","status":"in_progress",
   "priority":2,"issue_type":"task",
   "metadata":{"agent_pane":"wT:p2"},"parent":"atl-1",
   "dependencies":[{"depends_on_id":"atl-1","type":"parent-child"}]},
  {"id":"atl-2","title":"dredge the harbour","status":"open",
   "priority":2,"issue_type":"epic","dependencies":[]},
  {"id":"atl-2.1","title":"survey the silt","status":"open",
   "priority":2,"issue_type":"task","parent":"atl-2",
   "dependencies":[{"depends_on_id":"atl-2","type":"parent-child"}]}
]"#;

/// The quiet root, and one word of the bead beneath it that nothing else on
/// the screen says. The bead's word is what tells the tree open from the
/// tree shut.
const THE_QUIET_ROOT: &[u8] = b"atl-2";
const A_WORD_BENEATH_IT: &[u8] = b"silt";
/// One word of the quiet root's title, which only the bead window says in
/// full beside its id.
const A_WORD_OF_ITS_TITLE: &[u8] = b"dredge";

/// The pane no bead claims, and what is on it. The band draws a pane's text
/// in the pane's own colours, so one word of it is what reaches the wire.
const THE_LOOSE_PANE: &[u8] = b"wT:p3";
const ON_THE_LOOSE_PANE: &str = "moored at the quay\n";
const A_WORD_ON_IT: &[u8] = b"moored";

/// The word the line over the loose panes says, from `view::phrase`.
const NO_BEAD_CLAIMS_THEM: &[u8] = b"unattributed";

/// Where the rows are, counting from the top of the screen, before and after
/// the quiet trees are opened.
const THE_QUIET_TREES: u16 = 3;
const THE_QUIET_ROOTS_ROW_ONCE_OPENED: u16 = 4;
/// The loose pane's row once the quiet tree above it is open onto its one
/// bead: two rows further down than it started.
const THE_LOOSE_PANES_ROW_ONCE_OPENED: u16 = 7;

/// A `HOME` naming the project under test in a directory of its own, and a
/// second project in a directory that is not there, so its tracker cannot be
/// read and the group saying so is drawn below the trees. `bdi` is started in
/// `HOME` itself, which is under neither, so the directory chooses no scope
/// and both are read.
fn a_home_naming_a_project_and_one_that_cannot_be_read(named: &str) -> (PathBuf, PathBuf) {
    let home = std::env::temp_dir().join(format!("bdi-{named}-{}", std::process::id()));
    let atlas = home.join("atlas");
    std::fs::create_dir_all(&atlas).expect("the directory is ours to make");
    std::fs::create_dir_all(home.join(".config/beady-eye")).expect("the directory is ours to make");
    std::fs::write(
        home.join(".config/beady-eye/config.toml"),
        format!(
            "[[projects]]\nname = \"atlas\"\npath = \"{}\"\n\n\
             [[projects]]\nname = \"beacon\"\npath = \"/nowhere/beacon\"\n",
            atlas.display()
        ),
    )
    .expect("the config is ours to write");
    (home, atlas)
}

const OPEN_IT: &[u8] = b"l";
const SHOW_IT: &[u8] = b"\r";
const BACK: &[u8] = b"\x1b";
const REFRESH: &[u8] = b"\x12";

#[test]
fn a_projects_quiet_trees_and_loose_panes_hang_under_its_own_line() {
    let (home, atlas) = a_home_naming_a_project_and_one_that_cannot_be_read("under-one-line");
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_TRACKER);
    let herdr = ShimmedHerdr::beside(&home);
    herdr.lists(&format!(
        r#"{{"result":{{"agents":[
          {{"pane_id":"wT:p2","cwd":"{atlas}","agent_status":"working"}},
          {{"pane_id":"wT:p3","cwd":"{atlas}","agent_status":"idle"}}
        ]}}}}"#,
        atlas = atlas.display()
    ));
    herdr.shows(ON_THE_LOOSE_PANE);
    let mut environment = tracker.environment();
    environment.extend(herdr.environment());
    environment.push(a_socket_of_its_own(&home));

    let mut bdi = Driven::bdi(ROWS, COLS, home, &environment);
    bdi.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);
    assert_eq!(
        tracker.unanswered(),
        Vec::<String>::new(),
        "bd was asked something the shim had no answer for"
    );

    // The first screen: the quiet tree is behind its line and the loose pane
    // is on the screen under its own.
    let screen = repainted(&mut bdi);
    assert!(
        contains(&screen, NO_BEAD_CLAIMS_THEM) && contains(&screen, THE_LOOSE_PANE),
        "the loose pane is not drawn under its project: {}",
        shown(&screen)
    );
    assert!(
        !contains(&screen, THE_QUIET_ROOT),
        "the quiet tree is drawn before its line is opened: {}",
        shown(&screen)
    );

    // Opening the line draws the quiet tree's root, shut over its bead.
    bdi.send(&clicked_on(THE_QUIET_TREES));
    bdi.send(OPEN_IT);
    let screen = repainted(&mut bdi);
    assert!(
        contains(&screen, THE_QUIET_ROOT) && !contains(&screen, A_WORD_BENEATH_IT),
        "the quiet root is not drawn shut under the line: {}",
        shown(&screen)
    );

    // The root is a bead's row: it opens by hand, and Enter shows the bead.
    bdi.send(&clicked_on(THE_QUIET_ROOTS_ROW_ONCE_OPENED));
    bdi.send(OPEN_IT);
    let screen = repainted(&mut bdi);
    assert!(
        contains(&screen, A_WORD_BENEATH_IT),
        "the quiet tree did not open onto its bead: {}",
        shown(&screen)
    );
    bdi.send(SHOW_IT);
    bdi.read_until(A_WORD_OF_ITS_TITLE, GIVING_UP);
    bdi.send(BACK);
    bdi.settle(A_SILENCE, GIVING_UP);

    // A refresh keeps the fold where the reader left it.
    bdi.send(REFRESH);
    bdi.settle(A_SILENCE, GIVING_UP);
    let screen = repainted(&mut bdi);
    assert!(
        contains(&screen, A_WORD_BENEATH_IT),
        "the refresh shut the tree the reader opened: {}",
        shown(&screen)
    );

    // The loose pane's row is the pane's: selecting it puts what is on the
    // pane in the band.
    bdi.send(&clicked_on(THE_LOOSE_PANES_ROW_ONCE_OPENED));
    bdi.read_until(A_WORD_ON_IT, GIVING_UP);
}

/// The whole screen, drawn again from nothing, so what it holds can be read
/// rather than only what changed since the last frame.
fn repainted(bdi: &mut Driven) -> Vec<u8> {
    bdi.settle(A_SILENCE, GIVING_UP);
    let mark = bdi.resize(ROWS + 1, COLS);
    let screen = bdi.answer_to(mark, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);
    let mark = bdi.resize(ROWS, COLS);
    let mut back = bdi.answer_to(mark, GIVING_UP);
    back.extend(screen);
    back
}

fn shown(screen: &[u8]) -> String {
    String::from_utf8_lossy(screen).into_owned()
}
