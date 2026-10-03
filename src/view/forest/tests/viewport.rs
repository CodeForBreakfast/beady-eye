//! The part of the forest the band shows, and what moves it.

use super::*;
use pretty_assertions::assert_eq;

/// How far a notch is told to go here. The forest is told rather than
/// knowing, so what three means is settled in the config and its tests.
const A_NOTCH: usize = 3;

/// A band with room for every line has nowhere to scroll to, whichever
/// way the wheel turns.
#[test]
fn a_forest_the_band_has_room_for_never_scrolls() {
    let mut forest = flatten(snapshot());
    forest.fit(forest.lines().len());

    assert!(!forest.scrolled(Notch::Down, A_NOTCH));
    assert!(!forest.scrolled(Notch::Up, A_NOTCH));
    assert_eq!(forest.from(), 0);
}

/// A notch travels the distance it is handed, and no other.
#[test]
fn a_notch_moves_the_view_as_far_as_it_is_told() {
    let mut forest = flatten(snapshot());
    forest.fit(2);

    assert!(forest.scrolled(Notch::Down, A_NOTCH));
    assert_eq!(forest.from(), A_NOTCH);

    assert!(forest.scrolled(Notch::Down, A_NOTCH));
    assert_eq!(forest.from(), A_NOTCH * 2);

    assert!(forest.scrolled(Notch::Up, A_NOTCH));
    assert_eq!(forest.from(), A_NOTCH);
}

/// Scrolling past the end would draw blank rows under the last line,
/// which reads as a forest that has run out rather than one that has
/// ended. The top is the same promise the other way up.
#[test]
fn the_wheel_stops_at_either_end_of_the_forest() {
    let mut forest = flatten(snapshot());
    forest.fit(4);
    let furthest = forest.lines().len() - 4;

    for _ in 0..40 {
        forest.scrolled(Notch::Down, A_NOTCH);
    }
    assert_eq!(forest.from(), furthest);
    assert!(!forest.scrolled(Notch::Down, A_NOTCH));

    for _ in 0..40 {
        forest.scrolled(Notch::Up, A_NOTCH);
    }
    assert_eq!(forest.from(), 0);
    assert!(!forest.scrolled(Notch::Up, A_NOTCH));
}

/// The wheel leaves the selection where the reader put it, including
/// where that takes it off the screen — which is the whole difference
/// from the motion the same direction names.
#[test]
fn the_wheel_leaves_the_selection_alone() {
    let mut forest = flatten(snapshot());
    forest.fit(2);
    let was = forest.selected_line();

    forest.scrolled(Notch::Down, A_NOTCH);

    assert_eq!(forest.selected_line(), was);
    assert!(
        was < forest.from(),
        "the selection is still inside the band, so this says nothing \
         about one the notch took off it"
    );
}

/// A motion brings the selection back into the band by the least it can,
/// rather than by putting it back in the middle: a reader who wheeled to
/// somewhere and then stepped one row keeps what they were looking at.
///
/// Walked the whole way down rather than asserted at one place. Every
/// step that pushes the selection past the last row of the band leaves
/// the band starting exactly where that step asked and no further, and a
/// band that recentred instead would be wrong at the first of them.
#[test]
fn a_motion_reveals_the_selection_by_the_least_it_can() {
    let room = 4;
    let last = an_end(Motion::LastRow);
    let mut forest = flatten(snapshot());
    forest.fit(room);
    forest.apply(Action::Move(Motion::FirstRow));
    let mut scrolled = false;

    walk::until(
        &mut forest,
        |forest| forest.selected_line() == last,
        |forest| {
            forest.apply(Action::Move(Motion::NextRow));
            let (from, at) = (forest.from(), forest.selected_line());
            if from > 0 {
                assert_eq!(
                    from,
                    at + 1 - room,
                    "the band travelled further down than the row leaving it asked for"
                );
                scrolled = true;
            }
        },
        |forest| format!("a walk down stopped at row {}", forest.selected_line()),
    );

    assert!(scrolled, "the fixture never outgrew a band of {room}");
}

/// And the same going up, which the arithmetic that reveals downwards
/// cannot answer for: there the band starts on the selection itself.
#[test]
fn a_motion_upwards_reveals_the_selection_by_the_least_it_can() {
    let first = an_end(Motion::FirstRow);
    let mut forest = flatten(snapshot());
    forest.fit(4);
    forest.apply(Action::Move(Motion::LastRow));
    let mut was = forest.from();
    let mut scrolled = false;

    walk::until(
        &mut forest,
        |forest| forest.selected_line() == first,
        |forest| {
            forest.apply(Action::Move(Motion::PreviousRow));
            let (from, at) = (forest.from(), forest.selected_line());
            if from != was {
                assert_eq!(
                    from, at,
                    "the band travelled further up than the row leaving it asked for"
                );
                scrolled = true;
            }
            was = from;
        },
        |forest| format!("a walk up stopped at row {}", forest.selected_line()),
    );

    assert!(scrolled, "the fixture never outgrew a band of four");
}

/// Which line one end of the forest is, so a walk towards it knows what
/// it is walking to. Asked of a forest of its own, because the answer is
/// where the walk finishes rather than anywhere it passes through.
fn an_end(motion: Motion) -> usize {
    let mut forest = flatten(snapshot());
    forest.apply(Action::Move(motion));
    forest.selected_line()
}

/// Every keyboard motion leaves the selection somewhere the band is
/// showing, however far the wheel had taken the view from it.
#[test]
fn every_motion_leaves_the_selection_inside_the_band() {
    let room = 4;
    for motion in [
        Motion::PreviousRow,
        Motion::NextRow,
        Motion::HalfScreenUp,
        Motion::HalfScreenDown,
        Motion::FirstRow,
        Motion::LastRow,
    ] {
        let mut forest = flatten(snapshot());
        forest.fit(room);
        forest.scrolled(Notch::Down, A_NOTCH);
        forest.scrolled(Notch::Down, A_NOTCH);

        forest.apply(Action::Move(motion));

        let (from, at) = (forest.from(), forest.selected_line());
        assert!(
            (from..from + room).contains(&at),
            "{motion:?} left the selection on line {at}, outside {from}..{}",
            from + room
        );
    }
}

/// A motion that moves the band without moving the selection is still a
/// change. The wheel put the band somewhere the selection is not, so `g`
/// on a selection already on the first row brings the band back and
/// nothing else — and a screen not redrawn for that goes on showing the
/// rows the wheel left it on, under an offset the next click reads
/// against.
#[test]
fn a_motion_that_only_brings_the_band_back_is_a_change() {
    let mut forest = flatten(snapshot());
    forest.fit(4);
    forest.apply(Action::Move(Motion::FirstRow));
    let first = forest.selected_line();
    forest.scrolled(Notch::Down, A_NOTCH);
    assert!(forest.from() > first, "the wheel has to take the band away");

    assert!(forest.apply(Action::Move(Motion::FirstRow)));

    assert_eq!(
        forest.selected_line(),
        first,
        "the selection was already on the first row"
    );
    assert_eq!(forest.from(), first);
}

/// A click selects a row the band is showing, so it never moves the view
/// — including when the wheel has taken the band away from the top.
#[test]
fn a_click_on_a_row_the_band_is_showing_leaves_the_view_where_it_is() {
    let mut forest = flatten(snapshot());
    forest.fit(4);
    forest.scrolled(Notch::Down, A_NOTCH);
    let scrolled = forest.from();

    assert!(forest.select_line(scrolled + 1));
    assert_eq!(forest.from(), scrolled);
}

/// A collection keeps the reader's scroll rather than throwing it away:
/// what a shorter forest costs it is the distance past its end and no
/// more.
#[test]
fn a_collection_keeps_the_scroll_the_reader_set() {
    let mut forest = flatten(snapshot());
    forest.fit(4);
    forest.scrolled(Notch::Down, A_NOTCH);
    let scrolled = forest.from();

    forest.refresh(snapshot());
    forest.fit(4);

    assert_eq!(forest.from(), scrolled);
}

/// And a band that has shrunk under it brings the view back inside the
/// forest, rather than leaving it drawing blank rows past the last line.
#[test]
fn a_shorter_band_brings_the_view_back_inside_the_forest() {
    let mut forest = flatten(snapshot());
    let lines = forest.lines().len();
    forest.fit(2);
    for _ in 0..40 {
        forest.scrolled(Notch::Down, A_NOTCH);
    }
    assert_eq!(forest.from(), lines - 2);

    forest.fit(lines);

    assert_eq!(forest.from(), 0);
}

/// A keystroke asks whether the screen moved by comparing the lines, so a
/// line that carries anything the screen does not show makes that question
/// answerable by data no reader can see. Retitling a node no line draws
/// must leave the lines identical.
#[test]
fn a_line_carries_nothing_the_screen_does_not_show() {
    let drawn: BTreeSet<BeadKey> = flatten(snapshot())
        .lines()
        .iter()
        .filter_map(|line| line.bead().cloned())
        .collect();

    let mut altered = snapshot();
    let mut retitled = 0;
    for tree in &mut altered.trees {
        let tree = Arc::make_mut(tree);
        for node in &mut tree.beads {
            let key = BeadKey {
                project: tree.project.clone(),
                id: node.id.clone(),
            };
            if !drawn.contains(&key) {
                node.title = format!("{} (retitled)", node.title);
                retitled += 1;
            }
        }
    }
    assert!(
        retitled > 0,
        "every node in the fixture is drawn, so nothing here is undrawn to hide"
    );

    assert_eq!(flatten(snapshot()).lines(), flatten(altered).lines());
}

/// The forest cannot focus a pane, re-collect or quit; the loop does all
/// three, and none of them changes what is on screen.
#[test]
fn focus_refresh_and_quit_change_nothing_in_the_forest() {
    let mut forest = flatten(snapshot());
    let before = sketch(&forest);

    for action in [Action::Focus, Action::Refresh, Action::Quit] {
        assert!(!forest.apply(action), "{action:?}");
    }

    assert_eq!(sketch(&forest), before);
}
