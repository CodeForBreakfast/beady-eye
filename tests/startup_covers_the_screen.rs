//! What `bdi` leaves of the screen it was handed.
//!
//! `bdi` takes the whole terminal, so every cell of it is `bdi`'s to account
//! for. A cell the first frame neither writes nor erases keeps whatever was
//! there, and what was there is a shell's scrollback, or a killed `bdi`'s last
//! frame, or anything else the terminal was showing.
//!
//! Only the count is asserted, never what is in a cell. The layout is still
//! moving, and a test that pinned the frame's contents would go red on every
//! change to it while saying nothing about this property.
//!
//! One run on a fresh pty is the whole test, because covering the screen makes
//! what preceded `bdi` irrelevant by construction — there is nothing left of it
//! to show through. Staging a killed predecessor as well would measure the
//! terminal's answer to a repeated `\e[?1049h`, which is the emulator's choice
//! and not `bdi`'s.

mod terminal;

use std::time::Duration;

use terminal::a_home_naming_one_project;
use terminal::driver::{Driven, GIVING_UP};
use terminal::ENTER_ALTERNATE_SCREEN;

const ROWS: u16 = 40;
const COLS: u16 = 120;
const CELLS: usize = ROWS as usize * COLS as usize;

/// The screen is done when the terminal has been silent this long.
///
/// Waiting for the alternate screen only says the frame has started, so the
/// test then reads until the terminal falls silent. `bdi` writes one frame in
/// one burst, and it writes several bursts here: it opens the screen before
/// any tracker has answered, so it goes on drawing for as long as it is
/// collecting — a mark turning beside each project every eighty
/// milliseconds, and each project redrawn as its rows land. So a gap this
/// long is not one frame ending but the collecting being over, which is the
/// first moment `bdi` has nothing left to say.
///
/// That is later than this test needs and no worse for it: reading too much
/// is harmless, since a later frame accounts for the same cells the first
/// already did, and it is reading too *little* that would count cells no
/// frame had reached.
const A_FRAME_IS_OVER_AFTER: Duration = Duration::from_millis(600);

#[test]
fn the_first_frame_accounts_for_every_cell_of_the_terminal() {
    let mut bdi = Driven::bdi(ROWS, COLS, a_home_naming_one_project("covers"), &[]);
    bdi.read_until(ENTER_ALTERNATE_SCREEN, GIVING_UP);
    bdi.settle(A_FRAME_IS_OVER_AFTER, GIVING_UP);

    let accounted = accounted_for(&bdi.everything());

    assert_eq!(
        accounted,
        CELLS,
        "bdi's first frame left {} of {CELLS} cells as it found them, \
         so whatever the terminal was showing there is still on screen",
        CELLS - accounted
    );
}

/// How many cells of the screen this run of bytes accounts for — erases, or
/// writes over.
fn accounted_for(said: &[u8]) -> usize {
    let mut screen = Screen {
        accounted: vec![false; CELLS],
        row: 0,
        col: 0,
    };
    screen.reading(said);
    screen.accounted.iter().filter(|cell| **cell).count()
}

/// A screen that remembers only whether each cell was accounted for. It knows
/// where the cursor is because that is what decides which cell a byte lands
/// on, and nothing more: this asks how much of the screen was covered, not
/// what it was covered with.
struct Screen {
    accounted: Vec<bool>,
    row: u16,
    col: u16,
}

impl Screen {
    fn reading(&mut self, said: &[u8]) {
        let mut at = 0;
        while at < said.len() {
            at += match said[at] {
                0x1b => self.escape(&said[at..]),
                b'\r' => {
                    self.col = 0;
                    1
                }
                b'\n' => {
                    self.down();
                    1
                }
                byte if byte >= 0x20 => self.printed(&said[at..]),
                _ => 1,
            };
        }
    }

    /// How many bytes this escape sequence spends, having applied whatever it
    /// asks for that moves the cursor or erases.
    fn escape(&mut self, said: &[u8]) -> usize {
        match said.get(1) {
            Some(b'[') => self.control_sequence(said),
            // An operating system command, or one of the strings terminated
            // the same way. `bdi` sends none, but a terminal library is free
            // to, and swallowing one whole is cheaper than being surprised.
            Some(b']' | b'P' | b'X' | b'^' | b'_') => a_string_length(said),
            Some(_) => 2,
            None => 1,
        }
    }

    fn control_sequence(&mut self, said: &[u8]) -> usize {
        let Some(end) = said
            .iter()
            .position(|byte| (0x40..=0x7e).contains(byte) && *byte != b'[')
        else {
            return said.len();
        };
        let body = &said[2..end];
        // A private sequence — `\e[?1049h` and the mouse modes — asks the
        // terminal for a mode, and none of those moves the cursor or erases.
        let private = body.first().is_some_and(|byte| !byte.is_ascii_digit());
        if !private {
            let numbers = numbers_in(body);
            match said[end] {
                b'H' | b'f' => self.moved_to(&numbers),
                b'J' => self.erased(numbers.first().copied().unwrap_or(0)),
                _ => {}
            }
        }
        end + 1
    }

    fn moved_to(&mut self, numbers: &[u16]) {
        // The wire counts rows and columns from one.
        self.row = numbers.first().copied().unwrap_or(1).saturating_sub(1);
        self.col = numbers.get(1).copied().unwrap_or(1).saturating_sub(1);
    }

    fn erased(&mut self, how: u16) {
        let here = self.here();
        let erased = match how {
            0 => here..CELLS,
            1 => 0..here + 1,
            // 2 is the screen; 3 takes the scrollback with it.
            2 | 3 => 0..CELLS,
            _ => return,
        };
        for cell in &mut self.accounted[erased] {
            *cell = true;
        }
    }

    /// One character lands on one cell. `bdi` draws box-drawing characters
    /// and bead titles, all of them one column wide.
    fn printed(&mut self, said: &[u8]) -> usize {
        let here = self.here();
        if let Some(cell) = self.accounted.get_mut(here) {
            *cell = true;
        }
        if self.col + 1 >= COLS {
            self.col = 0;
            self.down();
        } else {
            self.col += 1;
        }
        a_character_length(said)
    }

    fn down(&mut self) {
        self.row = (self.row + 1).min(ROWS - 1);
    }

    fn here(&self) -> usize {
        (usize::from(self.row) * usize::from(COLS) + usize::from(self.col)).min(CELLS - 1)
    }
}

fn numbers_in(body: &[u8]) -> Vec<u16> {
    std::str::from_utf8(body)
        .unwrap_or_default()
        .split(';')
        .map(|number| number.parse().unwrap_or(0))
        .collect()
}

/// How many bytes this UTF-8 character spends.
fn a_character_length(said: &[u8]) -> usize {
    match said[0] {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        _ => 4,
    }
    .min(said.len())
}

/// How many bytes a string-terminated sequence spends, counting its
/// terminator: a bell, or an escape and a backslash.
fn a_string_length(said: &[u8]) -> usize {
    let mut end = 2;
    while end < said.len() {
        if said[end] == 0x07 {
            return end + 1;
        }
        if said[end] == 0x1b && said.get(end + 1) == Some(&b'\\') {
            return end + 2;
        }
        end += 1;
    }
    said.len()
}
