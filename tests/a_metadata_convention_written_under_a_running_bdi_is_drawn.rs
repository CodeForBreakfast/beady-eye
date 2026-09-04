//! `[[badges]]` and `[join] pane_key` written under a running `bdi` change
//! what is drawn beside a bead, with no restart.
//!
//! These two carry a project's own conventions, and `bdi` draws them without
//! interpretation — so what a reload changes is which metadata key is drawn
//! and which one ties a pane to a bead, never what `bdi` takes either to
//! mean. Nothing here teaches it a convention: the tracker holds three keys a
//! reader invented, and the config decides which of them the screen shows.
//!
//! Driven through the binary because that is the only place the question
//! lives. Both keys are read on the collector's side of the seam, which has
//! carried a reloaded config since `bdi-8un.2` — so the claim being checked
//! is not that the drawing follows the config, which `model/` says on its
//! own, but that the config reaching it is the file as the reader has just
//! left it. A test below the binary would be asking `model/` a question it
//! answers by construction.
//!
//! **Both assertions wait for something to arrive.** A reader who moves a
//! convention from one key to another could be tested either way round, and
//! the way round that asserts an absence is the one that passes when the row
//! it names has moved rather than gone. So each test starts from a config
//! that draws nothing and waits for the drawing.

mod terminal;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::{ShimmedHerdr, ShimmedTracker};
use terminal::{a_socket_of_its_own, contains, row_of, ENTER_ALTERNATE_SCREEN};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// The keys that draw every tree and open the one there is, so the bead
/// under it is on the screen: every tree rather than only the staffed ones,
/// back to the first row, down onto the tree, open it — and back and down
/// again, because the key that opens a tree is also the key that steps into
/// one.
///
/// `a` is what makes the walk the same whichever way the join goes. These
/// tests move which pane is tied to which bead, and a tree drawn only while
/// something is joined to it would come and go for the very reason under
/// test.
const OPEN_THE_TREE: &[u8] = b"agjlgj";

/// One tracker, holding one bead that carries three metadata keys a reader
/// invented. `bdi` knows nothing about any of them.
///
/// `agent_pane` names a pane herdr does not report and `seat` names the one
/// it does, so which key `[join]` is pointed at decides whether anything
/// joins — rather than both keys agreeing and the setting making no
/// difference either way.
const THE_TRACKER: &str = r#"[
  {"id":"orb-1","title":"lift the ground station","status":"open",
   "priority":1,"issue_type":"epic"},
  {"id":"orb-1.1","title":"repoint the dish","status":"in_progress","parent":"orb-1",
   "dependencies":[{"depends_on_id":"orb-1","type":"parent-child"}],
   "priority":2,"issue_type":"task",
   "metadata":{"agent_pane":"wT:p9","seat":"wT:p1","phase":"trimming-sails"}}
]"#;

/// A word of the bead's title, which is what names its row here.
///
/// Its title and not its id, because the forest draws a nested bead's id as
/// the part that is its own: `orb-1.1` under `orb-1` is drawn `.1`, and a
/// test looking for the whole id finds nothing on a row that is right there.
/// One word, because a row reaches the wire a word at a time with a cursor
/// move where each space would be — so a needle with a space in it is in no
/// run of the stream, and only the reassembled screen holds it.
const THE_BEADS_ROW: &[u8] = "repoint".as_bytes();

/// What the reader's own `phase` key holds on it, which is what a badge on
/// that key draws and nothing else on the screen says.
const ITS_PHASE: &[u8] = "trimming-sails".as_bytes();

/// A word of the title of the one pane herdr reports, which is what names
/// its row wherever the row is.
///
/// Its title rather than its `display_agent`, because the two rows draw
/// different things about a pane: a bead the join put a pane on says the
/// pane's title and state, and a pane no bead claims says its id and who is
/// sitting in it as well. The title is on whichever row the pane is drawn on,
/// which is the only thing these tests move.
const ITS_PANE: &[u8] = "manifest".as_bytes();

/// Long enough for a check to fall due and for the frame that draws what it
/// found. `tui::reload::CHECKED_EVERY` is a constant rather than a config
/// setting, so nothing here can shorten it.
const A_RELOAD: Duration = Duration::from_secs(20);

/// A gap this long between bytes means the collection is over and drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// A herdr session holding one working pane in the configured project's
/// directory, under the id the tracker's `seat` key names.
fn a_session_with_one_pane(cwd: &str) -> String {
    format!(
        r#"{{"id":"cli:agent:list","result":{{"type":"agent_list","agents":[
            {{"pane_id":"wT:p1","cwd":"{cwd}","agent_status":"working",
             "display_agent":"quartermaster","title":"the manifest"}}
        ]}}}}"#
    )
}

/// A `HOME` whose config names one project and carries `settings`.
fn a_home_settled(named: &str, settings: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("bdi-{named}-{}", std::process::id()));
    std::fs::create_dir_all(home.join(".config/beady-eye")).expect("the directory is ours to make");
    settling(&home, settings);
    home
}

/// The config rewritten to carry `settings`, as a reader editing the file
/// leaves it.
fn settling(home: &Path, settings: &str) {
    std::fs::write(
        home.join(".config/beady-eye/config.toml"),
        format!(
            "[[projects]]\nname = \"atlas\"\npath = \"{}\"\n{settings}",
            home.display()
        ),
    )
    .expect("the config is ours to write");
}

/// A `bdi` over the tracker above, with every tree drawn.
fn a_bdi_over_the_tracker(home: &Path, settings: &str) -> Driven {
    settling(home, settings);
    let tracker = ShimmedTracker::beside(home);
    tracker.holds(THE_TRACKER);
    let herdr = ShimmedHerdr::beside(home);
    herdr.lists(&a_session_with_one_pane(&home.display().to_string()));
    let mut environment = tracker.environment();
    environment.extend(herdr.environment());
    environment.push(a_socket_of_its_own(home));

    let mut bdi = Driven::bdi(ROWS, COLS, home.to_path_buf(), &environment);
    bdi.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);
    bdi.send(OPEN_THE_TREE);
    bdi.settle(A_SILENCE, GIVING_UP);
    bdi
}

/// One frame with every cell of the screen in it. A resize is what asks for
/// one, so the height alternates: a resize to the height the terminal already
/// is is no resize at all.
fn a_whole_frame(bdi: &mut Driven, rows: &mut u16) -> Vec<u8> {
    *rows = if *rows == ROWS { ROWS - 1 } else { ROWS };
    let whole = bdi.resize(*rows, COLS);
    bdi.answer_to(whole, GIVING_UP)
}

/// Whole frames until the bead and the agent are drawn on one row, handed
/// back either way — a frame that still has them apart is what the assertion
/// needs to fail against.
fn frames_until_joined(bdi: &mut Driven, rows: &mut u16, patience: Duration) -> Vec<u8> {
    let giving_up = Instant::now() + patience;
    loop {
        let frame = a_whole_frame(bdi, rows);
        let joined = row_of(&frame, THE_BEADS_ROW).is_some()
            && row_of(&frame, THE_BEADS_ROW) == row_of(&frame, ITS_PANE);
        if joined || Instant::now() >= giving_up {
            return frame;
        }
    }
}

#[test]
fn a_badge_the_reader_adds_is_drawn_without_a_restart() {
    let home = a_home_settled("badge-reloads", "");
    let mut bdi = a_bdi_over_the_tracker(&home, "");

    let mut rows = ROWS;
    let before = a_whole_frame(&mut bdi, &mut rows);
    assert!(
        row_of(&before, THE_BEADS_ROW).is_some(),
        "the bead carrying the key is on the screen, so what follows is about \
         its badge rather than about the row being missing\n{}",
        bdi.timeline()
    );
    assert!(
        !contains(&before, ITS_PHASE),
        "and nothing draws the reader's own key until they configure a badge \
         on it\n{}",
        bdi.timeline()
    );

    settling(&home, "\n[[badges]]\nkey = \"phase\"\nrender = \"{}\"\n");
    bdi.read_until(ITS_PHASE, A_RELOAD);
}

#[test]
fn a_join_key_the_reader_moves_puts_the_agent_on_the_bead_without_a_restart() {
    let home = a_home_settled("join-reloads", "");
    let joining_on_a_key_nothing_holds = "\n[join]\npane_key = \"agent_pane\"\n";
    let mut bdi = a_bdi_over_the_tracker(&home, joining_on_a_key_nothing_holds);

    let mut rows = ROWS;
    let before = a_whole_frame(&mut bdi, &mut rows);
    let bead = row_of(&before, THE_BEADS_ROW);
    assert!(
        bead.is_some() && row_of(&before, ITS_PANE).is_some(),
        "the bead and the pane are both on the screen, so what follows is \
         about which row the pane is drawn on\n{}",
        bdi.timeline()
    );
    assert_ne!(
        bead,
        row_of(&before, ITS_PANE),
        "and the pane is not on the bead's row, because the key the config \
         joins on names a pane herdr does not report\n{}",
        bdi.timeline()
    );

    settling(&home, "\n[join]\npane_key = \"seat\"\n");
    let after = frames_until_joined(&mut bdi, &mut rows, A_RELOAD);

    assert_eq!(
        row_of(&after, THE_BEADS_ROW),
        row_of(&after, ITS_PANE),
        "the pane is drawn on the bead the reader's own key ties it to, with \
         no restart\n{}",
        bdi.timeline()
    );
    assert!(
        row_of(&after, THE_BEADS_ROW).is_some(),
        "and both of them are drawn, rather than neither being found\n{}",
        bdi.timeline()
    );
}
