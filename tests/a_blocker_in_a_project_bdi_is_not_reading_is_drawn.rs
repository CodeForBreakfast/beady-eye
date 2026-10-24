//! A bead named on the command line, waiting on a bead in a project the run
//! was not reading, starts the board focused on it, and putting the forest
//! back draws the rest of its project.
//!
//! Run through the binary, because main decides which projects the run reads,
//! from the directory it was started in and the bead the command line names.

mod terminal;

use std::path::{Path, PathBuf};
use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::ShimmedTracker;

/// Dunwich's beads: the one ferry's waits on, and a tree of its own that
/// nothing ferry holds needs.
const DUNWICH: &str = r#"[
  {"id":"dun-7","title":"lift the ground station","status":"in_progress",
   "priority":1,"issue_type":"epic"},
  {"id":"dun-3","title":"survey the moor","status":"in_progress",
   "priority":1,"issue_type":"task"}
]"#;

/// Ferry's bead waiting on dunwich's, and a tree of ferry's own beside it.
const FERRY_WITH_ANOTHER_TREE: &str = r#"[
  {"id":"fer-2","title":"moor the barge","status":"open","priority":2,
   "issue_type":"task","dependencies":[{"depends_on_id":"dun-7","type":"blocks"}]},
  {"id":"fer-4","title":"caulk the hull","status":"open","priority":2,
   "issue_type":"task"}
]"#;

/// The title of ferry's other tree, which is drawn where nothing is focused.
const FERRYS_OTHER_TREE: &str = "caulk the hull";

/// A `HOME` that is ferry's directory. Its config names ferry there and
/// dunwich in a directory under it, with `dunwich_states` in dunwich's entry.
fn a_home_where_dunwich(named: &str, dunwich_states: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("bdi-{named}-{}", std::process::id()));
    for directory in ["dunwich", ".config/beady-eye"] {
        std::fs::create_dir_all(home.join(directory)).expect("the directory is ours to make");
    }
    std::fs::write(
        the_config_in(&home),
        format!(
            "[[projects]]\nname = \"dunwich\"\npath = \"{}\"\n{dunwich_states}\n\n\
             [[projects]]\nname = \"ferry\"\npath = \"{}\"\n",
            home.join("dunwich").display(),
            home.display()
        ),
    )
    .expect("the config is ours to write");
    home
}

fn the_config_in(home: &Path) -> PathBuf {
    home.join(".config/beady-eye/config.toml")
}

/// The trackers of a home made above, holding `ferry` and `dunwich`.
fn the_trackers_in_holding(home: &Path, ferry: &str, dunwich: &str) -> ShimmedTracker {
    let tracker = ShimmedTracker::beside(home);
    tracker.holds_for("dunwich", dunwich);
    tracker.holds_for(
        home.file_name()
            .and_then(|name| name.to_str())
            .expect("the home is named"),
        ferry,
    );
    tracker
}

const ROWS: u16 = 40;

/// Wide enough for the foot to keep its notice and its keys together.
const COLS: u16 = 160;

/// A gap this long between bytes means the frame is drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// Shift+F, which focuses the forest on the selected bead.
const FOCUS: &[u8] = b"F";

/// A board started on a bead builds only that bead's tree, and putting the
/// forest back builds the rest of the project again.
#[test]
fn putting_back_the_forest_a_named_bead_started_in_draws_the_rest_of_its_project() {
    let mut named = launched("put-back", &["fer-2"]);
    let focused = forest(&repaint(&mut named, ROWS + 1));
    assert!(
        !focused.iter().any(|row| row.contains(FERRYS_OTHER_TREE)),
        "ferry's other tree is drawn while focused: {focused:#?}"
    );

    named.send(FOCUS);
    named.settle(A_SILENCE, GIVING_UP);
    let put_back = forest(&repaint(&mut named, ROWS));

    assert!(
        put_back.iter().any(|row| row.contains(FERRYS_OTHER_TREE)),
        "putting the forest back left ferry's other tree undrawn: {put_back:#?}"
    );
}

/// A `bdi` started in ferry's directory, given `arguments`, with dunwich
/// stating its prefix, and its first collection drawn.
fn launched(named: &str, arguments: &[&str]) -> Driven {
    let home = a_home_where_dunwich(named, "prefix = \"dun\"");
    let mut environment =
        the_trackers_in_holding(&home, FERRY_WITH_ANOTHER_TREE, DUNWICH).environment();
    environment.push(terminal::a_socket_of_its_own(&home));
    let mut bdi = Driven::bdi_with_arguments(ROWS, COLS, home, arguments, &environment);
    bdi.read_until(terminal::ENTER_ALTERNATE_SCREEN, GIVING_UP);
    bdi.settle(A_SILENCE, GIVING_UP);
    bdi
}

/// The screen as it stands, rather than as it differs from the frame before:
/// a resize is answered by drawing every cell again.
#[track_caller]
fn repaint(bdi: &mut Driven, rows: u16) -> Vec<u8> {
    let repainted = bdi.resize(rows, COLS);
    bdi.answer_to(repainted, GIVING_UP)
}

/// The forest's rows, down to the line saying what the run reads. A project
/// line is cut at the age of its read, which differs between two runs.
fn forest(screen: &[u8]) -> Vec<String> {
    terminal::rows_drawn(screen)
        .into_iter()
        .take_while(|row| !row.trim_start().starts_with("reading "))
        .map(|row| match row.split_once("  ✓") {
            Some((project, _)) => project.to_string(),
            None => row.trim_end().to_string(),
        })
        .collect()
}
