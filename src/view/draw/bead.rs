//! A bead's own line, and the run of closed siblings drawn in place of the
//! several beads it stands for.

use ratatui::style::Style;
use ratatui::text::Span;

use crate::model::badges::Badged;
use crate::model::types::Status;
use crate::view::fitted::{openable, Fitted, Link, GAP};
use crate::view::palette;
use crate::view::phrase;
use crate::view::row::{self, Row, AGENT, WARNING};

use super::tone::{status_style, tone};
use super::{beside, done, structure};

/// A run of closed siblings said as a count, carrying the glyph each of them
/// would carry on a line of its own.
///
/// `lines::split` builds a run out of closed beads and nothing else, so this
/// is not a summary over mixed states — it is the one state every member
/// holds. It goes through `status_glyph` and `status_style` exactly as a
/// bead's does, so a run cannot drift away from the beads it stands for.
pub(super) fn elided_run(prefix: &str, count: usize) -> Fitted {
    let status = Status::Closed;
    let glyph = row::status_glyph(&status);
    Fitted::new(
        vec![
            structure(prefix),
            Span::styled(glyph.to_string(), status_style(&status)),
            Span::raw(format!(" {}", phrase::elided(count))),
        ],
        Vec::new(),
        Vec::new(),
    )
    .toned(palette::TIER_FINISHED)
}

/// One bead's line, under the box-drawing run its ancestors leave.
///
/// `id_width` is the widest abbreviated id in the tree, so a column of ids
/// lines up under one another and the titles start together.
pub(super) fn bead_line(row: &Row, prefix: &str, id_width: usize) -> Fitted {
    let identity = vec![
        structure(prefix),
        Span::styled(row.glyph.to_string(), status_style(&row.status)),
        Span::styled(format!(" {:id_width$}", row.id), status_style(&row.status)),
    ];

    let mut title = vec![Span::raw(row.title.clone())];
    let mut links = Vec::new();
    for badge in &row.badges {
        title.push(Span::raw(" ".repeat(GAP)));
        if let Some(to) = &badge.link {
            links.push(Link {
                at: title.len(),
                to: to.clone(),
            });
        }
        title.push(Span::styled(badge.text.clone(), badge_style(badge)));
    }

    let mut fitted = Fitted::new(identity, title, state(row, row.agent.as_ref())).linking(links);
    if let Some(briefly) = &row.agent_briefly {
        fitted = fitted.briefly(state(row, Some(briefly)));
    }
    fitted.toned(tone(row))
}

/// The underline is the whole of what a reader can *see* about a link: its
/// destination is nowhere in the row's text at any width. What the terminal
/// acts on is the hyperlink `Fitted` writes round the badge.
///
/// So the underline is drawn on the same answer the emitter gives, rather
/// than on the config having named a link: one the emitter refuses would draw
/// a badge that invites a click it cannot honour.
fn badge_style(badge: &Badged) -> Style {
    match &badge.link {
        Some(to) if openable(&badge.text, to) => palette::LINK,
        _ => Style::new(),
    }
}

/// The row's right-hand block, with the agent said in whichever of its two
/// forms it was given. Everything else on the block is a cell this program
/// wrote and knows the length of.
fn state(row: &Row, agent: Option<&String>) -> Vec<Span<'static>> {
    let mut state: Vec<Span<'static>> = Vec::new();
    let mut say = |text: &str, style: Style| {
        beside(&mut state, Span::styled(text.to_string(), style));
    };
    if let Some(progress) = row.progress {
        say(&done(progress.closed, progress.total), Style::new());
    }
    if let Some(agent) = agent {
        say(agent, palette::AGENT);
    }
    if let Some(anomalies) = &row.anomalies {
        say(anomalies, palette::ATTENTION);
    }
    // After the row's own two, because those name one bead and these count
    // several: a number met before the name it belongs beside reads as the
    // total the name is an example of.
    if let Some(shut_over) = &row.shut_over {
        if shut_over.live_agents > 0 {
            say(
                &format!("{AGENT} {}", phrase::agents_beneath(shut_over.live_agents)),
                palette::AGENT,
            );
        }
        if shut_over.anomalies > 0 {
            say(
                &format!(
                    "{WARNING} {}",
                    phrase::anomalies_beneath(shut_over.anomalies)
                ),
                palette::ATTENTION,
            );
        }
    }
    for note in &row.notes {
        say(note, palette::ATTENTION);
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::{Color, Modifier};
    use ratatui::widgets::Widget;

    use crate::model::badges::Undrawn;
    use crate::view::fitted::hyperlink;

    use crate::model::anomaly::Anomaly;
    use crate::model::badges::Badged;
    use crate::model::join::AgentRef;
    use crate::model::snapshot::Node;
    use crate::view::draw::tone::status_style;
    use crate::view::draw::{fitted, tests::*};
    use crate::view::painted::Run;

    #[test]
    fn a_bead_line_says_its_glyph_its_id_and_its_title_in_that_order() {
        let node = node("smt-4kd3p.20", "wallpaper timer calls dms", Status::Blocked);

        assert_eq!(
            Painted::of(bead_line(&row(&node), BRANCH, 4), 46, 1).rows(),
            vec!["  ├── ● .20   wallpaper timer calls dms       "]
        );
    }

    /// An epic reads like the root above it: how far along, then who is on
    /// it, then what wants looking at. The count leads the state column
    /// because that is the order a header already puts them in.
    #[test]
    fn a_bead_standing_for_a_subtree_says_how_much_of_it_is_done_before_who_is_on_it() {
        let mut epic = row(&node(
            "smt-4kd3p.2",
            "the noctalia widget",
            Status::InProgress,
        ));
        epic.progress = Some(row::Progress {
            closed: 3,
            total: 8,
        });
        epic.agent = Some(row::agent_marker(&a_pane()));

        let drawn = Painted::of(bead_line(&epic, BRANCH, 3), 60, 1).rows();

        let count = drawn[0].find("3/8").expect("the count is drawn");
        let agent = drawn[0].find("wCM:p9").expect("the agent is drawn");
        assert!(count < agent, "{drawn:?}");
    }

    /// The gap goes *between* the cells. A row is read by where its columns
    /// fall, and two cells that abut read as one — `3/8◍ wCM:p9` names no
    /// fraction and no pane. In front of the first cell the same two columns
    /// say nothing at all, because the block is set against the row's right
    /// edge and the padding swallows them.
    ///
    /// The whole block is written out here rather than asked of `phrase` or
    /// `done`, and read off the row's end rather than searched for, so the
    /// only thing that satisfies it is those words in that order with those
    /// two columns between them. A cut row ends in `…` and fails it too.
    #[test]
    fn a_bead_lines_state_cells_are_held_apart_rather_than_run_together() {
        let mut epic = row(&node(
            "smt-4kd3p.2",
            "the noctalia widget",
            Status::InProgress,
        ));
        epic.progress = Some(row::Progress {
            closed: 3,
            total: 8,
        });
        epic.agent = Some(row::agent_marker(&a_pane()));

        let drawn = Painted::of(bead_line(&epic, BRANCH, 3), 60, 1).rows();

        assert!(drawn[0].ends_with("3/8  ◍ wCM:p9 · working"), "{drawn:?}");
    }

    /// A shut row is the only thing on the screen standing for the beads
    /// under it, so the agents on them are nowhere else to be read. The count
    /// follows the row's own agent: that one is a name and this one is a
    /// number, and a number met first reads as the total the name is one of.
    #[test]
    fn a_row_shut_over_working_agents_says_how_many_after_naming_its_own() {
        let mut shut = row(&node(
            "smt-4kd3p.2",
            "the noctalia widget",
            Status::InProgress,
        ));
        shut.agent = Some(row::agent_marker(&a_pane()));
        shut.shut_over = Some(counts(1, 5, 3, 0));

        let drawn = Painted::of(bead_line(&shut, BRANCH, 3), 110, 1).rows();

        let own = drawn[0].find("wCM:p9").expect("its own agent is drawn");
        let beneath = drawn[0]
            .find("3 agents beneath")
            .expect("what it is shut over is drawn");
        assert!(own < beneath, "{drawn:?}");
    }

    /// The beads a fold hides that want looking at, said as beads rather than
    /// as rules fired, because the number is how many rows opening it would
    /// put in front of the reader.
    #[test]
    fn a_row_shut_over_beads_wanting_looking_at_says_how_many() {
        let mut shut = row(&node(
            "smt-4kd3p.2",
            "the noctalia widget",
            Status::InProgress,
        ));
        shut.shut_over = Some(counts(1, 5, 0, 2));

        let drawn = Painted::of(bead_line(&shut, BRANCH, 3), 110, 1).rows();

        says(&drawn[0], "2 beads beneath");
    }

    /// A count of nought is left out rather than drawn, exactly as the
    /// project line leaves it out: a row of noughts reads as something to
    /// check, and every shut row in a quiet tree would carry two.
    #[test]
    fn a_row_shut_over_nothing_live_says_nothing_about_it() {
        let mut shut = row(&node(
            "smt-4kd3p.2",
            "the noctalia widget",
            Status::InProgress,
        ));
        shut.shut_over = Some(counts(4, 5, 0, 0));

        let drawn = Painted::of(bead_line(&shut, BRANCH, 3), 110, 1).rows();

        does_not_say(&drawn[0], "beneath");
    }

    /// Live work is drawn in the colour live work is drawn in everywhere
    /// else, and work wanting looking at in that one.
    #[test]
    fn what_a_shut_row_hides_is_painted_live_and_look_at_this() {
        let mut shut = row(&node(
            "smt-4kd3p.2",
            "the noctalia widget",
            Status::InProgress,
        ));
        shut.shut_over = Some(counts(1, 5, 3, 2));

        let painted = Painted::of(bead_line(&shut, BRANCH, 3), 120, 1).row(0);
        let colour_of = |words: &str| {
            painted
                .iter()
                .find(|run| run.said.contains(words))
                .and_then(|run| run.style.fg)
        };

        assert_eq!(
            colour_of("3 agents beneath"),
            palette::AGENT.fg,
            "{painted:?}"
        );
        assert_eq!(
            colour_of("2 beads beneath"),
            palette::ATTENTION.fg,
            "{painted:?}"
        );
    }

    /// Width the row has not got comes off the note before it comes off the
    /// seats. `Fitted` cuts the state block from its own end, so the order
    /// these are said in is an order of importance, and this is which way it
    /// runs.
    ///
    /// A cut and not a drop: `cut_to` keeps whole spans while they fit and
    /// takes a character prefix of the next, so the note is still there in
    /// part. Both halves are asserted, because a test that only said the
    /// whole note was absent would pass on a row that had dropped it —
    /// and would send the next reader looking for a mechanism this has not
    /// got.
    ///
    /// The note is the right one to cut because the fraction beside it says
    /// the same thing: a reader left with `21 unfinished beads beneath …` on
    /// a row still reading `1/22` can do the subtraction. Nothing else on the
    /// row says four people are inside this one, and no fold above it will
    /// say so either.
    #[test]
    fn a_row_too_narrow_for_both_keeps_the_seats_whole_and_cuts_the_note() {
        let mut shut = row(&node(
            "smt-4kd3p.2",
            "the noctalia widget",
            Status::InProgress,
        ));
        shut.progress = Some(row::Progress {
            closed: 1,
            total: 22,
        });
        shut.shut_over = Some(counts(1, 22, 4, 0));
        shut.notes = vec![phrase::unfinished_beneath(21)];

        let wide = Painted::of(bead_line(&shut, BRANCH, 3), 120, 1).rows();
        let narrow = Painted::of(bead_line(&shut, BRANCH, 3), 68, 1).rows();

        says(&wide[0], "◍ 4 agents beneath");
        says(&wide[0], "21 unfinished beads beneath this");

        says(&narrow[0], "◍ 4 agents beneath");
        does_not_say(&narrow[0], "21 unfinished beads beneath this");
        says(&narrow[0], "21 unfinished beads beneath ");
    }

    /// A leaf stands for itself alone. A fraction over one bead would say
    /// nothing its glyph has not already said, and would spend width a title
    /// needs.
    #[test]
    fn a_bead_standing_only_for_itself_draws_no_count() {
        let leaf = row(&node(
            "smt-4kd3p.20",
            "wallpaper timer calls dms",
            Status::Open,
        ));

        let drawn = Painted::of(bead_line(&leaf, BRANCH, 4), 60, 1).rows();

        assert!(!drawn[0].contains('/'), "{drawn:?}");
    }

    /// Ids are padded to the widest in the tree so the titles start together;
    /// a column that did not line up would be read as a tree shape it is not.
    #[test]
    fn ids_are_padded_so_the_titles_below_one_another_start_together() {
        let short = node("smt-4kd3p.1", "wire the niri theme include", Status::Open);
        let long = node("smt-4kd3p.20", "wallpaper timer calls dms", Status::Open);

        let short = Painted::of(bead_line(&row(&short), BRANCH, 4), 60, 1).rows();
        let long = Painted::of(bead_line(&row(&long), BRANCH, 4), 60, 1).rows();

        assert_eq!(
            short[0].find("wire the"),
            long[0].find("wallpaper timer"),
            "{short:?} {long:?}"
        );
    }

    #[test]
    fn a_bead_line_carries_its_agent_and_its_anomalies() {
        let mut staffed = node(
            "smt-4kd3p.20",
            "wallpaper timer calls dms",
            Status::InProgress,
        );
        staffed.agent = Some(a_pane());
        staffed.anomalies = vec![Anomaly::StaleClaim { days: 58 }];
        let drawn = Painted::of(bead_line(&row(&staffed), LAST, 4), 100, 1).rows();

        assert!(drawn[0].contains("◍ wCM:p9 · working"), "{drawn:?}");
        assert!(drawn[0].contains("58"), "{drawn:?}");
    }

    fn captioned(caption: &str) -> Node {
        let mut staffed = node(
            "smt-4kd3p.20",
            "wallpaper timer calls dms",
            Status::InProgress,
        );
        staffed.agent = Some(AgentRef {
            title: Some(caption.into()),
            ..a_pane()
        });
        staffed
    }

    /// A caption is the one unbounded string to reach this row, and the row
    /// is still exactly as wide as it was given whatever the pane called
    /// itself. It goes whole rather than in part: a caption cut mid-phrase
    /// says less than the pane id it makes way for, which at least names the
    /// seat a reader can go and look at.
    #[test]
    fn a_caption_too_long_for_the_row_costs_the_row_none_of_its_width() {
        let staffed = captioned("teach the elided run to fold back open on a keypress");

        let drawn = Painted::of(bead_line(&row(&staffed), LAST, 4), 50, 1).rows();

        assert_eq!(drawn[0].chars().count(), 50, "{drawn:?}");
        assert!(!drawn[0].contains("keypress"), "{drawn:?}");
        assert!(!drawn[0].contains("teach"), "{drawn:?}");
    }

    /// A caption is the pane's own words and can be any length; the pane's id
    /// is `bdi`'s and is short. So the caption is the cell that gives way: a
    /// row too narrow for both names its seat by the pane and spends the
    /// columns on the bead it is a row for.
    ///
    /// This is the row the reader is hunting for. A caption that crowds out
    /// its title makes the one row that matters the one row you cannot read.
    #[test]
    fn a_caption_gives_its_columns_back_to_the_beads_own_title() {
        let staffed = captioned("teach the elided run to fold back open on a keypress");

        let drawn = Painted::of(bead_line(&row(&staffed), LAST, 4), 80, 1).rows();

        assert!(drawn[0].contains("wallpaper timer calls dms"), "{drawn:?}");
        assert!(drawn[0].contains("wCM:p9 · working"), "{drawn:?}");
        assert!(!drawn[0].contains("elided run"), "{drawn:?}");
    }

    /// And it gives way only where it costs the title something. A row wide
    /// enough for both says what the pane says it is doing, which is the
    /// whole reason the caption is read off herdr at all.
    #[test]
    fn a_caption_the_title_does_not_need_the_room_for_is_said_in_full() {
        let staffed = captioned("teach the elided run to fold back open");

        let drawn = Painted::of(bead_line(&row(&staffed), LAST, 4), 120, 1).rows();

        assert!(drawn[0].contains("wallpaper timer calls dms"), "{drawn:?}");
        assert!(
            drawn[0].contains("teach the elided run to fold back open"),
            "{drawn:?}"
        );
    }

    /// A caption shorter than the pane it names costs the title nothing. The
    /// two forms are a long one and a short one only by convention — a terse
    /// pane makes the caption the shorter of them — so the room kept back is
    /// whichever is smaller rather than whichever is named `briefly`.
    ///
    /// 57 columns is the width where the two answers differ: room for the
    /// caption form and the whole title, and not for the pane form and the
    /// whole title.
    #[test]
    fn a_caption_shorter_than_the_pane_it_names_keeps_back_none_of_its_room() {
        let staffed = captioned("dish");

        let drawn = Painted::of(bead_line(&row(&staffed), LAST, 4), 57, 1).rows();

        assert!(drawn[0].contains("wallpaper timer calls dms"), "{drawn:?}");
        assert!(drawn[0].contains("◍ dish · working"), "{drawn:?}");
    }

    /// The bead is still named by its id, which is fitted before either of
    /// them and cannot be crowded out by anything.
    #[test]
    fn nothing_on_the_row_can_crowd_out_the_beads_own_id() {
        let staffed = captioned("teach the elided run to fold back open on a keypress");

        let drawn = Painted::of(bead_line(&row(&staffed), LAST, 4), 30, 1).rows();

        assert!(drawn[0].contains(".20"), "{drawn:?}");
    }

    /// Narrow enough and the cell has no room at all. It goes whole rather
    /// than leaving a marker standing for a caption that is not there.
    #[test]
    fn a_caption_with_no_room_left_takes_the_whole_agent_cell_with_it() {
        let staffed = captioned("teach the elided run to fold back open on a keypress");

        let drawn = Painted::of(bead_line(&row(&staffed), LAST, 4), 14, 1).rows();

        assert!(!drawn[0].contains(row::AGENT), "{drawn:?}");
        assert!(drawn[0].contains(".20"), "{drawn:?}");
    }

    #[test]
    fn a_bead_lines_badges_are_drawn_in_the_order_they_were_configured() {
        let mut badged = node("smt-4kd3p.20", "a bead", Status::Blocked);
        badged.badges = vec![
            Badged {
                key: "delivery_pr".into(),
                text: "⇢ #12".into(),
                link: None,
            },
            Badged {
                key: "blocked_on".into(),
                text: "⏸ waiting".into(),
                link: None,
            },
        ];
        let drawn = Painted::of(bead_line(&row(&badged), BRANCH, 4), 100, 1).rows();
        let first = drawn[0].find("⇢ #12").expect("the first badge");
        let second = drawn[0].find("⏸ waiting").expect("the second badge");

        assert!(first < second, "{drawn:?}");
    }

    /// A badge with somewhere to go is underlined, and the underline is the
    /// only thing about it that changes: the badge beside it, whose config
    /// named no `link`, is drawn exactly as it was before links existed.
    #[test]
    fn a_badge_with_a_link_is_drawn_underlined_and_one_without_is_not() {
        let mut badged = node("smt-4kd3p.20", "a bead", Status::Blocked);
        badged.badges = vec![
            Badged {
                key: "delivery_pr".into(),
                text: "⇢ #12".into(),
                link: Some("https://forge.invalid/orbital/atlas/pull/12".into()),
            },
            Badged {
                key: "blocked_on".into(),
                text: "⏸ waiting".into(),
                link: None,
            },
        ];

        let painted = Painted::of(bead_line(&row(&badged), BRANCH, 4), 100, 1);
        let linked = run_saying(&painted, "⇢ #12");
        let plain = run_saying(&painted, "⏸ waiting");

        assert!(
            linked.style.add_modifier.contains(Modifier::UNDERLINED),
            "the badge with a link is not underlined: {linked:?}"
        );
        assert!(
            !plain.style.add_modifier.contains(Modifier::UNDERLINED),
            "the badge with no link is underlined: {plain:?}"
        );
    }

    /// The whole point of the rule is that the reader sees something, so the
    /// words have to survive the trip to the buffer rather than stopping at
    /// the row.
    #[test]
    fn a_badge_that_fell_short_of_its_config_says_so_on_the_drawn_row() {
        let mut short = row(&node("smt-4kd3p.20", "a bead", Status::Blocked));
        short.notes = vec![
            phrase::undrawn(&Undrawn::Badge {
                key: "delivery_pr".into(),
            }),
            phrase::unopenable_link("jira"),
        ];

        let drawn = Painted::of(bead_line(&short, BRANCH, 3), 160, 1).rows();

        says(&drawn[0], "delivery_pr");
        says(&drawn[0], "jira");
    }

    /// The underline is a promise the reader can act on, so it is drawn on
    /// the emitter's answer rather than on the config having named a link. A
    /// destination that cannot be written would otherwise leave a badge
    /// inviting a click nothing is there to honour.
    #[test]
    fn a_badge_whose_link_cannot_be_written_is_not_underlined() {
        let mut badged = node("smt-4kd3p.20", "a bead", Status::Blocked);
        badged.badges = vec![Badged {
            key: "delivery_pr".into(),
            text: "⇢ #12".into(),
            link: Some("https://forge.invalid/orbital\u{1b}]0;owned\u{7}/pull/12".into()),
        }];

        let painted = Painted::of(bead_line(&row(&badged), BRANCH, 4), 100, 1);
        let refused = run_saying(&painted, "⇢ #12");

        assert!(
            !refused.style.add_modifier.contains(Modifier::UNDERLINED),
            "a link the emitter refuses is still drawn as one: {refused:?}"
        );
    }

    /// The URL costs the row nothing. `Fitted` cuts a line by the visible
    /// width of what its spans say, so a URL held in a span's text would be
    /// counted in the columns the row has to spend.
    ///
    /// Read at the width the row's own words exactly fill, which is the
    /// width that tells the two apart: a URL counted there takes the row
    /// past its width and the badge is cut to nothing. Wider, nothing is cut
    /// either way; narrower, the words alone are cut and the same cut hides
    /// the URL behind it — a reading at either would pass whether the URL
    /// were measured or not.
    #[test]
    fn a_badges_link_is_not_among_the_columns_the_row_is_cut_to() {
        let unlinked = Badged {
            key: "delivery_pr".into(),
            text: "⇢ #12".into(),
            link: None,
        };
        let linked = Badged {
            link: Some("https://forge.invalid/orbital/atlas/pull/12".into()),
            ..unlinked.clone()
        };

        let said = |badge: Badged| {
            let mut badged = node("smt-4kd3p.20", "a bead", Status::Blocked);
            badged.badges = vec![badge];
            Painted::of(bead_line(&row(&badged), BRANCH, 4), EXACTLY_THE_ROW, 1).rows()
        };

        let drawn = said(linked);

        assert!(
            drawn[0].ends_with("a bead  ⇢ #12"),
            "the badge did not survive the row's own width: {drawn:?}"
        );
        assert_eq!(drawn, said(unlinked));
    }

    /// The badge that names a URL is the one the terminal is told about, and
    /// it is told round the badge's own words — so the reader clicks the badge
    /// rather than retyping what it stands for.
    #[test]
    fn the_badge_that_names_a_url_is_the_one_emitted_as_a_hyperlink() {
        let somewhere = "https://forge.invalid/orbital/atlas/pull/12";
        let badge = |link: Option<&str>| Badged {
            key: "delivery_pr".into(),
            text: "⇢ #12".into(),
            link: link.map(str::to_string),
        };
        let said = |badge: Badged| {
            let mut badged = node("smt-4kd3p.20", "a bead", Status::Blocked);
            badged.badges = vec![badge];
            symbols(bead_line(&row(&badged), BRANCH, 4), 100)
        };

        assert!(
            said(badge(Some(somewhere))).contains(
                &hyperlink("⇢ #12", somewhere).expect("this vocabulary holds no control character")
            ),
            "the badge naming a URL was not made a link"
        );
        assert!(
            !said(badge(None)).contains('\u{1b}'),
            "a badge naming no URL was made a link"
        );
    }

    /// Every symbol a row put in the buffer, escape bytes and all. `Painted`
    /// reports what a reader sees, and a hyperlink is not that.
    fn symbols(row: Fitted, width: u16) -> String {
        let area = Rect::new(0, 0, width, 1);
        let mut buf = Buffer::empty(area);
        row.render(area, &mut buf);
        (0..width).map(|x| buf[(x, 0)].symbol()).collect()
    }

    /// The columns `  ├── ● .20   a bead  ⇢ #12` fills, and not one more.
    const EXACTLY_THE_ROW: u16 = 27;

    /// The one run of `painted`'s first line that says `words`.
    fn run_saying(painted: &Painted, words: &str) -> Run {
        let said: Vec<Run> = painted
            .row(0)
            .into_iter()
            .filter(|run| run.said.contains(words))
            .collect();
        match said.as_slice() {
            [only] => only.clone(),
            _ => panic!("{words:?} is said by {} runs, not one", said.len()),
        }
    }

    #[test]
    fn a_bead_line_too_long_for_the_width_is_cut_rather_than_wrapped() {
        let long = node("smt-4kd3p.20", &"wallpaper ".repeat(20), Status::Open);
        let drawn = Painted::of(bead_line(&row(&long), BRANCH, 4), 40, 3).rows();

        assert_eq!(drawn[0], "  ├── ○ .20   wallpaper wallpaper wallp…");
        assert_eq!(drawn[1].trim(), "");
        assert_eq!(drawn[2].trim(), "");
    }

    /// A run stands for closed beads and nothing else — `split` selects on
    /// exactly that — so its glyph is not a summary over mixed states but the
    /// one state every member holds. Resolved through `status_glyph` and
    /// `status_style`, the same two the beads themselves go through, so a run
    /// and the beads it stands for cannot drift apart.
    #[test]
    fn an_elided_run_carries_the_closed_glyph_each_bead_it_stands_for_would() {
        let painted = Painted::of(fitted(&under(BRANCH, elided(15)), 0, &at_rest()), 72, 1).row(0);

        assert_eq!(
            painted[1].said,
            row::status_glyph(&Status::Closed).to_string()
        );
        assert_eq!(painted[1].style.fg, status_style(&Status::Closed).fg);
    }

    /// A reader follows the vertical rules down a tree. A sentence that took
    /// its box-drawing into its own colour would break that run wherever it
    /// fell, so the drawing stays in the terminal's own foreground and only
    /// the words beside it are coloured.
    #[test]
    fn an_elided_run_leaves_its_box_drawing_in_the_terminals_own_colour() {
        let painted = Painted::of(fitted(&under(BRANCH, elided(3)), 0, &at_rest()), 72, 1).row(0);

        assert_eq!(painted[0].said, BRANCH);
        assert_eq!(painted[0].style.fg, Some(Color::Reset));
        assert_eq!(painted[2].style.fg, palette::TIER_FINISHED.fg);
    }

    /// A bead's status is the one thing about it `bd` draws in colour, and it
    /// reaches the screen on a glyph one column wide. The id takes the same
    /// colour so that column is as wide as an id, and it says nothing the
    /// glyph beside it does not already say.
    #[test]
    fn a_beads_id_is_drawn_in_the_colour_of_its_own_status_glyph() {
        for status in [Status::Blocked, Status::InProgress, Status::Closed] {
            let drawn = bead_line(
                &row(&node("smt-4kd3p.2", "a bead", status.clone())),
                BRANCH,
                3,
            );
            let painted = Painted::of(drawn, 60, 1).row(0);

            let id = painted
                .iter()
                .find(|run| run.said.contains(".2"))
                .expect("the id is drawn");
            assert_eq!(id.style.fg, status_style(&status).fg, "{painted:?}");
            assert!(
                id.said.starts_with(row::status_glyph(&status)),
                "the glyph is in the same run, so it is in the same colour: {painted:?}"
            );
        }
    }

    /// Open is the one status `bd` gives no colour of its own, so the id has
    /// none either and the row's own tone reaches it as it does the rest.
    #[test]
    fn an_open_beads_id_is_left_in_the_colour_the_rest_of_its_row_is_in() {
        let node = node("smt-4kd3p.2", "a bead", Status::Open);

        let painted = Painted::of(bead_line(&row(&node), BRANCH, 3), 60, 1).row(0);

        let id = painted
            .iter()
            .find(|run| run.said.contains(".2"))
            .expect("the id is drawn");
        assert_eq!(id.style.fg, Some(Color::Reset), "{painted:?}");
    }
}
