//! The row at the foot of the screen: the keys, every notice the view
//! carries, and how fresh what is above it is.

use ratatui::style::{Color, Style};
use ratatui::text::Span;

use crate::model::snapshot::HerdrState;
use crate::view::fitted::{columns, Fitted, GAP};
use crate::view::phrase;
use crate::view::row::WARNING;
use crate::view::{Freshness, Notice};

use super::tone::LOOK_AT_THIS;

/// Everything the status bar has to say, in the order it should give it up.
///
/// The herdr one is read off the snapshot behind this frame and can change
/// under the reader; the rest were settled before the first collection and
/// hold for the session. Consequence decides the order, not provenance: a
/// herdr nobody can reach empties the agent column, which is what the reader
/// came for, so it is the last thing a narrow screen takes away.
pub(super) fn notices(herdr: HerdrState, at_startup: &[Notice]) -> Vec<Notice> {
    let collected = match herdr {
        HerdrState::Ok => None,
        HerdrState::Unavailable => Some(Notice::NoHerdr),
    };

    collected
        .into_iter()
        .chain(at_startup.iter().copied())
        .collect()
}

/// The row at the foot of the screen: the keys, every notice the view
/// carries, and how fresh what is above it is.
///
/// The notices are drawn first and yield last: keys can be rediscovered, and
/// a fact that is silently absent from the one row a reader can neither fold
/// nor scroll away from is a fact they will never learn. Where the screen is
/// too narrow even for those, they yield from the end, so the caller's order
/// is the order they are given up in.
///
/// How fresh the view is follows whatever the row leads with — the notices
/// where there are any, the keys where there are none — and yields before
/// either of them. A notice is something to act on and the keys are the
/// row's own subject; a clock is what a reader checks those against, so a
/// screen with no room for it has spent the columns on the more useful
/// things.
///
/// `width` is what this row will be drawn into. Choosing which words to say
/// is a different job from cutting the words chosen, and only the first of
/// them belongs here.
///
/// Nothing here knows what produced a notice. That is the point: a snapshot
/// and this process both reach the screen through the same list, and the next
/// thing that has something to say joins them by being one.
pub(super) fn status_bar(
    notices: &[Notice],
    how_fresh: Option<Freshness>,
    keys: &str,
    width: usize,
) -> Fitted {
    let keys = Span::raw(keys.to_string());
    // Drawn plain and dim: it is what a reader glances at to place the rest,
    // not one of the things the rest is asking them to look at.
    let how_fresh = how_fresh
        .map(|how_fresh| {
            Span::styled(
                phrase::freshness(how_fresh),
                Style::new().fg(Color::DarkGray),
            )
        })
        .into_iter()
        .collect::<Vec<_>>();

    if notices.is_empty() {
        return Fitted::new(vec![keys], how_fresh, Vec::new()).title_or_nothing();
    }

    Fitted::new(
        vec![Span::styled(
            said(notices, width),
            Style::new().fg(LOOK_AT_THIS),
        )],
        how_fresh,
        vec![keys],
    )
    .title_or_nothing()
}

/// Every notice the foot carries, in the fullest words that let all of them
/// still be said.
///
/// Words are given up before facts are, and from the end, which is the order
/// the notices themselves are given up in: the notice the caller put first
/// keeps its full phrase longest, because it is the one that costs the reader
/// most. Neither phrase fits in full on a forty-column screen, so there they
/// all speak briefly and all of them are still there.
///
/// Being cut is the one thing a notice must not be. The mark a cut leaves is
/// the mark any long line gets, so a severed warning reads as a sentence that
/// ran out of room rather than as a fact the reader has lost — and a foot with
/// two notices on it can be cut before the second one has begun.
fn said(notices: &[Notice], width: usize) -> String {
    let mut words = notices
        .iter()
        .map(|notice| phrase::notice(*notice))
        .collect::<Vec<_>>();

    for (at, notice) in notices.iter().enumerate().rev() {
        if columns(&[Span::raw(marked(&words))]) <= width {
            break;
        }
        words[at] = phrase::brief_notice(*notice);
    }

    marked(&words)
}

/// The notices as one run of text, each behind the mark that says it is a
/// warning and clear of the one before it.
fn marked(words: &[&'static str]) -> String {
    words
        .iter()
        .map(|said| format!("{WARNING} {said}"))
        .collect::<Vec<_>>()
        .join(&" ".repeat(GAP))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    use crate::view::draw::tests::*;
    use chrono::{Local, TimeZone, Utc};

    // ---- the key bar -----------------------------------------------------

    /// What the row says is the loop's to decide; the foot's job is to put it
    /// on screen whole where there is room for it.
    #[test]
    fn the_foot_of_the_screen_shows_the_keys_it_is_handed() {
        let drawn = drawn(status_bar(&[], None, A_KEY_ROW, 60), 60, 1);

        assert!(drawn[0].starts_with(A_KEY_ROW), "{drawn:?}");
    }

    /// With no herdr there is no agent on any row, and a screen that only
    /// stopped showing them would read as a fleet with nobody working in it.
    /// It goes at the foot because that is the one row that cannot be folded
    /// or scrolled away.
    #[test]
    fn a_herdr_that_could_not_be_reached_is_said_where_nothing_can_hide_it() {
        let drawn = drawn(status_bar(&[Notice::NoHerdr], None, A_KEY_ROW, 90), 90, 1);

        says(
            &drawn[0],
            "no herdr session · which agents are alive is unknown",
        );
    }

    /// The bead this row was built for: a `bdi` that could not open its
    /// inbound socket is told nothing when a project changes, so what is on
    /// screen is only as fresh as the last poll. Nothing above the foot could
    /// show that — no row is wrong — so the foot is the only place it can go.
    #[test]
    fn a_bdi_nothing_can_reach_says_so_for_the_life_of_the_session() {
        let drawn = drawn(
            status_bar(&[Notice::NoInboundChannel], None, A_KEY_ROW, 90),
            90,
            1,
        );

        says(
            &drawn[0],
            "nothing can tell bdi a project changed · every project is polled instead",
        );
    }

    /// Two notices are two facts and the reader needs both: neither one
    /// implies the other, and a foot that showed only the first would leave
    /// the second unsaid for the whole session.
    #[test]
    fn a_foot_with_room_says_every_notice_it_is_given() {
        let drawn = drawn(
            status_bar(
                &[Notice::NoHerdr, Notice::NoInboundChannel],
                None,
                A_KEY_ROW,
                200,
            ),
            200,
            1,
        );

        for words in [
            "no herdr session · which agents are alive is unknown",
            "nothing can tell bdi a project changed · every project is polled instead",
        ] {
            says(&drawn[0], words);
        }
    }

    /// The order the caller gives is the order the foot gives up, so a screen
    /// with room for one full phrase keeps it for the notice that costs the
    /// reader most. What the other gives up is its words, not its place.
    #[test]
    fn a_narrow_foot_gives_up_the_last_notices_words_first() {
        let drawn = drawn(
            status_bar(
                &[Notice::NoHerdr, Notice::NoInboundChannel],
                None,
                A_KEY_ROW,
                80,
            ),
            80,
            1,
        );

        says(
            &drawn[0],
            "no herdr session · which agents are alive is unknown",
        );
        says(&drawn[0], "polled, not reported");
    }

    // ---- how fresh the screen is ------------------------------------------

    /// The bead's second half: a refresh the timer started changes rows under
    /// a reader with nothing to mark that it did. The foot is where a fact
    /// about the whole view goes, and it is the row that cannot be scrolled
    /// away from.
    #[test]
    fn the_foot_says_when_what_is_on_the_screen_was_collected() {
        let at = Utc.with_ymd_and_hms(2026, 8, 30, 10, 22, 14).unwrap();

        let drawn = drawn(
            status_bar(&[], Some(Freshness::Collected(at)), A_KEY_ROW, 90),
            90,
            1,
        );

        says(
            &drawn[0],
            &format!("collected {}", at.with_timezone(&Local).format("%H:%M:%S")),
        );
    }

    /// The bead's first half. `^R` and the timer both start a collection that
    /// takes seconds, and until this the screen was byte-identical for every
    /// one of them.
    #[test]
    fn the_foot_says_while_a_collection_is_running() {
        let drawn = drawn(
            status_bar(&[], Some(Freshness::Collecting), A_KEY_ROW, 90),
            90,
            1,
        );

        says(&drawn[0], "collecting");
    }

    /// A notice is something a reader must act on and the keys are the row's
    /// own subject; a clock is what they check those against. So the clock is
    /// the first of the three a narrowing screen gives up, before the foot
    /// starts cutting the keys as it already did.
    #[test]
    fn a_narrow_foot_gives_up_the_clock_before_a_notice_or_the_keys() {
        let at = Utc.with_ymd_and_hms(2026, 8, 30, 10, 22, 14).unwrap();
        let foot = |width| {
            status_bar(
                &[Notice::NoHerdr],
                Some(Freshness::Collected(at)),
                A_KEY_ROW,
                width,
            )
        };

        let roomy = drawn(foot(130), 130, 1);
        says(&roomy[0], "collected");

        let narrow = drawn(foot(110), 110, 1);
        does_not_say(&narrow[0], "collected");
        says(
            &narrow[0],
            "no herdr session · which agents are alive is unknown",
        );
        says(&narrow[0], A_KEY_ROW);
    }

    /// The whole screen at a width with room for the foot's three parts, so
    /// the clock is seen where it lands rather than only where it is cut.
    /// The forest above it says nothing about freshness: which read the rows
    /// came from is a fact about the view, and no row is any different for
    /// it.
    #[test]
    fn a_frame_with_room_says_when_the_rows_on_it_were_collected() {
        let forest = opened(&snapshot(vec![grove(1)], Vec::new(), HerdrState::Ok));

        assert_eq!(
            frame_of(&forest, 74, 4),
            vec![
                "▾ summit-works                                                         0/2",
                "  └── ◐ nix-9670s  lift the ground station                             0/2",
                "                                                                          ",
                // The words are written out; only the clock is computed,
                // because it is the reader's own and CI reads it in another
                // zone.
                &format!(
                    "{A_KEY_ROW}  collected {}    ",
                    read_at().with_timezone(&Local).format("%H:%M:%S")
                ),
            ]
        );
    }

    /// The bead this was written for. On the narrowest supported screen
    /// neither phrase fits in full, so a foot that could only cut told the
    /// reader nothing: `NoHerdr` was severed mid-sentence and `NoInboundChannel`
    /// never began. The mark a cut leaves is the mark any long line gets, so
    /// there was nothing on screen to say either fact had been lost.
    #[test]
    fn the_narrowest_screen_still_says_the_view_is_polled() {
        let drawn = drawn(
            status_bar(
                &[Notice::NoHerdr, Notice::NoInboundChannel],
                None,
                A_KEY_ROW,
                40,
            ),
            40,
            1,
        );

        assert_eq!(drawn[0], "⚠ agents unknown  ⚠ polled, not reported");
    }

    /// A second `bdi` on a machine whose herdr is well: one notice, and in
    /// full it is wider than a side-by-side pane. The words a cut took were
    /// the ones that say what it costs the reader, because a cut takes the
    /// end.
    #[test]
    fn a_lone_notice_too_wide_for_the_row_is_said_briefly() {
        let drawn = drawn(
            status_bar(&[Notice::NoInboundChannel], None, A_KEY_ROW, 60),
            60,
            1,
        );

        assert!(drawn[0].starts_with("⚠ polled, not reported"), "{drawn:?}");
    }

    /// A notice is drawn in the colour that asks to be looked at, and saying
    /// it in fewer words does not make it something else. `drawn` reads
    /// symbols and is blind to styling, so this asks `painted`.
    #[test]
    fn a_notice_said_briefly_is_still_painted_as_a_warning() {
        let painted = painted(
            status_bar(&[Notice::NoInboundChannel], None, A_KEY_ROW, 60),
            60,
        );

        assert!(
            painted
                .iter()
                .any(|(said, colour)| said.contains("polled, not reported")
                    && *colour == LOOK_AT_THIS),
            "{painted:?}"
        );
    }

    /// A frame draws what the snapshot behind it found and what the session
    /// settled at startup through one list, and the snapshot's go first
    /// because a herdr nobody can reach empties the agent column.
    #[test]
    fn the_snapshots_notice_outranks_the_sessions() {
        assert_eq!(
            notices(HerdrState::Unavailable, &[Notice::NoInboundChannel]),
            vec![Notice::NoHerdr, Notice::NoInboundChannel]
        );
    }

    /// A session fact reaches the foot whether or not the collection behind
    /// the frame found anything to say — the two travel by the same road and
    /// neither depends on the other.
    #[test]
    fn a_session_notice_stands_alone_where_the_snapshot_is_well() {
        assert_eq!(
            notices(HerdrState::Ok, &[Notice::NoInboundChannel]),
            vec![Notice::NoInboundChannel]
        );
    }

    #[test]
    fn a_session_with_nothing_wrong_leaves_the_foot_to_the_keys() {
        assert_eq!(notices(HerdrState::Ok, &[]), Vec::new());
    }

    /// Keys can be rediscovered; a herdr that is silently absent cannot. So on
    /// a screen too narrow for both, the keys are what gives way.
    #[test]
    fn a_narrow_foot_gives_up_the_keys_before_the_missing_herdr() {
        let drawn = drawn(status_bar(&[Notice::NoHerdr], None, A_KEY_ROW, 60), 60, 1);

        assert!(drawn[0].contains("no herdr session"), "{drawn:?}");
        assert_eq!(drawn[0].chars().count(), 60);
    }
}
