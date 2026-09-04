//! `[theme] background` written under a running `bdi` reaches the band.
//!
//! Driven through the binary because the band's treatment is the one thing on
//! the screen only a terminal shows. `palette::voice` is a unit test's to
//! check and it answers about a `Background` handed to it; what nothing below
//! the binary can say is that the `Background` reaching it is the one in the
//! file the reader has just written rather than the one the run started on.
//!
//! **The absence is the dim attribute and not a word.** Both backgrounds draw
//! the same sentence in the same place — the band says there is no pane to
//! read either way — so a test looking for text to move would be a test that
//! passes on a `bdi` that changed nothing. What the reader's
//! declaration decides is the treatment: SGR `2` over the terminal's own
//! foreground on a dark background, colour 8 on a light one, and `voice` is
//! the only thing on the screen that asks for dim. So the sentence is what
//! says the band is still there, and the attribute is what says which palette
//! drew it.
//!
//! Read off frames `bdi` was made to repaint whole, and not off the stream.
//! The stream carries every byte of every frame, so the attribute the run
//! opened on is in it for as long as the run lasts and an absence read there
//! would be an absence from the whole run's drawing rather than from this
//! frame.
//!
//! And the frames are asked for repeatedly rather than once after a sleep.
//! The reload falls due on `tui::reload::CHECKED_EVERY`, which is not a config
//! setting and cannot be shortened for a test; a single frame taken at a
//! guessed instant is a test whose margin is whatever the machine had left.

mod terminal;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use terminal::driver::{Driven, GIVING_UP};
use terminal::shims::ShimmedTracker;
use terminal::{a_socket_of_its_own, contains, row_of, THE_DESCRIBED_SUBTREE};

const ROWS: u16 = 40;
const COLS: u16 = 120;

/// What the band says where nothing answers for panes, which is every run
/// here: the shims refuse a herdr call rather than hand it on.
///
/// Looked for on the screen the frame would draw rather than in its bytes: a
/// row reaches the wire as its words with a cursor move between each of them,
/// so a sentence is in no run of the stream. The foot says something of its
/// own about herdr, and this is the half of the band's sentence that is the
/// band's alone.
const NO_PANE_TO_READ: &[u8] = "there is no pane to read".as_bytes();

/// The dim attribute, which is the whole of the dark background's answer for
/// the band and is asked for nowhere else on the screen.
const DIM: &[u8] = b"\x1b[2m";

/// Long enough for a check to fall due and for the frame that draws what it
/// found.
const A_RELOAD: Duration = Duration::from_secs(20);

/// A `HOME` whose config names one project and declares `background`.
fn a_home_on(named: &str, background: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("bdi-{named}-{}", std::process::id()));
    std::fs::create_dir_all(home.join(".config/beady-eye")).expect("the directory is ours to make");
    declaring(&home, background);
    home
}

/// The config rewritten to declare `background`, as a reader editing the file
/// leaves it.
fn declaring(home: &Path, background: &str) {
    std::fs::write(
        home.join(".config/beady-eye/config.toml"),
        format!(
            "[[projects]]\nname = \"atlas\"\npath = \"{}\"\n\n[theme]\nbackground = \"{background}\"\n",
            home.display()
        ),
    )
    .expect("the config is ours to write");
}

/// One frame with every cell of the screen in it.
///
/// A resize is what asks for one, so the height alternates: a resize to the
/// height the terminal already is is no resize at all, and what comes back
/// for it is whatever the run happened to redraw meanwhile — which is a frame
/// holding neither the band nor its treatment, and passes an absence
/// assertion by having nothing on it.
fn a_whole_frame(bdi: &mut Driven, rows: &mut u16) -> Vec<u8> {
    *rows = if *rows == ROWS { ROWS - 1 } else { ROWS };
    let whole = bdi.resize(*rows, COLS);
    bdi.answer_to(whole, GIVING_UP)
}

/// Whole frames until `gone` is off one, handed back either way — a frame
/// that still holds it is what the assertion needs to fail against.
fn frames_until_gone(bdi: &mut Driven, rows: &mut u16, gone: &[u8], patience: Duration) -> Vec<u8> {
    let giving_up = Instant::now() + patience;
    loop {
        let frame = a_whole_frame(bdi, rows);
        if !contains(&frame, gone) || Instant::now() >= giving_up {
            return frame;
        }
    }
}

#[test]
fn a_reader_who_declares_a_light_background_gets_it_without_a_restart() {
    let home = a_home_on("theme-reloads", "dark");
    let tracker = ShimmedTracker::beside(&home);
    tracker.holds(THE_DESCRIBED_SUBTREE);
    let mut environment = tracker.environment();
    environment.push(a_socket_of_its_own(&home));

    let mut bdi = Driven::bdi(ROWS, COLS, home.clone(), &environment);
    bdi.read_until(b"herdr session", GIVING_UP);

    let mut rows = ROWS;
    let dark = a_whole_frame(&mut bdi, &mut rows);
    assert!(
        row_of(&dark, NO_PANE_TO_READ).is_some() && contains(&dark, DIM),
        "the run opened on the dark background it was configured with\n{}",
        bdi.timeline()
    );

    declaring(&home, "light");
    let light = frames_until_gone(&mut bdi, &mut rows, DIM, A_RELOAD);

    assert!(
        row_of(&light, NO_PANE_TO_READ).is_some(),
        "the band is still drawn, and still says what it said\n{}",
        bdi.timeline()
    );
    assert!(
        !contains(&light, DIM),
        "and it is drawn on the light palette the reader has just declared, \
         with no restart\n{}",
        bdi.timeline()
    );
}
