use super::*;
use pretty_assertions::assert_eq;

/// The reader types what they have, and what they have is a fragment:
/// part of an id, a word out of a title, or a whole id copied with `y`.
/// A bead holding it in either field is a match.
#[test]
fn a_search_matches_part_of_an_id() {
    let mut forest = flatten(snapshot());

    assert_eq!(
        forest.seek_here("7.1.1"),
        went_to("dunwich", "dun-7.1.1", 1, 1)
    );

    assert_eq!(cursor(&forest), Some(&key("dunwich", "dun-7.1.1")));
    assert!(
        drawn_here(&forest, "true the mount"),
        "{:#?}",
        sketch(&forest)
    );
}

/// The forest row is the one place `bdi` ever prints a shortened id —
/// `row::abbreviate` has no other caller — so on a long screen the short
/// id is the only spelling the reader has been shown, and what they type
/// has to reach the bead they read it off. The query is taken from the row
/// rather than written out here, because a search matches on the whole id
/// and a query spelled by hand would pass on that alone.
///
/// A substring is what reaches it: the drawn form is a prefix of nothing.
#[test]
fn a_search_matches_the_shortened_id_the_row_draws() {
    let mut forest = flatten(snapshot());
    let drawn = row_of(&forest, "dun-7.1").id.clone();

    assert_eq!(
        forest.seek_here(&drawn),
        went_to("dunwich", "dun-7.1", 1, 4)
    );
}

#[test]
fn a_search_matches_part_of_a_title() {
    let mut forest = flatten(snapshot());

    assert_eq!(
        forest.seek_here("mount"),
        went_to("dunwich", "dun-7.1.1", 1, 1)
    );
}

/// A title is prose and the reader is retyping a word they read off a
/// row, so the capitals the row happened to carry are not theirs to
/// reproduce.
#[test]
fn a_search_ignores_letter_case() {
    let mut forest = flatten(snapshot());

    assert_eq!(
        forest.seek_here("MoUnT"),
        went_to("dunwich", "dun-7.1.1", 1, 1)
    );
}

/// Matches are numbered in the order the forest draws them, which is the
/// order a reader scrolling would have met them — `place_of` names the
/// same doctrine for a single jump. So the ordinal is a fact about the
/// forest, and a reader who doubts it can count it off the screen.
#[test]
fn matches_are_numbered_in_the_order_the_forest_draws_them() {
    let mut forest = flatten(snapshot());

    assert_eq!(forest.seek_here("7.1"), went_to("dunwich", "dun-7.1", 1, 3));
    assert_eq!(
        forest.next_match(true),
        Some(went_to("dunwich", "dun-7.1.1", 2, 3))
    );
    assert_eq!(
        forest.next_match(true),
        Some(went_to("dunwich", "dun-7.1.2", 3, 3))
    );
}

/// Stepping past the last comes round to the first, so a reader walking
/// the set is never stopped at an end they cannot see.
#[test]
fn stepping_past_the_last_match_comes_round_to_the_first() {
    let mut forest = flatten(snapshot());
    forest.seek_here("7.1");
    forest.next_match(true);
    forest.next_match(true);

    assert_eq!(
        forest.next_match(true),
        Some(went_to("dunwich", "dun-7.1", 1, 3))
    );
}

#[test]
fn stepping_back_walks_the_matches_the_other_way() {
    let mut forest = flatten(snapshot());
    forest.seek_here("7.1");

    assert_eq!(
        forest.next_match(false),
        Some(went_to("dunwich", "dun-7.1.2", 3, 3))
    );
}

/// A reader resting below the last bead on the screen stands past every
/// match, so stepping back reaches the last one drawn.
#[test]
fn stepping_back_from_below_the_last_bead_reaches_the_last_match() {
    let mut forest = flatten(snapshot());
    forest.seek_here("7.1");
    let last = forest.lines().len() - 1;
    step_onto(&mut forest, last);

    assert_eq!(
        forest.next_match(false),
        Some(went_to("dunwich", "dun-7.1.2", 3, 3))
    );
}

#[test]
fn stepping_on_from_below_the_last_bead_comes_round_to_the_first_match() {
    let mut forest = flatten(snapshot());
    forest.seek_here("7.1");
    let last = forest.lines().len() - 1;
    step_onto(&mut forest, last);

    assert_eq!(
        forest.next_match(true),
        Some(went_to("dunwich", "dun-7.1", 1, 3))
    );
}

/// A reader standing on a bead that does not match has not seen the next
/// match yet, so stepping on lands on it rather than the one after.
#[test]
fn stepping_on_from_a_bead_that_does_not_match_lands_on_the_next_match() {
    let mut forest = flatten(snapshot());
    forest.seek_here("7.1");
    assert!(forest.go_to(&key("dunwich", "dun-7")));

    assert_eq!(
        forest.next_match(true),
        Some(went_to("dunwich", "dun-7.1", 1, 3))
    );
}

#[test]
fn stepping_back_through_a_search_matching_nothing_goes_nowhere() {
    let mut forest = flatten(snapshot());
    forest.seek_here("dun-404");

    assert_eq!(
        forest.next_match(false),
        Some(Landed::Nowhere("dun-404".into()))
    );
}

/// Stepping asks where the selection is now rather than where the last
/// step left it, so a reader who has moved by hand between presses
/// carries on from where they are standing.
#[test]
fn stepping_carries_on_from_where_the_reader_has_moved_to() {
    let mut forest = flatten(snapshot());
    forest.seek_here("7.1");
    assert!(forest.go_to(&key("dunwich", "dun-7.1.1")));

    assert_eq!(
        forest.next_match(true),
        Some(went_to("dunwich", "dun-7.1.2", 3, 3))
    );
}

/// One way through the matches for `survey`, from `dun-7.7`, which is
/// drawn and is where the search lands.
///
/// The other two rest shut: `dun-7.2` in the run of finished branches and
/// `hbr-3.1` in the group of hidden trees. So each step opens one of them
/// — the run first going on, the group first going back.
struct SurveyWalk {
    forward: bool,
    /// The lines resting shut over the first step's match, outermost
    /// first.
    first_folds: &'static [&'static str],
    /// The first step's match.
    first: &'static str,
    /// The second step's match, and where that step lands.
    second: &'static str,
    second_landed: Landed,
}

fn both_ways_through_the_survey() -> [SurveyWalk; 2] {
    [
        SurveyWalk {
            forward: true,
            first_folds: &["… 3 more"],
            first: "survey the mast",
            second: "survey the silt",
            second_landed: went_to("harbour", "hbr-3.1", 3, 3),
        },
        SurveyWalk {
            forward: false,
            first_folds: &["[HiddenTrees harbour]", "hbr-3 dredge the channel"],
            first: "survey the silt",
            second: "survey the mast",
            second_landed: went_to("dunwich", "dun-7.2", 2, 3),
        },
    ]
}

/// What a search step opens is the step's, and the next step shuts it
/// again.
#[test]
fn a_search_step_shuts_what_the_step_before_it_opened() {
    for walk in both_ways_through_the_survey() {
        let mut forest = flatten(snapshot());
        forest.seek_here("survey");
        forest.next_match(walk.forward);
        assert!(drawn_here(&forest, walk.first), "{:#?}", sketch(&forest));

        assert_eq!(forest.next_match(walk.forward), Some(walk.second_landed));

        assert!(
            !drawn_here(&forest, walk.first),
            "forward {}: {:#?}",
            walk.forward,
            sketch(&forest)
        );
    }
}

#[test]
fn the_branches_the_last_match_needed_stay_open() {
    for walk in both_ways_through_the_survey() {
        let mut forest = flatten(snapshot());
        forest.seek_here("survey");
        forest.next_match(walk.forward);

        forest.next_match(walk.forward);

        assert!(
            drawn_here(&forest, walk.second),
            "forward {}: {:#?}",
            walk.forward,
            sketch(&forest)
        );
    }
}

/// The Enter that lands a search is a step as well, so the step after it
/// shuts what it opened.
#[test]
fn a_search_step_shuts_what_the_landing_opened() {
    for forward in [true, false] {
        let mut forest = flatten(snapshot());
        assert_eq!(
            forest.seek_here("survey the"),
            went_to("dunwich", "dun-7.2", 1, 2)
        );

        assert_eq!(
            forest.next_match(forward),
            Some(went_to("harbour", "hbr-3.1", 2, 2))
        );

        assert!(
            !drawn_here(&forest, "survey the mast"),
            "forward {forward}: {:#?}",
            sketch(&forest)
        );
    }
}

#[test]
fn a_search_shuts_what_the_step_before_it_opened() {
    let mut forest = flatten(snapshot());
    forest.seek_here("survey the mast");

    forest.seek_here("survey the silt");

    assert!(
        !drawn_here(&forest, "survey the mast"),
        "{:#?}",
        sketch(&forest)
    );
    assert!(
        drawn_here(&forest, "survey the silt"),
        "{:#?}",
        sketch(&forest)
    );
}

/// `dun-7.1.1` and `dun-7.1.2` both hang under `dun-7.1`, which rests
/// shut. The step between them shuts it and opens it again, so it stays.
#[test]
fn a_search_step_leaves_open_what_its_own_match_needs() {
    for forward in [true, false] {
        let mut forest = flatten(snapshot());
        assert!(!drawn_here(&forest, "true the mount"));
        forest.seek_here("dun-7.1.");

        assert_eq!(
            forest.next_match(forward),
            Some(went_to("dunwich", "dun-7.1.2", 2, 2))
        );

        assert!(
            drawn_here(&forest, "true the mount"),
            "forward {forward}: {:#?}",
            sketch(&forest)
        );
    }
}

/// A search matching nothing leaves the forest as it was, and that
/// includes what the step before it opened.
#[test]
fn a_search_matching_nothing_leaves_open_what_the_step_before_it_opened() {
    let mut forest = flatten(snapshot());
    forest.seek_here("survey the mast");

    assert_eq!(
        forest.seek_here("dun-404"),
        Landed::Nowhere("dun-404".into())
    );

    assert!(
        drawn_here(&forest, "survey the mast"),
        "{:#?}",
        sketch(&forest)
    );
}

/// A search lands on the first match after where it began, as `n` would
/// step to, so a match the selection already stood on is behind it.
#[test]
fn a_search_lands_on_the_first_match_after_where_it_began() {
    let mut forest = flatten(snapshot());
    assert!(forest.go_to(&key("dunwich", "dun-7.7")));
    let origin = forest.origin();

    assert_eq!(
        forest.seek("survey", &origin),
        went_to("dunwich", "dun-7.2", 2, 3)
    );
}

#[test]
fn a_search_begun_past_the_last_match_comes_round_to_the_first() {
    let mut forest = flatten(snapshot());
    assert!(forest.go_to(&key("harbour", "hbr-3.1")));
    let origin = forest.origin();

    assert_eq!(
        forest.seek("survey", &origin),
        went_to("dunwich", "dun-7.7", 1, 3)
    );
}

/// Each keystroke searches again from where the search began rather than
/// from where the last keystroke landed. Taking a character back widens
/// the search, and it lands on the first match after the beginning.
#[test]
fn each_keystroke_counts_from_where_the_search_began() {
    let mut forest = flatten(snapshot());
    assert!(forest.go_to(&key("dunwich", "dun-7.7")));
    let origin = forest.origin();
    assert_eq!(
        forest.seek("survey the s", &origin),
        went_to("harbour", "hbr-3.1", 1, 1)
    );

    assert_eq!(
        forest.seek("survey", &origin),
        went_to("dunwich", "dun-7.2", 2, 3)
    );
}

/// A keystroke is a search step, so the keystroke after it shuts what it
/// opened.
#[test]
fn the_next_keystroke_shuts_what_a_keystroke_opened() {
    let mut forest = flatten(snapshot());
    let origin = forest.origin();
    assert_eq!(
        forest.seek("survey the ", &origin),
        went_to("dunwich", "dun-7.2", 1, 2)
    );
    assert!(drawn_here(&forest, "survey the mast"));

    assert_eq!(
        forest.seek("survey the s", &origin),
        went_to("harbour", "hbr-3.1", 1, 1)
    );

    assert!(
        !drawn_here(&forest, "survey the mast"),
        "{:#?}",
        sketch(&forest)
    );
}

/// A keystroke matching nothing puts the selection and the folds back as
/// they stood when the search began, shutting what the keystroke before
/// it opened.
#[test]
fn a_keystroke_matching_nothing_goes_back_to_where_the_search_began() {
    let mut forest = flatten(snapshot());
    let origin = forest.origin();
    let (line, was) = (forest.selected_line(), sketch(&forest));
    forest.seek("survey the m", &origin);
    assert!(drawn_here(&forest, "survey the mast"));

    assert_eq!(
        forest.seek("survey the mx", &origin),
        Landed::Nowhere("survey the mx".into())
    );

    assert_eq!(sketch(&forest), was);
    assert_eq!(forest.selected_line(), line);
}

/// Going back to where a search began puts back the selection, the scroll
/// and every fold as they stood, what an earlier step opened included.
#[test]
fn going_back_to_where_a_search_began_puts_the_forest_back_as_it_stood() {
    let mut forest = flatten(snapshot());
    forest.fit(4);
    forest.seek_here("survey the mast");
    let origin = forest.origin();
    let (line, from, was) = (forest.selected_line(), forest.from(), sketch(&forest));
    forest.seek("survey the silt", &origin);
    assert_ne!(sketch(&forest), was);
    assert_ne!(forest.from(), from, "the search never scrolled the band");

    forest.restore(&origin);

    assert_eq!(sketch(&forest), was);
    assert_eq!(forest.selected_line(), line);
    assert_eq!(forest.from(), from);
}

/// A view the wheel scrolled away from the selection stays where the
/// reader left it.
#[test]
fn going_back_to_where_a_search_began_leaves_a_scrolled_view_where_it_was() {
    let mut forest = flatten(snapshot());
    forest.fit(4);
    forest.scrolled(Notch::Down, 3);
    let from = forest.from();
    assert_ne!(from, 0, "the wheel moved nothing");
    let origin = forest.origin();
    forest.seek("survey the silt", &origin);

    forest.restore(&origin);

    assert_eq!(forest.from(), from);
}

/// What an earlier step opened comes back as that step's, so the next
/// step still shuts it.
#[test]
fn going_back_to_where_a_search_began_leaves_an_earlier_steps_opens_its_own() {
    let mut forest = flatten(snapshot());
    forest.seek_here("survey the mast");
    let origin = forest.origin();
    forest.seek("survey the silt", &origin);
    forest.restore(&origin);
    assert!(drawn_here(&forest, "survey the mast"));

    forest.seek_here("survey the silt");

    assert!(
        !drawn_here(&forest, "survey the mast"),
        "{:#?}",
        sketch(&forest)
    );
}

/// `n` after going back steps through what was searched for before the
/// search that was abandoned.
#[test]
fn going_back_to_where_a_search_began_steps_through_the_search_before_it() {
    let mut forest = flatten(snapshot());
    assert_eq!(
        forest.seek_here("survey"),
        went_to("dunwich", "dun-7.7", 1, 3)
    );
    let origin = forest.origin();
    forest.seek("dun-7.1.", &origin);

    forest.restore(&origin);

    assert_eq!(
        forest.next_match(true),
        Some(went_to("dunwich", "dun-7.2", 2, 3))
    );
}

/// Moving by hand makes whatever is open the reader's, so no later step
/// shuts it. The move is a row further the way the walk goes, which
/// leaves the next step landing where it would have.
#[test]
fn moving_by_hand_keeps_open_what_a_search_step_opened() {
    for walk in both_ways_through_the_survey() {
        let mut forest = flatten(snapshot());
        forest.seek_here("survey");
        forest.next_match(walk.forward);
        let further = if walk.forward {
            Motion::NextRow
        } else {
            Motion::PreviousRow
        };
        forest.apply(Action::Move(further));

        assert_eq!(forest.next_match(walk.forward), Some(walk.second_landed));

        assert!(
            drawn_here(&forest, walk.first),
            "forward {}: {:#?}",
            walk.forward,
            sketch(&forest)
        );
    }
}

/// A click is a move by hand as much as a key is.
#[test]
fn a_click_keeps_open_what_a_search_step_opened() {
    for walk in both_ways_through_the_survey() {
        let mut forest = flatten(snapshot());
        forest.seek_here("survey");
        forest.next_match(walk.forward);
        let further = if walk.forward {
            forest.selected_line() + 1
        } else {
            forest.selected_line() - 1
        };
        assert!(forest.select_line(further));

        assert_eq!(forest.next_match(walk.forward), Some(walk.second_landed));

        assert!(
            drawn_here(&forest, walk.first),
            "forward {}: {:#?}",
            walk.forward,
            sketch(&forest)
        );
    }
}

/// Following a reference goes to a bead, and so does coming back from
/// one. Either is the reader's act, even onto the bead they are on.
#[test]
fn going_to_a_bead_keeps_open_what_a_search_step_opened() {
    for walk in both_ways_through_the_survey() {
        let mut forest = flatten(snapshot());
        forest.seek_here("survey");
        forest.next_match(walk.forward);
        let on = forest.place().cloned().expect("the step landed on a bead");
        assert!(forest.go_to_place(&on));

        assert_eq!(forest.next_match(walk.forward), Some(walk.second_landed));

        assert!(
            drawn_here(&forest, walk.first),
            "forward {}: {:#?}",
            walk.forward,
            sketch(&forest)
        );
    }
}

/// A fold the reader opened is theirs. A step landing under it opened
/// nothing there, so the next step has nothing there to shut.
#[test]
fn a_fold_the_reader_opened_survives_the_search_steps_past_it() {
    for walk in both_ways_through_the_survey() {
        let mut forest = flatten(snapshot());
        for fold in walk.first_folds {
            let at = sketch(&forest)
                .iter()
                .position(|row| row.contains(fold))
                .unwrap_or_else(|| panic!("no {fold}: {:#?}", sketch(&forest)));
            step_onto(&mut forest, at);
            forest.apply(Action::ToggleFold);
        }
        assert!(drawn_here(&forest, walk.first), "{:#?}", sketch(&forest));
        forest.apply(Action::Move(Motion::FirstRow));
        forest.seek_here("survey");
        forest.next_match(walk.forward);

        assert_eq!(forest.next_match(walk.forward), Some(walk.second_landed));

        assert!(
            drawn_here(&forest, walk.first),
            "forward {}: {:#?}",
            walk.forward,
            sketch(&forest)
        );
    }
}

/// A whole id lands on its own bead however many rows above it match. An
/// id is the one thing the reader can have meant exactly, and that is
/// what `bdi-2bb.37` shipped — a substring match that let a row merely
/// titled after a bead shadow the bead would take it back.
///
/// The numbering is untouched by it: `dun-6.1` is drawn first and holds
/// the id in its title, so the bead landed on is the second of two. The
/// landing is the only thing the whole id decides.
#[test]
fn a_whole_id_lands_on_its_own_bead_however_many_rows_above_it_match() {
    let mut forest = flatten(alone("dunwich", NAMED_IN_A_TITLE, &[]));

    assert_eq!(
        forest.seek_here("dun-6.2"),
        went_to("dunwich", "dun-6.2", 2, 2)
    );

    assert_eq!(cursor(&forest), Some(&key("dunwich", "dun-6.2")));
}

#[test]
fn a_whole_id_lands_on_its_own_bead_whatever_its_letter_case() {
    let mut forest = flatten(alone("dunwich", NAMED_IN_A_TITLE, &[]));

    assert_eq!(
        forest.seek_here("DUN-6.2"),
        went_to("dunwich", "dun-6.2", 2, 2)
    );
}

/// `dun-6.2.1` holds `dun-6.2` in its id and is drawn first, under
/// `dun-6.1`, but it is not the bead the whole id names.
#[test]
fn a_whole_id_lands_past_a_longer_id_drawn_above_it() {
    let mut forest = flatten(alone("dunwich", NAMED_INSIDE_A_LONGER_ID, &[]));

    assert_eq!(
        forest.seek_here("dun-6.2"),
        went_to("dunwich", "dun-6.2", 2, 3)
    );
}

/// Search counts the way vim does, since `bdi-7ao.136`: every drawn copy
/// is a match of its own. `dun-9` is drawn under `dun-8.1` and again
/// under `dun-8.2`, and `dun-9.1` is drawn once under each copy of its
/// parent, so the text is on four lines and there are four matches.
#[test]
fn every_drawn_copy_of_a_bead_is_its_own_match() {
    let mut forest = flatten(drawn_twice_in_one_tree());

    assert_eq!(forest.seek_here("dun-9"), went_to("dunwich", "dun-9", 1, 4));
    assert_eq!(
        forest.next_match(true),
        Some(went_to("dunwich", "dun-9.1", 2, 4))
    );
    assert_eq!(
        forest.next_match(true),
        Some(went_to("dunwich", "dun-9", 3, 4))
    );
    assert_eq!(
        forest.next_match(true),
        Some(went_to("dunwich", "dun-9.1", 4, 4))
    );
}

/// Counting every copy costs no more than counting every bead: forty
/// nested diamonds draw the keystone on two to the fortieth rows, and a
/// search that visited each of them would not come back. Stepping back
/// from the first comes round to the last, and on from there to the
/// first again, so a step is asked where the selection stands as well.
#[test]
fn nested_diamonds_are_counted_without_walking_every_copy() {
    let mut forest = flatten(nested_diamonds(40));
    let copies = 1 << 40;

    assert_eq!(
        forest.seek_here("keystone"),
        went_to("dunwich", "dun-50.40", 1, copies)
    );
    assert_eq!(
        forest.next_match(false),
        Some(went_to("dunwich", "dun-50.40", copies, copies))
    );
    assert_eq!(
        forest.next_match(true),
        Some(went_to("dunwich", "dun-50.40", 1, copies))
    );
}

/// Seventy nested diamonds draw more matches than a `usize` holds, so the
/// counts saturate. A place under the west pier comes after everything
/// the east pier holds, and counting on past that still saturates, so
/// stepping on from there comes round to the first.
#[test]
fn a_count_past_what_a_usize_holds_saturates() {
    let mut forest = flatten(nested_diamonds(70));
    let under_the_west_pier = Place::root(key("dunwich", "dun-50.0"))
        .step_to(key("dunwich", "dun-50.0.2"))
        .step_to(key("dunwich", "dun-50.1"));
    let mut matched = forest.matches(Sought::holding("the"));

    assert_eq!(matched.len(), usize::MAX);
    assert_eq!(
        matched.before(&under_the_west_pier),
        Some((usize::MAX, true))
    );

    forest.seek_here("the");
    assert!(forest.go_to_place(&under_the_west_pier));
    assert_eq!(
        forest.next_match(true),
        Some(went_to("dunwich", "dun-50.0", 1, usize::MAX))
    );
}

/// Rooted at one copy of a bead the tree also reaches another way, the
/// root behind the line leaves out every copy of it, since the mode draws
/// it where the forest is rooted instead — `drawn_at_the_root` already
/// says so for a single jump, and a search must agree: the other copy and
/// what only it reaches are rows nothing draws.
#[test]
fn rooting_at_one_copy_excludes_the_others_from_the_root_behind_the_line() {
    let mut forest = under_every_copy(drawn_twice_in_one_tree());
    let [_, lower] = copies_of(&forest, "dun-9");
    step_onto(&mut forest, lower);
    assert!(forest.apply(Action::FocusForest));

    assert_eq!(forest.seek_here("dun-9"), went_to("dunwich", "dun-9", 1, 2));
    assert_eq!(
        forest.next_match(true),
        Some(went_to("dunwich", "dun-9.1", 2, 2))
    );
}

/// `standing_at` finds the exact copy the selection sits on, since
/// `bdi-7ao.136`, rather than the bead's first drawn copy — so a reader
/// who has stepped onto the later copy by hand steps on from there, and
/// the ordinal never drops except at the wrap.
#[test]
fn stepping_forward_from_a_later_copy_never_lowers_the_ordinal() {
    let mut forest = under_every_copy(drawn_twice_in_one_tree());
    forest.seek_here("dun-9");
    let [_, lower] = copies_of(&forest, "dun-9");
    step_onto(&mut forest, lower);

    assert_eq!(
        forest.next_match(true),
        Some(went_to("dunwich", "dun-9.1", 4, 4))
    );
}

/// A tree the filter hid is drawn in its project's hidden-trees group
/// rather than taken off the screen, so a bead the filter hid is one a
/// search still reaches — after the shown ones, which is the order
/// `place_of` already documents.
#[test]
fn a_bead_the_filter_hid_is_one_a_search_still_reaches() {
    let mut forest = flatten(snapshot());

    assert_eq!(
        forest.seek_here("hbr-3.1"),
        went_to("harbour", "hbr-3.1", 1, 1)
    );

    assert_eq!(cursor(&forest), Some(&key("harbour", "hbr-3.1")));
}

/// Nothing matching is nowhere to go, and the forest is left exactly as
/// it was rather than half-opened on the way to nothing.
#[test]
fn a_search_matching_nothing_leaves_the_forest_as_it_was() {
    let mut forest = flatten(snapshot());
    let was = sketch(&forest);
    let selected = forest.selected_line();

    assert_eq!(
        forest.seek_here("dun-404"),
        Landed::Nowhere("dun-404".into())
    );

    assert_eq!(sketch(&forest), was);
    assert_eq!(forest.selected_line(), selected);
}

/// Bead prefixes are per-tracker and uncoordinated, so one id can name a
/// bead in more than one project. That is two matches now rather than one
/// landing and a sentence about the other: the reader steps to the second
/// and looks at it, instead of being told about a bead they cannot see.
#[test]
fn one_id_two_trackers_hold_is_two_matches() {
    let mut forest = flatten(two_trackers_holding_one_id());

    assert_eq!(
        forest.seek_here("dun-7.1.1"),
        went_to("dunwich", "dun-7.1.1", 1, 2)
    );
    assert_eq!(
        forest.next_match(true),
        Some(went_to("ferry", "dun-7.1.1", 2, 2))
    );
}

/// A reader can rest on a row that is not a bead — a project's own line,
/// a group, a pane in one — and stepping from there carries on from where
/// they are, rather than starting the walk again at the top.
///
/// The row has to have matches *above* it or the question cannot be
/// asked: from the top of the forest, carrying on and starting again are
/// the same answer, and a test taken there passes whichever the code
/// does. So the walk stops on the second project's own line, which is
/// drawn below every bead of the first.
///
/// Both projects read a tracker using the same prefix, so every bead
/// matches once in each. From `ferry`'s line, carrying on reaches
/// `ferry`'s first bead — and starting again would reach `dunwich`'s,
/// which is what this used to do.
///
/// **The match asked for is the anchor itself, which is the one position
/// that says where the boundary sits.** The bead below the row counts as
/// *after* it, because the reader is above that bead rather than on it —
/// so `ferry`'s root is a match the walk should reach, not one it should
/// step over. Ask for a match further down and the two readings return
/// the same bead and this proves only that the anchor was consulted:
/// `dun-7.1.1` was the first thing tried here and it could not tell them
/// apart, because the root is drawn above it.
///
/// **The anchor is the first bead drawn below the selection, so a row
/// with none below it has nothing to carry on from and the walk comes
/// round.**
#[test]
fn stepping_from_a_row_that_is_not_a_bead_carries_on_from_there() {
    let mut forest = flatten(two_trackers_holding_one_id());
    forest.seek_here("dun-7");
    walk::until(
        &mut forest,
        |forest| {
            matches!(
                &forest.lines()[forest.selected_line()].content,
                Content::Project(line) if line.project == "ferry"
            )
        },
        |forest| {
            forest.apply(Action::Move(Motion::NextRow));
        },
        |forest| format!("no second project line to rest on: {:#?}", sketch(forest)),
    );

    assert_eq!(
        forest.next_match(true),
        Some(went_to("ferry", "dun-7", 10, 18)),
        "stepping from the project line either started the walk again \
         or stepped over the bead the reader is standing above"
    );
}

/// One project holding a tree the filter draws and a tree it hides, with
/// six matches for `"the"` in the first and two in the second.
///
/// The two blocks are separated by exactly one row — the group's own
/// line — so a walk anchored anywhere but inside the group answers with a
/// bead from the other block, whichever way it steps.
fn a_group_shut_over_matches() -> Snapshot {
    let mut staffed = together("dunwich", &[TOWER, HARBOUR], &panes_on(&["tow-1.1"]));
    staffed.refilter(Filter::LiveAgents);
    staffed
}

/// Rest the selection on the hidden-trees group with the group shut, and
/// say which line that is.
///
/// The fold is asserted rather than assumed. A group drawn open draws its
/// beads on lines below it, and the walk reads its anchor off those — so
/// a fixture that drifted open would take the path these tests are not
/// about and pass without exercising anything.
fn rest_on_the_shut_group(forest: &mut Forest) {
    let at = forest
        .lines()
        .iter()
        .position(|line| {
            matches!(&line.content, Content::Group(group) if group.kind == GroupKind::HiddenTrees)
        })
        .expect("the hidden-trees group is drawn");
    assert_eq!(
        forest.lines()[at].folded,
        Some(false),
        "the group is drawn open, so its beads are on lines of their own: {:#?}",
        sketch(forest)
    );
    forest.select_line(at);
    assert!(on_the_hidden_trees_group(forest), "{:#?}", sketch(forest));
}

/// A group resting shut hides beads that are still in the search's order,
/// so a reader standing on its line is standing above them and stepping
/// on reaches them rather than the block beyond.
///
/// `hbr-3` is the first of them and the one match that tells the two
/// readings apart: the beads above the group are the six the drawn lines
/// would anchor on, and the come-round would answer with the first of
/// those. Ask for a match further inside the group and both wrong answers
/// still land outside it, but so would an anchor one bead off.
///
/// **Anchoring on the lines below has two answers and this pair sees only
/// one of them.** Nothing bead-bearing is drawn below the group here, so
/// there is no anchor to find and the walk comes round. A screen with
/// another project under the group gives the other answer — an anchor on
/// a bead drawn *after* everything the group hides, which steps over all
/// of it — and these two tests would be red for that as well without
/// telling it from the come-round. Adding a project below this fixture
/// would not separate them.
#[test]
fn stepping_from_a_shut_group_goes_into_the_matches_it_hides() {
    let mut forest = flatten(a_group_shut_over_matches());
    forest.seek_here("the");
    rest_on_the_shut_group(&mut forest);

    assert_eq!(
        forest.next_match(true),
        Some(went_to("dunwich", "hbr-3", 7, 8)),
        "stepping on from the shut group left the beads it hides behind: {:#?}",
        sketch(&forest)
    );
}

/// And stepping back from it reaches the match above the group rather
/// than coming round to the last one inside it, which is the bead the
/// reader would have to walk the whole screen to get back from.
#[test]
fn stepping_back_from_a_shut_group_reaches_the_match_above_it() {
    let mut forest = flatten(a_group_shut_over_matches());
    forest.seek_here("the");
    rest_on_the_shut_group(&mut forest);

    assert_eq!(
        forest.next_match(false),
        Some(went_to("dunwich", "tow-1.2.1", 6, 8)),
        "stepping back from the shut group came round instead: {:#?}",
        sketch(&forest)
    );
}

/// The header of a root whose tracker refused is a line with a place, but
/// no bead is drawn there for a match to be counted at. A reader resting
/// on it carries on from there like one resting on a project or a group.
#[test]
fn stepping_from_a_root_whose_tracker_refused_carries_on_below_it() {
    let mut forest = flatten(built(Filter::All));
    forest.seek_here("survey");
    let header = forest
        .lines()
        .iter()
        .position(|line| matches!(&line.content, Content::Unread(unread) if unread.root == "fer-2"))
        .expect("the shared snapshot draws a tree whose tracker refused");
    forest.select_line(header);

    assert_eq!(
        forest.next_match(true),
        Some(went_to("harbour", "hbr-3.1", 3, 3)),
        "stepping on from the refused root came round: {:#?}",
        sketch(&forest)
    );
}

/// A reader resting above a refused root's header, on its project's line,
/// is standing above the first bead *drawn* below, not above the header.
#[test]
fn stepping_from_above_a_root_whose_tracker_refused_passes_its_header() {
    let mut forest = flatten(built(Filter::All));
    forest.seek_here("survey");
    let project = forest
        .lines()
        .iter()
        .position(|line| matches!(&line.content, Content::Project(line) if line.project == "ferry"))
        .expect("the shared snapshot draws the ferry project");
    forest.select_line(project);

    assert_eq!(
        forest.next_match(true),
        Some(went_to("harbour", "hbr-3.1", 3, 3)),
        "stepping on from above the refused root came round: {:#?}",
        sketch(&forest)
    );
}

/// Stepping before anything has been searched for has nothing to step
/// through, which is not a failure: nothing has gone wrong, nothing
/// moves, and there is nothing to say about it.
#[test]
fn stepping_before_a_search_has_nothing_to_step_through() {
    let mut forest = flatten(snapshot());
    let was = sketch(&forest);

    assert_eq!(forest.next_match(true), None);

    assert_eq!(sketch(&forest), was);
}

/// Going to a bead a tree reaches twice lands on the copy the screen
/// draws first, which is not the copy its siblings sort first.
///
/// `twn-9` hangs under `twn-1.1` and again under `twn-1.4`. All four of
/// `twn-1`'s children are closed, so the sort puts `.1` above `.4`; but
/// `.4` has a pane working it, so it is not a *finished* branch, and the
/// screen draws it above the run that `.1`, `.2` and `.3` collapse into.
/// The copy under `.4` is on screen; the copy under `.1` is inside a shut
/// run.
///
/// This is `Forest::go_to`, which is every jump: the search's landing and
/// Enter on a reference in the bead window both come through here. Taking
/// the reader into a run when a drawn copy of the same bead was above it
/// is the wrong one either way — and until this, nothing said so.
///
/// **The three quiet children are the run, not three of them.** `MANY` is
/// the fewest finished siblings that make one, and below it `split_by`
/// hands back the links untouched — so a fixture one child smaller puts
/// the two orders back into agreement, and this passes whichever one it
/// is walking. The run is asserted before the jump rather than left to
/// the fixture, so trimming it fails here instead of quietly proving
/// nothing.
#[test]
fn going_to_a_bead_drawn_twice_lands_on_the_copy_drawn_above_a_run() {
    let mut forest = flatten(alone("dunwich", TWIN_BESIDE_A_RUN, &panes_on(&["twn-1.4"])));

    // Four, not three: a run counts the work it stands over, and `twn-9`
    // hangs under one of the three.
    assert!(
        drawn_here(&forest, "… 4 more"),
        "no run formed, so the two orders agree and this test cannot \
         tell them apart: {:#?}",
        sketch(&forest)
    );

    assert!(forest.go_to(&key("dunwich", "twn-9")));

    assert_eq!(
        forest.place().map(|place| place.steps.clone()),
        Some(vec![key("dunwich", "twn-1.4"), key("dunwich", "twn-9")]),
        "{:#?}",
        sketch(&forest)
    );
}

/// A tree whose root has four closed children, one of them staffed. The
/// three quiet ones make a run; the staffed one is drawn above it. A
/// fifth bead hangs under one of each, so the two orders reach it by
/// different ways down.
const TWIN_BESIDE_A_RUN: &str = r#"[
  {"id":"twn-1","title":"re-roof the shed","status":"in_progress",
   "priority":1,"issue_type":"epic"},
  {"id":"twn-1.1","title":"strip the felt","status":"closed",
   "dependencies":[{"depends_on_id":"twn-1","type":"parent-child"}],
   "priority":2,"issue_type":"task","closed_at":"2026-08-28T09:00:00Z"},
  {"id":"twn-1.2","title":"clear the gutters","status":"closed",
   "dependencies":[{"depends_on_id":"twn-1","type":"parent-child"}],
   "priority":2,"issue_type":"task","closed_at":"2026-08-27T09:00:00Z"},
  {"id":"twn-1.3","title":"sweep the yard","status":"closed",
   "dependencies":[{"depends_on_id":"twn-1","type":"parent-child"}],
   "priority":2,"issue_type":"task","closed_at":"2026-08-26T09:00:00Z"},
  {"id":"twn-1.4","title":"lay the new felt","status":"closed",
   "dependencies":[{"depends_on_id":"twn-1","type":"parent-child"},
                   {"depends_on_id":"twn-9","type":"blocks"}],
   "priority":2,"issue_type":"task","closed_at":"2026-08-25T09:00:00Z"},
  {"id":"twn-9","title":"borrow the ladder","status":"closed",
   "dependencies":[{"depends_on_id":"twn-1.1","type":"parent-child"}],
   "priority":2,"issue_type":"task","closed_at":"2026-08-24T09:00:00Z"}
]"#;

/// A parent with enough finished children draws the rest of them first
/// and the run after, so the order siblings are *sorted* into is not the
/// order they are drawn in — and a walk taking `links_below` alone gets
/// the difference wrong.
///
/// The difference is not open against closed, which the sibling sort
/// already handles. It is that a run is of *finished branches*, and
/// `lines::finished` is every bead in the branch closed **and no agent on
/// it** — a stricter thing than the status the sort reads. `dun-7.4` is
/// closed with a pane working it, so it is not finished, and it stays
/// drawn among its siblings while `.2`, `.3` and `.5` elide beneath them.
/// Sorted it is fifth of its siblings; drawn it is third.
///
/// A bead inside a run is drawn nowhere until the run is opened, and a
/// search opens it, so the run's members are matches like any others —
/// after the siblings drawn above them.
#[test]
fn a_run_of_finished_children_is_walked_where_the_screen_draws_it() {
    let mut forest = flatten(snapshot());

    assert_eq!(
        forest.seek_here("dun-7."),
        went_to("dunwich", "dun-7.1", 1, 8)
    );
    // `.1`'s own children, drawn under it and above its siblings, then
    // `.7`, which the sibling sort already puts above the closed ones.
    forest.next_match(true);
    forest.next_match(true);
    forest.next_match(true);

    // The staffed closed bead, drawn above the run rather than in it.
    // Walked from `links_below` alone this is `dun-7.2`.
    assert_eq!(
        forest.next_match(true),
        Some(went_to("dunwich", "dun-7.4", 5, 8))
    );
}

/// The screen draws projects in the order the config names them, and
/// `snapshot.trees` is in the order they were *read* — which is why
/// `snapshot.projects` exists. So a search cannot take its order from the
/// trees: a project read second and drawn first would be walked in the
/// wrong place, and the ordinal at the foot would count rows the reader
/// cannot count to.
#[test]
fn matches_are_numbered_by_the_order_projects_are_drawn_not_read() {
    let mut forest = flatten(read_in_reverse());

    // `dunwich` is named first by the config and read second here.
    assert_eq!(
        forest.seek_here("dun-7.1.1"),
        went_to("dunwich", "dun-7.1.1", 1, 2)
    );
    assert_eq!(
        forest.next_match(true),
        Some(went_to("harbour", "dun-7.1.1", 2, 2))
    );
}

/// Two projects whose trees were read in the opposite order to the one
/// the config names them in.
fn read_in_reverse() -> Snapshot {
    gather(
        vec![tree_of("harbour", DUNWICH), tree_of("dunwich", DUNWICH)],
        Vec::new(),
        Filter::All,
    )
}

/// Two projects reading trackers that use the same prefix, which nothing
/// coordinates and nothing forbids.
fn two_trackers_holding_one_id() -> Snapshot {
    gather(
        vec![tree_of("dunwich", DUNWICH), tree_of("ferry", DUNWICH)],
        Vec::new(),
        Filter::All,
    )
}

/// A tree where one bead's title names another bead's whole id, and is
/// drawn above it. A search for that id matches both of them.
const NAMED_IN_A_TITLE: &str = r#"[
  {"id":"dun-6","title":"re-site the mast","status":"in_progress",
   "priority":1,"issue_type":"epic"},
  {"id":"dun-6.1","title":"wait on dun-6.2 before pouring","status":"open",
   "dependencies":[{"depends_on_id":"dun-6","type":"parent-child"}],
   "priority":2,"issue_type":"task"},
  {"id":"dun-6.2","title":"cure the base","status":"open",
   "dependencies":[{"depends_on_id":"dun-6","type":"parent-child"}],
   "priority":2,"issue_type":"task"}
]"#;

const NAMED_INSIDE_A_LONGER_ID: &str = r#"[
  {"id":"dun-6","title":"re-site the mast","status":"in_progress",
   "priority":1,"issue_type":"epic"},
  {"id":"dun-6.1","title":"pour the footing","status":"open",
   "dependencies":[{"depends_on_id":"dun-6","type":"parent-child"},
                   {"depends_on_id":"dun-6.2.1","type":"blocks"}],
   "priority":2,"issue_type":"task"},
  {"id":"dun-6.2","title":"cure the base","status":"open",
   "dependencies":[{"depends_on_id":"dun-6","type":"parent-child"}],
   "priority":2,"issue_type":"task"},
  {"id":"dun-6.2.1","title":"strike the formwork","status":"open",
   "dependencies":[{"depends_on_id":"dun-6.2","type":"parent-child"}],
   "priority":2,"issue_type":"task"}
]"#;
