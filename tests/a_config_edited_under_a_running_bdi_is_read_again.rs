//! A config edited while `bdi` is running is read again, and one that will
//! not parse leaves the running config standing and says so at the foot.
//!
//! Driven through the binary because the whole of what this bead adds is a
//! seam between three things that only meet in a real run: the file on disk,
//! the loop's deadline set, and the row at the foot. `Reload`'s own tests say
//! what a check finds and `Shown`'s say what the foot does with one; neither
//! of them can say that a `bdi` a reader is looking at ever looks at the file.
//!
//! The absence half is read off a whole frame rather than off everything
//! `bdi` has ever written. The notice was on the screen a moment before by
//! construction, so a search over the run would find it there and pass
//! whatever the foot says now. A resize is answered by drawing every cell
//! again, which is what makes one frame readable on its own.

mod terminal;

use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::{a_home_naming_one_project, a_socket_of_its_own, contains};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// The configured project's own header, which is on the screen in the first
/// frame — before any tracker has answered, and whether or not this machine
/// is running a herdr for `bdi` to ask about panes. The config is read
/// before the screen opens, so a `bdi` that has drawn this is a `bdi` whose
/// config file has been read once already.
///
/// It has to be that rather than anything a collection puts there: the
/// build sandbox has no herdr and no tracker at the temp `HOME`, so the
/// headings the other pty tests wait for never arrive.
const THE_FIRST_FRAME: &[u8] = "atlas".as_bytes();

/// The notice, from `view::phrase`. Its first words rather than the whole
/// line: the foot is drawn as a difference from the frame before, so a phrase
/// that lands where another had letters reaches the wire in pieces.
const WOULD_NOT_LOAD: &[u8] = "the config would not load".as_bytes();

/// The same row, read off a later frame, where it says `bdi` is still
/// drawing a forest: a broken config does not take it down and does not
/// blank the view.
///
/// It does **not** say the running config stands — nothing consumes a
/// reloaded config yet, so the collector is working from the copy it was
/// handed at startup whatever the reload does with its own. What says the
/// running one stands is `Reload`'s own test, which puts the file back to
/// exactly what was in force and reads `Unchanged` off it.
const THE_CONFIGURED_PROJECT: &[u8] = THE_FIRST_FRAME;

/// Long enough for a check to fall due and be answered. The interval is
/// `tui::reload::CHECKED_EVERY`, which is not a config setting and so cannot
/// be shortened for a test.
const A_CHECK_OR_TWO: Duration = Duration::from_secs(6);

#[test]
fn a_config_that_will_not_parse_leaves_the_running_one_standing_and_says_so() {
    let home = a_home_naming_one_project("config-reload");
    let config = home.join(".config/beady-eye/config.toml");
    let as_written = std::fs::read_to_string(&config).expect("the config is ours to read");
    let environment = [a_socket_of_its_own(&home)];

    let mut bdi = Driven::bdi(ROWS, COLS, home, &environment);
    bdi.read_until(THE_FIRST_FRAME, GIVING_UP);

    std::fs::write(&config, "[[projects]\nthis is not toml\n")
        .expect("the config is ours to write");
    bdi.read_until(WOULD_NOT_LOAD, A_CHECK_OR_TWO);

    let whole = bdi.resize(ROWS - 1, COLS);
    let frame = bdi.answer_to(whole, GIVING_UP);
    assert!(
        contains(&frame, THE_CONFIGURED_PROJECT),
        "a broken config left bdi drawing its forest\n{}",
        bdi.timeline()
    );

    // Undone, which puts the file back to exactly what `bdi` is already
    // working to: nothing reloads, and the notice has to come off all the
    // same.
    std::fs::write(&config, &as_written).expect("the config is ours to write");
    std::thread::sleep(A_CHECK_OR_TWO);

    let whole = bdi.resize(ROWS, COLS);
    let frame = bdi.answer_to(whole, GIVING_UP);
    assert!(
        !contains(&frame, WOULD_NOT_LOAD),
        "the reader has put the config back and the foot still says it \
         would not load\n{}",
        bdi.timeline()
    );
    assert!(
        contains(&frame, THE_CONFIGURED_PROJECT),
        "and the forest is still there to read it off\n{}",
        bdi.timeline()
    );
}
