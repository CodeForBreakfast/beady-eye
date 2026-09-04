//! The row at the foot of the screen: the keys and every notice the view
//! carries.

use ratatui::text::Span;

use crate::model::snapshot::{ProviderState, Snapshot};
use crate::view::fitted::{columns, Fitted, GAP};
use crate::view::palette;
use crate::view::phrase;
use crate::view::row::WARNING;
use crate::view::Notice;

/// Everything the status bar has to say, in the order it should give it up.
///
/// All but the last are read off the snapshot behind this frame, and that is
/// the point rather than a convenience: the snapshot is also what `--json`
/// publishes, so a fact the foot draws from it is a fact a consumer of the
/// contract has too, and neither mouth can qualify a run the other does not.
/// What is left standing is this process's own — a channel that would not
/// open, a config that would not reload — which a run that prints one
/// snapshot and exits has neither of.
///
/// Consequence decides the order, not provenance: a provider nobody can
/// reach empties the agent column, which is what the reader came for, so it
/// is the last thing a narrow screen takes away, and a session nobody can
/// reach empties that session's part of it. A guessed project name outranks
/// the standing ones because it is half of every key on the screen, where a
/// channel that would not open costs freshness alone.
///
/// A provider nobody installed says nothing here. The reader has lost
/// nothing — they never had an agent column — and a warning about a program
/// they have never heard of is a warning they cannot act on.
pub(super) fn notices(snapshot: &Snapshot, standing: &[Notice]) -> Vec<Notice> {
    let collected = match snapshot.agents.state {
        ProviderState::Answering | ProviderState::Absent => None,
        ProviderState::NotAnswering => Some(Notice::AgentsUnknown),
    };
    let unanswered = snapshot
        .agents
        .unanswered()
        .map(|session| Notice::SessionUnanswered(session.to_string()));
    // One line however many projects were guessed, because the remedy is one
    // line in a shell profile and saying it per project would say it once and
    // repeat it.
    let guessed =
        (!snapshot.projects_named_without_git.is_empty()).then_some(Notice::ProjectNamedWithoutGit);

    collected
        .into_iter()
        .chain(unanswered)
        .chain(guessed)
        .chain(standing.iter().cloned())
        .collect()
}

/// The row at the foot of the screen: the keys and every notice the view
/// carries.
///
/// The notices are drawn first and yield last: keys can be rediscovered, and
/// a fact that is silently absent from the one row a reader can neither fold
/// nor scroll away from is a fact they will never learn. Where the screen is
/// too narrow even for those, they yield from the end, so the caller's order
/// is the order they are given up in.
///
/// How fresh the rows are is not here. It was, while it was one claim about
/// the whole screen; it is now a project's own fact, said beside the
/// project's name where it is exact.
///
/// `copied` is the id the reader has just put on the clipboard, where they
/// have. It is the first thing the row gives up and it goes whole or not at
/// all: half an id names nothing, and the keys were there before the reader
/// pressed anything, so they are never the thing that moves to make room.
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
    copied: Option<&str>,
    keys: &str,
    width: usize,
) -> Fitted {
    let keys = Span::raw(keys.to_string());
    let copied: Vec<Span<'static>> = copied
        .map(|id| Span::raw(phrase::copied(id)))
        .into_iter()
        .collect();

    if notices.is_empty() {
        return Fitted::new(vec![keys], Vec::new(), copied).state_or_nothing();
    }

    Fitted::new(
        vec![Span::styled(said(notices, width), palette::ATTENTION)],
        copied,
        vec![keys],
    )
    .title_or_nothing()
    .state_or_nothing()
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
    let mut words = notices.iter().map(phrase::notice).collect::<Vec<_>>();

    for (at, notice) in notices.iter().enumerate().rev() {
        if columns(&[Span::raw(marked(&words))]) <= width {
            break;
        }
        words[at] = phrase::brief_notice(notice);
    }

    marked(&words)
}

/// The notices as one run of text, each behind the mark that says it is a
/// warning and clear of the one before it.
fn marked(words: &[String]) -> String {
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

    use crate::config::Scope;
    use crate::model::snapshot::{a_provider, Filter, ProviderState as State, A_PROVIDER};
    use crate::view::draw::tests::*;

    /// A frame's snapshot with the provider a test is about and nothing else
    /// to say.
    fn behind_a_frame(agents: crate::model::snapshot::AgentProvider) -> Snapshot {
        Snapshot {
            agents,
            ..nothing_to_qualify()
        }
    }

    /// A snapshot from a run in which everything answered and no name was
    /// guessed — the state every notice here is measured against.
    fn nothing_to_qualify() -> Snapshot {
        Snapshot::awaiting(
            Vec::new(),
            Vec::new(),
            A_PROVIDER,
            Scope::Everything,
            Filter::LiveAgents,
            "2026-08-30T12:00:00Z".parse().expect("the instant parses"),
        )
    }

    /// A guessed project name is the snapshot's to report, and the same field
    /// `--json` publishes is what the foot reads: one fact, so a screen and a
    /// printed snapshot of the same moment cannot disagree about it.
    #[test]
    fn a_project_named_without_git_is_said_at_the_foot_from_the_snapshot() {
        let guessed = Snapshot {
            projects_named_without_git: vec!["orbital".to_string()],
            ..nothing_to_qualify()
        };

        assert_eq!(
            notices(&guessed, &[]),
            vec![Notice::ProjectNamedWithoutGit],
            "the foot did not read the guess off the snapshot"
        );
        assert_eq!(
            notices(&nothing_to_qualify(), &[]),
            Vec::new(),
            "a run that guessed no name was warned about one"
        );
    }

    /// Two projects guessed is still one remedy — a line in a shell profile —
    /// so the foot says it once rather than once per project.
    #[test]
    fn several_guessed_names_are_one_notice() {
        let guessed = Snapshot {
            projects_named_without_git: vec!["orbital".to_string(), "ferry".to_string()],
            ..nothing_to_qualify()
        };

        assert_eq!(notices(&guessed, &[]), vec![Notice::ProjectNamedWithoutGit]);
    }

    /// A guessed name costs the reader more than a channel that would not
    /// open — it is half of every key on the screen — so a narrowing foot
    /// gives up the channel first.
    #[test]
    fn a_guessed_name_outranks_what_this_process_settled() {
        let guessed = Snapshot {
            projects_named_without_git: vec!["orbital".to_string()],
            ..behind_a_frame(a_provider(State::NotAnswering))
        };

        assert_eq!(
            notices(&guessed, &[Notice::NoInboundChannel]),
            vec![
                Notice::AgentsUnknown,
                Notice::ProjectNamedWithoutGit,
                Notice::NoInboundChannel
            ]
        );
    }

    // ---- the key bar -----------------------------------------------------

    /// What the row says is the loop's to decide; the foot's job is to put it
    /// on screen whole where there is room for it.
    #[test]
    fn the_foot_of_the_screen_shows_the_keys_it_is_handed() {
        let drawn = Painted::of(status_bar(&[], None, A_KEY_ROW, 60), 60, 1).rows();

        assert!(drawn[0].starts_with(A_KEY_ROW), "{drawn:?}");
    }

    /// With no herdr there is no agent on any row, and a screen that only
    /// stopped showing them would read as a fleet with nobody working in it.
    /// It goes at the foot because that is the one row that cannot be folded
    /// or scrolled away.
    #[test]
    fn a_herdr_that_could_not_be_reached_is_said_where_nothing_can_hide_it() {
        let drawn = Painted::of(
            status_bar(&[Notice::AgentsUnknown], None, A_KEY_ROW, 90),
            90,
            1,
        )
        .rows();

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
        let drawn = Painted::of(
            status_bar(&[Notice::NoInboundChannel], None, A_KEY_ROW, 90),
            90,
            1,
        )
        .rows();

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
        let drawn = Painted::of(
            status_bar(
                &[Notice::AgentsUnknown, Notice::NoInboundChannel],
                None,
                A_KEY_ROW,
                200,
            ),
            200,
            1,
        )
        .rows();

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
        let drawn = Painted::of(
            status_bar(
                &[Notice::AgentsUnknown, Notice::NoInboundChannel],
                None,
                A_KEY_ROW,
                80,
            ),
            80,
            1,
        )
        .rows();

        says(
            &drawn[0],
            "no herdr session · which agents are alive is unknown",
        );
        says(&drawn[0], "polled, not reported");
    }

    // ---- what the reader just copied ---------------------------------------

    /// The keys stay where they are and the id joins them: a reader looking
    /// for the legend must not find it moved by a key they pressed a moment
    /// ago.
    #[test]
    fn a_copied_id_is_said_after_the_keys_where_nothing_is_wrong() {
        let drawn = Painted::of(status_bar(&[], Some("grv-1"), A_KEY_ROW, 80), 80, 1).rows();

        assert!(drawn[0].starts_with(A_KEY_ROW), "{drawn:?}");
        assert!(drawn[0].trim_end().ends_with("copied grv-1"), "{drawn:?}");
    }

    /// Beside a notice the keys keep their place at the end of the row, and
    /// the id goes between.
    #[test]
    fn a_copied_id_is_said_between_a_notice_and_the_keys() {
        let drawn = Painted::of(
            status_bar(&[Notice::AgentsUnknown], Some("grv-1"), A_KEY_ROW, 120),
            120,
            1,
        )
        .rows();

        assert_eq!(
            drawn[0].trim_end(),
            format!(
                "⚠ no herdr session · which agents are alive is unknown  copied grv-1{}{A_KEY_ROW}",
                " ".repeat(120 - 54 - 2 - 12 - A_KEY_ROW.chars().count())
            )
        );
    }

    /// Half an id pastes as nothing anyone asked for, so a row with no room
    /// for the whole of it says none of it — and keeps the keys, which were
    /// there before the reader pressed anything.
    #[test]
    fn a_copied_id_the_row_has_no_room_for_is_dropped_whole_and_the_keys_stay() {
        let drawn = Painted::of(status_bar(&[], Some("grv-1"), A_KEY_ROW, 50), 50, 1).rows();

        assert_eq!(drawn[0].trim_end(), A_KEY_ROW);
    }

    // ---- how fresh the screen is ------------------------------------------

    /// `bdi-7ao.27`: the foot spoke for every project at once, so a refresh
    /// naming one project said the whole screen was being read, and a
    /// resting foot had to quote the oldest read on it to stay true. It is a
    /// project's own fact and it is now said on the project's own line.
    #[test]
    fn the_foot_says_nothing_about_how_fresh_the_rows_above_it_are() {
        let forest = opened(&snapshot(
            vec![grove(1)],
            Vec::new(),
            ProviderState::Answering,
        ));

        let foot = frame_of(&forest, 74, 4).rows().remove(3);

        assert_eq!(foot.trim_end(), A_KEY_ROW);
    }

    /// The bead this was written for. On the narrowest supported screen
    /// neither phrase fits in full, so a foot that could only cut told the
    /// reader nothing: `NoHerdr` was severed mid-sentence and `NoInboundChannel`
    /// never began. The mark a cut leaves is the mark any long line gets, so
    /// there was nothing on screen to say either fact had been lost.
    #[test]
    fn the_narrowest_screen_still_says_the_view_is_polled() {
        let drawn = Painted::of(
            status_bar(
                &[Notice::AgentsUnknown, Notice::NoInboundChannel],
                None,
                A_KEY_ROW,
                40,
            ),
            40,
            1,
        )
        .rows();

        assert_eq!(drawn[0], "⚠ agents unknown  ⚠ polled, not reported");
    }

    /// A second `bdi` on a machine whose herdr is well: one notice, and in
    /// full it is wider than a side-by-side pane. The words a cut took were
    /// the ones that say what it costs the reader, because a cut takes the
    /// end.
    #[test]
    fn a_lone_notice_too_wide_for_the_row_is_said_briefly() {
        let drawn = Painted::of(
            status_bar(&[Notice::NoInboundChannel], None, A_KEY_ROW, 60),
            60,
            1,
        )
        .rows();

        assert!(drawn[0].starts_with("⚠ polled, not reported"), "{drawn:?}");
    }

    /// A notice is drawn in the colour that asks to be looked at, and saying
    /// it in fewer words does not make it something else.
    #[test]
    fn a_notice_said_briefly_is_still_painted_as_a_warning() {
        let painted = Painted::of(
            status_bar(&[Notice::NoInboundChannel], None, A_KEY_ROW, 60),
            60,
            1,
        )
        .row(0);

        assert!(
            painted
                .iter()
                .any(|run| run.said.contains("polled, not reported")
                    && run.style.fg == palette::ATTENTION.fg),
            "{painted:?}"
        );
    }

    /// `bdi-7ao.61`: measured on this machine, a `bdi` refused the socket
    /// drew *nothing can tell bdi a project changed · every project is polled
    /// instead* for an hour while a finished mutation run held the socket,
    /// and there was no way to learn that from the screen. The cause is what
    /// was missing, and it is the only one of the three a reader can put
    /// right.
    #[test]
    fn a_socket_another_bdi_holds_says_that_rather_than_only_what_it_cost() {
        let drawn = Painted::of(
            status_bar(
                &[Notice::AnotherBdiHadTheInboundChannel],
                None,
                A_KEY_ROW,
                100,
            ),
            100,
            1,
        )
        .rows();

        says(
            &drawn[0],
            "another bdi held the inbound channel · every project is polled instead",
        );
    }

    /// The cause is the half a reader can act on, so it survives the width
    /// that takes the words around it. A brief notice that gave it up would
    /// say no more than the notice this bead replaced.
    #[test]
    fn the_narrowest_screen_still_says_another_bdi_took_the_channel() {
        let drawn = Painted::of(
            status_bar(
                &[
                    Notice::AgentsUnknown,
                    Notice::AnotherBdiHadTheInboundChannel,
                ],
                None,
                A_KEY_ROW,
                40,
            ),
            40,
            1,
        )
        .rows();

        assert_eq!(
            drawn[0].trim_end(),
            "⚠ agents unknown  ⚠ another bdi had it"
        );
    }

    /// A notice nobody looks at is a notice nobody has, and this one is
    /// asking the reader to go and close something.
    #[test]
    fn a_socket_another_bdi_holds_is_painted_as_a_warning() {
        let painted = Painted::of(
            status_bar(
                &[Notice::AnotherBdiHadTheInboundChannel],
                None,
                A_KEY_ROW,
                100,
            ),
            100,
            1,
        )
        .row(0);

        assert!(
            painted.iter().any(
                |run| run.said.contains("another bdi") && run.style.fg == palette::ATTENTION.fg
            ),
            "{painted:?}"
        );
    }

    /// A frame draws what the snapshot behind it found and what the session
    /// settled at startup through one list, and the snapshot's go first
    /// because a herdr nobody can reach empties the agent column.
    #[test]
    fn the_snapshots_notice_outranks_the_sessions() {
        assert_eq!(
            notices(
                &behind_a_frame(a_provider(ProviderState::NotAnswering)),
                &[Notice::NoInboundChannel]
            ),
            vec![Notice::AgentsUnknown, Notice::NoInboundChannel]
        );
    }

    /// A provider nobody installed is not a finding, so it costs the foot
    /// nothing — while a provider that is installed and will not answer costs
    /// it the notice above. The two states put the same empty agent column on
    /// the screen and only one of them is something the reader lost.
    #[test]
    fn a_provider_that_was_never_installed_is_not_warned_about() {
        assert_eq!(
            notices(
                &behind_a_frame(a_provider(ProviderState::Absent)),
                &[Notice::NoInboundChannel]
            ),
            vec![Notice::NoInboundChannel]
        );
        assert_eq!(
            notices(&behind_a_frame(a_provider(ProviderState::Absent)), &[]),
            Vec::new()
        );
    }

    /// A session fact reaches the foot whether or not the collection behind
    /// the frame found anything to say — the two travel by the same road and
    /// neither depends on the other.
    #[test]
    fn a_session_notice_stands_alone_where_the_snapshot_is_well() {
        assert_eq!(
            notices(
                &behind_a_frame(a_provider(ProviderState::Answering)),
                &[Notice::NoInboundChannel]
            ),
            vec![Notice::NoInboundChannel]
        );
    }

    #[test]
    fn a_session_with_nothing_wrong_leaves_the_foot_to_the_keys() {
        assert_eq!(
            notices(&behind_a_frame(a_provider(ProviderState::Answering)), &[]),
            Vec::new()
        );
    }

    /// Keys can be rediscovered; a herdr that is silently absent cannot. So on
    /// a screen too narrow for both, the keys are what gives way.
    #[test]
    fn a_narrow_foot_gives_up_the_keys_before_the_missing_herdr() {
        let drawn = Painted::of(
            status_bar(&[Notice::AgentsUnknown], None, A_KEY_ROW, 60),
            60,
            1,
        )
        .rows();

        assert!(drawn[0].contains("no herdr session"), "{drawn:?}");
        assert_eq!(drawn[0].chars().count(), 60);
    }

    /// The bead this was written for. Measured at 60 columns, the foot drew
    /// `Ent…` after the notice: four columns saying a key row exists, which
    /// the reader could already see. Half a key name presses nothing, so a
    /// row that cannot be drawn whole is not drawn.
    #[test]
    fn a_foot_too_narrow_for_the_whole_key_row_draws_none_of_it() {
        let drawn = Painted::of(
            status_bar(&[Notice::AgentsUnknown], None, A_KEY_ROW, 60),
            60,
            1,
        )
        .rows();

        assert_eq!(
            drawn[0].trim_end(),
            "⚠ no herdr session · which agents are alive is unknown"
        );
    }

    /// On the narrowest screen the brief notice leaves a couple of dozen
    /// columns over, which was room for `Enter focus   a all  …` and nothing
    /// a reader could press.
    #[test]
    fn the_narrowest_screen_draws_no_part_of_the_key_row_beside_a_notice() {
        let drawn = Painted::of(
            status_bar(&[Notice::AgentsUnknown], None, A_KEY_ROW, 40),
            40,
            1,
        )
        .rows();

        assert_eq!(drawn[0].trim_end(), "⚠ agents unknown");
    }

    /// Whole where it fits, and the column under that is the whole difference.
    #[test]
    fn the_key_row_is_drawn_whole_at_the_first_width_that_holds_it() {
        let notice = "⚠ no herdr session · which agents are alive is unknown";
        let fits = notice.chars().count() + GAP + A_KEY_ROW.chars().count();
        let row = |width: usize| {
            Painted::of(
                status_bar(&[Notice::AgentsUnknown], None, A_KEY_ROW, width),
                width as u16,
                1,
            )
            .rows()
            .remove(0)
        };

        assert_eq!(row(fits), format!("{notice}  {A_KEY_ROW}"));
        assert_eq!(row(fits - 1).trim_end(), notice);
    }
}
