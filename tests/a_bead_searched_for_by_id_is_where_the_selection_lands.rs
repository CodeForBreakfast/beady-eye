//! `/`, an id, and the selection is on that bead.
//!
//! Nothing reaches the prompt through the binary otherwise. `Screen::typing`
//! is a delegation a run never makes under the unit tests, and the loop's
//! whole reason for holding a third `Showing` is that a keystroke means
//! something else while the prompt is up — so `typing` could return a
//! constant, or the loop could hand every key to the mapping instead, and the
//! suite would stay green while `/` did nothing a reader could see.
//!
//! Where the selection landed is read off a bead window's title rather than
//! off the forest. The window is drawn from the *selection*, so its title
//! names the row the search left it on and nothing else can stand in for
//! that: a row's own text is on the screen whether it is selected or not.

mod terminal;

use std::time::Duration;

use terminal::driver::{Driven, GIVING_UP};
use terminal::{contains, over_the_described_subtree, window_over};

const ROWS: u16 = 40;

/// Wider than the eighty columns most of these tests use. This machine has no
/// herdr, so its foot already carries the notice saying which agents are
/// alive is unknown — and the foot gives up what it said back to a keystroke
/// whole rather than cut it, before it gives up a notice. At a hundred and
/// twenty columns the notice, the answer and the keys do not all fit, and the
/// answer is the one that goes.
const COLS: u16 = 160;

/// A gap this long between bytes means the frame is drawn.
const A_SILENCE: Duration = Duration::from_millis(300);

/// `c`, which shuts the selected node and everything under it. The walk
/// leaves the selection on the tree's header, so this puts every bead in the
/// tree behind a fold and leaves the search something to open.
const SHUT_THE_TREE: &[u8] = b"c";

/// `/` and the id of a bead in that tree, without the Enter that closes the
/// prompt.
const TYPE_AN_ID: &[u8] = b"/dun-0tp.7";

/// `/` and the end of that id, which the start is typed in front of.
const TYPE_THE_END_OF_THE_ID: &[u8] = b"/0tp.7";

/// `/` alone, which opens the prompt for the paste below.
const OPEN_THE_PROMPT: &[u8] = b"/";

/// The id of a bead in that tree pasted, as a terminal sends a paste once
/// the program has asked for it bracketed.
const PASTE_AN_ID: &[u8] = b"\x1b[200~dun-0tp.7\x1b[201~";

/// `Home`, as a terminal sends it.
const HOME: &[u8] = b"\x1b[H";

/// The start of the id, which with the end makes `dun-0tp.7`.
const TYPE_THE_START_OF_THE_ID: &[u8] = b"dun-";

/// `Enter`, which closes the prompt — and, on the forest, shows the selected
/// bead.
const ENTER: &[u8] = b"\r";

/// `Esc`, which leaves the prompt with the forest put back as it stood at `/`.
const ESC: &[u8] = b"\x1b";

/// The tree's header, where the walk leaves the selection.
const THE_TREES_HEADER: &str = "dun-0tp";

/// `/`, an id no tracker read holds, and Enter.
const SEARCH_FOR_A_BEAD_NOBODY_HAS: &[u8] = b"/bdi-404\r";

/// The prompt with that id typed into it, as `view::phrase` draws it.
const THE_PROMPT: &[u8] = "/dun-0tp.7".as_bytes();

/// The bead the search lands on, which is the bead a window opened there is
/// over.
const THE_SEARCHED_BEAD: &str = "dun-0tp.7";

/// That bead's own title, which is what says its row is drawn at all. Read
/// rather than its id, because every bead in this tree carries the header's
/// id as a prefix and the ids on screen are abbreviated to what is left.
const THE_SEARCHED_BEADS_TITLE: &[u8] = "phrase.rs is one file".as_bytes();

/// The one word of what the foot says about an id no tracker read holds that
/// nothing else on this screen says. One word rather than the sentence,
/// because the foot is drawn in the terminal's own colour: its spaces are
/// cells nothing has to write, so the line reaches the wire a word at a time
/// and no run of it carries a space.
///
/// The id, and not one of `bdi`'s own words, because every other word of the
/// phrase is in some bead's title in this fixture — `tracker` in `.6`'s and
/// `read` inside the scope line's *reading*. This id is in no tree.
const THE_ID_THAT_REACHED_NOTHING: &[u8] = "bdi-404".as_bytes();

/// The same id with the key that opened the prompt in front of it, which is
/// how it reaches the screen while the reader is still typing. Enter takes
/// the prompt down, so this is what tells the answer from the question — and
/// the two are the same letters otherwise.
const THE_PROMPT_STILL_UP: &[u8] = "/bdi-404".as_bytes();

/// The whole key, end to end: the prompt takes the characters and takes the
/// reader to the bead, Enter leaves them there, and the fold that was over it
/// is open.
#[test]
fn an_id_typed_into_the_prompt_takes_the_reader_to_that_bead() {
    let (mut bdi, _tracker) = over_the_described_subtree("searched", ROWS, COLS, A_SILENCE);

    bdi.send(SHUT_THE_TREE);
    bdi.settle(A_SILENCE, GIVING_UP);
    let shut = repaint(&mut bdi, ROWS + 1);
    assert!(
        !contains(&shut, THE_SEARCHED_BEADS_TITLE),
        "the bead is drawn with its tree shut, so the search below has no \
         fold to open and nothing to prove. The screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&shut),
        bdi.timeline()
    );

    bdi.send(TYPE_AN_ID);
    bdi.settle(A_SILENCE, GIVING_UP);
    let typing = repaint(&mut bdi, ROWS);
    assert!(
        contains(&typing, THE_PROMPT),
        "the characters typed after `/` are not on the foot, so either the \
         key never opened a prompt or the prompt did not take them. The \
         screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&typing),
        bdi.timeline()
    );

    bdi.send(ENTER);
    bdi.settle(A_SILENCE, GIVING_UP);
    bdi.send(ENTER);
    bdi.settle(A_SILENCE, GIVING_UP);
    let landed = repaint(&mut bdi, ROWS + 1);
    assert_eq!(
        window_over(&landed).as_deref(),
        Some(THE_SEARCHED_BEAD),
        "the search did not leave the selection on the bead it named. The \
         screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&landed),
        bdi.timeline()
    );
}

/// A paste reaches the prompt as the id it carries, and takes the reader to
/// that bead as typing it would.
#[test]
fn an_id_pasted_into_the_prompt_takes_the_reader_to_that_bead() {
    let (mut bdi, _tracker) = over_the_described_subtree("pasted", ROWS, COLS, A_SILENCE);
    bdi.send(SHUT_THE_TREE);
    bdi.settle(A_SILENCE, GIVING_UP);

    bdi.send(OPEN_THE_PROMPT);
    bdi.send(PASTE_AN_ID);
    bdi.settle(A_SILENCE, GIVING_UP);
    bdi.send(ENTER);
    bdi.settle(A_SILENCE, GIVING_UP);
    bdi.send(ENTER);
    bdi.settle(A_SILENCE, GIVING_UP);

    let landed = repaint(&mut bdi, ROWS + 1);
    assert_eq!(
        window_over(&landed).as_deref(),
        Some(THE_SEARCHED_BEAD),
        "the pasted id did not reach the prompt. The screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&landed),
        bdi.timeline()
    );
}

/// The search moves while the prompt is still up, opening the fold over the
/// bead the id names, and Esc shuts it again with the selection back on the
/// header it left.
#[test]
fn the_search_moves_as_the_id_is_typed_and_esc_puts_the_tree_back() {
    let (mut bdi, _tracker) = over_the_described_subtree("incsearch", ROWS, COLS, A_SILENCE);
    bdi.send(SHUT_THE_TREE);
    bdi.settle(A_SILENCE, GIVING_UP);

    bdi.send(TYPE_AN_ID);
    bdi.settle(A_SILENCE, GIVING_UP);
    let typing = repaint(&mut bdi, ROWS + 1);
    assert!(
        contains(&typing, THE_PROMPT) && contains(&typing, THE_SEARCHED_BEADS_TITLE),
        "with the prompt still up, the search has not opened the fold over \
         the bead it names. The screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&typing),
        bdi.timeline()
    );

    bdi.send(ESC);
    bdi.settle(A_SILENCE, GIVING_UP);
    let abandoned = repaint(&mut bdi, ROWS);
    assert!(
        !contains(&abandoned, THE_SEARCHED_BEADS_TITLE),
        "Esc left open the fold the search opened. The screen it drew: \
         {:?}\n{}",
        String::from_utf8_lossy(&abandoned),
        bdi.timeline()
    );

    bdi.send(ENTER);
    bdi.settle(A_SILENCE, GIVING_UP);
    let window = repaint(&mut bdi, ROWS + 1);
    assert_eq!(
        window_over(&window).as_deref(),
        Some(THE_TREES_HEADER),
        "Esc did not put the selection back where `/` found it. The screen \
         it drew: {:?}\n{}",
        String::from_utf8_lossy(&window),
        bdi.timeline()
    );
}

/// The prompt edits at a position, as a shell's does. The end of the id is
/// typed first, then Home, then its start: a prompt that took Home as nothing
/// searches for `0tp.7dun-`, which no bead holds, and leaves the selection on
/// the header.
#[test]
fn home_puts_what_is_typed_next_at_the_start_of_the_id() {
    let (mut bdi, _tracker) = over_the_described_subtree("home", ROWS, COLS, A_SILENCE);
    bdi.send(SHUT_THE_TREE);
    bdi.settle(A_SILENCE, GIVING_UP);

    bdi.send(TYPE_THE_END_OF_THE_ID);
    bdi.send(HOME);
    bdi.send(TYPE_THE_START_OF_THE_ID);
    bdi.settle(A_SILENCE, GIVING_UP);
    bdi.send(ENTER);
    bdi.settle(A_SILENCE, GIVING_UP);
    bdi.send(ENTER);
    bdi.settle(A_SILENCE, GIVING_UP);

    let landed = repaint(&mut bdi, ROWS + 1);
    assert_eq!(
        window_over(&landed).as_deref(),
        Some(THE_SEARCHED_BEAD),
        "what was typed after Home did not go in at the start of the id. The \
         screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&landed),
        bdi.timeline()
    );
}

/// The other answer. An id no read holds is not silence: the foot says so,
/// and says it as what `bdi` has read rather than as what exists.
#[test]
fn an_id_no_tracker_read_holds_is_said_at_the_foot() {
    let (mut bdi, _tracker) = over_the_described_subtree("unfound", ROWS, COLS, A_SILENCE);

    bdi.send(SEARCH_FOR_A_BEAD_NOBODY_HAS);
    bdi.settle(A_SILENCE, GIVING_UP);

    let said = repaint(&mut bdi, ROWS + 1);
    assert!(
        contains(&said, THE_ID_THAT_REACHED_NOTHING),
        "a search that reached nothing said nothing. The screen it drew: \
         {:?}\n{}",
        String::from_utf8_lossy(&said),
        bdi.timeline()
    );
    assert!(
        !contains(&said, THE_PROMPT_STILL_UP),
        "the prompt is still up, so the id above is the reader's question \
         rather than the foot's answer to it. The screen it drew: {:?}\n{}",
        String::from_utf8_lossy(&said),
        bdi.timeline()
    );
}

/// The screen as it stands, rather than as it differs from the frame before:
/// a resize is answered by drawing every cell again.
#[track_caller]
fn repaint(bdi: &mut Driven, rows: u16) -> Vec<u8> {
    let repainted = bdi.resize(rows, COLS);
    bdi.answer_to(repainted, GIVING_UP)
}
